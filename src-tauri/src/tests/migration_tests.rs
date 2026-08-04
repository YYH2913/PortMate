use super::*;

#[test]
fn vault_command_types_keep_stable_json_contracts() {
    let request: ProfileSecretMigrationRequest = serde_json::from_value(serde_json::json!({
        "targetStorage": "portable",
        "profileIds": ["ssh-1"]
    }))
    .unwrap();
    assert_eq!(request.target_storage, SecretStorage::Portable);
    assert_eq!(request.profile_ids, vec!["ssh-1".to_string()]);
    assert!(request.cleanup_source);

    let rotate = serde_json::to_value(PortableVaultRotatePasswordRequest {
        current_password: "current".to_string(),
        new_password: "replacement".to_string(),
    })
    .unwrap();
    assert_eq!(rotate["currentPassword"], "current");
    assert_eq!(rotate["newPassword"], "replacement");

    assert_eq!(
        serde_json::to_value(ProfileSecretMigrationJournalState::SourceCleanupPending).unwrap(),
        "source-cleanup-pending"
    );
    assert_eq!(
        serde_json::to_value(ProfileSecretMigrationRecoveryDisposition::Committed).unwrap(),
        "committed"
    );
}

#[test]
fn tcp_proxy_credentials_participate_in_migration_and_legacy_journals() {
    let mut profile = test_tcp_profile(ConnectionConfig::Tcp(TcpConnection {
        proxy: ProxyConfig {
            enabled: true,
            username: "proxy-user".to_string(),
            password_secret_ref: Some("keychain:proxy-source".to_string()),
            ..ProxyConfig::default()
        },
        ..TcpConnection::default()
    }));
    profile.id = "tcp-proxy-session".to_string();
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec!["tcp-proxy-session".to_string()],
        cleanup_source: true,
    };
    let response = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |secret_ref| {
            assert_eq!(secret_ref, "keychain:proxy-source");
            Ok("private-proxy-password".to_string())
        },
        |storage, prepared| {
            assert_eq!(storage, SecretStorage::Portable);
            assert_eq!(prepared.len(), 1);
            Ok(false)
        },
        |storage, secret_refs| {
            assert_eq!(storage, SecretStorage::Native);
            SecretBatchDeleteOutcome {
                results: secret_refs
                    .iter()
                    .map(|secret_ref| (secret_ref.clone(), Ok(())))
                    .collect(),
                portable_vault_requires_reunlock: false,
            }
        },
        |_, affected, targets| {
            assert_eq!(affected, ["tcp-proxy-session"]);
            assert_eq!(targets.len(), 1);
            ProfileSecretStoreCommit::Committed { warning: None }
        },
    )
    .unwrap();
    assert_eq!(response.migrated_reference_count, 1);
    let projection = profile_secret_migration_projection(&store.profiles[0]).unwrap();
    assert!(projection
        .proxy_password_secret_ref
        .as_deref()
        .is_some_and(|secret_ref| secret_ref.starts_with("stronghold:")));
    assert!(projection.password_secret_ref.is_none());
    assert!(projection.passphrase_secret_ref.is_none());

    let mut legacy = serde_json::to_value(&projection).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("proxyPasswordSecretRef");
    let legacy: ProfileSecretMigrationJournalProjection = serde_json::from_value(legacy).unwrap();
    assert!(legacy.proxy_password_secret_ref.is_none());
}

#[test]
fn migration_does_not_commit_profiles_when_portable_target_commit_is_unknown() {
    let fixture = test_migration_journal_fixture();
    let mut store = fixture.before.clone();
    let error = migrate_profile_secrets_with_journal_io(
        &mut store,
        &ProfileSecretMigrationRequest {
            target_storage: SecretStorage::Portable,
            profile_ids: vec!["ssh-session-1".to_string()],
            cleanup_source: true,
        },
        |secret_ref| {
            fixture
                .values
                .get(secret_ref)
                .cloned()
                .ok_or_else(|| "missing".to_string())
        },
        |storage, _| {
            assert_eq!(storage, SecretStorage::Portable);
            Ok(true)
        },
        |_, _| panic!("unknown portable commit must preserve both providers"),
        |_, _, _, _| panic!("unknown portable commit must not switch Profile refs"),
        |_| Ok(()),
    )
    .unwrap_err();
    assert!(error.contains(PROFILE_SECRET_MIGRATION_RESTART_REQUIRED));
    assert!(error.contains("版本指纹无法确认"));
    assert_eq!(
        profile_secret_migration_projection(&store.profiles[0]).unwrap(),
        fixture.journal.payload.profiles[0].before
    );
}

#[test]
fn migration_preserves_sources_when_cleanup_checkpoint_cannot_be_verified() {
    let fixture = test_migration_journal_fixture();
    let mut store = fixture.before.clone();
    let delete_called = std::cell::Cell::new(false);
    let response = migrate_profile_secrets_with_journal_io(
        &mut store,
        &ProfileSecretMigrationRequest {
            target_storage: SecretStorage::Portable,
            profile_ids: vec!["ssh-session-1".to_string()],
            cleanup_source: true,
        },
        |secret_ref| {
            fixture
                .values
                .get(secret_ref)
                .cloned()
                .ok_or_else(|| "missing".to_string())
        },
        |_, _| Ok(false),
        |_, _| {
            delete_called.set(true);
            SecretBatchDeleteOutcome {
                results: BTreeMap::new(),
                portable_vault_requires_reunlock: false,
            }
        },
        |_, _, _, _| ProfileSecretStoreCommit::Committed { warning: None },
        |event| match event {
            ProfileSecretMigrationJournalEvent::Transition {
                state: ProfileSecretMigrationJournalState::SourceCleanupPending,
                ..
            } => Err("injected checkpoint read-back failure".to_string()),
            _ => Ok(()),
        },
    )
    .unwrap();
    assert!(!delete_called.get());
    assert!(response.recovery_pending);
    assert!(response
        .warnings
        .iter()
        .any(|warning| warning.contains("checkpoint")));
    assert!(response
        .items
        .iter()
        .all(|item| item.cleanup_status == ProfileSecretCleanupStatus::Failed));
    let projection = profile_secret_migration_projection(&store.profiles[0]).unwrap();
    assert_ne!(projection, fixture.journal.payload.profiles[0].before);
    assert!(projection
        .password_secret_ref
        .as_deref()
        .is_some_and(|secret_ref| secret_ref.starts_with("stronghold:")));
    assert!(projection
        .passphrase_secret_ref
        .as_deref()
        .is_some_and(|secret_ref| secret_ref.starts_with("stronghold:")));
}

#[test]
fn profile_secret_migration_preserves_sharing_scope_and_reserved_tokens() {
    let mut first = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut first.connection {
        ssh.password_secret_ref = Some(" keychain:shared ".to_string());
        ssh.passphrase_secret_ref = Some("keychain:target-passphrase".to_string());
        ssh.identity_refs = vec![
            vault_identity("shared-key", "keychain:shared"),
            vault_identity("portable-key", "stronghold:already-portable"),
        ];
        ssh.jumps = vec![
            portmate_core::JumpHop {
                host: "bastion.example".to_string(),
                port: 22,
                username: "root".to_string(),
                password_secret_ref: Some("keychain:jump-password".to_string()),
                passphrase_secret_ref: Some("keychain:jump-passphrase".to_string()),
                identity_ref: None,
                host_key_policy: None,
            },
            portmate_core::JumpHop {
                host: "reserved.example".to_string(),
                port: 22,
                username: "root".to_string(),
                password_secret_ref: Some(MCP_HTTP_TOKEN_REF.to_string()),
                passphrase_secret_ref: None,
                identity_ref: None,
                host_key_policy: None,
            },
        ];
    }
    let mut unselected = test_ssh_profile();
    unselected.id = "ssh-session-2".to_string();
    unselected.name = "Unselected SSH".to_string();
    if let ConnectionConfig::Ssh(ssh) = &mut unselected.connection {
        ssh.password_secret_ref = Some("shared".to_string());
    }
    let mut tmux = test_ssh_profile();
    tmux.id = "tmux-session-1".to_string();
    tmux.name = "Selected Tmux".to_string();
    tmux.kind = SessionKind::Tmux;
    let mut tmux_ssh = match tmux.connection {
        ConnectionConfig::Ssh(ssh) => ssh,
        _ => panic!("expected SSH profile"),
    };
    tmux_ssh.password_secret_ref = Some("keychain:tmux-password".to_string());
    tmux.connection = ConnectionConfig::Tmux(tmux_ssh);

    let mut store = SessionStore::default();
    store.upsert_profile(first);
    store.upsert_profile(unselected.clone());
    store.upsert_profile(tmux);
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec!["ssh-session-1".to_string(), "tmux-session-1".to_string()],
        cleanup_source: true,
    };
    let plan = build_profile_secret_migration_plan(&store, &request).unwrap();
    assert_eq!(plan.preview.selected_profile_count, 2);
    assert_eq!(plan.preview.affected_profile_count, 2);
    assert_eq!(plan.preview.eligible_reference_count, 6);
    assert_eq!(plan.preview.eligible_secret_count, 5);
    assert_eq!(plan.preview.retained_shared_secret_count, 1);
    assert_eq!(plan.preview.already_target_reference_count, 1);
    assert_eq!(plan.preview.excluded_reserved_reference_count, 1);
    let plan_token = profile_secret_migration_plan_token(&plan, &request);
    assert_eq!(plan_token.len(), 64);
    let mut changed_store = store.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut changed_store.profiles[0].connection {
        ssh.password_secret_ref = Some("keychain:changed".to_string());
    }
    let changed_plan = build_profile_secret_migration_plan(&changed_store, &request).unwrap();
    assert_ne!(
        profile_secret_migration_plan_token(&changed_plan, &request),
        plan_token
    );
    let mut unrelated_store = store.clone();
    unrelated_store.profiles[0].name = "renamed only".to_string();
    let unrelated_plan = build_profile_secret_migration_plan(&unrelated_store, &request).unwrap();
    assert_eq!(
        profile_secret_migration_plan_token(&unrelated_plan, &request),
        plan_token
    );

    let source_values = HashMap::from([
        ("keychain:shared".to_string(), "secret-shared".to_string()),
        (
            "keychain:target-passphrase".to_string(),
            "secret-target-passphrase".to_string(),
        ),
        (
            "keychain:jump-password".to_string(),
            "secret-jump-password".to_string(),
        ),
        (
            "keychain:jump-passphrase".to_string(),
            "secret-jump-passphrase".to_string(),
        ),
        (
            "keychain:tmux-password".to_string(),
            "secret-tmux-password".to_string(),
        ),
    ]);
    let written = std::rc::Rc::new(std::cell::RefCell::new(HashMap::new()));
    let deleted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let persisted = std::rc::Rc::new(std::cell::RefCell::new(None));
    let source_values_for_write = source_values.clone();
    let response = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |secret_ref| {
            source_values
                .get(secret_ref)
                .cloned()
                .ok_or_else(|| format!("missing test secret: {secret_ref}"))
        },
        {
            let written = std::rc::Rc::clone(&written);
            move |storage, entries| {
                assert_eq!(storage, SecretStorage::Portable);
                let mut written = written.borrow_mut();
                for entry in entries {
                    assert_eq!(
                        entry.secret.as_str(),
                        source_values_for_write[&entry.source_ref]
                    );
                    written.insert(entry.target_ref.clone(), entry.secret.to_string());
                }
                Ok(false)
            }
        },
        {
            let deleted = std::rc::Rc::clone(&deleted);
            move |storage, secret_refs| {
                assert_eq!(storage, SecretStorage::Native);
                deleted.borrow_mut().extend(secret_refs.iter().cloned());
                SecretBatchDeleteOutcome {
                    results: secret_refs
                        .iter()
                        .map(|secret_ref| (secret_ref.clone(), Ok(())))
                        .collect(),
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        {
            let persisted = std::rc::Rc::clone(&persisted);
            move |next_store, _, _| {
                *persisted.borrow_mut() = Some(next_store.clone());
                ProfileSecretStoreCommit::Committed { warning: None }
            }
        },
    )
    .unwrap();

    assert_eq!(response.selected_profile_count, 2);
    assert_eq!(response.migrated_profile_count, 2);
    assert_eq!(response.migrated_reference_count, 6);
    assert_eq!(response.migrated_secret_count, 5);
    assert_eq!(response.summaries.len(), 2);
    assert_eq!(written.borrow().len(), 5);
    assert_eq!(deleted.borrow().len(), 4);
    assert!(!deleted.borrow().contains(&"keychain:shared".to_string()));
    assert!(persisted.borrow().is_some());

    let migrated = store.profile("ssh-session-1").unwrap();
    let migrated_ssh = ssh_connection(&migrated).unwrap();
    let shared_target = migrated_ssh.password_secret_ref.as_deref().unwrap();
    assert!(shared_target.starts_with("stronghold:"));
    assert_eq!(
        migrated_ssh.identity_refs[0].secret_ref.as_deref(),
        Some(shared_target)
    );
    assert_eq!(
        migrated_ssh.identity_refs[1].secret_ref.as_deref(),
        Some("stronghold:already-portable")
    );
    assert_eq!(
        migrated_ssh.jumps[1].password_secret_ref.as_deref(),
        Some(MCP_HTTP_TOKEN_REF)
    );
    assert_eq!(store.profile("ssh-session-2").unwrap(), unselected);
    let shared_item = response
        .items
        .iter()
        .find(|item| item.source_ref == "keychain:shared")
        .unwrap();
    assert_eq!(shared_item.reference_count, 2);
    assert_eq!(shared_item.remaining_source_references, 1);
    assert_eq!(
        shared_item.cleanup_status,
        ProfileSecretCleanupStatus::RetainedShared
    );
    let encoded = serde_json::to_string(&response).unwrap();
    for secret in source_values.values() {
        assert!(!encoded.contains(secret));
    }
}

#[test]
fn profile_secret_migration_rolls_back_targets_when_store_did_not_commit() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.password_secret_ref = Some("keychain:old".to_string());
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile.clone());
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec![profile.id.clone()],
        cleanup_source: true,
    };
    let deleted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let error = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |_| Ok("private value".to_string()),
        |_, _| Ok(false),
        {
            let deleted = std::rc::Rc::clone(&deleted);
            move |storage, refs| {
                assert_eq!(storage, SecretStorage::Portable);
                deleted.borrow_mut().extend(refs.iter().cloned());
                SecretBatchDeleteOutcome {
                    results: refs
                        .iter()
                        .map(|secret_ref| (secret_ref.clone(), Ok(())))
                        .collect(),
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        |_, _, _| ProfileSecretStoreCommit::NotCommitted("disk full".to_string()),
    )
    .unwrap_err();
    assert!(error.contains("disk full"));
    assert_eq!(deleted.borrow().len(), 1);
    assert!(deleted.borrow()[0].starts_with("stronghold:"));
    assert_eq!(store.profile(&profile.id).unwrap(), profile);
    assert!(!deleted.borrow().contains(&"keychain:old".to_string()));
}

#[test]
fn profile_secret_migration_keeps_both_sides_when_store_commit_is_unknown() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.password_secret_ref = Some("keychain:old".to_string());
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile.clone());
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec![profile.id.clone()],
        cleanup_source: true,
    };
    let delete_called = std::cell::Cell::new(false);
    let error = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |_| Ok("private value".to_string()),
        |_, _| Ok(false),
        |_, _| {
            delete_called.set(true);
            SecretBatchDeleteOutcome {
                results: BTreeMap::new(),
                portable_vault_requires_reunlock: false,
            }
        },
        |_, _, _| ProfileSecretStoreCommit::Unknown("commit state unavailable".to_string()),
    )
    .unwrap_err();
    assert!(error.contains("无法确认"));
    assert!(!delete_called.get());
    assert_eq!(store.profile(&profile.id).unwrap(), profile);
}

#[test]
fn profile_secret_migration_preflights_all_reads_before_writing() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.password_secret_ref = Some("keychain:a".to_string());
        ssh.passphrase_secret_ref = Some("keychain:b".to_string());
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile.clone());
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec![profile.id.clone()],
        cleanup_source: true,
    };
    let write_called = std::cell::Cell::new(false);
    let error = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |secret_ref| {
            if secret_ref == "keychain:b" {
                Err("unavailable".to_string())
            } else {
                Ok("first value".to_string())
            }
        },
        |_, _| {
            write_called.set(true);
            Ok(false)
        },
        |_, _| SecretBatchDeleteOutcome {
            results: BTreeMap::new(),
            portable_vault_requires_reunlock: false,
        },
        |_, _, _| ProfileSecretStoreCommit::Committed { warning: None },
    )
    .unwrap_err();
    assert!(error.contains("尚未写入任何目标"));
    assert!(!write_called.get());
    assert_eq!(store.profile(&profile.id).unwrap(), profile);
}

#[test]
fn profile_secret_migration_reports_post_commit_cleanup_failure() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.password_secret_ref = Some("stronghold:old".to_string());
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Native,
        profile_ids: vec!["ssh-session-1".to_string()],
        cleanup_source: true,
    };
    let response = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |_| Ok("private value".to_string()),
        |storage, _| {
            assert_eq!(storage, SecretStorage::Native);
            Ok(false)
        },
        |storage, refs| {
            assert_eq!(storage, SecretStorage::Portable);
            SecretBatchDeleteOutcome {
                results: refs
                    .iter()
                    .map(|secret_ref| (secret_ref.clone(), Err("snapshot read-only".to_string())))
                    .collect(),
                portable_vault_requires_reunlock: false,
            }
        },
        |_, _, _| ProfileSecretStoreCommit::Committed { warning: None },
    )
    .unwrap();
    assert_eq!(response.migrated_secret_count, 1);
    assert_eq!(
        response.items[0].cleanup_status,
        ProfileSecretCleanupStatus::Failed
    );
    assert!(response.warnings[0].contains("snapshot read-only"));
    assert!(ssh_connection(&store.profile("ssh-session-1").unwrap())
        .unwrap()
        .password_secret_ref
        .as_deref()
        .is_some_and(|secret_ref| secret_ref.starts_with("keychain:")));
}

#[test]
fn profile_secret_migration_is_idempotent_and_requires_supported_explicit_scope() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.password_secret_ref = Some("stronghold:already-portable".to_string());
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile.clone());
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec![profile.id.clone(), profile.id.clone()],
        cleanup_source: true,
    };
    let response = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |_| panic!("idempotent migration must not read secrets"),
        |_, _| panic!("idempotent migration must not write secrets"),
        |_, _| panic!("idempotent migration must not delete secrets"),
        |_, _, _| panic!("idempotent migration must not save the store"),
    )
    .unwrap();
    assert_eq!(response.selected_profile_count, 1);
    assert_eq!(response.migrated_secret_count, 0);
    assert_eq!(store.profile(&profile.id).unwrap(), profile);

    let empty_scope = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: Vec::new(),
        cleanup_source: true,
    };
    assert!(build_profile_secret_migration_plan(&store, &empty_scope)
        .unwrap_err()
        .contains("显式选择"));
    let shell = test_shell_profile();
    let shell_id = shell.id.clone();
    store.upsert_profile(shell);
    let unsupported_scope = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec![shell_id],
        cleanup_source: true,
    };
    assert!(
        build_profile_secret_migration_plan(&store, &unsupported_scope)
            .unwrap_err()
            .contains("不支持 Profile 凭据迁移")
    );
    assert!(is_reserved_internal_secret_ref(MCP_HTTP_TOKEN_REF));
    assert!(is_reserved_internal_secret_ref("mcp-http-token"));
    assert!(is_reserved_internal_secret_ref("keychain:ipc-test-token"));
    assert!(is_reserved_internal_secret_ref("ipc-test-token"));
    assert!(is_reserved_internal_secret_ref(BUNDLE_SIGNING_KEY_REF));
    assert!(is_reserved_internal_secret_ref(
        BUNDLE_SIGNING_KEY_PORTABLE_REF
    ));
}

#[test]
fn migration_retains_source_secrets_for_an_in_flight_connection() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.password_secret_ref = Some("keychain:old".to_string());
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    store.runtimes[0].status = SessionStatus::Connecting;
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec!["ssh-session-1".to_string()],
        cleanup_source: true,
    };
    let response = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |_| Ok("private value".to_string()),
        |_, _| Ok(false),
        |_, refs| {
            assert!(refs.is_empty(), "in-flight source must not be deleted");
            SecretBatchDeleteOutcome {
                results: BTreeMap::new(),
                portable_vault_requires_reunlock: false,
            }
        },
        |_, _, _| ProfileSecretStoreCommit::Committed { warning: None },
    )
    .unwrap();
    assert_eq!(
        response.items[0].cleanup_status,
        ProfileSecretCleanupStatus::RetainedInUse
    );
    assert_eq!(response.items[0].remaining_source_references, 0);
}

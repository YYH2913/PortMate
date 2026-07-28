use super::*;

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
fn migration_projection_classifies_old_new_and_slot_conflicts() {
    let fixture = test_migration_journal_fixture();
    assert_eq!(
        profile_secret_migration_disposition(&fixture.before, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::NotCommitted
    );

    let mut committed = fixture.journal.clone();
    committed.state = ProfileSecretMigrationJournalState::ProfilesCommitted;
    assert_eq!(
        profile_secret_migration_disposition(&fixture.after, &committed),
        ProfileSecretMigrationRecoveryDisposition::Committed
    );

    let mut partial = fixture.before.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut partial.profiles[0].connection {
        ssh.password_secret_ref = fixture
            .journal
            .payload
            .items
            .first()
            .map(|item| item.target_ref.clone());
    }
    assert_eq!(
        profile_secret_migration_disposition(&partial, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::Conflict
    );

    let mut swapped = fixture.before.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut swapped.profiles[0].connection {
        std::mem::swap(&mut ssh.password_secret_ref, &mut ssh.passphrase_secret_ref);
    }
    assert_eq!(
        profile_secret_migration_disposition(&swapped, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::Conflict,
        "equal ref counts must not hide credential-slot swaps"
    );

    let mut missing = fixture.before.clone();
    missing.profiles.clear();
    assert_eq!(
        profile_secret_migration_disposition(&missing, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::Conflict
    );
    assert_eq!(
        profile_secret_migration_disposition(&fixture.after, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::Conflict,
        "a NEW projection cannot be paired with a pre-commit journal state"
    );
}

#[test]
fn migration_diagnostic_reports_slots_and_provider_evidence_without_secret_material() {
    let fixture = test_migration_journal_fixture();
    let mut mixed = fixture.before.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut mixed.profiles[0].connection {
        ssh.password_secret_ref = Some(fixture.journal.payload.items[0].target_ref.clone());
    }
    let payload_bytes = serde_json::to_vec(&fixture.journal.payload).unwrap().len() as u64;
    let metadata = ActiveProfileSecretMigrationJournalMetadata {
        row_id: fixture.journal.payload.migration_id.clone(),
        state: fixture.journal.state.as_str().to_string(),
        payload_bytes,
        created_at: fixture.journal.created_at.to_rfc3339(),
        updated_at: fixture.journal.updated_at.to_rfc3339(),
    };
    let report = build_profile_secret_migration_diagnostic_report(
        &mixed,
        &fixture.journal,
        &metadata,
        |secret_ref| match fixture.values.get(secret_ref) {
            Some(value) => SecretProbeResult::Present(Zeroizing::new(value.clone())),
            None => SecretProbeResult::Missing,
        },
        ProfileSecretMigrationDiagnosticPortableVault {
            exists: Some(true),
            unlocked: Some(true),
            recovery_ready: Some(true),
            error: None,
        },
    );
    let json = serde_json::to_value(&report).unwrap();
    let encoded = serde_json::to_string(&json).unwrap();
    assert!(!encoded.contains("private-a"));
    assert!(!encoded.contains("private-b"));
    assert_eq!(json["containsSecretMaterial"], false);
    assert_eq!(json["journal"]["disposition"], "conflict");
    assert_eq!(json["profiles"][0]["status"], "conflict");
    assert_eq!(json["secrets"][0]["source"]["status"], "present");
    assert_eq!(json["secrets"][0]["target"]["status"], "present");
    assert_eq!(json["secrets"][0]["contentsMatch"], true);
}

#[test]
fn corrupt_migration_diagnostic_never_exports_the_raw_payload_or_state() {
    let fixture = test_migration_journal_fixture();
    let root = std::env::temp_dir().join(format!(
        "portmate-migration-diagnostic-corrupt-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    save_store(&store_path, &fixture.before).unwrap();
    persist_profile_secret_migration_journal_event(
        &store_path,
        ProfileSecretMigrationJournalEvent::Prepared(fixture.journal.payload.clone()),
    )
    .unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute(
            "update profile_secret_migrations
             set state = 'TOP-SECRET-STATE', payload_json = ?1 where active = 1",
            params![serde_json::to_string("TOP-SECRET-JOURNAL-BODY").unwrap()],
        )
        .unwrap();
    drop(connection);

    let result = export_profile_secret_migration_diagnostics_with_io(
        &store_path,
        &fixture.before,
        |_| panic!("a corrupt journal must not probe providers"),
        ProfileSecretMigrationDiagnosticPortableVault {
            exists: Some(true),
            unlocked: Some(false),
            recovery_ready: Some(false),
            error: None,
        },
    )
    .unwrap();
    assert!(!result.journal_valid);
    let exported = fs::read_to_string(&result.path).unwrap();
    assert!(!exported.contains("TOP-SECRET-STATE"));
    assert!(!exported.contains("TOP-SECRET-JOURNAL-BODY"));
    assert!(exported.contains("未包含原始 payload"));
    assert!(exported.contains("JSON 损坏"));
    let checksum = fs::read_to_string(&result.checksum_path).unwrap();
    assert!(checksum.starts_with(&result.sha256));
    assert_eq!(result.sha256, sha256_hex(exported.as_bytes()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&result.checksum_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn recovery_rolls_back_only_unreferenced_targets_for_old_projection() {
    let fixture = test_migration_journal_fixture();
    let values = std::rc::Rc::new(std::cell::RefCell::new(fixture.values.clone()));
    let deleted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.before,
        &fixture.journal,
        {
            let values = std::rc::Rc::clone(&values);
            move |secret_ref| match values.borrow().get(secret_ref).cloned() {
                Some(value) => SecretProbeResult::Present(Zeroizing::new(value)),
                None => SecretProbeResult::Missing,
            }
        },
        {
            let values = std::rc::Rc::clone(&values);
            let deleted = std::rc::Rc::clone(&deleted);
            move |storage, secret_refs| {
                assert_eq!(storage, SecretStorage::Portable);
                let results = secret_refs
                    .iter()
                    .map(|secret_ref| {
                        values.borrow_mut().remove(secret_ref);
                        deleted.borrow_mut().push(secret_ref.clone());
                        (secret_ref.clone(), Ok(()))
                    })
                    .collect();
                SecretBatchDeleteOutcome {
                    results,
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        {
            let events = std::rc::Rc::clone(&events);
            move |event| {
                events.borrow_mut().push(match event {
                    ProfileSecretMigrationJournalEvent::Prepared(_) => "prepared".to_string(),
                    ProfileSecretMigrationJournalEvent::Transition { state, .. } => {
                        state.as_str().to_string()
                    }
                    ProfileSecretMigrationJournalEvent::Clear { .. } => "clear".to_string(),
                });
                Ok(())
            }
        },
    )
    .unwrap();
    assert!(outcome.resolved);
    assert_eq!(outcome.action, "rolled-back-targets");
    assert_eq!(deleted.borrow().len(), fixture.journal.payload.items.len());
    assert_eq!(
        events.borrow().as_slice(),
        ["target-cleanup-pending", "clear"]
    );
    for item in &fixture.journal.payload.items {
        assert!(values.borrow().contains_key(&item.source_ref));
        assert!(!values.borrow().contains_key(&item.target_ref));
    }
}

#[test]
fn recovery_freezes_old_projection_when_source_is_missing_or_provider_unavailable() {
    let fixture = test_migration_journal_fixture();
    let missing_source = fixture.journal.payload.items[0].source_ref.clone();
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.before,
        &fixture.journal,
        |secret_ref| {
            if secret_ref == missing_source {
                SecretProbeResult::Missing
            } else {
                SecretProbeResult::Present(Zeroizing::new("private".to_string()))
            }
        },
        |_, _| panic!("missing authoritative source must preserve targets"),
        |event| {
            assert!(matches!(
                event,
                ProfileSecretMigrationJournalEvent::Transition {
                    state: ProfileSecretMigrationJournalState::NeedsResolution,
                    ..
                }
            ));
            Ok(())
        },
    )
    .unwrap();
    assert!(!outcome.resolved);
    assert_eq!(outcome.action, "needs-resolution");

    let outcome = recover_profile_secret_migration_with_io(
        &fixture.before,
        &fixture.journal,
        |_| SecretProbeResult::Unavailable("keyring service offline".to_string()),
        |_, _| panic!("unavailable provider must not be treated as missing"),
        |_| panic!("provider unavailability must keep the current journal state"),
    )
    .unwrap();
    assert_eq!(outcome.action, "blocked");
    assert!(outcome.warnings[0].contains("offline"));
}

#[test]
fn recovery_finalizes_new_projection_only_after_exact_secret_verification() {
    let fixture = test_migration_journal_fixture();
    let mut journal = fixture.journal.clone();
    journal.state = ProfileSecretMigrationJournalState::ProfilesCommitted;
    let values = std::rc::Rc::new(std::cell::RefCell::new(fixture.values.clone()));
    let deleted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        {
            let values = std::rc::Rc::clone(&values);
            move |secret_ref| match values.borrow().get(secret_ref).cloned() {
                Some(value) => SecretProbeResult::Present(Zeroizing::new(value)),
                None => SecretProbeResult::Missing,
            }
        },
        {
            let values = std::rc::Rc::clone(&values);
            let deleted = std::rc::Rc::clone(&deleted);
            move |storage, secret_refs| {
                assert_eq!(storage, SecretStorage::Native);
                let results = secret_refs
                    .iter()
                    .map(|secret_ref| {
                        values.borrow_mut().remove(secret_ref);
                        deleted.borrow_mut().push(secret_ref.clone());
                        (secret_ref.clone(), Ok(()))
                    })
                    .collect();
                SecretBatchDeleteOutcome {
                    results,
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(outcome.resolved);
    assert_eq!(outcome.action, "finalized-source-cleanup");
    assert_eq!(deleted.borrow().len(), journal.payload.items.len());
    for item in &journal.payload.items {
        assert!(!values.borrow().contains_key(&item.source_ref));
        assert!(values.borrow().contains_key(&item.target_ref));
    }

    let mismatched_source = journal.payload.items[0].source_ref.clone();
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        |secret_ref| {
            let value = if secret_ref == mismatched_source {
                "changed-source"
            } else if secret_ref.ends_with("11111111-1111-4111-8111-111111111111") {
                "private-a"
            } else {
                "private-b"
            };
            SecretProbeResult::Present(Zeroizing::new(value.to_string()))
        },
        |_, _| panic!("mismatched values must preserve both providers"),
        |event| {
            assert!(matches!(
                event,
                ProfileSecretMigrationJournalEvent::Transition {
                    state: ProfileSecretMigrationJournalState::NeedsResolution,
                    ..
                }
            ));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome.action, "needs-resolution");
}

#[test]
fn recovery_retries_partial_source_cleanup_idempotently() {
    let fixture = test_migration_journal_fixture();
    let mut journal = fixture.journal.clone();
    journal.state = ProfileSecretMigrationJournalState::ProfilesCommitted;
    let values = std::rc::Rc::new(std::cell::RefCell::new(fixture.values.clone()));
    let first_delete = std::cell::Cell::new(true);
    let first = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        {
            let values = std::rc::Rc::clone(&values);
            move |secret_ref| match values.borrow().get(secret_ref).cloned() {
                Some(value) => SecretProbeResult::Present(Zeroizing::new(value)),
                None => SecretProbeResult::Missing,
            }
        },
        {
            let values = std::rc::Rc::clone(&values);
            move |_, secret_refs| {
                let results = secret_refs
                    .iter()
                    .enumerate()
                    .map(|(index, secret_ref)| {
                        if first_delete.get() && index == 1 {
                            (
                                secret_ref.clone(),
                                Err("injected keyring failure".to_string()),
                            )
                        } else {
                            values.borrow_mut().remove(secret_ref);
                            (secret_ref.clone(), Ok(()))
                        }
                    })
                    .collect();
                first_delete.set(false);
                SecretBatchDeleteOutcome {
                    results,
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(!first.resolved);
    assert_eq!(first.action, "source-cleanup-pending");
    assert_eq!(
        fixture
            .journal
            .payload
            .items
            .iter()
            .filter(|item| values.borrow().contains_key(&item.source_ref))
            .count(),
        1
    );

    journal.state = ProfileSecretMigrationJournalState::SourceCleanupPending;
    let second = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        {
            let values = std::rc::Rc::clone(&values);
            move |secret_ref| match values.borrow().get(secret_ref).cloned() {
                Some(value) => SecretProbeResult::Present(Zeroizing::new(value)),
                None => SecretProbeResult::Missing,
            }
        },
        {
            let values = std::rc::Rc::clone(&values);
            move |_, secret_refs| SecretBatchDeleteOutcome {
                results: secret_refs
                    .iter()
                    .map(|secret_ref| {
                        values.borrow_mut().remove(secret_ref);
                        (secret_ref.clone(), Ok(()))
                    })
                    .collect(),
                portable_vault_requires_reunlock: false,
            }
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(second.resolved);
    assert_eq!(second.action, "finalized-source-cleanup");
    assert!(fixture
        .journal
        .payload
        .items
        .iter()
        .all(|item| !values.borrow().contains_key(&item.source_ref)));
}

#[test]
fn recovery_freezes_new_projection_when_a_target_is_missing() {
    let fixture = test_migration_journal_fixture();
    let mut journal = fixture.journal.clone();
    journal.state = ProfileSecretMigrationJournalState::ProfilesCommitted;
    let missing_target = journal.payload.items[0].target_ref.clone();
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        |secret_ref| {
            if secret_ref == missing_target {
                SecretProbeResult::Missing
            } else {
                let value = if secret_ref.contains("source-a")
                    || secret_ref.ends_with("11111111-1111-4111-8111-111111111111")
                {
                    "private-a"
                } else {
                    "private-b"
                };
                SecretProbeResult::Present(Zeroizing::new(value.to_string()))
            }
        },
        |_, _| panic!("missing authoritative target must preserve source copies"),
        |event| {
            assert!(matches!(
                event,
                ProfileSecretMigrationJournalEvent::Transition {
                    state: ProfileSecretMigrationJournalState::NeedsResolution,
                    ..
                }
            ));
            Ok(())
        },
    )
    .unwrap();
    assert!(!outcome.resolved);
    assert_eq!(outcome.action, "needs-resolution");
}

#[test]
fn recovery_preserves_both_sides_for_mixed_projection_without_provider_access() {
    let fixture = test_migration_journal_fixture();
    let mut mixed = fixture.before.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut mixed.profiles[0].connection {
        ssh.password_secret_ref = Some(fixture.journal.payload.items[0].target_ref.clone());
    }
    let outcome = recover_profile_secret_migration_with_io(
        &mixed,
        &fixture.journal,
        |_| panic!("conflict classification must precede provider access"),
        |_, _| panic!("mixed projection must preserve both providers"),
        |event| {
            assert!(matches!(
                event,
                ProfileSecretMigrationJournalEvent::Transition {
                    state: ProfileSecretMigrationJournalState::NeedsResolution,
                    ..
                }
            ));
            Ok(())
        },
    )
    .unwrap();
    assert!(!outcome.resolved);
    assert_eq!(outcome.action, "needs-resolution");
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
fn concurrent_profile_updates_merge_non_overlapping_fields_and_reject_conflicts() {
    let mut expected = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut expected.connection {
        ssh.password_secret_ref = Some("keychain:old".to_string());
        ssh.proxy.password_secret_ref = Some("keychain:old-proxy".to_string());
    }
    let mut current = expected.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut current.connection {
        ssh.password_secret_ref = Some("stronghold:new".to_string());
        ssh.proxy.password_secret_ref = Some("stronghold:new-proxy".to_string());
    }
    current.terminal.cols = 173;
    let mut incoming = expected.clone();
    incoming.name = "Operator edit".to_string();

    let merged = merge_expected_profile_update(Some(&current), Some(&expected), incoming.clone());
    let merged = merged.unwrap();
    assert_eq!(merged.name, "Operator edit");
    assert_eq!(merged.terminal.cols, 173);
    let ConnectionConfig::Ssh(merged_ssh) = merged.connection else {
        unreachable!("merged profile must remain SSH");
    };
    assert_eq!(
        merged_ssh.password_secret_ref.as_deref(),
        Some("stronghold:new")
    );
    assert_eq!(
        merged_ssh.proxy.password_secret_ref.as_deref(),
        Some("stronghold:new-proxy")
    );
    assert!(
        validate_expected_proxy_password(Some(&current), Some(&expected))
            .unwrap_err()
            .contains("代理密码")
    );
    validate_expected_proxy_password(Some(&current), Some(&current)).unwrap();

    let mut conflicting_current = expected.clone();
    conflicting_current.group = "current group".to_string();
    let mut conflicting_incoming = expected.clone();
    conflicting_incoming.group = "incoming group".to_string();
    let error = merge_expected_profile_update(
        Some(&conflicting_current),
        Some(&expected),
        conflicting_incoming,
    )
    .unwrap_err();
    assert!(error.contains("Profile 字段"), "{error}");
    assert!(error.contains("profile.group"), "{error}");

    let mut matching_incoming = expected.clone();
    matching_incoming.group = "current group".to_string();
    let matching = merge_expected_profile_update(
        Some(&conflicting_current),
        Some(&expected),
        matching_incoming,
    )
    .unwrap();
    assert_eq!(matching.group, "current group");

    assert!(
        merge_expected_profile_update(Some(&current), None, incoming.clone())
            .unwrap_err()
            .contains("expectedProfile")
    );
    assert!(
        merge_expected_profile_update(None, Some(&expected), incoming.clone())
            .unwrap_err()
            .contains("删除")
    );
    assert_eq!(
        merge_expected_profile_update(None, None, incoming.clone()).unwrap(),
        incoming
    );

    let mut wrong_expected = expected.clone();
    wrong_expected.id = "another-profile".to_string();
    assert!(
        merge_expected_profile_update(Some(&current), Some(&wrong_expected), expected)
            .unwrap_err()
            .contains("不是同一个 Profile")
    );
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

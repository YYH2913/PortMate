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

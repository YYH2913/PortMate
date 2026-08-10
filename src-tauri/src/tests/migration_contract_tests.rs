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
fn new_profile_secret_migrations_only_target_stronghold() {
    let portable = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec!["ssh-1".to_string()],
        cleanup_source: true,
    };
    crate::vault_commands::ensure_supported_profile_secret_migration_request(&portable).unwrap();

    let native = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Native,
        ..portable
    };
    assert!(crate::vault_commands::ensure_supported_profile_secret_migration_request(&native)
        .unwrap_err()
        .contains("仅支持从系统密钥库迁移到 Stronghold"));
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

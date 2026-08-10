#[test]
fn one_key_command_types_keep_stable_json_defaults() {
    let request: SaveOneKeyRequest = serde_json::from_value(serde_json::json!({
        "id": null,
        "label": "Lab login",
        "kind": "ssh",
        "username": "operator",
        "passwordUpdate": { "action": "preserve" },
        "passphraseUpdate": { "action": "clear" },
        "sessionIds": ["ssh-1"]
    }))
    .unwrap();
    assert!(request.id.is_none());
    assert!(matches!(
        request.identity_update,
        OneKeyIdentityUpdate::Preserve
    ));
    assert!(matches!(
        request.password_update,
        OneKeySecretUpdate::Preserve
    ));
    assert!(matches!(
        request.passphrase_update,
        OneKeySecretUpdate::Clear
    ));
    assert_eq!(request.session_ids, vec!["ssh-1".to_string()]);
}

#[test]
fn keyring_initialization_is_persistent_only_and_retries_transient_failures() {
    let initialized = Mutex::new(false);
    let attempts = std::cell::Cell::new(0_u32);
    let first = ensure_keyring_store_with(&initialized, || {
        attempts.set(attempts.get() + 1);
        Err("secret service offline".to_string())
    });
    assert_eq!(first.unwrap_err(), "secret service offline");
    assert!(!*initialized.lock().unwrap());

    ensure_keyring_store_with(&initialized, || {
        attempts.set(attempts.get() + 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(attempts.get(), 2);
    ensure_keyring_store_with(&initialized, || {
        panic!("successful initialization must be cached")
    })
    .unwrap();

    let calls = std::cell::Cell::new(0_u32);
    let error = initialize_persistent_native_keyring_with(|| {
        calls.set(calls.get() + 1);
        Err("persistent store unavailable".to_string())
    })
    .unwrap_err();
    assert_eq!(error, "persistent store unavailable");
    assert_eq!(calls.get(), 1);
}

#[test]
fn new_user_secrets_default_to_stronghold_and_reject_native_storage() {
    for storage in [None, Some(SecretStorage::Portable)] {
        let secret_ref = new_user_secret_ref(storage).unwrap();
        assert!(secret_ref.starts_with("stronghold:"));
        assert!(ensure_user_secret_ref_is_writable(&secret_ref).is_ok());
    }

    let native_error = new_user_secret_ref(Some(SecretStorage::Native)).unwrap_err();
    assert!(native_error.contains("必须保存到 Stronghold"));
    let overwrite_error =
        ensure_user_secret_ref_is_writable("keychain:legacy-password").unwrap_err();
    assert!(overwrite_error.contains("只支持读取、删除和迁移"));
    assert!(ensure_user_secret_ref_is_writable("legacy-password")
        .unwrap_err()
        .contains("stronghold: 前缀"));
}

#[test]
fn client_identity_validation_enforces_immutable_id_and_source_fields() {
    let immutable_error = normalize_client_identity(
        "identity-a",
        IdentityRef {
            id: "identity-b".to_string(),
            label: "Key".to_string(),
            source: IdentitySource::Agent,
            fingerprint_sha256: None,
            path: None,
            secret_ref: None,
        },
        |_| Ok(()),
    )
    .unwrap_err();
    assert!(immutable_error.contains("不可修改"));

    let path_error = normalize_client_identity(
        "identity-a",
        IdentityRef {
            id: "identity-a".to_string(),
            label: "Key".to_string(),
            source: IdentitySource::SystemFile,
            fingerprint_sha256: None,
            path: Some("  ".to_string()),
            secret_ref: Some("keychain:ignored".to_string()),
        },
        |_| Ok(()),
    )
    .unwrap_err();
    assert!(path_error.contains("私钥路径"));

    let exact_path = normalize_client_identity(
        "identity-a",
        IdentityRef {
            id: "identity-a".to_string(),
            label: "Key".to_string(),
            source: IdentitySource::SystemFile,
            fingerprint_sha256: None,
            path: Some(" /home/operator/.ssh/id_ed25519 ".to_string()),
            secret_ref: None,
        },
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(
        exact_path.path.as_deref(),
        Some(" /home/operator/.ssh/id_ed25519 ")
    );

    let mut invalid_profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut invalid_profile.connection {
        ssh.identity_refs = vec![IdentityRef {
            id: "identity-a".to_string(),
            label: "Key".to_string(),
            source: IdentitySource::SystemFile,
            fingerprint_sha256: None,
            path: Some(" \t ".to_string()),
            secret_ref: None,
        }];
    } else {
        panic!("expected SSH profile");
    }
    assert!(validate_profile_client_identity_ids(&invalid_profile)
        .unwrap_err()
        .contains("缺少私钥路径"));
    if let ConnectionConfig::Ssh(ssh) = &mut invalid_profile.connection {
        ssh.identity_refs[0].path = exact_path.path.clone();
    }
    validate_profile_client_identity_ids(&invalid_profile).unwrap();

    let vault_error = normalize_client_identity(
        "identity-a",
        vault_identity("identity-a", "keychain:missing"),
        |_| Err("secret unavailable".to_string()),
    )
    .unwrap_err();
    assert_eq!(vault_error, "secret unavailable");

    let agent = normalize_client_identity(
        "identity-a",
        IdentityRef {
            id: "identity-a".to_string(),
            label: "  Agent Key  ".to_string(),
            source: IdentitySource::Agent,
            fingerprint_sha256: Some("  SHA256:agent  ".to_string()),
            path: Some(" socket comment ".to_string()),
            secret_ref: Some("keychain:must-clear".to_string()),
        },
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(agent.label, "Agent Key");
    assert_eq!(agent.fingerprint_sha256.as_deref(), Some("SHA256:agent"));
    assert!(agent.secret_ref.is_none());
}

#[test]
fn concurrent_client_identity_updates_merge_fields_and_preserve_new_secrets() {
    let mut expected = vault_identity("identity-a", "keychain:old");
    expected.label = "Original label".to_string();
    expected.fingerprint_sha256 = Some("SHA256:original".to_string());

    let mut current = expected.clone();
    current.label = "Current label".to_string();
    current.secret_ref = Some("keychain:rotated".to_string());

    let mut incoming = expected.clone();
    incoming.fingerprint_sha256 = Some("SHA256:incoming".to_string());

    let merged =
        merge_expected_client_identity_update(&current, &expected, incoming.clone()).unwrap();
    assert_eq!(merged.label, "Current label");
    assert_eq!(
        merged.fingerprint_sha256.as_deref(),
        Some("SHA256:incoming")
    );
    assert_eq!(merged.secret_ref.as_deref(), Some("keychain:rotated"));

    let mut conflicting = incoming;
    conflicting.label = "Incoming label".to_string();
    let error =
        merge_expected_client_identity_update(&current, &expected, conflicting).unwrap_err();
    assert!(error.contains("Client Identity 字段"), "{error}");
    assert!(!error.contains("Profile 字段"), "{error}");
    assert!(error.contains("identity.label"), "{error}");
    assert!(!error.contains("Current label"), "{error}");
    assert!(!error.contains("Incoming label"), "{error}");

    let mut wrong_expected = expected.clone();
    wrong_expected.id = "identity-b".to_string();
    assert!(
        merge_expected_client_identity_update(&current, &wrong_expected, expected)
            .unwrap_err()
            .contains("不是同一个 identity")
    );
}

#[test]
fn rotating_shared_identity_keeps_the_old_secret_for_other_profiles() {
    let mut first = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut first.connection {
        ssh.identity_refs = vec![vault_identity("shared-key", "keychain:shared")];
    }
    let mut second = first.clone();
    second.id = "ssh-session-2".to_string();
    second.name = "Bench SSH 2".to_string();
    let mut store = SessionStore::default();
    store.upsert_profile(first);
    store.upsert_profile(second);

    let (summary, old_secret_ref) = replace_client_identity(
        &mut store,
        "ssh-session-1",
        "shared-key",
        vault_identity("shared-key", "keychain:rotated"),
    )
    .unwrap();
    let delete_called = std::cell::Cell::new(false);
    let response =
        client_identity_mutation_response(&store, summary, old_secret_ref.as_deref(), true, |_| {
            delete_called.set(true);
            Ok(())
        });
    assert!(response.old_secret_shared);
    assert!(!response.old_secret_deleted);
    assert!(!delete_called.get());
    assert_eq!(secret_ref_usage_count(&store, "keychain:shared"), 1);
    assert_eq!(secret_ref_usage_count(&store, "keychain:rotated"), 1);
}

#[test]
fn failed_orphan_cleanup_keeps_the_persisted_identity_valid() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.identity_refs = vec![vault_identity("vault-key", "keychain:old")];
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    let (summary, old_secret_ref) = replace_client_identity(
        &mut store,
        "ssh-session-1",
        "vault-key",
        vault_identity("vault-key", "keychain:new"),
    )
    .unwrap();
    let response =
        client_identity_mutation_response(&store, summary, old_secret_ref.as_deref(), true, |_| {
            Err("keyring locked".to_string())
        });
    assert!(!response.old_secret_deleted);
    assert!(!response.old_secret_shared);
    assert!(response
        .cleanup_warning
        .as_deref()
        .is_some_and(|warning| warning.contains("keyring locked")));
    let saved = find_client_identity(&store, "ssh-session-1", "vault-key").unwrap();
    assert_eq!(saved.secret_ref.as_deref(), Some("keychain:new"));
}

#[test]
fn deleting_jump_identity_is_blocked_and_duplicate_ids_are_rejected() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.identity_refs = vec![vault_identity("jump-key", "keychain:jump")];
        ssh.jumps.push(portmate_core::JumpHop {
            host: "bastion.example".to_string(),
            port: 22,
            username: "root".to_string(),
            password_secret_ref: None,
            passphrase_secret_ref: None,
            identity_ref: Some("jump-key".to_string()),
            host_key_policy: None,
        });
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    assert!(
        remove_client_identity(&mut store, "ssh-session-1", "jump-key")
            .unwrap_err()
            .contains("Jump Host")
    );

    if let ConnectionConfig::Ssh(ssh) = &mut store.profiles[0].connection {
        ssh.jumps.clear();
        ssh.identity_refs
            .push(vault_identity("jump-key", "keychain:duplicate"));
    }
    assert!(find_client_identity(&store, "ssh-session-1", "jump-key")
        .unwrap_err()
        .contains("重复"));
}

#[test]
fn secret_usage_counts_target_jump_and_identity_credentials() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.password_secret_ref = Some(" keychain:shared ".to_string());
        ssh.passphrase_secret_ref = Some("keychain:shared".to_string());
        ssh.identity_refs = vec![vault_identity("vault-key", "keychain:shared")];
        ssh.jumps.push(portmate_core::JumpHop {
            host: "bastion.example".to_string(),
            port: 22,
            username: "root".to_string(),
            password_secret_ref: Some("keychain:shared".to_string()),
            passphrase_secret_ref: Some("keychain:shared".to_string()),
            identity_ref: None,
            host_key_policy: None,
        });
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    assert_eq!(secret_ref_usage_count(&store, "keychain:shared"), 5);
}

use super::*;

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

#[test]
fn one_key_identity_updates_clone_only_bound_authenticating_identities() {
    let mut profile = test_ssh_profile();
    let selected_identity = vault_identity("vault-key", "keychain:vault-key");
    let public_key_only = IdentityRef {
        id: "public-key".to_string(),
        label: "Public key".to_string(),
        source: IdentitySource::PublicKeyOnly,
        fingerprint_sha256: Some("SHA256:public".to_string()),
        path: Some("/tmp/public-key.pub".to_string()),
        secret_ref: None,
    };
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.identity_refs = vec![selected_identity.clone(), public_key_only];
    }
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    let sessions = vec!["ssh-session-1".to_string()];

    let selected = apply_one_key_identity_update(
        &store,
        OneKeyKind::Ssh,
        &sessions,
        None,
        OneKeyIdentityUpdate::Set {
            source_profile_id: "ssh-session-1".to_string(),
            identity_id: "vault-key".to_string(),
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.source_profile_id, "ssh-session-1");
    assert_eq!(selected.identity, selected_identity);

    assert!(apply_one_key_identity_update(
        &store,
        OneKeyKind::Ssh,
        &["other-session".to_string()],
        None,
        OneKeyIdentityUpdate::Set {
            source_profile_id: "ssh-session-1".to_string(),
            identity_id: "vault-key".to_string(),
        },
    )
    .unwrap_err()
    .contains("已绑定"));
    assert!(apply_one_key_identity_update(
        &store,
        OneKeyKind::Ssh,
        &sessions,
        None,
        OneKeyIdentityUpdate::Set {
            source_profile_id: "ssh-session-1".to_string(),
            identity_id: "public-key".to_string(),
        },
    )
    .unwrap_err()
    .contains("私钥"));

    assert!(apply_one_key_identity_update(
        &store,
        OneKeyKind::Ssh,
        &["other-session".to_string()],
        Some(selected),
        OneKeyIdentityUpdate::Preserve,
    )
    .unwrap()
    .is_none());
}

#[test]
fn one_key_prompt_completion_revalidates_field_username_and_event_freshness() {
    let legacy_request: SendOneKeyRequest = serde_json::from_value(serde_json::json!({
        "id": "onekey:legacy",
        "sessionId": "ssh-session-1",
        "field": "username"
    }))
    .unwrap();
    assert_eq!(legacy_request.source, OneKeySendSource::Manual);
    assert!(legacy_request.prompt_event_id.is_none());

    let mut store = SessionStore::default();
    store.upsert_profile(test_ssh_profile());
    let now = Utc::now();
    let one_key = OneKeyCredential {
        id: "onekey:prompt".to_string(),
        label: "Prompt login".to_string(),
        kind: OneKeyKind::Account,
        username: "operator".to_string(),
        password_secret_ref: Some("keychain:prompt-password".to_string()),
        passphrase_secret_ref: None,
        identity: None,
        session_ids: vec!["ssh-session-1".to_string()],
        created_at: now,
        updated_at: now,
    };
    store
        .record_stream_event(
            "ssh-session-1",
            EventDirection::Inbound,
            EventStream::Stdout,
            "\x1b[33mPass",
        )
        .unwrap();
    let prompt = store
        .record_stream_event(
            "ssh-session-1",
            EventDirection::Inbound,
            EventStream::Stdout,
            "word for operator:\x1b[0m",
        )
        .unwrap();
    store.record_system_event("ssh-session-1", "PortMate: diagnostic");

    validate_one_key_prompt_completion(
        &store,
        &one_key,
        "ssh-session-1",
        OneKeyField::Password,
        &prompt.id,
    )
    .unwrap();
    assert!(validate_one_key_prompt_completion(
        &store,
        &one_key,
        "ssh-session-1",
        OneKeyField::Username,
        &prompt.id,
    )
    .unwrap_err()
    .contains("字段"));

    let mut wrong_username = one_key.clone();
    wrong_username.username = "root".to_string();
    assert!(validate_one_key_prompt_completion(
        &store,
        &wrong_username,
        "ssh-session-1",
        OneKeyField::Password,
        &prompt.id,
    )
    .unwrap_err()
    .contains("用户名"));

    store
        .record_event(
            "ssh-session-1",
            EventDirection::Outbound,
            EventStream::Control,
            None,
            None,
            BTreeMap::new(),
        )
        .unwrap();
    assert!(validate_one_key_prompt_completion(
        &store,
        &one_key,
        "ssh-session-1",
        OneKeyField::Password,
        &prompt.id,
    )
    .unwrap_err()
    .contains("已变化"));

    assert_eq!(
        detect_one_key_terminal_prompt("root@router's password:"),
        Some(DetectedOneKeyPrompt::Password {
            username_hint: Some("root".to_string()),
        })
    );
    assert_eq!(
        detect_one_key_terminal_prompt("device login:"),
        Some(DetectedOneKeyPrompt::Username)
    );
    assert!(detect_one_key_terminal_prompt("Password:\r\n").is_none());
    assert!(detect_one_key_terminal_prompt("New password:").is_none());
    assert!(detect_one_key_terminal_prompt("Retype new password:").is_none());
}

#[test]
fn one_key_summaries_hide_refs_and_count_secret_usage() {
    let mut store = SessionStore::default();
    let now = Utc::now();
    store.one_keys.push(OneKeyCredential {
        id: "onekey:test".to_string(),
        label: "Lab account".to_string(),
        kind: OneKeyKind::Ssh,
        username: "operator".to_string(),
        password_secret_ref: Some("keychain:onekey-password".to_string()),
        passphrase_secret_ref: Some("keychain:onekey-passphrase".to_string()),
        identity: Some(OneKeyIdentity {
            source_profile_id: "ssh-session-1".to_string(),
            identity: vault_identity("onekey-key", "keychain:onekey-identity"),
        }),
        session_ids: vec!["ssh-session-1".to_string()],
        created_at: now,
        updated_at: now,
    });

    assert_eq!(
        secret_ref_usage_count(&store, "keychain:onekey-password"),
        1
    );
    assert_eq!(
        secret_ref_usage_count(&store, "keychain:onekey-passphrase"),
        1
    );
    assert_eq!(
        secret_ref_usage_count(&store, "keychain:onekey-identity"),
        1
    );
    let summaries = one_key_summaries(&store);
    assert!(summaries[0].has_password);
    assert!(summaries[0].has_passphrase);
    assert_eq!(
        summaries[0]
            .identity
            .as_ref()
            .map(|identity| identity.id.as_str()),
        Some("onekey-key")
    );
    let json = serde_json::to_string(&summaries).unwrap();
    assert!(!json.contains("onekey-password"));
    assert!(!json.contains("onekey-passphrase"));
    assert!(!json.contains("onekey-identity"));
}

#[test]
fn one_key_login_resolves_only_bound_ssh_credentials() {
    let mut store = SessionStore::default();
    let mut profile = test_ssh_profile();
    let selected_identity = vault_identity("login-key", "keychain:login-identity");
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.identity_refs.push(selected_identity.clone());
    }
    store.upsert_profile(profile);
    let now = Utc::now();
    store.one_keys.push(OneKeyCredential {
        id: "onekey:login".to_string(),
        label: "Operations".to_string(),
        kind: OneKeyKind::Ssh,
        username: "operator".to_string(),
        password_secret_ref: Some(" keychain:login-password ".to_string()),
        passphrase_secret_ref: Some("stronghold:login-passphrase".to_string()),
        identity: Some(OneKeyIdentity {
            source_profile_id: "ssh-session-1".to_string(),
            identity: selected_identity.clone(),
        }),
        session_ids: vec!["ssh-session-1".to_string()],
        created_at: now,
        updated_at: now,
    });

    let mut reads = Vec::new();
    let resolved = resolve_one_key_login_credentials_with(
        &store,
        "ssh-session-1",
        "onekey:login",
        |secret_ref| {
            reads.push(secret_ref.to_string());
            Ok(match secret_ref {
                "keychain:login-password" => "login-secret",
                "stronghold:login-passphrase" => "key-secret",
                _ => panic!("unexpected OneKey Secret reference"),
            }
            .to_string())
        },
    )
    .unwrap();
    assert_eq!(
        resolved,
        OneKeyLoginCredentials {
            username: "operator".to_string(),
            password: Some("login-secret".to_string()),
            passphrase: Some("key-secret".to_string()),
            identity: Some(selected_identity.clone()),
        }
    );
    assert_eq!(
        reads,
        [
            "keychain:login-password".to_string(),
            "stronghold:login-passphrase".to_string(),
        ]
    );

    let mut runtime_profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut runtime_profile.connection {
        ssh.password_secret_ref = Some("keychain:profile-password".to_string());
        ssh.passphrase_secret_ref = Some("keychain:profile-passphrase".to_string());
    }
    apply_session_open_profile_credentials(
        &mut runtime_profile,
        Some("operator"),
        Some(&selected_identity),
        true,
    )
    .unwrap();
    let runtime_ssh = ssh_connection(&runtime_profile).unwrap();
    assert_eq!(runtime_ssh.username, "operator");
    assert!(runtime_ssh.password_secret_ref.is_none());
    assert!(runtime_ssh.passphrase_secret_ref.is_none());
    assert_eq!(
        runtime_ssh.identity_refs.as_slice(),
        std::slice::from_ref(&selected_identity)
    );
    assert!(runtime_ssh.identity_policy.identities_only);
    assert!(runtime_ssh
        .identity_policy
        .auth_order
        .contains(&AuthMethod::PublicKey));

    store.one_keys[0].password_secret_ref = None;
    store.one_keys[0].passphrase_secret_ref = None;
    assert_eq!(
        resolve_one_key_login_credentials_with(
            &store,
            "ssh-session-1",
            "onekey:login",
            |_| panic!("identity-only OneKey must not read Secret data"),
        )
        .unwrap(),
        OneKeyLoginCredentials {
            username: "operator".to_string(),
            password: None,
            passphrase: None,
            identity: Some(selected_identity),
        }
    );

    store.one_keys[0].session_ids = vec!["another-session".to_string()];
    assert!(resolve_one_key_login_credentials_with(
        &store,
        "ssh-session-1",
        "onekey:login",
        |_| panic!("unbound OneKey must not read Secret data"),
    )
    .unwrap_err()
    .contains("未绑定"));

    store.one_keys[0].session_ids = vec!["ssh-session-1".to_string()];
    store.one_keys[0].kind = OneKeyKind::Account;
    assert!(resolve_one_key_login_credentials_with(
        &store,
        "ssh-session-1",
        "onekey:login",
        |_| panic!("Account OneKey must not read SSH Secret data"),
    )
    .unwrap_err()
    .contains("SSH OneKey"));
}

#[test]
fn one_key_completion_writes_value_with_prompt_audit_without_readable_text() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = b"private-value\r".to_vec();
        let expected_len = expected.len();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = vec![0_u8; expected_len];
            socket.read_exact(&mut received).await.unwrap();
            let _ = release_rx.await;
            received
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!("portmate-one-key-send-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();
        let (prompt_event_id, one_key_updated_at) = {
            let mut store = state.store.lock().unwrap();
            let now = Utc::now();
            store.one_keys.push(OneKeyCredential {
                id: "onekey:completion".to_string(),
                label: "Completion".to_string(),
                kind: OneKeyKind::Account,
                username: "operator".to_string(),
                password_secret_ref: Some("keychain:completion".to_string()),
                passphrase_secret_ref: None,
                identity: None,
                session_ids: vec![profile.id.clone()],
                created_at: now,
                updated_at: now,
            });
            let prompt_event_id = store
                .record_stream_event(
                    &profile.id,
                    EventDirection::Inbound,
                    EventStream::Stdout,
                    "Password:",
                )
                .unwrap()
                .id;
            (prompt_event_id, now)
        };
        let validation = OneKeyPromptValidation {
            one_key_id: "onekey:completion".to_string(),
            one_key_updated_at,
            field: OneKeyField::Password,
            prompt_event_id: prompt_event_id.clone(),
        };

        let event = send_one_key_value(
            state.session_io(),
            &profile.id,
            "private-value",
            "one-key-completion",
            Some(&prompt_event_id),
            Some(&validation),
        )
        .await
        .unwrap();
        assert!(event.text.is_none());
        assert_eq!(
            event.annotations.get("origin").map(String::as_str),
            Some("one-key-completion")
        );
        assert_eq!(
            event.annotations.get("relatedEventId").map(String::as_str),
            Some(prompt_event_id.as_str())
        );
        assert!(!serde_json::to_string(&event)
            .unwrap()
            .contains("private-value"));
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        let _ = release_tx.send(());
        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("OneKey loopback server timed out")
            .expect("OneKey loopback server failed");
        assert_eq!(received, expected);
        let _ = fs::remove_dir_all(root);
    });
}

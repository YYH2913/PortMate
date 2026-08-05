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

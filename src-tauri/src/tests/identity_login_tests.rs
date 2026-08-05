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

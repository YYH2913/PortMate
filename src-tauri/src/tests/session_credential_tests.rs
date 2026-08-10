use super::*;

fn test_credential_store() -> SessionStore {
    let mut store = SessionStore::default();
    store.upsert_profile(test_ssh_profile());
    store
}

fn credential_request(
    password: Option<&str>,
    passphrase: Option<&str>,
) -> StageSessionCredentialsRequest {
    StageSessionCredentialsRequest {
        session_id: "ssh-session-1".to_string(),
        password: password.map(str::to_string),
        passphrase: passphrase.map(str::to_string),
    }
}

#[test]
fn staged_session_credentials_are_window_and_session_bound_and_single_use() {
    let registry = Mutex::new(SessionCredentialRegistry::default());
    let store = test_credential_store();
    let now = Instant::now();
    let response = stage_session_credentials_for_owner(
        &registry,
        &store,
        "main",
        credential_request(Some("password"), Some("passphrase")),
        now,
    )
    .unwrap();

    assert_eq!(response.expires_in_ms, 30_000);
    assert!(response
        .credential_handle
        .starts_with("session-credential:"));
    assert!(consume_session_credentials_for_owner(
        &registry,
        "pane-1",
        "ssh-session-1",
        &response.credential_handle,
        now,
    )
    .err()
    .unwrap()
    .contains("窗口或会话不匹配"));
    assert!(consume_session_credentials_for_owner(
        &registry,
        "main",
        "ssh-session-2",
        &response.credential_handle,
        now,
    )
    .err()
    .unwrap()
    .contains("窗口或会话不匹配"));

    let consumed = consume_session_credentials_for_owner(
        &registry,
        "main",
        "ssh-session-1",
        &response.credential_handle,
        now,
    )
    .unwrap();
    assert_eq!(consumed.password.as_deref(), Some("password"));
    assert_eq!(consumed.passphrase.as_deref(), Some("passphrase"));
    assert!(consume_session_credentials_for_owner(
        &registry,
        "main",
        "ssh-session-1",
        &response.credential_handle,
        now,
    )
    .err()
    .unwrap()
    .contains("已过期或已使用"));
}

#[test]
fn staged_session_credentials_expire_and_replacement_revokes_the_old_handle() {
    let registry = Mutex::new(SessionCredentialRegistry::default());
    let store = test_credential_store();
    let now = Instant::now();
    let first = stage_session_credentials_for_owner(
        &registry,
        &store,
        "main",
        credential_request(Some("first"), None),
        now,
    )
    .unwrap();
    let second = stage_session_credentials_for_owner(
        &registry,
        &store,
        "main",
        credential_request(Some("second"), None),
        now,
    )
    .unwrap();
    assert_ne!(first.credential_handle, second.credential_handle);
    assert!(consume_session_credentials_for_owner(
        &registry,
        "main",
        "ssh-session-1",
        &first.credential_handle,
        now,
    )
    .is_err());
    assert!(consume_session_credentials_for_owner(
        &registry,
        "main",
        "ssh-session-1",
        &second.credential_handle,
        now + SESSION_CREDENTIAL_TTL,
    )
    .err()
    .unwrap()
    .contains("已过期或已使用"));
}

#[test]
fn staged_session_credentials_reject_invalid_values_and_profile_changes() {
    let registry = Mutex::new(SessionCredentialRegistry::default());
    let mut store = test_credential_store();
    let now = Instant::now();
    assert!(stage_session_credentials_for_owner(
        &registry,
        &store,
        "main",
        credential_request(None, None),
        now,
    )
    .unwrap_err()
    .contains("没有可暂存"));
    assert!(stage_session_credentials_for_owner(
        &registry,
        &store,
        "main",
        credential_request(Some("bad\0password"), None),
        now,
    )
    .unwrap_err()
    .contains("不能包含 NUL"));

    let response = stage_session_credentials_for_owner(
        &registry,
        &store,
        "main",
        credential_request(Some("password"), None),
        now,
    )
    .unwrap();
    let consumed = consume_session_credentials_for_owner(
        &registry,
        "main",
        "ssh-session-1",
        &response.credential_handle,
        now,
    )
    .unwrap();
    let mut changed = store.profile("ssh-session-1").unwrap();
    if let ConnectionConfig::Ssh(ssh) = &mut changed.connection {
        ssh.endpoint.host = "different.example".to_string();
    }
    store.upsert_profile(changed.clone());
    assert!(
        validate_session_credential_binding(&changed, &consumed.binding)
            .unwrap_err()
            .contains("配置已在凭据暂存后改变")
    );
}

#[test]
fn desktop_open_session_contract_cannot_deserialize_inline_secrets() {
    let request: OpenSessionRequest = serde_json::from_value(serde_json::json!({
        "sessionId": "ssh-session-1",
        "credentialHandle": "session-credential:11111111-1111-4111-8111-111111111111"
    }))
    .unwrap();
    assert_eq!(request.session_id, "ssh-session-1");
    assert!(
        serde_json::from_value::<OpenSessionRequest>(serde_json::json!({
            "sessionId": "ssh-session-1",
            "password": "must-not-cross-open-session"
        }))
        .is_err()
    );

    let response = SessionCredentialHandleResponse {
        credential_handle: "session-credential:11111111-1111-4111-8111-111111111111".to_string(),
        expires_in_ms: 30_000,
    };
    let encoded = serde_json::to_string(&response).unwrap();
    assert!(!encoded.contains("password"));
    assert!(!encoded.contains("passphrase"));
}

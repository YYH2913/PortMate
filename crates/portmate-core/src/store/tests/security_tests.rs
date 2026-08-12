use super::*;

#[test]
fn legacy_store_without_one_keys_deserializes_empty() {
    let mut value = serde_json::to_value(SessionStore::default()).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("oneKeys");
    object.remove("commandHistory");
    object.remove("commandHistoryMigrated");
    object.remove("commandHistoryRevision");
    object.remove("mcpHttpSettings");
    let store: SessionStore = serde_json::from_value(value).unwrap();
    assert!(store.one_keys.is_empty());
    assert!(store.command_history.is_empty());
    assert!(!store.command_history_migrated);
    assert_eq!(store.command_history_revision, 0);
    assert_eq!(store.mcp_http_settings, McpHttpSettings::default());
}

#[test]
fn write_scope_requires_grant() {
    let store = test_store();
    assert!(store.mcp_can("test-client", McpScope::ReadLogs, Some("test-session")));
    assert!(store.mcp_can("test-client", McpScope::WriteInput, Some("test-session")));
    assert!(!store.mcp_can("readonly", McpScope::WriteInput, Some("test-session")));
}

#[test]
fn read_scopes_default_open_then_follow_explicit_grants() {
    let mut store = test_store();
    store.grants.clear();
    assert!(store.mcp_can_read("reader", McpScope::ReadSessions, None));
    assert!(store.mcp_can_read("reader", McpScope::ReadLogs, Some("test-session")));
    assert!(!store.mcp_can_read("  ", McpScope::ReadSessions, None));
    assert!(!store.mcp_can_read("bad\nreader", McpScope::ReadSessions, None));
    assert!(!store.mcp_can_read(&"x".repeat(129), McpScope::ReadSessions, None));

    store.grants.push(McpGrant {
        client_id: "scoped-reader".to_string(),
        name: "Scoped reader".to_string(),
        scopes: vec![McpScope::ReadLogs],
        allowed_sessions: vec!["test-session".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    assert!(!store.mcp_can_read("unknown", McpScope::ReadLogs, Some("test-session")));
    assert!(!store.mcp_can_read("scoped-reader", McpScope::ReadSessions, None));
    assert!(store.mcp_can_read(" scoped-reader ", McpScope::ReadLogs, Some("test-session")));
    assert!(!store.mcp_can_read("scoped-reader", McpScope::ReadLogs, Some("other-session")));

    store.grants[0].revoked_at = Some(Utc::now());
    assert!(!store.mcp_can_read("scoped-reader", McpScope::ReadLogs, Some("test-session")));
}

#[test]
fn grant_is_expired_at_its_exact_deadline() {
    let now = Utc::now();
    let grant = McpGrant {
        client_id: "reader".to_string(),
        name: "Reader".to_string(),
        scopes: vec![McpScope::ReadLogs],
        allowed_sessions: Vec::new(),
        confirm_writes: false,
        expires_at: Some(now),
        revoked_at: None,
    };

    assert!(!grant.allows(McpScope::ReadLogs, Some("test-session"), now));
}

#[test]
fn transfer_and_tunnel_write_scopes_imply_only_their_matching_read_scope() {
    let now = Utc::now();
    let transfer = McpGrant {
        client_id: "transfer-client".to_string(),
        name: "Transfer client".to_string(),
        scopes: vec![McpScope::Transfer],
        allowed_sessions: vec!["test-session".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    };
    assert!(transfer.allows(McpScope::Transfer, Some("test-session"), now));
    assert!(transfer.allows(McpScope::ReadTransfers, Some("test-session"), now));
    assert!(!transfer.allows(McpScope::ReadTunnels, Some("test-session"), now));
    assert!(!transfer.allows(McpScope::ReadTransfers, Some("other-session"), now));

    let tunnel = McpGrant {
        client_id: "tunnel-client".to_string(),
        name: "Tunnel client".to_string(),
        scopes: vec![McpScope::Tunnel],
        allowed_sessions: vec!["test-session".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    };
    assert!(tunnel.allows(McpScope::Tunnel, Some("test-session"), now));
    assert!(tunnel.allows(McpScope::ReadTunnels, Some("test-session"), now));
    assert!(!tunnel.allows(McpScope::ReadTransfers, Some("test-session"), now));
}

#[test]
fn auth_success_recording_respects_the_enabled_policy() {
    let mut store = test_store();
    let mut ssh = sensitive_ssh_connection();
    ssh.identity_policy.auth_order = vec![AuthMethod::Password];
    store.profiles[0].kind = SessionKind::Ssh;
    store.profiles[0].connection = ConnectionConfig::Ssh(ssh);

    store
        .record_auth_success("test-session", AuthMethod::PublicKey)
        .unwrap();
    let profile = store.profile("test-session").unwrap();
    let ConnectionConfig::Ssh(ssh) = profile.connection else {
        panic!("test profile must remain SSH");
    };
    assert_eq!(ssh.identity_policy.last_successful, None);

    store
        .record_auth_success("test-session", AuthMethod::Password)
        .unwrap();
    let profile = store.profile("test-session").unwrap();
    let ConnectionConfig::Ssh(ssh) = profile.connection else {
        panic!("test profile must remain SSH");
    };
    assert_eq!(
        ssh.identity_policy.last_successful,
        Some(AuthMethod::Password)
    );

    let ConnectionConfig::Ssh(ssh) = &mut store.profiles[0].connection else {
        panic!("test profile must remain SSH");
    };
    ssh.identity_policy.record_success = false;
    store
        .record_auth_success("test-session", AuthMethod::Password)
        .unwrap();
    let profile = store.profile("test-session").unwrap();
    let ConnectionConfig::Ssh(ssh) = profile.connection else {
        panic!("test profile must remain SSH");
    };
    assert_eq!(ssh.identity_policy.last_successful, None);
}

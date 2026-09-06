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
fn read_scopes_fail_closed_then_follow_explicit_grants() {
    let mut store = test_store();
    store.grants.clear();
    assert!(!store.mcp_can_read("reader", McpScope::ReadSessions, None));
    assert!(!store.mcp_can_read("reader", McpScope::ReadLogs, Some("test-session")));
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
fn http_client_identity_auto_unifies_only_when_the_boundary_is_unambiguous() {
    let mut store = test_store();
    store.grants.clear();
    store.mcp_http_settings.client_id = DEFAULT_MCP_HTTP_CLIENT_ID.to_string();
    store.grants.push(McpGrant {
        client_id: "remote-console".to_string(),
        name: "Remote console".to_string(),
        scopes: vec![McpScope::ReadSessions],
        allowed_sessions: vec![],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    assert_eq!(
        store.mcp_resolved_client_id(Some(DEFAULT_MCP_HTTP_CLIENT_ID)),
        "remote-console"
    );

    store.grants.push(McpGrant {
        client_id: "audit-console".to_string(),
        name: "Audit console".to_string(),
        scopes: vec![McpScope::ReadLogs],
        allowed_sessions: vec![],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    assert_eq!(
        store.mcp_resolved_client_id(Some(DEFAULT_MCP_HTTP_CLIENT_ID)),
        DEFAULT_MCP_HTTP_CLIENT_ID
    );
    assert_eq!(
        store.mcp_resolved_client_id(Some("audit-console")),
        "audit-console"
    );

    store.grants[0].revoked_at = Some(Utc::now());
    assert_eq!(
        store.mcp_resolved_client_id(Some(DEFAULT_MCP_HTTP_CLIENT_ID)),
        "audit-console"
    );
    assert_eq!(
        store.mcp_resolved_client_id(Some("unknown-explicit-client")),
        "unknown-explicit-client"
    );
}

#[test]
fn explicit_mcp_identity_never_falls_back_to_a_different_stored_grant() {
    let mut store = test_store();
    store.mcp_http_settings.client_id = "test-client".into();
    for configured in ["missing-reader", "bad\nreader", "removed-reader"] {
        let resolved = store.mcp_resolved_client_id(Some(configured));
        assert_eq!(resolved, configured);
        assert!(!store.mcp_can_read(&resolved, McpScope::ReadLogs, Some("test-session")));
        assert!(!store.mcp_can(&resolved, McpScope::WriteInput, Some("test-session")));
    }
    assert_eq!(store.mcp_resolved_client_id(Some(" readonly ")), "readonly");
    store
        .grants
        .iter_mut()
        .find(|grant| grant.client_id == "readonly")
        .unwrap()
        .revoked_at = Some(Utc::now());
    assert_eq!(store.mcp_resolved_client_id(Some("readonly")), "readonly");
    store
        .grants
        .iter_mut()
        .find(|grant| grant.client_id == "readonly")
        .unwrap()
        .revoked_at = None;
    store
        .grants
        .iter_mut()
        .find(|grant| grant.client_id == "readonly")
        .unwrap()
        .expires_at = Some(Utc::now());
    assert_eq!(store.mcp_resolved_client_id(Some("readonly")), "readonly");
    store.grants.retain(|grant| grant.client_id != "readonly");
    assert_eq!(store.mcp_resolved_client_id(Some("readonly")), "readonly");
    // Unconfigured/legacy clients still follow the desktop-selected identity.
    assert_eq!(store.mcp_resolved_client_id(None), "test-client");
    assert_eq!(
        store.mcp_resolved_client_id(Some(DEFAULT_MCP_HTTP_CLIENT_ID)),
        "test-client"
    );
}

#[test]
fn explicit_no_session_grant_allows_collection_filtering_but_no_session_data() {
    let now = Utc::now();
    let grant = McpGrant {
        client_id: "none-reader".to_string(),
        name: "No session reader".to_string(),
        scopes: vec![McpScope::ReadSessions, McpScope::ReadLogs],
        allowed_sessions: vec![MCP_NO_SESSIONS_SENTINEL.to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    };
    assert!(grant.allows(McpScope::ReadSessions, None, now));
    assert!(!grant.allows(McpScope::ReadSessions, Some("test-session"), now));
    assert!(!grant.allows(McpScope::ReadLogs, Some("test-session"), now));
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

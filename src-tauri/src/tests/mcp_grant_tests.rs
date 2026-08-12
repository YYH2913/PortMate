use super::*;

#[test]
fn empty_mcp_grant_store_requires_trusted_bootstrap() {
    let mut store = SessionStore::default();
    assert!(!mcp_scope_allowed(
        &store,
        "portmate-local",
        false,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(mcp_scope_allowed(
        &store,
        "portmate-local",
        true,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "",
        true,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "bad\nclient",
        true,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_scope_allowed(
        &store,
        &"x".repeat(MAX_MCP_GRANT_CLIENT_ID_BYTES + 1),
        true,
        McpScope::WriteInput,
        "session-1",
    ));

    store.grants.push(McpGrant {
        client_id: "granted-client".to_string(),
        name: "Granted client".to_string(),
        scopes: vec![McpScope::WriteInput],
        allowed_sessions: vec!["session-1".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    assert!(mcp_scope_allowed(
        &store,
        " granted-client ",
        false,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "ungranted-client",
        true,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::WriteInput,
        "session-1",
    ));
    store.grants[0].confirm_writes = true;
    assert!(mcp_write_confirmation_required(
        &store,
        " granted-client ",
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::ReadLogs,
        "session-1",
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::WriteInput,
        "other-session",
    ));
}

#[test]
fn mcp_grant_validation_normalizes_and_rejects_ambiguous_inputs() {
    let grant = McpGrant {
        client_id: "  ops-client  ".to_string(),
        name: "  Operations  ".to_string(),
        scopes: vec![McpScope::ReadSessions, McpScope::WriteInput],
        allowed_sessions: vec!["  edge  ".to_string(), "lab".to_string()],
        confirm_writes: true,
        expires_at: Some("2031-04-05T06:07:00Z".parse().unwrap()),
        revoked_at: None,
    };
    let normalized = normalize_mcp_grant(grant.clone()).unwrap();
    assert_eq!(normalized.client_id, "ops-client");
    assert_eq!(normalized.name, "Operations");
    assert_eq!(normalized.allowed_sessions, ["edge", "lab"]);
    assert_eq!(normalized.expires_at, grant.expires_at);

    let mut invalid = grant.clone();
    invalid.client_id = " \n ".to_string();
    assert!(normalize_mcp_grant(invalid)
        .unwrap_err()
        .contains("client ID"));

    let mut invalid = grant.clone();
    invalid.scopes = vec![McpScope::ReadLogs, McpScope::ReadLogs];
    assert!(normalize_mcp_grant(invalid)
        .unwrap_err()
        .contains("duplicate scopes"));

    let mut invalid = grant;
    invalid.allowed_sessions = vec![" edge ".to_string(), "edge".to_string()];
    assert!(normalize_mcp_grant(invalid)
        .unwrap_err()
        .contains("duplicate session IDs"));
}

#[test]
fn mcp_grant_validation_accepts_the_complete_scope_set() {
    let grant = McpGrant {
        client_id: "complete-client".to_string(),
        name: "Complete client".to_string(),
        scopes: vec![
            McpScope::ReadSessions,
            McpScope::ReadLogs,
            McpScope::ReadTransfers,
            McpScope::ReadTunnels,
            McpScope::WriteInput,
            McpScope::Transfer,
            McpScope::Tunnel,
            McpScope::ManageSessions,
        ],
        allowed_sessions: Vec::new(),
        confirm_writes: true,
        expires_at: None,
        revoked_at: None,
    };

    assert_eq!(normalize_mcp_grant(grant).unwrap().scopes.len(), 8);
    assert_eq!(mcp_scope_label(McpScope::ReadTransfers), "read-transfers");
    assert_eq!(mcp_scope_label(McpScope::ReadTunnels), "read-tunnels");
}

#[test]
fn mcp_grant_mutations_change_memory_only_after_persistence_succeeds() {
    let mut store = SessionStore::default();
    store.grants.push(McpGrant {
        client_id: "ops-client".to_string(),
        name: "Old grant".to_string(),
        scopes: vec![McpScope::ReadSessions],
        allowed_sessions: Vec::new(),
        confirm_writes: true,
        expires_at: None,
        revoked_at: None,
    });
    let updated = McpGrant {
        client_id: "ops-client".to_string(),
        name: "Updated grant".to_string(),
        scopes: vec![McpScope::ReadSessions, McpScope::WriteInput],
        allowed_sessions: vec!["edge".to_string()],
        confirm_writes: true,
        expires_at: None,
        revoked_at: None,
    };
    let before = serde_json::to_value(&store).unwrap();

    let error = commit_store_mutation_with(
        &mut store,
        |next_store| upsert_mcp_grant_in_store(next_store, updated.clone()),
        |next_store| {
            assert_eq!(next_store.grants[0].name, "Updated grant");
            Err("store conflict".to_string())
        },
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "store conflict");
    assert_eq!(serde_json::to_value(&store).unwrap(), before);

    commit_store_mutation_with(
        &mut store,
        |next_store| upsert_mcp_grant_in_store(next_store, updated),
        |_| Ok(()),
        |_| panic!("successful persistence must not be reverified"),
    )
    .unwrap();
    assert_eq!(store.grants[0].name, "Updated grant");

    let error = commit_store_mutation_with(
        &mut store,
        |next_store| Ok(revoke_mcp_grant_from_store(next_store, "ops-client")),
        |_| Err("disk full".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "disk full");
    assert_eq!(store.grants[0].client_id, "ops-client");
}

#[test]
fn mcp_http_settings_change_memory_only_after_persistence_succeeds() {
    let mut store = SessionStore::default();
    let settings = McpHttpSettings {
        listen_host: "0.0.0.0".to_string(),
        client_host: "192.168.33.222".to_string(),
        port: 9888,
        allowed_origins: vec!["https://console.example.test".to_string()],
        client_id: "automation-client".to_string(),
        trusted: true,
        allow_remote: true,
    };
    let before = store.mcp_http_settings.clone();

    let error = commit_store_mutation_with(
        &mut store,
        |next_store| Ok(set_mcp_http_settings_in_store(next_store, settings.clone())),
        |_| Err("store conflict".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "store conflict");
    assert_eq!(store.mcp_http_settings, before);

    commit_store_mutation_with(
        &mut store,
        |next_store| Ok(set_mcp_http_settings_in_store(next_store, settings.clone())),
        |_| Ok(()),
        |_| panic!("successful persistence must not be reverified"),
    )
    .unwrap();
    assert_eq!(store.mcp_http_settings, settings);
}

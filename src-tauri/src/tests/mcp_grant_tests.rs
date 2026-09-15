use super::*;

#[test]
fn mcp_http_settings_precondition_rejects_stale_identity_and_network() {
    let current = McpHttpSettings::default();
    assert!(require_unchanged_mcp_http_settings(&current, Some(&current)).is_ok());
    // The main-window quick start explicitly operates on saved settings.
    assert!(require_unchanged_mcp_http_settings(&current, None).is_ok());
    let mut stale = current.clone();
    stale.client_id = "other-client".to_string();
    assert!(
        require_unchanged_mcp_http_settings(&current, Some(&stale))
            .unwrap_err()
            .contains("其他窗口")
    );
    stale = current.clone();
    stale.listen_host = "0.0.0.0".to_string();
    stale.allow_remote = true;
    assert!(require_unchanged_mcp_http_settings(&current, Some(&stale)).is_err());
    stale = current.clone();
    stale.allowed_origins.clear();
    assert!(require_unchanged_mcp_http_settings(&current, Some(&stale)).is_err());
}

#[test]
fn empty_mcp_grant_store_rejects_caller_claimed_trusted_bootstrap() {
    let mut store = SessionStore::default();
    assert!(!mcp_scope_allowed(
        &store,
        "portmate-local",
        false,
        McpScope::WriteInput,
        Some("session-1"),
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "portmate-local",
        true,
        McpScope::WriteInput,
        Some("session-1"),
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "",
        true,
        McpScope::WriteInput,
        Some("session-1"),
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "bad\nclient",
        true,
        McpScope::WriteInput,
        Some("session-1"),
    ));
    assert!(!mcp_scope_allowed(
        &store,
        &"x".repeat(MAX_MCP_GRANT_CLIENT_ID_BYTES + 1),
        true,
        McpScope::WriteInput,
        Some("session-1"),
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
        Some("session-1"),
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "ungranted-client",
        true,
        McpScope::WriteInput,
        Some("session-1"),
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::WriteInput,
        Some("session-1"),
    ));
    store.grants[0].confirm_writes = true;
    assert!(mcp_write_confirmation_required(
        &store,
        " granted-client ",
        McpScope::WriteInput,
        Some("session-1"),
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::ReadLogs,
        Some("session-1"),
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::WriteInput,
        Some("other-session"),
    ));
    store.grants[0].scopes = vec![McpScope::Tunnel];
    assert!(mcp_scope_allowed(
        &store,
        "granted-client",
        false,
        McpScope::Tunnel,
        None,
    ));
    assert!(mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::Tunnel,
        None,
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

    let no_sessions = McpGrant {
        client_id: "none-client".to_string(),
        name: "No sessions".to_string(),
        scopes: vec![McpScope::ReadSessions],
        allowed_sessions: vec![MCP_NO_SESSIONS_SENTINEL.to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    };
    assert_eq!(
        normalize_mcp_grant(no_sessions.clone())
            .unwrap()
            .allowed_sessions,
        [MCP_NO_SESSIONS_SENTINEL]
    );
    let mut ambiguous = no_sessions;
    ambiguous.allowed_sessions.push("edge".to_string());
    assert!(normalize_mcp_grant(ambiguous)
        .unwrap_err()
        .contains("no-session marker"));
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
            McpScope::ReadScripts,
            McpScope::ReadMcp,
            McpScope::WriteInput,
            McpScope::Transfer,
            McpScope::HostFiles,
            McpScope::Tunnel,
            McpScope::ManageSessions,
            McpScope::RunScripts,
            McpScope::ManageMcp,
        ],
        allowed_sessions: Vec::new(),
        confirm_writes: true,
        expires_at: None,
        revoked_at: None,
    };

    assert_eq!(normalize_mcp_grant(grant).unwrap().scopes.len(), 13);
    assert_eq!(mcp_scope_label(McpScope::ReadTransfers), "read-transfers");
    assert_eq!(mcp_scope_label(McpScope::ReadTunnels), "read-tunnels");
    assert_eq!(mcp_scope_label(McpScope::ReadScripts), "read-scripts");
    assert_eq!(mcp_scope_label(McpScope::RunScripts), "run-scripts");
    assert_eq!(mcp_scope_label(McpScope::HostFiles), "host-files");
    assert_eq!(mcp_scope_label(McpScope::ReadMcp), "read-mcp");
    assert_eq!(mcp_scope_label(McpScope::ManageMcp), "manage-mcp");
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

#[test]
fn mcp_http_requires_an_explicit_active_client_without_guessing() {
    let mut store = SessionStore::default();
    store.grants.push(McpGrant {
        client_id: "single-client".to_string(),
        name: "Single client".to_string(),
        scopes: vec![McpScope::ReadSessions],
        allowed_sessions: Vec::new(),
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    assert!(require_active_mcp_http_client(&store, "single-client").is_ok());
    assert!(require_active_mcp_http_client(&store, "portmate-local").is_err());
    assert_eq!(store.mcp_http_settings.client_id, "portmate-local");

    store.grants.push(McpGrant {
        client_id: "second-client".to_string(),
        name: "Second client".to_string(),
        scopes: vec![McpScope::ReadLogs],
        allowed_sessions: Vec::new(),
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    store.mcp_http_settings.client_id = "single-client".to_string();
    revoke_mcp_grant_from_store(&mut store, "single-client");
    assert!(require_active_mcp_http_client(&store, "single-client").is_err());
    assert_eq!(store.mcp_resolved_client_id(None), "single-client");
    assert!(require_active_mcp_http_client(&store, "second-client").is_ok());
    store.grants[0].expires_at = Some(Utc::now());
    assert!(require_active_mcp_http_client(&store, "second-client").is_err());
    store.grants[0].expires_at = None;
    store.grants[0].revoked_at = Some(Utc::now());
    assert!(require_active_mcp_http_client(&store, "second-client").is_err());
    assert!(require_active_mcp_http_client(&store, "").is_err());
}

#[test]
fn revoked_http_binding_stops_transport_and_deletes_token_without_rebinding() {
    use crate::mcp_commands::finish_mcp_grant_change_with;
    let mut store = SessionStore::default();
    store.mcp_http_settings.client_id = "revoked".into();
    let calls = std::cell::RefCell::new(Vec::new());
    let result = finish_mcp_grant_change_with(&store, "revoked",
        || { calls.borrow_mut().push("stop"); Ok(()) },
        || { calls.borrow_mut().push("delete-token"); Ok(()) });
    assert!(result.http_access_invalidated);
    assert!(result.warnings.is_empty());
    assert_eq!(*calls.borrow(), ["stop", "delete-token"]);
    assert_eq!(store.mcp_http_settings.client_id, "revoked");
    assert!(result.grants.is_empty());
}

#[test]
fn unrelated_revocation_does_not_change_http_transport_or_token() {
    let mut store = SessionStore::default();
    store.mcp_http_settings.client_id = "other".into();
    let result = crate::mcp_commands::finish_mcp_grant_change_with(&store, "revoked",
        || panic!("must not stop another client's service"),
        || panic!("must not delete another client's token"));
    assert!(!result.http_access_invalidated);
    assert!(result.warnings.is_empty());
}

#[test]
fn revocation_reports_cleanup_failures_without_restoring_permissions() {
    let mut store = SessionStore::default();
    store.mcp_http_settings.client_id = "revoked".into();
    let result = crate::mcp_commands::finish_mcp_grant_change_with(&store, "revoked",
        || Err("process busy".into()), || Err("keyring locked".into()));
    assert!(result.http_access_invalidated);
    assert_eq!(result.warnings.len(), 2);
    assert!(result.warnings[0].contains("process busy"));
    assert!(result.warnings[1].contains("keyring locked"));
    assert!(result.grants.is_empty());
    assert!(require_active_mcp_http_client(&store, "revoked").is_err());
}

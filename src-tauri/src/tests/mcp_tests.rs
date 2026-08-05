use super::*;

#[test]
fn invalid_mcp_identifiers_are_bounded_before_audit() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-mcp-invalid-identifiers-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let invalid_client = format!("{}\nsecret-client", "x".repeat(1024));
        let error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: invalid_client.clone(),
                trusted_write: true,
                command: "open_session".to_string(),
                args: serde_json::json!({ "sessionId": "bad\nsession" }),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("session ID"));
        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].actor, "<invalid-client-id>");
        assert_eq!(audit[0].session_id, None);
        let encoded = serde_json::to_string(&audit).unwrap();
        assert!(!encoded.contains("secret-client"));
        assert!(!encoded.contains("bad\\nsession"));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn denied_and_invalid_mcp_writes_are_audited_without_arguments() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-denied-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();

        let denied = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "readonly-client".to_string(),
                trusted_write: false,
                command: "open_session".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "password": "denied-password-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(denied.contains("does not permit"));

        let invalid = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "trusted-client".to_string(),
                trusted_write: true,
                command: "run_command".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "command": 42,
                    "password": "invalid-password-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(invalid.contains("missing string argument `command`"));

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].actor, "readonly-client");
        assert_eq!(audit[0].action, "open_session");
        assert_eq!(audit[0].decision, "denied");
        assert_eq!(audit[0].session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(
            audit[0].details.get("scope").map(String::as_str),
            Some("manage-sessions")
        );
        assert_eq!(audit[1].actor, "trusted-client");
        assert_eq!(audit[1].action, "run_command");
        assert_eq!(audit[1].decision, "invalid");
        assert_eq!(
            audit[1].details.get("scope").map(String::as_str),
            Some("write-input")
        );
        let encoded = serde_json::to_string(&audit).unwrap();
        assert!(!encoded.contains("denied-password-secret"));
        assert!(!encoded.contains("invalid-password-secret"));
        assert!(!encoded.contains("\"command\":42"));

        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn trusted_mcp_input_uses_client_actor_and_exact_tool_audit() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 8];
            socket.read_exact(&mut received).await.unwrap();
            received
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-input-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let stream = TcpStream::connect(address).await.unwrap();
        let (_reader, writer) = stream.into_split();
        let (tap, _) = broadcast::channel(8);
        state.tcp.lock().unwrap().insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: Uuid::new_v4().to_string(),
                writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
                tap,
                closed: Arc::new(AtomicBool::new(false)),
                telnet: None,
            },
        );

        let key_event: SessionEvent = serde_json::from_value(
            handle_ipc_request(
                state.clone(),
                IpcRequest {
                    token: "authenticated-token".to_string(),
                    client_id: "mcp-e2e-client".to_string(),
                    trusted_write: true,
                    command: "send_key".to_string(),
                    args: serde_json::json!({
                        "sessionId": profile.id,
                        "key": "Enter"
                    }),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();
        let command_event: SessionEvent = serde_json::from_value(
            handle_ipc_request(
                state.clone(),
                IpcRequest {
                    token: "authenticated-token".to_string(),
                    client_id: "mcp-e2e-client".to_string(),
                    trusted_write: true,
                    command: "run_command".to_string(),
                    args: serde_json::json!({
                        "sessionId": profile.id,
                        "command": "status"
                    }),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();

        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("TCP server timed out")
            .expect("TCP server failed");
        assert_eq!(&received, b"\rstatus\n");
        for event in [&key_event, &command_event] {
            assert_eq!(
                event.annotations.get("actor").map(String::as_str),
                Some("mcp-e2e-client")
            );
        }
        assert!(!key_event.annotations.contains_key("commandId"));
        assert!(command_event.annotations.contains_key("commandId"));
        assert_eq!(
            command_event
                .annotations
                .get("commandState")
                .map(String::as_str),
            Some("started")
        );

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(
            audit.len(),
            2,
            "MCP input must not add implicit send_text audits"
        );
        assert_eq!(audit[0].action, "send_key");
        assert_eq!(audit[1].action, "run_command");
        assert!(audit.iter().all(|record| record.actor == "mcp-e2e-client"));
        assert!(audit.iter().all(|record| record.decision == "succeeded"));
        assert!(audit.iter().all(|record| {
            record.details.get("trustedBootstrap").map(String::as_str) == Some("true")
        }));
        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn failed_mcp_write_finalizes_audit_without_secret_arguments() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-failed-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        let error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "failed-client".to_string(),
                trusted_write: true,
                command: "run_command".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "command": "password=failed-command-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(!error.is_empty());
        assert!(!error.contains("failed-command-secret"));

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].actor, "failed-client");
        assert_eq!(audit[0].action, "run_command");
        assert_eq!(audit[0].decision, "failed");
        assert!(!serde_json::to_string(&audit)
            .unwrap()
            .contains("failed-command-secret"));
        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn applied_mcp_audit_keeps_final_truth_when_persistence_fails() {
    let mut store = SessionStore::default();
    store.record_audit(AuditRecord {
        id: "audit-applied".to_string(),
        ts: Utc::now(),
        actor: "mcp:test-client".to_string(),
        action: "send_text".to_string(),
        session_id: Some("session-1".to_string()),
        decision: "authorized".to_string(),
        details: BTreeMap::new(),
    });

    let error = finish_applied_mcp_write_audit_with(
        &mut store,
        "audit-applied",
        "succeeded",
        Some("approved"),
        |_| Err("disk full".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();

    assert_eq!(error, "disk full");
    let audit = store
        .audit
        .iter()
        .find(|record| record.id == "audit-applied")
        .unwrap();
    assert_eq!(audit.decision, "succeeded");
    assert_eq!(
        audit.details.get("approval").map(String::as_str),
        Some("approved")
    );
    assert_eq!(
        audit
            .details
            .get("finalizationPersistence")
            .map(String::as_str),
        Some("degraded")
    );
}

#[test]
fn applied_mcp_audit_accepts_a_verified_post_commit_error() {
    let mut store = SessionStore::default();
    store.record_audit(AuditRecord {
        id: "audit-verified".to_string(),
        ts: Utc::now(),
        actor: "mcp:test-client".to_string(),
        action: "run_command".to_string(),
        session_id: Some("session-1".to_string()),
        decision: "authorized".to_string(),
        details: BTreeMap::new(),
    });

    finish_applied_mcp_write_audit_with(
        &mut store,
        "audit-verified",
        "failed",
        None,
        |_| Err("post-commit version read failed".to_string()),
        |_| Ok(true),
    )
    .unwrap();

    let audit = store
        .audit
        .iter()
        .find(|record| record.id == "audit-verified")
        .unwrap();
    assert_eq!(audit.decision, "failed");
    assert!(!audit.details.contains_key("finalizationPersistence"));
}

#[test]
fn log_query_limit_matches_mcp_schema_bounds() {
    assert_eq!(bounded_log_query_limit(None), 100);
    assert_eq!(bounded_log_query_limit(Some(0)), 1);
    assert_eq!(bounded_log_query_limit(Some(600)), 600);
    assert_eq!(bounded_log_query_limit(Some(u64::MAX)), 1000);
}

#[test]
fn mcp_http_config_uses_bridge_token_ref_and_loopback_endpoint() {
    let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
    let store_path = Path::new("/home/operator/PortMate Data/portmate-store.sqlite3");
    let config =
        build_mcp_http_config_for_request(true, executable, store_path, McpHttpSettings::default())
            .unwrap();
    assert_eq!(config.token_ref, MCP_HTTP_TOKEN_REF);
    assert_eq!(config.endpoint, "http://127.0.0.1:8787/mcp");
    assert_eq!(config.settings, McpHttpSettings::default());
    assert!(!config.remote_access);
    assert!(config.token_available);
    assert!(config.start_command.contains("PORTMATE_MCP_HTTP=1"));
    assert_eq!(config.executable, executable.to_string_lossy());
    assert_eq!(config.store_path, store_path.to_string_lossy());
    assert!(config
        .start_command
        .contains("/opt/PortMate/bin/portmate-mcp"));
    assert!(config
        .start_command
        .contains("'/home/operator/PortMate Data/portmate-store.sqlite3'"));
    assert!(!config.start_command.contains("cargo run"));
    assert!(!config.start_command.contains(MCP_HTTP_TOKEN_REF));
    assert!(!config.start_command.contains("keychain:"));
    assert!(!config.start_command.contains("PORTMATE_MCP_HTTP_TOKEN"));
    assert!(!config.start_command.contains("example-token-body"));
    #[cfg(not(windows))]
    assert_eq!(
        config.start_command,
        "PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3' PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787' PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787,http://localhost:8787' PORTMATE_MCP_CLIENT_ID='portmate-local' PORTMATE_MCP_HTTP_ALLOW_REMOTE=0 PORTMATE_MCP_TRUSTED=0 '/opt/PortMate/bin/portmate-mcp' --http"
    );
    #[cfg(windows)]
    assert_eq!(
        config.start_command,
        "$env:PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3'; $env:PORTMATE_MCP_HTTP='1'; $env:PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787'; $env:PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787,http://localhost:8787'; $env:PORTMATE_MCP_CLIENT_ID='portmate-local'; $env:PORTMATE_MCP_HTTP_ALLOW_REMOTE='0'; $env:PORTMATE_MCP_TRUSTED='0'; & '/opt/PortMate/bin/portmate-mcp' --http"
    );
}

#[test]
fn mcp_http_config_supports_explicit_remote_listeners_and_validates_origins() {
    let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
    let store_path = Path::new("/tmp/portmate-store.sqlite3");
    let remote = McpHttpSettings {
        listen_host: "0.0.0.0".to_string(),
        port: 9888,
        allowed_origins: vec!["https://console.example.test".to_string()],
        client_id: "automation-client".to_string(),
        trusted: true,
        allow_remote: true,
    };
    let config = build_mcp_http_config_for_request(false, executable, store_path, remote).unwrap();
    assert_eq!(config.endpoint, "http://0.0.0.0:9888/mcp");
    assert!(config.remote_access);
    assert!(config
        .start_command
        .contains("PORTMATE_MCP_HTTP_ALLOW_REMOTE"));
    assert!(config.start_command.contains("PORTMATE_MCP_TRUSTED"));
    assert!(config.start_command.contains("automation-client"));
    assert!(config.start_command.contains("console.example.test"));

    let mut denied = config.settings.clone();
    denied.allow_remote = false;
    assert!(normalize_mcp_http_settings(denied)
        .unwrap_err()
        .contains("explicit remote access"));

    let mut invalid_origin = McpHttpSettings::default();
    invalid_origin.allowed_origins = vec!["https://console.example.test/path".to_string()];
    assert!(normalize_mcp_http_settings(invalid_origin)
        .unwrap_err()
        .contains("scheme and authority"));

    let mut invalid_scheme = McpHttpSettings::default();
    invalid_scheme.allowed_origins = vec!["ftp://console.example.test".to_string()];
    assert!(normalize_mcp_http_settings(invalid_scheme)
        .unwrap_err()
        .contains("HTTP(S)"));

    let mut loopback = McpHttpSettings::default();
    loopback.allow_remote = true;
    assert!(
        !normalize_mcp_http_settings(loopback)
            .unwrap()
            .0
            .allow_remote
    );
}

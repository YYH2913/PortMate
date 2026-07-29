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
fn mcp_ipc_reads_enforce_grants_and_reject_unlisted_commands() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!("portmate-mcp-read-scope-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "scoped-reader".to_string(),
            name: "Scoped reader".to_string(),
            scopes: vec![McpScope::ReadSessions],
            allowed_sessions: vec![session_id.clone()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let sessions = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "scoped-reader".to_string(),
                trusted_write: false,
                command: "list_sessions".to_string(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        assert_eq!(sessions.as_array().unwrap().len(), 1);

        let read_screen = || IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "scoped-reader".to_string(),
            trusted_write: false,
            command: "read_screen".to_string(),
            args: serde_json::json!({ "sessionId": session_id }),
        };
        assert!(handle_ipc_request(state.clone(), read_screen())
            .await
            .unwrap_err()
            .contains("ReadLogs"));
        assert!(handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "unknown-reader".to_string(),
                trusted_write: false,
                command: "list_sessions".to_string(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap_err()
        .contains("ReadSessions"));
        assert!(handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "bad\nreader".to_string(),
                trusted_write: false,
                command: "list_sessions".to_string(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap_err()
        .contains("ReadSessions"));
        assert!(handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "scoped-reader".to_string(),
                trusted_write: false,
                command: "list_files".to_string(),
                args: serde_json::json!({ "path": "/" }),
            },
        )
        .await
        .unwrap_err()
        .contains("unsupported IPC command"));

        state.store.lock().unwrap().grants[0]
            .scopes
            .push(McpScope::ReadLogs);
        assert!(handle_ipc_request(state.clone(), read_screen())
            .await
            .is_ok());
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn mcp_ipc_reads_redact_profiles_and_complete_event_metadata() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-read-redaction-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = {
            let mut store = state.store.lock().unwrap();
            let session_id = store.profiles[0].id.clone();
            let ConnectionConfig::Shell(shell) = &mut store.profiles[0].connection else {
                panic!("test profile should use a shell connection");
            };
            shell.args = vec!["--password".to_string(), "opaque-shell-secret".to_string()];
            shell.cwd = Some("/home/operator/private-shell-cwd".to_string());
            store.profiles[0].logging.path_template =
                "/home/operator/private-logs/{session}.raw".to_string();
            store.profiles[0].transfer.default_local_dir =
                Some("/home/operator/private-downloads".to_string());
            store.runtimes[0].cwd = Some("/home/operator/runtime-cwd".to_string());
            store
                .record_event(
                    &session_id,
                    EventDirection::Inbound,
                    EventStream::Stdout,
                    Some("password=event-secret".to_string()),
                    Some("v2:/home/operator/private-logs/raw:0:12:digest".to_string()),
                    BTreeMap::from([(
                        "diagnostic".to_string(),
                        "token=annotation-secret".to_string(),
                    )]),
                )
                .unwrap();
            session_id
        };
        let raw_store = serde_json::to_string(&*state.store.lock().unwrap()).unwrap();
        let request = |command: &str, args: serde_json::Value| IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "redaction-reader".to_string(),
            trusted_write: false,
            command: command.to_string(),
            args,
        };

        let surfaces = [
            handle_ipc_request(
                state.clone(),
                request("list_sessions", serde_json::json!({})),
            )
            .await
            .unwrap(),
            handle_ipc_request(
                state.clone(),
                request(
                    "read_screen",
                    serde_json::json!({ "sessionId": session_id }),
                ),
            )
            .await
            .unwrap(),
            handle_ipc_request(
                state.clone(),
                request("tail_log", serde_json::json!({ "sessionId": session_id })),
            )
            .await
            .unwrap(),
            handle_ipc_request(
                state.clone(),
                request(
                    "search_logs",
                    serde_json::json!({
                        "query": "password",
                        "sessionId": session_id
                    }),
                ),
            )
            .await
            .unwrap(),
        ];
        let encoded = surfaces
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for sensitive in [
            "opaque-shell-secret",
            "/home/operator/private-shell-cwd",
            "/home/operator/private-logs/{session}.raw",
            "/home/operator/private-downloads",
            "/home/operator/runtime-cwd",
            "event-secret",
            "annotation-secret",
            "v2:/home/operator/private-logs/raw:0:12:digest",
        ] {
            assert!(
                !encoded.contains(sensitive),
                "IPC response leaked {sensitive}"
            );
        }
        assert!(encoded.contains("<redacted>"));
        assert!(encoded.contains("Bench/Device"));
        assert_eq!(
            serde_json::to_string(&*state.store.lock().unwrap()).unwrap(),
            raw_store
        );
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
    let config = build_mcp_http_config(true, executable, store_path);
    assert_eq!(config.token_ref, MCP_HTTP_TOKEN_REF);
    assert_eq!(config.endpoint, "http://127.0.0.1:8787/mcp");
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
        "PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3' PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR=127.0.0.1:8787 PORTMATE_MCP_HTTP_ORIGINS=http://127.0.0.1:8787 '/opt/PortMate/bin/portmate-mcp' --http"
    );
    #[cfg(windows)]
    assert_eq!(
        config.start_command,
        "$env:PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3'; $env:PORTMATE_MCP_HTTP='1'; $env:PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787'; $env:PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787'; & '/opt/PortMate/bin/portmate-mcp' --http"
    );
}

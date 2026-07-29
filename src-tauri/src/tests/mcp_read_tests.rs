use super::*;

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

use super::*;

#[test]
fn mcp_log_search_applies_limit_after_session_authorization() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
        let visible_id = state.store.lock().unwrap().profiles[0].id.clone();
        {
            let mut store = state.store.lock().unwrap();
            let mut hidden = store.profiles[0].clone();
            hidden.id = "hidden-session".into();
            store.upsert_profile(hidden);
            store.events.clear();
            for (session, text) in [
                (visible_id.as_str(), "match old"),
                (visible_id.as_str(), "match new password=hidden-value"),
                ("hidden-session", "match hidden one"),
                ("hidden-session", "match hidden two"),
                ("hidden-session", "match hidden three"),
            ] {
                store
                    .record_stream_event(
                        session,
                        EventDirection::Inbound,
                        EventStream::Stdout,
                        text,
                    )
                    .unwrap();
            }
        }
        grant_test_mcp_access(
            &state,
            "search-reader",
            vec![McpScope::ReadLogs],
            vec![visible_id.clone()],
        );
        let request = |args| IpcRequest {
            token: "authenticated-token".into(),
            client_id: "search-reader".into(),
            trusted_write: false,
            command: "search_logs".into(),
            args,
        };
        for limit in [1, 2] {
            let response = handle_ipc_request(
                state.clone(),
                request(serde_json::json!({"query":"MATCH", "limit":limit})),
            )
            .await
            .unwrap();
            let events: Vec<SessionEvent> = serde_json::from_value(response).unwrap();
            assert_eq!(
                events.len(),
                limit,
                "unauthorized events consumed the search limit"
            );
            assert!(events.iter().all(|event| event.session_id == visible_id));
            assert!(events
                .last()
                .unwrap()
                .text
                .as_ref()
                .unwrap()
                .contains("match new"));
            assert!(!serde_json::to_string(&events)
                .unwrap()
                .contains("hidden-value"));
            if limit == 2 {
                assert_eq!(events[0].text.as_deref(), Some("match old"));
            }
        }
        assert!(handle_ipc_request(
            state.clone(),
            request(serde_json::json!({"query":"match", "sessionId":"hidden-session"}))
        )
        .await
        .unwrap_err()
        .contains("ReadLogs"));
        state.store.lock().unwrap().grants.clear();
        assert!(
            handle_ipc_request(state, request(serde_json::json!({"query":"match"})))
                .await
                .unwrap_err()
                .contains("ReadLogs")
        );
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
        grant_test_mcp_access(
            &state,
            "redaction-reader",
            vec![McpScope::ReadSessions, McpScope::ReadLogs],
            vec![session_id.clone()],
        );
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
fn mcp_transfer_reads_are_scoped_limited_and_path_redacted() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-transfer-read-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        let transfer = |id: &str, source: &str| TransferTask {
            id: id.to_string(),
            session_id: session_id.clone(),
            protocol: TransferProtocol::Sftp,
            source: source.to_string(),
            destination: format!("remote:/srv/{id}"),
            bytes_total: 10,
            bytes_done: 10,
            status: TransferStatus::Completed,
            message: Some(format!("token={id}-secret")),
            started_at: None,
            finished_at: None,
            average_bytes_per_second: None,
        };
        {
            let mut store = state.store.lock().unwrap();
            store.record_transfer(transfer("old-transfer", "/home/operator/private-old"));
            store.record_transfer(transfer("new-transfer", "/home/operator/private-new"));
            store.grants.push(McpGrant {
                client_id: "transfer-reader".to_string(),
                name: "Transfer reader".to_string(),
                scopes: vec![McpScope::ReadTransfers],
                allowed_sessions: vec![session_id.clone()],
                confirm_writes: false,
                expires_at: None,
                revoked_at: None,
            });
        }
        let request = |command: &str, args: serde_json::Value| IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "transfer-reader".to_string(),
            trusted_write: false,
            command: command.to_string(),
            args,
        };

        let listed = handle_ipc_request(
            state.clone(),
            request(
                "list_transfers",
                serde_json::json!({ "sessionId": session_id, "limit": 1 }),
            ),
        )
        .await
        .unwrap();
        let listed = listed.as_array().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["id"], "new-transfer");
        assert_eq!(listed[0]["source"], "<redacted-path>");
        assert_eq!(listed[0]["destination"], "<redacted-path>");
        assert_eq!(listed[0]["message"], "completed");

        let one = handle_ipc_request(
            state.clone(),
            request(
                "get_transfer",
                serde_json::json!({ "transferId": "old-transfer" }),
            ),
        )
        .await
        .unwrap();
        assert_eq!(one["source"], "<redacted-path>");
        assert_eq!(one["destination"], "<redacted-path>");

        state.store.lock().unwrap().grants[0].scopes = vec![McpScope::ReadLogs];
        assert!(handle_ipc_request(
            state.clone(),
            request("list_transfers", serde_json::json!({})),
        )
        .await
        .unwrap_err()
        .contains("ReadTransfers"));

        let raw = serde_json::to_string(&*state.store.lock().unwrap()).unwrap();
        assert!(raw.contains("/home/operator/private-old"));
        assert!(!serde_json::to_string(&listed)
            .unwrap()
            .contains("/home/operator"));
        let _ = fs::remove_dir_all(root);
    });
}

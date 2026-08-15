use super::*;
use crate::custom_script_commands::{
    delete_custom_script_from_store, normalize_custom_script_request, run_custom_script_inner,
    upsert_custom_script_in_store,
};

fn save_request(session_id: &str) -> SaveCustomScriptRequest {
    SaveCustomScriptRequest {
        id: None,
        name: "  Inspect service  ".to_string(),
        description: "  Read state  ".to_string(),
        content: "uptime\r\nwhoami\r".to_string(),
        allow_all_sessions: false,
        allowed_session_ids: vec![session_id.to_string(), session_id.to_string()],
        mcp_enabled: true,
        expected_updated_at: None,
    }
}

fn stored_script(session_id: &str, mcp_enabled: bool) -> CustomScript {
    let timestamp = "2026-08-14T00:00:00Z".parse().unwrap();
    CustomScript {
        id: "69c06a07-dc48-4d4e-9498-6f42b6deab21".to_string(),
        name: "Inspect service".to_string(),
        description: "Read state".to_string(),
        content: "private-script-body-marker".to_string(),
        allow_all_sessions: false,
        allowed_session_ids: vec![session_id.to_string()],
        mcp_enabled,
        created_at: timestamp,
        updated_at: timestamp,
    }
}

#[test]
fn custom_script_mutations_normalize_and_enforce_optimistic_concurrency() {
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    let created_at = "2026-08-14T01:00:00Z".parse().unwrap();
    let script =
        normalize_custom_script_request(&store, save_request(&session_id), created_at).unwrap();
    assert_eq!(script.name, "Inspect service");
    assert_eq!(script.description, "Read state");
    assert_eq!(script.content, "uptime\nwhoami\n");
    assert_eq!(script.allowed_session_ids, [session_id.as_str()]);
    upsert_custom_script_in_store(&mut store, script.clone()).unwrap();

    let mut stale = save_request(&session_id);
    stale.id = Some(script.id.clone());
    stale.expected_updated_at = Some("2026-08-14T00:59:59Z".parse().unwrap());
    assert!(normalize_custom_script_request(
        &store,
        stale,
        "2026-08-14T01:01:00Z".parse().unwrap(),
    )
    .unwrap_err()
    .contains("changed in another window"));

    assert!(delete_custom_script_from_store(
        &mut store,
        &DeleteCustomScriptRequest {
            id: script.id.clone(),
            expected_updated_at: "2026-08-14T00:59:59Z".parse().unwrap(),
        },
    )
    .unwrap_err()
    .contains("changed in another window"));
    delete_custom_script_from_store(
        &mut store,
        &DeleteCustomScriptRequest {
            id: script.id,
            expected_updated_at: script.updated_at,
        },
    )
    .unwrap();
    assert!(store.custom_scripts.is_empty());
}

#[test]
fn deleting_profile_targets_never_widens_a_custom_script_boundary() {
    let mut store = SessionStore::default();
    let first = test_shell_profile();
    let mut second = first.clone();
    second.id = "session:2".to_string();
    second.name = "Second session".to_string();
    store.upsert_profile(first.clone());
    store.upsert_profile(second.clone());
    let mut scoped = stored_script(&first.id, true);
    scoped.allowed_session_ids.push(second.id.clone());
    store.custom_scripts.push(scoped);

    store.delete_profile(&first.id).unwrap();
    assert_eq!(
        store.custom_scripts[0].allowed_session_ids,
        [second.id.as_str()]
    );
    assert!(store.custom_scripts[0].mcp_enabled);
    assert!(!store.custom_scripts[0].allow_all_sessions);

    store.delete_profile(&second.id).unwrap();
    assert!(store.custom_scripts[0].allowed_session_ids.is_empty());
    assert!(!store.custom_scripts[0].mcp_enabled);
    assert!(!store.custom_scripts[0].allow_all_sessions);
}

#[test]
fn mcp_custom_script_listing_is_scoped_and_redacted() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!("portmate-script-list-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        {
            let mut store = state.store.lock().unwrap();
            store.custom_scripts.push(stored_script(&session_id, true));
            let mut desktop_only = stored_script(&session_id, false);
            desktop_only.id = "599b2954-60bf-4f81-bb38-a3af45b0cbf0".to_string();
            desktop_only.name = "Desktop only".to_string();
            store.custom_scripts.push(desktop_only);
            store.grants.push(McpGrant {
                client_id: "script-reader".to_string(),
                name: "Script reader".to_string(),
                scopes: vec![McpScope::ReadScripts],
                allowed_sessions: vec![session_id.clone()],
                confirm_writes: false,
                expires_at: None,
                revoked_at: None,
            });
        }

        let value = execute_ipc_request(
            state,
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "script-reader".to_string(),
                trusted_write: false,
                command: "list_custom_scripts".to_string(),
                args: serde_json::json!({ "sessionId": session_id }),
            },
        )
        .await
        .unwrap();
        let encoded = value.to_string();
        assert!(encoded.contains("Inspect service"));
        assert!(!encoded.contains("Desktop only"));
        assert!(!encoded.contains("private-script-body-marker"));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn mcp_custom_script_execution_revalidates_script_and_grant_boundaries() {
    let root = std::env::temp_dir().join(format!("portmate-script-run-{}", Uuid::new_v4()));
    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let session_id = state.store.lock().unwrap().profiles[0].id.clone();
    let script = stored_script(&session_id, true);
    let request = IpcRequest {
        token: "authenticated-token".to_string(),
        client_id: "script-runner".to_string(),
        trusted_write: false,
        command: "run_custom_script".to_string(),
        args: serde_json::json!({
            "sessionId": session_id.clone(),
            "scriptId": script.id.clone(),
        }),
    };
    {
        let mut store = state.store.lock().unwrap();
        store.custom_scripts.push(script.clone());
        store.grants.push(McpGrant {
            client_id: request.client_id.clone(),
            name: "Script runner".to_string(),
            scopes: vec![McpScope::RunScripts],
            allowed_sessions: vec![session_id.clone()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });
    }
    validate_ipc_write_args(&state, &request).unwrap();
    assert!(mcp_scope_allowed(
        &state.store.lock().unwrap(),
        &request.client_id,
        false,
        McpScope::RunScripts,
        &session_id,
    ));
    let details = mcp_audit_details(&request, McpScope::RunScripts, false, false);
    assert_eq!(
        details.get("scriptId").map(String::as_str),
        Some(script.id.as_str())
    );

    let execution_context = capture_mcp_write_execution_context(&state, &request).unwrap();
    let approval = build_mcp_approval_request_with_target(
        &request.client_id,
        &request.command,
        &session_id,
        McpScope::RunScripts,
        execution_context.approval_target(),
    )
    .unwrap();
    let approval_json = serde_json::to_value(&approval).unwrap();
    assert_eq!(approval_json["target"]["kind"], "custom-script");
    assert_eq!(approval_json["target"]["id"], script.id);
    assert_eq!(approval_json["target"]["label"], script.name);
    assert!(!approval_json.to_string().contains(&script.content));

    state.store.lock().unwrap().custom_scripts[0].updated_at =
        script.updated_at + chrono::Duration::seconds(1);
    assert!(revalidate_ipc_write_target_with_context(
        &state,
        &request,
        McpScope::RunScripts,
        &session_id,
        false,
        &execution_context,
    )
    .unwrap_err()
    .contains("changed after authorization"));
    assert!(tauri::async_runtime::block_on(run_custom_script_inner(
        &state,
        RunCustomScriptRequest {
            script_id: script.id.clone(),
            session_id: session_id.clone(),
        },
        Some(script.updated_at),
        "script-runner",
        None,
        true,
    ))
    .unwrap_err()
    .contains("changed after authorization"));
    state.store.lock().unwrap().custom_scripts[0].updated_at = script.updated_at;

    state.store.lock().unwrap().custom_scripts[0].mcp_enabled = false;
    assert!(revalidate_ipc_write_target(
        &state,
        &request,
        McpScope::RunScripts,
        &session_id,
        false,
    )
    .unwrap_err()
    .contains("not exposed to MCP"));

    state.store.lock().unwrap().custom_scripts[0].mcp_enabled = true;
    state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());
    assert!(revalidate_ipc_write_target(
        &state,
        &request,
        McpScope::RunScripts,
        &session_id,
        false,
    )
    .unwrap_err()
    .contains("grant changed"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn custom_script_execution_keeps_the_body_out_of_structured_surfaces() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected_wire = b"private-script-body-marker\n".to_vec();
        let server_expected = expected_wire.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = vec![0_u8; server_expected.len()];
            socket.read_exact(&mut received).await.unwrap();
            received
        });

        let mut profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        profile.logging.enabled = true;
        profile.logging.raw = false;
        profile.logging.text = true;
        profile.logging.jsonl = true;
        let root = std::env::temp_dir().join(format!("portmate-script-wire-{}", Uuid::new_v4()));
        let store_path = root.join("portmate-store.sqlite3");
        let state = test_app_state(profile.clone(), store_path.clone());
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
        let script = stored_script(&profile.id, true);
        state
            .store
            .lock()
            .unwrap()
            .custom_scripts
            .push(script.clone());

        let event = run_custom_script_inner(
            &state,
            RunCustomScriptRequest {
                script_id: script.id.clone(),
                session_id: profile.id.clone(),
            },
            Some(script.updated_at),
            "desktop-user",
            Some("run_custom_script"),
            false,
        )
        .await
        .unwrap();

        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("custom script TCP server timed out")
            .expect("custom script TCP server failed");
        assert_eq!(received, expected_wire);
        assert_eq!(event.text.as_deref(), Some(CUSTOM_SCRIPT_EVENT_TEXT));
        assert_eq!(
            event.annotations.get("customScriptId").map(String::as_str),
            Some(script.id.as_str())
        );
        assert!(!serde_json::to_string(&event)
            .unwrap()
            .contains("private-script-body-marker"));

        {
            let store = state.store.lock().unwrap();
            assert_eq!(
                store.screen(&profile.id).as_deref(),
                Some(CUSTOM_SCRIPT_EVENT_TEXT)
            );
            assert_eq!(
                store.summaries()[0].last_line.as_deref(),
                Some(CUSTOM_SCRIPT_EVENT_TEXT)
            );
            let audit = store
                .audit
                .iter()
                .find(|record| record.action == "run_custom_script")
                .unwrap();
            assert_eq!(
                audit.details.get("bytes").cloned(),
                Some(expected_wire.len().to_string())
            );
            assert!(store.events.iter().all(|stored| {
                !stored
                    .text
                    .as_deref()
                    .unwrap_or_default()
                    .contains("private-script-body-marker")
            }));
        }

        for extension in ["txt", "jsonl"] {
            let log = fs::read_to_string(log_shard_path(&store_path, &profile, extension).unwrap())
                .unwrap();
            assert!(log.contains(CUSTOM_SCRIPT_EVENT_TEXT));
            assert!(!log.contains("private-script-body-marker"));
        }
        let persisted = load_store_sqlite(&store_path).unwrap();
        assert_eq!(
            persisted
                .events
                .last()
                .and_then(|event| event.text.as_deref()),
            Some(CUSTOM_SCRIPT_EVENT_TEXT)
        );
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn custom_script_fallback_lists_only_authorized_summaries_and_never_executes() {
    let mut store = test_snapshot_store("script session");
    let timestamp = "2026-08-14T00:00:00Z".parse().unwrap();
    let visible_id = "69c06a07-dc48-4d4e-9498-6f42b6deab21";
    store.custom_scripts.push(portmate_core::CustomScript {
        id: visible_id.to_string(),
        name: "Inspect service".to_string(),
        description: "Read service state".to_string(),
        content: "private-script-body-marker".to_string(),
        allow_all_sessions: false,
        allowed_session_ids: vec!["refresh-session".to_string()],
        mcp_enabled: true,
        created_at: timestamp,
        updated_at: timestamp,
    });
    store.custom_scripts.push(portmate_core::CustomScript {
        id: "599b2954-60bf-4f81-bb38-a3af45b0cbf0".to_string(),
        name: "Desktop only".to_string(),
        description: String::new(),
        content: "hidden-script-body-marker".to_string(),
        allow_all_sessions: true,
        allowed_session_ids: Vec::new(),
        mcp_enabled: false,
        created_at: timestamp,
        updated_at: timestamp,
    });
    let mut legacy_event = store
        .record_stream_event(
            "refresh-session",
            portmate_core::EventDirection::Outbound,
            portmate_core::EventStream::Stdout,
            "private-script-body-marker",
        )
        .unwrap();
    legacy_event
        .annotations
        .insert("customScriptId".to_string(), visible_id.to_string());
    *store.events.last_mut().unwrap() = legacy_event;
    store.grants.push(portmate_core::McpGrant {
        client_id: "script-client".to_string(),
        name: "Script client".to_string(),
        scopes: vec![McpScope::ReadScripts, McpScope::ReadLogs],
        allowed_sessions: vec!["refresh-session".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    let mut server = PortMateMcp {
        store: prepare_loaded_store(store).unwrap(),
        store_path: None,
        ipc: None,
        client_id: "script-client".to_string(),
        allow_write: false,
    };

    let listed = server
        .tool_call(&json!({
            "name": "list_custom_scripts",
            "arguments": { "sessionId": "refresh-session" }
        }))
        .unwrap();
    let listed = listed["content"][0]["text"].as_str().unwrap();
    assert!(listed.contains("Inspect service"));
    assert!(!listed.contains("Desktop only"));
    assert!(!listed.contains("private-script-body-marker"));
    assert!(!listed.contains("hidden-script-body-marker"));

    let screen = server
        .tool_call(&json!({
            "name": "read_screen",
            "arguments": { "sessionId": "refresh-session" }
        }))
        .unwrap();
    let screen = screen["content"][0]["text"].as_str().unwrap();
    assert!(screen.contains(portmate_core::CUSTOM_SCRIPT_EVENT_TEXT));
    assert!(!screen.contains("private-script-body-marker"));

    server.store.grants[0].scopes = vec![McpScope::RunScripts];
    assert!(server
        .tool_call(&json!({
            "name": "list_custom_scripts",
            "arguments": { "sessionId": "refresh-session" }
        }))
        .is_ok());
    let run = server
        .tool_call(&json!({
            "name": "run_custom_script",
            "arguments": {
                "sessionId": "refresh-session",
                "scriptId": visible_id
            }
        }))
        .unwrap();
    assert_eq!(run["isError"], true);
    assert!(run["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("desktop IPC is not available"));
    assert!(!run.to_string().contains("private-script-body-marker"));

    server.store.grants[0].scopes.clear();
    assert!(server
        .tool_call(&json!({
            "name": "list_custom_scripts",
            "arguments": { "sessionId": "refresh-session" }
        }))
        .unwrap_err()
        .to_string()
        .contains("ReadScripts"));
}

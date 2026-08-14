use super::*;
use crate::custom_script_commands::{
    delete_custom_script_from_store, normalize_custom_script_request, upsert_custom_script_in_store,
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

use super::*;
use crate::custom_script_commands::{
    delete_custom_script_from_store, normalize_custom_script_request, upsert_custom_script_in_store,
};
use crate::host_script_commands::{run_host_script_inner, RunHostScriptRequest};
use portmate_core::{
    HostScriptConfig, HostScriptLanguage, HostScriptParameter, HostScriptParameterKind,
};

fn config() -> HostScriptConfig {
    HostScriptConfig {
        language: HostScriptLanguage::Python,
        interpreter: String::new(),
        working_directory: String::new(),
        timeout_seconds: 5,
        allowed_client_ids: vec!["script-client".into()],
        parameters: vec![],
    }
}
fn stored_script() -> CustomScript {
    let now = Utc::now();
    CustomScript {
        id: Uuid::new_v4().to_string(),
        name: "Host diagnostics".into(),
        description: "Local diagnostics".into(),
        content: "print('host-ok')".into(),
        host: config(),
        mcp_enabled: true,
        created_at: now,
        updated_at: now,
    }
}
fn grant() -> McpGrant {
    McpGrant {
        client_id: "script-client".into(),
        name: "Script client".into(),
        scopes: vec![McpScope::RunScripts],
        allowed_sessions: vec![MCP_NO_SESSIONS_SENTINEL.into()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    }
}
fn run_request(script: &CustomScript) -> RunHostScriptRequest {
    RunHostScriptRequest {
        script_id: script.id.clone(),
        expected_updated_at: script.updated_at,
        run_id: Uuid::new_v4().to_string(),
        parameters: serde_json::json!({}),
    }
}

#[test]
fn host_script_save_versions_client_scope_and_storage() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("store.sqlite3");
    let mut store = SessionStore::default();
    store.grants.push(grant());
    let script = stored_script();
    store.custom_scripts.push(script.clone());
    let request = SaveCustomScriptRequest {
        id: Some(script.id.clone()),
        name: " Host diagnostics ".into(),
        description: String::new(),
        content: "print('one')\r\n".into(),
        host: config(),
        mcp_enabled: true,
        expected_updated_at: Some(script.updated_at),
    };
    for delta in [0, -3600] {
        let saved = normalize_custom_script_request(
            &store,
            request.clone(),
            script.updated_at + chrono::Duration::seconds(delta),
        )
        .unwrap();
        assert!(saved.updated_at > script.updated_at);
        assert_eq!(saved.content, "print('one')\n");
    }
    let saved =
        normalize_custom_script_request(&store, request.clone(), script.updated_at).unwrap();
    upsert_custom_script_in_store(&mut store, saved.clone()).unwrap();
    assert!(normalize_custom_script_request(&store, request, Utc::now()).is_err());
    assert!(delete_custom_script_from_store(
        &mut store,
        &DeleteCustomScriptRequest {
            id: script.id.clone(),
            expected_updated_at: script.updated_at
        }
    )
    .is_err());
    save_store(&path, &store).unwrap();
    assert_eq!(
        load_store_sqlite(&path).unwrap().custom_scripts,
        store.custom_scripts
    );
    let encoded = serde_json::to_value(&store).unwrap();
    assert!(encoded.get("hostScripts").is_some());
    assert!(encoded.get("customScripts").is_none());
    let mut old_store = serde_json::to_value(SessionStore::default()).unwrap();
    old_store["customScripts"] = serde_json::json!([{"content":"must-not-migrate"}]);
    assert!(serde_json::from_value::<SessionStore>(old_store)
        .unwrap()
        .custom_scripts
        .is_empty());
    let mut old_request = serde_json::to_value(&saved).unwrap();
    old_request.as_object_mut().unwrap().remove("host");
    assert!(serde_json::from_value::<CustomScript>(old_request).is_err());
    delete_custom_script_from_store(
        &mut store,
        &DeleteCustomScriptRequest {
            id: saved.id,
            expected_updated_at: saved.updated_at,
        },
    )
    .unwrap();
    assert!(store.custom_scripts.is_empty());
}

#[test]
fn host_script_discovery_and_authorization_are_client_scoped_not_session_scoped() {
    let root = tempfile::tempdir().unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    let script = stored_script();
    {
        let mut store = state.store.lock().unwrap();
        store.grants.push(grant());
        store.custom_scripts.push(script.clone());
        store.delete_profile(&profile.id).unwrap();
        assert_eq!(store.custom_scripts[0], script);
        let tools = host_script_tools(&store, "script-client");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, script.host_tool_name());
        assert!(!serde_json::to_string(&tools)
            .unwrap()
            .contains(&script.content));
        assert!(host_script_tools(&store, "other-client").is_empty());
    }
    let mut request = IpcRequest {
        token: "test".into(),
        client_id: "script-client".into(),
        trusted_write: false,
        command: "run_custom_script".into(),
        args: serde_json::json!({"scriptId":script.id,"parameters":{}}),
    };
    assert_eq!(ipc_write_session_id(&state, &request).unwrap(), None);
    validate_ipc_write_args(&state, &request).unwrap();
    let context = capture_mcp_write_execution_context(&state, &request).unwrap();
    assert_eq!(
        context.approval_target().unwrap().kind,
        "portmate-host-script"
    );
    for key in ["sessionId", "content", "interpreter"] {
        request.args[key] = serde_json::json!("injected");
        assert!(validate_ipc_write_args(&state, &request).is_err());
        request.args.as_object_mut().unwrap().remove(key);
    }
    state.store.lock().unwrap().custom_scripts[0].updated_at += chrono::Duration::seconds(1);
    assert!(context
        .revalidate(&state, &request)
        .unwrap_err()
        .contains("changed after authorization"));
    state.store.lock().unwrap().custom_scripts[0] = script;
    state.store.lock().unwrap().custom_scripts[0]
        .host
        .allowed_client_ids
        .clear();
    assert!(validate_ipc_write_args(&state, &request).is_err());
}

#[test]
fn host_script_real_python_shell_limits_cancellation_and_revocation() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
        let mut script = stored_script();
        script.host.working_directory = root.path().to_string_lossy().into_owned();
        script.host.parameters.push(HostScriptParameter {
            name: "value".into(),
            description: "literal input".into(),
            kind: HostScriptParameterKind::String,
            required: true,
        });
        script.content = "import json,sys,os\np=json.load(sys.stdin)\nprint(json.dumps(p))\nprint(os.getcwd())\nprint('stderr-ok',file=sys.stderr)".into();
        state
            .store
            .lock()
            .unwrap()
            .custom_scripts
            .push(script.clone());
        state.store.lock().unwrap().grants.push(grant());
        let mut request = run_request(&script);
        request.parameters = serde_json::json!({"value":"$(touch injected); ' \" 中文"});
        let result = run_host_script_inner(&state, request.clone(), "test", None, None)
            .await
            .unwrap();
        assert_eq!(result.exit_code, Some(0));
        assert!(result.failure.is_none(), "{:?}", result.failure);
        assert!(result.stdout.contains("$(touch injected)"));
        assert!(result.stdout.contains(root.path().to_str().unwrap()));
        assert_eq!(result.stderr.trim(), "stderr-ok");
        assert!(!root.path().join("injected").exists());
        assert!(state.store.lock().unwrap().events.is_empty());
        let ipc = IpcRequest {
            token: "test".into(),
            client_id: "script-client".into(),
            trusted_write: false,
            command: "run_custom_script".into(),
            args: serde_json::json!({"scriptId":script.id,"parameters":request.parameters}),
        };
        let result = handle_ipc_request(state.clone(), ipc.clone())
            .await
            .unwrap();
        assert_eq!(result["exitCode"], 0);
        assert_eq!(
            state.store.lock().unwrap().audit.last().unwrap().decision,
            "succeeded"
        );
        let mut unknown = request.clone();
        unknown.parameters["other"] = serde_json::json!(true);
        assert!(run_host_script_inner(&state, unknown, "test", None, None)
            .await
            .is_err());
        assert!(
            run_host_script_inner(&state, request.clone(), "test", Some("other-client"), None)
                .await
                .is_err()
        );
        assert!(run_host_script_inner(
            &state,
            request.clone(),
            "test",
            None,
            Some(Box::new(|| Err("revoked at commit".into())))
        )
        .await
        .unwrap_err()
        .contains("revoked at commit"));
        #[cfg(unix)]
        {
            script.host.language = HostScriptLanguage::Shell;
            script.content = "printf '%s\\n' \"$PORTMATE_PARAM_value\"".into();
            state.store.lock().unwrap().custom_scripts[0] = script.clone();
            let result = run_host_script_inner(&state, request.clone(), "test", None, None)
                .await
                .unwrap();
            assert_eq!(result.stdout.trim(), "$(touch injected); ' \" 中文");
            assert!(!root.path().join("injected").exists());
        }
        script.host.language = HostScriptLanguage::Python;
        script.content = "import sys\nprint('bad',file=sys.stderr)\nsys.exit(7)".into();
        state.store.lock().unwrap().custom_scripts[0] = script.clone();
        let result = handle_ipc_request(state.clone(), ipc.clone())
            .await
            .unwrap();
        assert_eq!(result["exitCode"], 7);
        assert!(result["failure"].is_string());
        assert_eq!(
            state.store.lock().unwrap().audit.last().unwrap().decision,
            "failed"
        );
        for (source, expected) in [
            ("print('x'*200000)", "output exceeded"),
            ("import time\ntime.sleep(20)", "timed out"),
        ] {
            script.content = source.into();
            script.host.timeout_seconds = 1;
            state.store.lock().unwrap().custom_scripts[0] = script.clone();
            let result = run_host_script_inner(&state, request.clone(), "test", None, None)
                .await
                .unwrap();
            assert!(result.failure.unwrap().contains(expected));
            assert!(result.stdout.len() <= 128 * 1024);
        }
        script.content = "import time\nprint('started',flush=True)\ntime.sleep(20)".into();
        script.host.timeout_seconds = 5;
        for revoke in [false, true] {
            state.store.lock().unwrap().custom_scripts[0] = script.clone();
            let state2 = state.clone();
            let request2 = request.clone();
            let run = tokio::spawn(async move {
                run_host_script_inner(&state2, request2, "test-owner", Some("script-client"), None)
                    .await
            });
            tokio::time::sleep(Duration::from_millis(200)).await;
            if revoke {
                state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());
            } else {
                crate::host_script_commands::cancel_owner(&state.store_path, Some("test-owner"));
            }
            let result = tokio::time::timeout(Duration::from_secs(2), run)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(result
                .failure
                .unwrap()
                .contains(if revoke { "revoked" } else { "cancelled" }));
        }
        #[cfg(unix)]
        {
            state.store.lock().unwrap().grants[0].revoked_at = None;
            script.content = "import subprocess,time\nsubprocess.Popen(['sh','-c','sleep 2; touch orphan-marker'])\ntime.sleep(20)".into();
            script.host.timeout_seconds = 1;
            state.store.lock().unwrap().custom_scripts[0] = script.clone();
            let result = run_host_script_inner(&state, request.clone(), "test", None, None)
                .await
                .unwrap();
            assert!(result.failure.unwrap().contains("timed out"));
            tokio::time::sleep(Duration::from_millis(1400)).await;
            assert!(!root.path().join("orphan-marker").exists());
        }
        // Saturate the process budget, then ensure cancelling one owner's jobs
        // does not silently cancel another owner's work or leak capacity.
        script.host.timeout_seconds = 10;
        script.content =
            "import os,time\nopen(os.environ['PORTMATE_PARAM_value'],'w').close()\ntime.sleep(20)"
                .into();
        state.store.lock().unwrap().custom_scripts[0] = script.clone();
        let mut runs = Vec::new();
        for index in 0..4 {
            let state2 = state.clone();
            let mut request2 = request.clone();
            request2.parameters = serde_json::json!({"value":format!("slot-{index}")});
            runs.push(tokio::spawn(async move {
                run_host_script_inner(&state2, request2, &format!("owner-{index}"), None, None)
                    .await
            }));
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            while !(0..4).all(|index| root.path().join(format!("slot-{index}")).exists()) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            run_host_script_inner(&state, request.clone(), "fifth", None, None)
                .await
                .unwrap_err()
                .contains("concurrency limit")
        );
        crate::host_script_commands::cancel_owner(&state.store_path, Some("owner-0"));
        let first = runs.remove(0).await.unwrap().unwrap();
        assert!(first.failure.unwrap().contains("cancelled"));
        assert!(runs.iter().all(|task| !task.is_finished()));
        // Dropping the driving future must retire its process tree too.
        runs[0].abort();
        assert!(runs.remove(0).await.unwrap_err().is_cancelled());
        crate::host_script_commands::cancel_owner(&state.store_path, None);
        for run in runs {
            assert!(run
                .await
                .unwrap()
                .unwrap()
                .failure
                .unwrap()
                .contains("cancelled"));
        }
        script.host.interpreter = root
            .path()
            .join("missing-python")
            .to_string_lossy()
            .into_owned();
        state.store.lock().unwrap().custom_scripts[0] = script;
        assert!(run_host_script_inner(&state, request, "test", None, None)
            .await
            .unwrap_err()
            .contains("could not start"));
    });
}

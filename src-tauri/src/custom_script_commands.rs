use super::*;

pub(super) fn normalize_custom_script_request(
    store: &SessionStore,
    request: SaveCustomScriptRequest,
    now: DateTime<Utc>,
) -> Result<CustomScript, String> {
    let existing = match request.id.as_deref() {
        Some(id) => {
            Uuid::parse_str(id).map_err(|_| "custom script ID must be a UUID".to_string())?;
            Some(
                store
                    .custom_scripts
                    .iter()
                    .find(|script| script.id == id)
                    .ok_or_else(|| "custom script was deleted; refresh and try again".to_string())?,
            )
        }
        None => None,
    };
    match (existing, request.expected_updated_at) {
        (Some(existing), Some(expected)) if existing.updated_at != expected => {
            return Err("custom script changed in another window; refresh and try again".to_string())
        }
        (Some(_), None) => {
            return Err("custom script update is missing expectedUpdatedAt".to_string())
        }
        (None, Some(_)) => {
            return Err("new custom script must not provide expectedUpdatedAt".to_string())
        }
        _ => {}
    }

    let mut allowed_session_ids = Vec::new();
    if !request.allow_all_sessions {
        for session_id in request.allowed_session_ids {
            let session_id = session_id.trim();
            if !store.profiles.iter().any(|profile| profile.id == session_id) {
                return Err(format!("unknown custom script session: {session_id}"));
            }
            if !allowed_session_ids
                .iter()
                .any(|existing| existing == session_id)
            {
                allowed_session_ids.push(session_id.to_string());
            }
        }
        if allowed_session_ids.is_empty() {
            return Err("select at least one session or enable all sessions".to_string());
        }
    }

    let script = CustomScript {
        id: existing
            .map(|script| script.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        name: request.name.trim().to_string(),
        description: request.description.trim().to_string(),
        content: normalize_custom_script_content(&request.content),
        allow_all_sessions: request.allow_all_sessions,
        allowed_session_ids,
        mcp_enabled: request.mcp_enabled,
        created_at: existing.map(|script| script.created_at).unwrap_or(now),
        updated_at: now,
    };
    validate_custom_script(&script)?;
    Ok(script)
}

pub(super) fn upsert_custom_script_in_store(
    store: &mut SessionStore,
    script: CustomScript,
) -> Result<SaveCustomScriptResponse, String> {
    let saved_id = script.id.clone();
    if let Some(existing) = store
        .custom_scripts
        .iter_mut()
        .find(|existing| existing.id == script.id)
    {
        *existing = script;
    } else {
        if store.custom_scripts.len() >= MAX_CUSTOM_SCRIPTS {
            return Err(format!(
                "custom script limit exceeded ({MAX_CUSTOM_SCRIPTS})"
            ));
        }
        store.custom_scripts.push(script);
    }
    Ok(SaveCustomScriptResponse {
        scripts: store.custom_scripts.clone(),
        saved_id,
    })
}

pub(super) fn delete_custom_script_from_store(
    store: &mut SessionStore,
    request: &DeleteCustomScriptRequest,
) -> Result<Vec<CustomScript>, String> {
    let index = store
        .custom_scripts
        .iter()
        .position(|script| script.id == request.id)
        .ok_or_else(|| "custom script was deleted; refresh and try again".to_string())?;
    if store.custom_scripts[index].updated_at != request.expected_updated_at {
        return Err("custom script changed in another window; refresh and try again".to_string());
    }
    store.custom_scripts.remove(index);
    Ok(store.custom_scripts.clone())
}

pub(super) fn custom_script_for_session(
    store: &SessionStore,
    script_id: &str,
    session_id: &str,
    require_mcp_enabled: bool,
) -> Result<CustomScript, String> {
    Uuid::parse_str(script_id).map_err(|_| "custom script ID must be a UUID".to_string())?;
    if !store.profiles.iter().any(|profile| profile.id == session_id) {
        return Err("unknown or unavailable session".to_string());
    }
    let script = store
        .custom_scripts
        .iter()
        .find(|script| script.id == script_id)
        .ok_or_else(|| "unknown or unavailable custom script".to_string())?;
    validate_custom_script(script)?;
    if !script.allows_session(session_id) {
        return Err("custom script is not enabled for the requested session".to_string());
    }
    if require_mcp_enabled && !script.mcp_enabled {
        return Err("custom script is not exposed to MCP".to_string());
    }
    Ok(script.clone())
}

pub(super) async fn run_custom_script_inner(
    state: &AppState,
    request: RunCustomScriptRequest,
    actor: &str,
    audit_action: Option<&str>,
    require_mcp_enabled: bool,
    commit_validation: Option<CommitValidation>,
) -> Result<SessionEvent, String> {
    let io = state.session_io();
    let initial_updated_at = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        custom_script_for_session(
            &store,
            &request.script_id,
            &request.session_id,
            require_mcp_enabled,
        )?
        .updated_at
    };
    if initial_updated_at != request.expected_updated_at {
        return Err(if require_mcp_enabled {
            "MCP custom script changed after authorization; review and approve it again".to_string()
        } else {
            "custom script changed in another window; refresh and try again".to_string()
        });
    }
    let expected_runtime_id = current_session_runtime_id(&io.runtimes, &request.session_id)?
        .ok_or_else(|| "会话尚未连接，无法运行自定义脚本".to_string())?;
    let lane = outbound_lane(&io.store_path, &request.session_id)?;
    let _lane_guard = lane.lock().await;
    let script = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        custom_script_for_session(
            &store,
            &request.script_id,
            &request.session_id,
            require_mcp_enabled,
        )?
    };
    if script.updated_at != request.expected_updated_at {
        return Err(if require_mcp_enabled {
            "MCP custom script changed after authorization; review and approve it again".to_string()
        } else {
            "custom script changed in another window; refresh and try again".to_string()
        });
    }
    let text = terminate_command_for_protocol(
        script.content,
        is_telnet_session(&state.store, &request.session_id)?,
    );
    run_command_under_outbound_lane_with_annotations_and_display_text_for_runtime(
        &io,
        &request.session_id,
        &text,
        RunCommandContext {
            display_text: Some(CUSTOM_SCRIPT_EVENT_TEXT),
            actor,
            audit_action,
            additional_annotations: BTreeMap::from([("customScriptId".to_string(), script.id)]),
            expected_runtime_id: Some(&expected_runtime_id),
            commit_validation,
        },
    )
    .await
}

#[tauri::command]
pub(crate) fn list_custom_scripts(
    state: State<'_, AppState>,
) -> Result<Vec<CustomScript>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.custom_scripts.clone())
}

#[tauri::command]
pub(crate) fn save_custom_script(
    state: State<'_, AppState>,
    request: SaveCustomScriptRequest,
) -> Result<SaveCustomScriptResponse, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let script = normalize_custom_script_request(&store, request, Utc::now())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        upsert_custom_script_in_store(next_store, script)
    })
}

#[tauri::command]
pub(crate) fn delete_custom_script(
    state: State<'_, AppState>,
    request: DeleteCustomScriptRequest,
) -> Result<Vec<CustomScript>, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        delete_custom_script_from_store(next_store, &request)
    })
}

#[tauri::command]
pub(crate) async fn run_custom_script(
    state: State<'_, AppState>,
    request: RunCustomScriptRequest,
) -> Result<SessionEvent, String> {
    run_custom_script_inner(
        state.inner(),
        request,
        "desktop-user",
        Some("run_custom_script"),
        false,
        None,
    )
    .await
}

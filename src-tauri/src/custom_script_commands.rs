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
                    .ok_or_else(|| {
                        "custom script was deleted; refresh and try again".to_string()
                    })?,
            )
        }
        None => None,
    };
    match (existing, request.expected_updated_at) {
        (Some(existing), Some(expected)) if existing.updated_at != expected => {
            return Err(
                "custom script changed in another window; refresh and try again".to_string(),
            )
        }
        (Some(_), None) => {
            return Err("custom script update is missing expectedUpdatedAt".to_string())
        }
        (None, Some(_)) => {
            return Err("new custom script must not provide expectedUpdatedAt".to_string())
        }
        _ => {}
    }

    // updated_at is also the optimistic concurrency token. Wall-clock time
    // alone can repeat or move backwards; every successful save must advance it.
    let updated_at = match existing {
        Some(script) if now <= script.updated_at => script
            .updated_at
            .checked_add_signed(chrono::Duration::nanoseconds(1))
            .ok_or_else(|| "custom script version timestamp overflow".to_string())?,
        _ => now,
    };
    let script = CustomScript {
        id: existing
            .map(|script| script.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        name: request.name.trim().to_string(),
        description: request.description.trim().to_string(),
        content: normalize_custom_script_content(&request.content),
        host: request.host,
        mcp_enabled: request.mcp_enabled,
        created_at: existing.map(|script| script.created_at).unwrap_or(now),
        updated_at,
    };
    {
        let host = &script.host;
        for id in &host.allowed_client_ids {
            if !store
                .grants
                .iter()
                .any(|grant| grant.client_id == *id && grant.revoked_at.is_none())
            {
                return Err(format!("unknown or revoked MCP client: {id}"));
            }
        }
    }
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

#[tauri::command]
pub(crate) fn list_custom_scripts(state: State<'_, AppState>) -> Result<Vec<CustomScript>, String> {
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

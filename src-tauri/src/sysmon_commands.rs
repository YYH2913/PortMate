use super::*;

#[tauri::command]
pub(crate) async fn refresh_sysmon(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SysmonSnapshot, String> {
    refresh_sysmon_inner(state.inner(), &session_id).await
}

#[tauri::command]
pub(crate) fn list_sysmon_history(
    state: State<'_, AppState>,
    session_id: String,
    limit: Option<usize>,
) -> Result<Vec<SysmonSnapshot>, String> {
    let limit = validate_sysmon_history_query_limit(limit)?;
    let store = state.store.lock().map_err(|error| error.to_string())?;
    if store.profile(&session_id).is_none() {
        return Err(format!("unknown session: {session_id}"));
    }
    Ok(store.sysmon_history_for(&session_id, limit))
}

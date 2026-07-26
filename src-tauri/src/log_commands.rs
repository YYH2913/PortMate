use super::*;

#[tauri::command]
pub(crate) fn tail_log(
    state: State<'_, AppState>,
    session_id: String,
    limit: Option<u64>,
) -> Result<Vec<SessionEvent>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.tail_log(&session_id, bounded_log_query_limit(limit)))
}

#[tauri::command]
pub(crate) fn search_logs(
    state: State<'_, AppState>,
    query: String,
    session_id: Option<String>,
    limit: Option<u64>,
) -> Result<Vec<SessionEvent>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.search_logs(
        &query,
        session_id.as_deref(),
        bounded_log_query_limit(limit),
    ))
}

#[tauri::command]
pub(crate) fn list_log_shards(state: State<'_, AppState>) -> Result<Vec<LogShardInfo>, String> {
    list_log_shards_inner(&state.store_path)
}

#[tauri::command]
pub(crate) fn read_log_shard(
    state: State<'_, AppState>,
    path: String,
    max_bytes: Option<u64>,
) -> Result<LogShardPreview, String> {
    read_log_shard_inner(&state.store_path, &path, max_bytes)
}

#[tauri::command]
pub(crate) fn delete_log_shards(
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<DeleteLogShardsResult, String> {
    delete_log_shards_inner(&state.store_path, &paths)
}

#[tauri::command]
pub(crate) fn search_log_shards(
    state: State<'_, AppState>,
    request: SearchLogShardsRequest,
) -> Result<SearchLogShardsResult, String> {
    search_log_shards_inner(&state.store_path, request)
}

#[tauri::command]
pub(crate) fn archive_log_shards(
    state: State<'_, AppState>,
    request: ArchiveLogShardsRequest,
) -> Result<ArchiveLogShardsResult, String> {
    archive_log_shards_inner(&state.store_path, request)
}

#[tauri::command]
pub(crate) fn export_session_bundle_archive(
    state: State<'_, AppState>,
    request: ExportSessionBundleArchiveRequest,
) -> Result<ExportSessionBundleArchiveResult, String> {
    let signing_key = load_or_create_bundle_signing_key()?;
    let snapshot = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .clone();
    export_session_bundle_archive_inner(&state.store_path, &snapshot, request, &signing_key)
}

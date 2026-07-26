use super::*;

#[tauri::command]
pub(crate) fn list_transfers(state: State<'_, AppState>) -> Result<Vec<TransferTask>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.transfers.clone())
}

#[tauri::command]
pub(crate) async fn retry_transfer(
    state: State<'_, AppState>,
    transfer_id: String,
) -> Result<TransferTask, String> {
    retry_transfer_inner(state.inner(), &transfer_id).await
}

#[tauri::command]
pub(crate) fn cancel_transfer(
    state: State<'_, AppState>,
    transfer_id: String,
) -> Result<TransferTask, String> {
    cancel_transfer_inner(state.inner(), &transfer_id)
}

#[tauri::command]
pub(crate) async fn start_transfer(
    state: State<'_, AppState>,
    request: StartTransferRequest,
) -> Result<TransferTask, String> {
    start_transfer_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) async fn start_external_drop(
    state: State<'_, AppState>,
    request: StartExternalDropRequest,
) -> Result<ExternalDropResult, String> {
    start_external_drop_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) async fn start_file_batch(
    state: State<'_, AppState>,
    request: StartFileBatchRequest,
) -> Result<ExternalDropResult, String> {
    start_file_batch_inner(state.inner(), request).await
}

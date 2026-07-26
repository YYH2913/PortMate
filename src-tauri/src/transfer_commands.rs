use super::*;

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

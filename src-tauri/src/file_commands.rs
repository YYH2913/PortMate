use super::*;

#[tauri::command]
pub(crate) async fn list_files(
    state: State<'_, AppState>,
    request: ListFilesRequest,
) -> Result<Vec<FileEntry>, String> {
    list_files_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) async fn file_properties(
    state: State<'_, AppState>,
    request: FilePropertiesRequest,
) -> Result<FileProperties, String> {
    file_properties_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) async fn create_directory(
    state: State<'_, AppState>,
    request: FileOperationRequest,
) -> Result<(), String> {
    file_operation_inner(state.inner(), request, FileOperation::CreateDirectory).await
}

#[tauri::command]
pub(crate) async fn create_file(
    state: State<'_, AppState>,
    request: FileOperationRequest,
) -> Result<(), String> {
    file_operation_inner(state.inner(), request, FileOperation::CreateFile).await
}

#[tauri::command]
pub(crate) async fn delete_path(
    state: State<'_, AppState>,
    request: FileOperationRequest,
) -> Result<(), String> {
    file_operation_inner(state.inner(), request, FileOperation::Delete).await
}

#[tauri::command]
pub(crate) async fn delete_paths(
    state: State<'_, AppState>,
    request: DeletePathsRequest,
) -> Result<(), String> {
    delete_paths_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) async fn rename_path(
    state: State<'_, AppState>,
    request: RenamePathRequest,
) -> Result<(), String> {
    rename_path_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) async fn move_paths(
    state: State<'_, AppState>,
    request: MovePathsRequest,
) -> Result<(), String> {
    move_paths_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) async fn chmod_path(
    state: State<'_, AppState>,
    request: ChmodPathRequest,
) -> Result<(), String> {
    chmod_path_inner(state.inner(), request).await
}

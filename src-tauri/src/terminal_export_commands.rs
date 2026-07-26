use super::*;

#[tauri::command]
pub(crate) fn export_terminal_text(
    state: State<'_, AppState>,
    request: ExportTerminalTextRequest,
) -> Result<ExportTerminalTextResult, String> {
    validate_terminal_text_export_request(&request, MAX_TERMINAL_TEXT_EXPORT_BYTES)?;
    {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if store.profile(&request.session_id).is_none() {
            return Err("unknown terminal export session".to_string());
        }
    }
    export_terminal_text_inner(&state.store_path, request)
}

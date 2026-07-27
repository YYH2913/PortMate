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

pub(super) fn export_terminal_text_inner(
    store_path: &Path,
    request: ExportTerminalTextRequest,
) -> Result<ExportTerminalTextResult, String> {
    validate_terminal_text_export_request(&request, MAX_TERMINAL_TEXT_EXPORT_BYTES)?;
    let created_at = Utc::now();
    let export_dir = prepare_export_directory(store_path, "terminal text")?;
    let session_name = sanitize_log_path_segment(&request.session_id);
    let timestamp = created_at.format("%Y%m%dT%H%M%SZ");
    let name = format!(
        "{}-{timestamp}-{}-{}.txt",
        if session_name.is_empty() {
            "session"
        } else {
            &session_name
        },
        request.source.as_str(),
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let final_path = export_dir.join(name);
    let finalized = write_atomic_export_with_checksum(
        &final_path,
        request.text.as_bytes(),
        "terminal text export",
    )?;
    Ok(ExportTerminalTextResult {
        path: final_path.display().to_string(),
        checksum_path: finalized.checksum_path.display().to_string(),
        sha256: finalized.sha256,
        size: finalized.size,
        session_id: request.session_id,
        view_id: request.view_id,
        source: request.source,
    })
}

pub(super) fn validate_terminal_text_export_request(
    request: &ExportTerminalTextRequest,
    max_bytes: usize,
) -> Result<(), String> {
    if request.session_id.trim().is_empty()
        || request.session_id.len() > 256
        || request.session_id.chars().any(char::is_control)
    {
        return Err("invalid terminal export session id".to_string());
    }
    if request.view_id.trim().is_empty()
        || request.view_id.len() > 128
        || request.view_id.chars().any(char::is_control)
    {
        return Err("invalid terminal export view id".to_string());
    }
    if request.text.is_empty() {
        return Err("terminal export text is empty".to_string());
    }
    if request.text.len() > max_bytes {
        return Err(format!("terminal export exceeds {max_bytes} byte limit"));
    }
    Ok(())
}

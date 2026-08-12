use super::*;

const MAX_TERMINAL_TEXT_EXPORT_BYTES: usize = 16 * 1024 * 1024;
const MAX_TERMINAL_TEXT_EXPORT_PATH_BYTES: usize = 32 * 1024;

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
    let (final_path, overwrite) = terminal_text_export_path(store_path, &request, created_at)?;
    let finalized = write_atomic_export_with_checksum_policy(
        &final_path,
        request.text.as_bytes(),
        "terminal text export",
        overwrite,
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
    for (label, path) in [
        ("destination directory", request.destination_directory.as_deref()),
        ("destination path", request.destination_path.as_deref()),
    ] {
        if let Some(path) = path {
            if path.trim().is_empty()
                || path.len() > MAX_TERMINAL_TEXT_EXPORT_PATH_BYTES
                || path.contains(['\0', '\n', '\r'])
            {
                return Err(format!("invalid terminal export {label}"));
            }
        }
    }
    if request.destination_directory.is_some() && request.destination_path.is_some() {
        return Err("terminal export accepts either a destination directory or a destination path, not both".to_string());
    }
    if request.overwrite && request.destination_path.is_none() {
        return Err("terminal export overwrite requires an explicit destination path".to_string());
    }
    Ok(())
}

fn terminal_text_export_path(
    store_path: &Path,
    request: &ExportTerminalTextRequest,
    created_at: DateTime<Utc>,
) -> Result<(PathBuf, bool), String> {
    if let Some(path) = request.destination_path.as_deref() {
        let path = PathBuf::from(path);
        validate_terminal_text_export_target(&path)?;
        return Ok((path, request.overwrite));
    }

    let export_dir = match request.destination_directory.as_deref() {
        Some(path) => validate_terminal_text_export_directory(Path::new(path))?,
        None => prepare_export_directory(store_path, "terminal text")?,
    };
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
    Ok((export_dir.join(name), false))
}

fn validate_terminal_text_export_directory(path: &Path) -> Result<PathBuf, String> {
    validate_terminal_export_absolute_path(path, "directory")?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect terminal export directory {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "terminal export directory must not be a symbolic link: {}",
            path.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "terminal export path is not a directory: {}",
            path.display()
        ));
    }
    Ok(path.to_path_buf())
}

fn validate_terminal_text_export_target(path: &Path) -> Result<(), String> {
    validate_terminal_export_absolute_path(path, "path")?;
    if path.file_name().is_none() {
        return Err("terminal export destination must name a file".to_string());
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| "terminal export destination has no parent directory".to_string())?;
    validate_terminal_text_export_directory(parent)?;
    Ok(())
}

fn validate_terminal_export_absolute_path(path: &Path, label: &str) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(format!("terminal export {label} must be absolute"));
    }
    let raw = path.as_os_str().to_string_lossy();
    #[cfg(windows)]
    let has_dot_component = raw
        .split(['/', '\\'])
        .any(|component| matches!(component, "." | ".."));
    #[cfg(not(windows))]
    let has_dot_component = raw
        .split('/')
        .any(|component| matches!(component, "." | ".."));
    if has_dot_component {
        return Err(format!(
            "terminal export {label} must not contain . or .. components"
        ));
    }
    Ok(())
}

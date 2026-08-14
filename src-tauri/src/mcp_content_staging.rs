use super::*;

pub(super) fn stage_mcp_content_transfer(
    state: &AppState,
    request: &StartMcpContentTransferRequest,
) -> Result<(String, PathBuf), String> {
    validate_mcp_content_transfer_request(request)?;
    let content = BASE64_STANDARD
        .decode(&request.content_base64)
        .map_err(|_| "MCP contentBase64 is not valid standard Base64".to_string())?;
    let staging_dir = ensure_mcp_content_staging_root(state)?;
    let task_dir = staging_dir.join(Uuid::new_v4().to_string());
    fs::create_dir(&task_dir)
        .map_err(|error| format!("failed to create MCP content staging task directory: {error}"))?;
    #[cfg(unix)]
    if let Err(error) = fs::set_permissions(&task_dir, fs::Permissions::from_mode(0o700)) {
        let _ = fs::remove_dir(&task_dir);
        return Err(format!(
            "failed to secure MCP content staging task directory: {error}"
        ));
    }
    let path = task_dir.join(&request.file_name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&path).map_err(|error| {
        let _ = fs::remove_dir(&task_dir);
        format!("failed to create MCP content staging file: {error}")
    })?;
    if let Err(error) = file.write_all(&content).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&task_dir);
        return Err(format!(
            "failed to write MCP content staging file: {error}"
        ));
    }
    Ok((path.display().to_string(), path))
}

pub(super) fn load_mcp_content_upload_metadata(
    state: &AppState,
    client_id: &str,
    upload_id: &str,
) -> Result<McpContentUploadMetadata, String> {
    Uuid::parse_str(upload_id).map_err(|_| "MCP uploadId must be a UUID".to_string())?;
    let upload_dir = mcp_content_upload_root(state)?.join(upload_id);
    require_regular_directory(&upload_dir, "MCP content upload")?;
    let metadata_path = upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE);
    let metadata_file = open_mcp_upload_file(&metadata_path, "metadata")
        .map_err(|_| "unknown or unavailable MCP content upload".to_string())?;
    if metadata_file
        .metadata()
        .map_err(|error| format!("failed to inspect MCP content upload metadata: {error}"))?
        .len()
        > 64 * 1024
    {
        return Err("invalid MCP content upload metadata file".to_string());
    }
    let mut metadata_bytes = Vec::new();
    metadata_file
        .take(64 * 1024 + 1)
        .read_to_end(&mut metadata_bytes)
        .map_err(|error| format!("failed to read MCP content upload metadata: {error}"))?;
    if metadata_bytes.len() > 64 * 1024 {
        return Err("invalid MCP content upload metadata file".to_string());
    }
    let metadata: McpContentUploadMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|error| format!("invalid MCP content upload metadata: {error}"))?;
    if metadata.version != MCP_CONTENT_UPLOAD_METADATA_VERSION
        || metadata.upload_id != upload_id
        || metadata.client_id != client_id
    {
        return Err("unknown or unavailable MCP content upload".to_string());
    }
    validate_mcp_content_upload_metadata(&metadata)?;
    Ok(metadata)
}

pub(super) fn stage_mcp_content_upload(
    state: &AppState,
    metadata: &McpContentUploadMetadata,
) -> Result<(String, PathBuf), String> {
    validate_mcp_content_upload_metadata(metadata)?;
    validate_mcp_uploaded_content_route(metadata)?;
    let upload_dir = mcp_content_upload_root(state)?.join(&metadata.upload_id);
    require_regular_directory(&upload_dir, "MCP content upload")?;
    let payload_path = upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE);
    let mut payload = open_mcp_upload_file(&payload_path, "payload")?;
    let opened_metadata = payload
        .metadata()
        .map_err(|error| format!("failed to inspect MCP content upload payload: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() != metadata.size_bytes {
        return Err(format!(
            "MCP content upload is incomplete: expected {} bytes, received {}",
            metadata.size_bytes,
            opened_metadata.len()
        ));
    }

    let staging_dir = mcp_content_staging_root(state)?;
    require_regular_directory(&staging_dir, "MCP content staging")?;
    let task_dir = staging_dir.join(Uuid::new_v4().to_string());
    create_private_mcp_directory(&task_dir)?;
    let staged_path = task_dir.join(&metadata.file_name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut staged = match options.open(&staged_path) {
        Ok(file) => file,
        Err(error) => {
            let _ = fs::remove_dir(&task_dir);
            return Err(format!(
                "failed to create MCP content staging file: {error}"
            ));
        }
    };

    let result = (|| -> Result<(), String> {
        let mut digest = Sha256::new();
        let mut copied = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = payload
                .read(&mut buffer)
                .map_err(|error| format!("failed to read MCP content upload: {error}"))?;
            if read == 0 {
                break;
            }
            copied = copied
                .checked_add(read as u64)
                .ok_or_else(|| "MCP content upload size overflow".to_string())?;
            if copied > metadata.size_bytes {
                return Err("MCP content upload exceeds its declared size".to_string());
            }
            digest.update(&buffer[..read]);
            staged
                .write_all(&buffer[..read])
                .map_err(|error| format!("failed to stage MCP content upload: {error}"))?;
        }
        if copied != metadata.size_bytes {
            return Err(format!(
                "MCP content upload is incomplete: expected {} bytes, received {copied}",
                metadata.size_bytes
            ));
        }
        let actual = format!("{:x}", digest.finalize());
        if actual != metadata.sha256 {
            return Err("MCP content upload SHA-256 mismatch".to_string());
        }
        staged
            .sync_all()
            .map_err(|error| format!("failed to sync MCP content staging file: {error}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        cleanup_staged_mcp_content_path(&staged_path);
        return Err(error);
    }
    Ok((staged_path.display().to_string(), staged_path))
}

fn validate_mcp_content_upload_metadata(
    metadata: &McpContentUploadMetadata,
) -> Result<(), String> {
    if metadata.version != MCP_CONTENT_UPLOAD_METADATA_VERSION
        || Uuid::parse_str(&metadata.upload_id).is_err()
        || metadata.size_bytes == 0
        || metadata.size_bytes > MAX_MCP_CONTENT_UPLOAD_BYTES
    {
        return Err("invalid MCP content upload metadata".to_string());
    }
    if metadata.session_id.is_empty()
        || metadata.session_id.len() > 128
        || metadata.session_id.chars().any(char::is_control)
    {
        return Err("invalid MCP content upload session".to_string());
    }
    if metadata.file_name.is_empty()
        || metadata.file_name.len() > 255
        || matches!(metadata.file_name.as_str(), "." | "..")
        || metadata.file_name.chars().any(|character| {
            character.is_control() || matches!(character, '\0' | '/' | '\\' | ':')
        })
    {
        return Err("invalid MCP content upload file name".to_string());
    }
    if metadata.sha256.len() != 64
        || !metadata
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("invalid MCP content upload SHA-256".to_string());
    }
    Ok(())
}

fn mcp_content_staging_root(state: &AppState) -> Result<PathBuf, String> {
    state
        .store_path
        .parent()
        .map(|parent| parent.join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY))
        .ok_or_else(|| "MCP content staging directory is unavailable".to_string())
}

fn ensure_mcp_content_staging_root(state: &AppState) -> Result<PathBuf, String> {
    let staging_root = mcp_content_staging_root(state)?;
    let parent = staging_root
        .parent()
        .ok_or_else(|| "MCP content staging directory is unavailable".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create MCP content staging parent directory: {error}"))?;
    match fs::create_dir(&staging_root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(format!(
                "failed to create MCP content staging directory: {error}"
            ));
        }
    }
    require_regular_directory(&staging_root, "MCP content staging")?;
    #[cfg(unix)]
    fs::set_permissions(&staging_root, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to secure MCP content staging directory: {error}"))?;
    Ok(staging_root)
}

fn mcp_content_upload_root(state: &AppState) -> Result<PathBuf, String> {
    let staging_root = mcp_content_staging_root(state)?;
    require_regular_directory(&staging_root, "MCP content staging")?;
    let uploads_root = staging_root.join(MCP_CONTENT_UPLOADS_DIRECTORY);
    require_regular_directory(&uploads_root, "MCP content uploads")?;
    Ok(uploads_root)
}

fn require_regular_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("{label} directory is unavailable: {error}"))?;
    if !metadata.is_dir() || mcp_upload_metadata_is_link(&metadata) {
        return Err(format!("invalid {label} directory"));
    }
    Ok(())
}

fn open_mcp_upload_file(path: &Path, label: &str) -> Result<fs::File, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect MCP content upload {label}: {error}"))?;
    if !metadata.is_file() || mcp_upload_metadata_is_link(&metadata) {
        return Err(format!("invalid MCP content upload {label}"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    #[cfg(windows)]
    options.custom_flags(0x0020_0000);
    let file = options
        .open(path)
        .map_err(|error| format!("failed to open MCP content upload {label}: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("failed to inspect opened MCP content upload {label}: {error}"))?;
    if !opened.is_file() || mcp_upload_metadata_is_link(&opened) {
        return Err(format!("invalid MCP content upload {label}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.nlink() != 1 {
            return Err(format!(
                "MCP content upload {label} must not have hard links"
            ));
        }
    }
    Ok(file)
}

fn mcp_upload_metadata_is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        metadata.file_attributes() & 0x0400 != 0
    }
    #[cfg(not(windows))]
    false
}

fn create_private_mcp_directory(path: &Path) -> Result<(), String> {
    fs::create_dir(path)
        .map_err(|error| format!("failed to create MCP content staging directory: {error}"))?;
    #[cfg(unix)]
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o700)) {
        let _ = fs::remove_dir(path);
        return Err(format!(
            "failed to secure MCP content staging directory: {error}"
        ));
    }
    Ok(())
}

pub(super) fn cleanup_staged_mcp_content_path(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = path
        .parent()
        .filter(|parent| parent.file_name().is_some())
        .map(fs::remove_dir);
}

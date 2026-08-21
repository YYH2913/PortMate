use super::PortMateMcp;
use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use fs2::FileExt;
use portmate_core::{
    misplaced_mcp_tftp_destination_option, validate_tftp_file_name, McpContentUploadMetadata,
    McpTransferDestination, TransferProtocol, MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH,
    MAX_MCP_CONTENT_TRANSFER_BYTES, MAX_MCP_CONTENT_UPLOADS, MAX_MCP_CONTENT_UPLOAD_BYTES,
    MAX_MCP_CONTENT_UPLOAD_TOTAL_BYTES, MCP_CONTENT_UPLOADS_DIRECTORY,
    MCP_CONTENT_UPLOAD_EXPIRY_SECONDS, MCP_CONTENT_UPLOAD_METADATA_FILE,
    MCP_CONTENT_UPLOAD_METADATA_VERSION, MCP_CONTENT_UPLOAD_PAYLOAD_FILE,
    MCP_CONTENT_UPLOAD_STAGING_DIRECTORY,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt, PermissionsExt};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const CONTENT_UPLOAD_LOCK_FILE: &str = "upload.lock";
const CONTENT_UPLOAD_ROOT_LOCK_FILE: &str = "uploads.lock";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BeginContentUploadRequest {
    session_id: String,
    protocol: TransferProtocol,
    file_name: String,
    size_bytes: u64,
    sha256: String,
    destination: McpTransferDestination,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppendContentUploadRequest {
    upload_id: String,
    offset: u64,
    content_base64: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ContentUploadIdRequest {
    upload_id: String,
}

impl PortMateMcp {
    pub(super) fn begin_content_upload(&self, arguments: &Value) -> Result<Value> {
        if let Some(field) = arguments
            .as_object()
            .and_then(misplaced_mcp_tftp_destination_option)
        {
            return Err(anyhow!(
                "begin_content_upload TFTP option `{field}` must be nested in the structured `destination` object or encoded in the legacy load:tftpboot query string"
            ));
        }
        let request: BeginContentUploadRequest = serde_json::from_value(arguments.clone())
            .map_err(|error| anyhow!("invalid begin_content_upload arguments: {error}"))?;
        validate_file_name(&request.file_name)?;
        if request.protocol == TransferProtocol::Tftp {
            validate_tftp_file_name(&request.file_name).map_err(anyhow::Error::msg)?;
        }
        if request.size_bytes == 0 || request.size_bytes > MAX_MCP_CONTENT_UPLOAD_BYTES {
            return Err(anyhow!(
                "sizeBytes must be between 1 and {MAX_MCP_CONTENT_UPLOAD_BYTES}"
            ));
        }
        validate_sha256(&request.sha256)?;
        if request.session_id.is_empty() || request.session_id.len() > 128 {
            return Err(anyhow!("sessionId must be between 1 and 128 bytes"));
        }
        self.require_known_session(&request.session_id)?;
        self.require_content_upload_scope(&request.session_id)?;
        let destination = request
            .destination
            .normalize(&request.protocol)
            .map_err(anyhow::Error::msg)?;

        let staging_root = self.content_upload_staging_root()?;
        create_private_directory(&staging_root)?;
        let uploads_root = staging_root.join(MCP_CONTENT_UPLOADS_DIRECTORY);
        create_private_directory(&uploads_root)?;
        let root_lock = open_private_lock(&staging_root.join(CONTENT_UPLOAD_ROOT_LOCK_FILE))?;
        root_lock.lock_exclusive()?;
        cleanup_expired_uploads(&uploads_root, unix_seconds_now())?;
        let (active_uploads, declared_bytes) = upload_usage(&uploads_root)?;
        if active_uploads >= MAX_MCP_CONTENT_UPLOADS {
            return Err(anyhow!(
                "content upload limit reached ({MAX_MCP_CONTENT_UPLOADS})"
            ));
        }
        if declared_bytes
            .checked_add(request.size_bytes)
            .is_none_or(|total| total > MAX_MCP_CONTENT_UPLOAD_TOTAL_BYTES)
        {
            return Err(anyhow!(
                "content upload declared-size quota exceeded ({MAX_MCP_CONTENT_UPLOAD_TOTAL_BYTES} bytes)"
            ));
        }

        let upload_id = Uuid::new_v4().to_string();
        let upload_dir = uploads_root.join(&upload_id);
        create_new_private_directory(&upload_dir)?;
        let metadata = McpContentUploadMetadata {
            version: MCP_CONTENT_UPLOAD_METADATA_VERSION,
            upload_id: upload_id.clone(),
            client_id: self.client_id.clone(),
            session_id: request.session_id,
            protocol: request.protocol,
            file_name: request.file_name,
            size_bytes: request.size_bytes,
            sha256: request.sha256,
            destination,
            created_at_unix_seconds: unix_seconds_now(),
        };
        if let Err(error) = write_upload_metadata(&upload_dir, &metadata).and_then(|_| {
            create_private_file(&upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE)).map(|_| ())
        }) {
            let _ = fs::remove_dir_all(&upload_dir);
            return Err(error);
        }
        Ok(json!({
            "uploadId": upload_id,
            "nextOffset": 0,
            "sizeBytes": metadata.size_bytes,
            "maxChunkBytes": MAX_MCP_CONTENT_TRANSFER_BYTES
        }))
    }

    pub(super) fn append_content_upload(&self, arguments: &Value) -> Result<Value> {
        let request: AppendContentUploadRequest = serde_json::from_value(arguments.clone())
            .context("invalid append_content_upload arguments")?;
        if request.content_base64.is_empty()
            || request.content_base64.len() > MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
        {
            return Err(anyhow!(
                "contentBase64 must not exceed {MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH} encoded bytes"
            ));
        }
        let chunk = BASE64_STANDARD
            .decode(&request.content_base64)
            .context("contentBase64 is not valid standard Base64")?;
        if chunk.is_empty() || chunk.len() > MAX_MCP_CONTENT_TRANSFER_BYTES {
            return Err(anyhow!(
                "decoded chunk must be between 1 and {MAX_MCP_CONTENT_TRANSFER_BYTES} bytes"
            ));
        }

        let upload_dir = self.owned_upload_dir(&request.upload_id)?;
        let lock = open_upload_lock(&upload_dir)?;
        lock.lock_exclusive()?;
        let metadata = read_upload_metadata(&upload_dir)?;
        ensure_upload_owner(&metadata, &self.client_id, &request.upload_id)?;
        self.require_known_session(&metadata.session_id)?;
        self.require_content_upload_scope(&metadata.session_id)?;
        let payload_path = upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE);
        let mut payload = open_existing_private_file(&payload_path, true)?;
        let current = payload.metadata()?.len();
        if request.offset != current {
            return Err(anyhow!(
                "upload offset mismatch: expected {current}, received {}",
                request.offset
            ));
        }
        let next = current
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow!("upload size overflow"))?;
        if next > metadata.size_bytes || next > MAX_MCP_CONTENT_UPLOAD_BYTES {
            return Err(anyhow!(
                "chunk exceeds the declared upload size of {} bytes",
                metadata.size_bytes
            ));
        }
        payload.seek(SeekFrom::Start(current))?;
        payload.write_all(&chunk)?;
        payload.sync_data()?;
        Ok(json!({
            "uploadId": metadata.upload_id,
            "nextOffset": next,
            "sizeBytes": metadata.size_bytes,
            "complete": next == metadata.size_bytes
        }))
    }

    pub(super) fn start_completed_upload_transfer(&self, arguments: &Value) -> Result<Value> {
        let request: ContentUploadIdRequest = serde_json::from_value(arguments.clone())
            .context("invalid start_transfer upload arguments")?;
        let upload_id = request.upload_id;
        let upload_dir = self.owned_upload_dir(&upload_id)?;
        let lock = open_upload_lock(&upload_dir)?;
        lock.lock_exclusive()?;
        let metadata = read_upload_metadata(&upload_dir)?;
        ensure_upload_owner(&metadata, &self.client_id, &upload_id)?;
        self.require_known_session(&metadata.session_id)?;
        self.require_content_upload_scope(&metadata.session_id)?;
        let payload_path = upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE);
        verify_payload(&payload_path, &metadata)?;
        let value = self
            .call_ipc_value("start_transfer", json!({ "uploadId": &upload_id }))?
            .ok_or_else(|| {
                anyhow!("start_transfer was NOT executed: desktop IPC is not available")
            })?;
        remove_upload_content_files(&upload_dir);
        drop(lock);
        finish_upload_directory_cleanup(&upload_dir, &upload_id);
        Ok(value)
    }

    pub(super) fn cancel_content_upload(&self, arguments: &Value) -> Result<Value> {
        let request: ContentUploadIdRequest = serde_json::from_value(arguments.clone())
            .context("invalid cancel_content_upload arguments")?;
        let upload_id = request.upload_id;
        let upload_dir = self.owned_upload_dir(&upload_id)?;
        let lock = open_upload_lock(&upload_dir)?;
        lock.lock_exclusive()?;
        let metadata = read_upload_metadata(&upload_dir)?;
        ensure_upload_owner(&metadata, &self.client_id, &upload_id)?;
        fs::remove_file(upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE))?;
        fs::remove_file(upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE))?;
        drop(lock);
        let _ = fs::remove_file(upload_dir.join(CONTENT_UPLOAD_LOCK_FILE));
        fs::remove_dir(&upload_dir)?;
        Ok(json!({ "uploadId": upload_id, "cancelled": true }))
    }

    fn content_upload_root(&self) -> Result<PathBuf> {
        Ok(self
            .content_upload_staging_root()?
            .join(MCP_CONTENT_UPLOADS_DIRECTORY))
    }

    fn content_upload_staging_root(&self) -> Result<PathBuf> {
        let store_path = self
            .store_path
            .as_deref()
            .ok_or_else(|| anyhow!("content uploads require PORTMATE_STORE_PATH"))?;
        let parent = store_path
            .parent()
            .ok_or_else(|| anyhow!("content upload directory is unavailable"))?;
        Ok(parent.join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY))
    }

    fn require_content_upload_scope(&self, session_id: &str) -> Result<()> {
        if self.store.mcp_can(
            &self.client_id,
            portmate_core::McpScope::Transfer,
            Some(session_id),
        ) || (self.allow_write && self.store.grants.is_empty())
        {
            Ok(())
        } else {
            Err(anyhow!(
                "MCP transfer grant does not permit content upload for the requested session"
            ))
        }
    }

    fn owned_upload_dir(&self, upload_id: &str) -> Result<PathBuf> {
        Uuid::parse_str(upload_id).context("uploadId must be a UUID")?;
        let root = self.content_upload_root()?;
        require_regular_directory(&root, "content upload root")?;
        let upload_dir = root.join(upload_id);
        let metadata = fs::symlink_metadata(&upload_dir)
            .with_context(|| format!("unknown content upload: {upload_id}"))?;
        if !metadata.is_dir() || metadata_is_link(&metadata) {
            return Err(anyhow!("invalid content upload directory"));
        }
        Ok(upload_dir)
    }
}

fn remove_upload_content_files(upload_dir: &Path) {
    for path in [
        upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE),
        upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE),
    ] {
        if let Err(error) = fs::remove_file(&path) {
            eprintln!(
                "PortMate MCP: completed upload cleanup failed for {}: {error}",
                path.display()
            );
        }
    }
}

fn finish_upload_directory_cleanup(upload_dir: &Path, upload_id: &str) {
    let _ = fs::remove_file(upload_dir.join(CONTENT_UPLOAD_LOCK_FILE));
    if let Err(error) = fs::remove_dir(upload_dir) {
        eprintln!(
            "PortMate MCP: completed upload directory cleanup failed for {upload_id}: {error}"
        );
    }
}

fn require_regular_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| format!("{label} is unavailable"))?;
    if !metadata.is_dir() || metadata_is_link(&metadata) {
        return Err(anyhow!("invalid {label}"));
    }
    Ok(())
}

pub(crate) fn read_upload_metadata(upload_dir: &Path) -> Result<McpContentUploadMetadata> {
    let path = upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE);
    let file = open_existing_private_file(&path, false)?;
    if file.metadata()?.len() > 64 * 1024 {
        return Err(anyhow!("invalid content upload metadata file"));
    }
    let mut bytes = Vec::new();
    file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 {
        return Err(anyhow!("invalid content upload metadata file"));
    }
    let parsed: McpContentUploadMetadata = serde_json::from_slice(&bytes)?;
    if parsed.version != MCP_CONTENT_UPLOAD_METADATA_VERSION {
        return Err(anyhow!("unsupported content upload metadata version"));
    }
    Ok(parsed)
}

pub(crate) fn verify_payload(
    payload_path: &Path,
    metadata: &McpContentUploadMetadata,
) -> Result<()> {
    let mut file = open_existing_private_file(payload_path, false)?;
    let file_metadata = file.metadata()?;
    if file_metadata.len() != metadata.size_bytes {
        return Err(anyhow!(
            "content upload is incomplete: expected {} bytes, received {}",
            metadata.size_bytes,
            file_metadata.len()
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = format!("{:x}", digest.finalize());
    if actual != metadata.sha256 {
        return Err(anyhow!("content upload SHA-256 mismatch"));
    }
    Ok(())
}

fn write_upload_metadata(upload_dir: &Path, metadata: &McpContentUploadMetadata) -> Result<()> {
    let path = upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE);
    let mut file = create_private_file(&path)?;
    file.write_all(&serde_json::to_vec(metadata)?)?;
    file.sync_all()?;
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata_is_link(&metadata) {
        return Err(anyhow!("invalid private content upload directory"));
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn create_private_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    Ok(options.open(path)?)
}

fn create_new_private_directory(path: &Path) -> Result<()> {
    fs::create_dir(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata_is_link(&metadata) {
        return Err(anyhow!("invalid private content upload directory"));
    }
    Ok(())
}

fn open_upload_lock(upload_dir: &Path) -> Result<File> {
    open_private_lock(&upload_dir.join(CONTENT_UPLOAD_LOCK_FILE))
}

fn open_private_lock(path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() || metadata_is_link(&metadata) => {
            return Err(anyhow!("invalid content upload lock file"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    configure_no_follow(&mut options);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path)?;
    validate_opened_private_file(&file)?;
    Ok(file)
}

fn open_existing_private_file(path: &Path, writable: bool) -> Result<File> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata_is_link(&metadata) {
        return Err(anyhow!("invalid content upload file"));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(writable);
    configure_no_follow(&mut options);
    let file = options.open(path)?;
    validate_opened_private_file(&file)?;
    Ok(file)
}

fn configure_no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    #[cfg(windows)]
    options.custom_flags(0x0020_0000);
}

fn validate_opened_private_file(file: &File) -> Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata_is_link(&metadata) {
        return Err(anyhow!("invalid content upload file"));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(anyhow!("content upload files must not have hard links"));
    }
    Ok(())
}

fn metadata_is_link(metadata: &fs::Metadata) -> bool {
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

fn upload_usage(uploads_root: &Path) -> Result<(usize, u64)> {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in fs::read_dir(uploads_root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        count = count.saturating_add(1);
        let declared = read_upload_metadata(&entry.path())
            .map(|metadata| metadata.size_bytes)
            .unwrap_or(MAX_MCP_CONTENT_UPLOAD_BYTES);
        bytes = bytes.saturating_add(declared.min(MAX_MCP_CONTENT_UPLOAD_BYTES));
    }
    Ok((count, bytes))
}

fn cleanup_expired_uploads(uploads_root: &Path, now: u64) -> Result<()> {
    for entry in fs::read_dir(uploads_root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if !kind.is_dir() || kind.is_symlink() {
            continue;
        }
        let upload_dir = entry.path();
        let Ok(metadata) = read_upload_metadata(&upload_dir) else {
            continue;
        };
        if now.saturating_sub(metadata.created_at_unix_seconds) <= MCP_CONTENT_UPLOAD_EXPIRY_SECONDS
        {
            continue;
        }
        let lock = open_upload_lock(&upload_dir)?;
        if lock.try_lock_exclusive().is_ok() {
            remove_upload_content_files(&upload_dir);
            drop(lock);
            finish_upload_directory_cleanup(&upload_dir, &metadata.upload_id);
        }
    }
    Ok(())
}

fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn validate_file_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 255
        || matches!(value, "." | "..")
        || value
            .chars()
            .any(|character| character.is_control() || matches!(character, '\0' | '/' | '\\' | ':'))
    {
        return Err(anyhow!(
            "fileName must be one printable file name without path separators"
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(anyhow!(
            "sha256 must contain exactly 64 lowercase hexadecimal characters"
        ));
    }
    Ok(())
}

fn ensure_upload_owner(
    metadata: &McpContentUploadMetadata,
    client_id: &str,
    upload_id: &str,
) -> Result<()> {
    if metadata.upload_id != upload_id || metadata.client_id != client_id {
        return Err(anyhow!("unknown content upload"));
    }
    Ok(())
}

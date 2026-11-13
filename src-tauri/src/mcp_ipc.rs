use super::*;
use portmate_core::MAX_MCP_BRIDGE_REQUEST_BYTES;

pub(super) const MAX_IPC_ENDPOINT_BYTES: usize = 64 * 1024;
pub(super) const MAX_IPC_REQUEST_BYTES: usize = MAX_MCP_BRIDGE_REQUEST_BYTES;
pub(super) const MAX_IPC_CONNECTIONS: usize = 64;
pub(super) const IPC_IO_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const IPC_REJECTION_TIMEOUT: Duration = Duration::from_millis(100);
const IPC_PUBLICATION_CHECK_INTERVAL: Duration = Duration::from_millis(250);
const IPC_PUBLICATION_SUPERSEDED: &str = "MCP IPC publication was superseded by another desktop";

pub(super) struct IpcPublicationState {
    owner_id: Uuid,
    owner_claimed: bool,
    shutting_down: bool,
    published: Option<PublishedIpcEndpoint>,
}

impl Default for IpcPublicationState {
    fn default() -> Self {
        Self {
            owner_id: Uuid::new_v4(),
            owner_claimed: false,
            shutting_down: false,
            published: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum IpcPublicationHealth {
    Current,
    Restored,
    Superseded,
    ShuttingDown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IpcEndpointFile {
    pub(super) addr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) token_ref: Option<String>,
    pub(super) store_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PublishedIpcEndpoint {
    path: PathBuf,
    endpoint: IpcEndpointFile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IpcRequest {
    pub(super) token: String,
    #[serde(default)]
    pub(super) client_id: String,
    #[serde(default)]
    pub(super) trusted_write: bool,
    pub(super) command: String,
    #[serde(default)]
    pub(super) args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IpcResponse {
    pub(super) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<String>,
}

pub(super) fn start_ipc_server(state: AppState, endpoint_path: PathBuf, token: String) {
    tauri::async_runtime::spawn(async move {
        let mut previous_error = None;
        while !ipc_is_shutting_down(&state) {
            if let Err(error) = serve_ipc(&state, &endpoint_path, &token).await {
                if previous_error.as_ref() != Some(&error) {
                    eprintln!("PortMate: MCP IPC unavailable, retrying: {error}");
                    previous_error = Some(error);
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            } else {
                return;
            }
        }
    });
}

fn ipc_is_shutting_down(state: &AppState) -> bool {
    state
        .ipc_publication
        .lock()
        .map(|publication| publication.shutting_down)
        .unwrap_or(true)
}

async fn serve_ipc(state: &AppState, endpoint_path: &Path, token: &str) -> Result<(), String> {
    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(listener) => listener,
        Err(error) => {
            return Err(format!("failed to bind MCP IPC server: {error}"));
        }
    };
    let addr = match listener.local_addr() {
        Ok(addr) => addr.to_string(),
        Err(error) => {
            return Err(format!("failed to inspect MCP IPC server addr: {error}"));
        }
    };
    if let Some(parent) = endpoint_path.parent() {
        if let Err(error) = fs::create_dir_all(parent) {
            return Err(format!("failed to create IPC endpoint directory: {error}"));
        }
    }
    let endpoint = inline_ipc_endpoint(&addr, token, &state.store_path);
    let publication_state = state.clone();
    let publication_path = endpoint_path.to_path_buf();
    let publication_endpoint = endpoint.clone();
    let publication = tauri::async_runtime::spawn_blocking(move || {
        publish_ipc_endpoint(&publication_state, &publication_path, &publication_endpoint)
    })
    .await
    .map_err(|error| format!("MCP IPC publication task failed: {error}"))?;
    match publication {
        Ok(previous_token_ref) => {
            if let Some(previous_token_ref) = previous_token_ref {
                if let Err(error) = delete_secret_from_keyring(&previous_token_ref) {
                    eprintln!(
                            "PortMate: failed to retire previous MCP IPC token {previous_token_ref}: {error}"
                        );
                }
            }
        }
        Err(error) => {
            if let Some(token_ref) = endpoint.token_ref.as_deref() {
                if let Err(cleanup_error) = delete_secret_from_keyring(token_ref) {
                    eprintln!(
                            "PortMate: failed to clean up unused MCP IPC token {token_ref}: {cleanup_error}"
                        );
                }
            }
            if error == IPC_PUBLICATION_SUPERSEDED {
                return Ok(());
            }
            return Err(format!("failed to write MCP IPC endpoint: {error}"));
        }
    }
    let connection_slots = Arc::new(tokio::sync::Semaphore::new(MAX_IPC_CONNECTIONS));
    let mut publication_check = tokio::time::interval_at(
        tokio::time::Instant::now() + IPC_PUBLICATION_CHECK_INTERVAL,
        IPC_PUBLICATION_CHECK_INTERVAL,
    );
    publication_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut previous_publication_error = None;

    while !ipc_is_shutting_down(state) {
        tokio::select! {
            // Keep discovery alive even when the listener receives continuous
            // traffic. File I/O and cross-process locking stay off the executor.
            biased;
            _ = publication_check.tick() => {
                let check_state = state.clone();
                let path = endpoint_path.to_path_buf();
                let expected = endpoint.clone();
                let check = tauri::async_runtime::spawn_blocking(move || {
                    maintain_ipc_publication(&check_state, &path, &expected)
                }).await.map_err(|error| format!("MCP IPC publication check failed: {error}"))?;
                match check {
                    Ok(IpcPublicationHealth::Current | IpcPublicationHealth::Restored) => {
                        previous_publication_error = None;
                    }
                    // A newer desktop owns discovery now. Never revive the
                    // previous instance when the new one subsequently exits.
                    Ok(IpcPublicationHealth::Superseded | IpcPublicationHealth::ShuttingDown) => return Ok(()),
                    Err(error) => {
                        if previous_publication_error.as_ref() != Some(&error) {
                            eprintln!("PortMate: MCP IPC discovery repair will retry: {error}");
                            previous_publication_error = Some(error);
                        }
                    }
                }
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|error| format!("MCP IPC accept failed: {error}"))?;
                spawn_ipc_client(
                    state.clone(),
                    token.to_string(),
                    stream,
                    Arc::clone(&connection_slots),
                )
                .await;
            }
        }
    }
    Ok(())
}

pub(super) fn inline_ipc_endpoint(addr: &str, token: &str, store_path: &Path) -> IpcEndpointFile {
    IpcEndpointFile {
        addr: addr.to_string(),
        token: Some(token.to_string()),
        token_ref: None,
        store_path: store_path.display().to_string(),
    }
}

pub(super) async fn spawn_ipc_client(
    state: AppState,
    token: String,
    mut stream: TcpStream,
    connection_slots: Arc<tokio::sync::Semaphore>,
) -> bool {
    let permit = match connection_slots.try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            write_ipc_response(
                &mut stream,
                &IpcResponse {
                    ok: false,
                    value: None,
                    error: Some(format!(
                        "MCP IPC connection limit reached ({MAX_IPC_CONNECTIONS})"
                    )),
                },
                IPC_REJECTION_TIMEOUT,
            )
            .await;
            return false;
        }
    };
    tauri::async_runtime::spawn(async move {
        let _permit = permit;
        handle_ipc_client(state, token, stream).await;
    });
    true
}

fn ipc_endpoint_lock_path(endpoint_path: &Path) -> PathBuf {
    let file_name = endpoint_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("portmate-ipc.json");
    endpoint_path.with_file_name(format!("{file_name}.lock"))
}

fn open_ipc_endpoint_lock(endpoint_path: &Path) -> Result<fs::File, String> {
    if let Some(parent) = endpoint_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create MCP IPC endpoint lock directory {}: {error}",
                    parent.display()
                )
            })?;
        }
    }
    let lock_path = ipc_endpoint_lock_path(endpoint_path);
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    options
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };
        options
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let lock = options
        .open(&lock_path)
        .map_err(|error| format!("failed to open MCP IPC endpoint lock: {error}"))?;
    let metadata = lock
        .metadata()
        .map_err(|error| format!("failed to inspect MCP IPC endpoint lock: {error}"))?;
    if !metadata.is_file() {
        return Err("MCP IPC endpoint lock must be a regular file".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err("MCP IPC endpoint lock must not be a reparse point".into());
        }
    }
    Ok(lock)
}

fn lock_ipc_endpoint(endpoint_path: &Path) -> Result<fs::File, String> {
    let lock = open_ipc_endpoint_lock(endpoint_path)?;
    lock.lock()
        .map_err(|error| format!("failed to acquire MCP IPC endpoint lock: {error}"))?;
    Ok(lock)
}

fn write_ipc_publication_owner(lock: &mut fs::File, owner_id: Uuid) -> Result<(), String> {
    lock.rewind()
        .and_then(|()| lock.write_all(owner_id.hyphenated().to_string().as_bytes()))
        .and_then(|()| lock.set_len(36))
        .and_then(|()| lock.sync_all())
        .map_err(|error| format!("failed to record MCP IPC publication owner: {error}"))
}

fn ipc_publication_owner_matches(lock: &mut fs::File, owner_id: Uuid) -> Result<bool, String> {
    let mut bytes = [0u8; 37];
    lock.rewind().map_err(|error| error.to_string())?;
    let mut count = 0;
    while count < bytes.len() {
        let read = lock
            .read(&mut bytes[count..])
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        count += read;
    }
    Ok(&bytes[..count] == owner_id.hyphenated().to_string().as_bytes())
}

/// Restore only a missing endpoint owned by this live listener. The owner marker
/// survives shutdown in the lock file, fencing earlier desktops during upgrades.
pub(super) fn maintain_ipc_publication(
    state: &AppState,
    endpoint_path: &Path,
    endpoint: &IpcEndpointFile,
) -> Result<IpcPublicationHealth, String> {
    let publication = state
        .ipc_publication
        .lock()
        .map_err(|error| error.to_string())?;
    if publication.shutting_down {
        return Ok(IpcPublicationHealth::ShuttingDown);
    }
    if !publication
        .published
        .as_ref()
        .is_some_and(|published| published.path == endpoint_path && published.endpoint == *endpoint)
    {
        return Ok(IpcPublicationHealth::Superseded);
    }
    if read_private_ipc_endpoint_file(endpoint_path)?.as_ref() == Some(endpoint) {
        return Ok(IpcPublicationHealth::Current);
    }
    let mut lock = open_ipc_endpoint_lock(endpoint_path)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Ok(IpcPublicationHealth::Current),
        Err(std::fs::TryLockError::Error(error)) => {
            return Err(format!(
                "failed to lock MCP IPC publication repair: {error}"
            ))
        }
    }
    if !ipc_publication_owner_matches(&mut lock, publication.owner_id)? {
        return Ok(IpcPublicationHealth::Superseded);
    }
    // Recheck while holding the same lock used for publish/retire, so a repair
    // cannot replace a newly published endpoint or follow an unrecognized file.
    match read_private_ipc_endpoint_file(endpoint_path)? {
        Some(current) if current == *endpoint => Ok(IpcPublicationHealth::Current),
        Some(_) => Ok(IpcPublicationHealth::Superseded),
        None => {
            write_ipc_endpoint_file(endpoint_path, endpoint)?;
            Ok(IpcPublicationHealth::Restored)
        }
    }
}

pub(super) fn read_private_ipc_endpoint_file(
    path: &Path,
) -> Result<Option<IpcEndpointFile>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "failed to inspect MCP IPC endpoint {}: {error}",
                path.display()
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("MCP IPC endpoint must be a regular file".to_string());
    }
    if metadata.len() > MAX_IPC_ENDPOINT_BYTES as u64 {
        return Err(format!(
            "MCP IPC endpoint exceeds the {MAX_IPC_ENDPOINT_BYTES}-byte limit"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("MCP IPC endpoint must be owner-only".to_string());
        }
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(path)
        .map_err(|error| format!("failed to open MCP IPC endpoint: {error}"))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect opened MCP IPC endpoint: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() > MAX_IPC_ENDPOINT_BYTES as u64 {
        return Err("opened MCP IPC endpoint is not a bounded regular file".to_string());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if opened_metadata.permissions().mode() & 0o077 != 0 {
            return Err("opened MCP IPC endpoint must be owner-only".to_string());
        }
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_IPC_ENDPOINT_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read MCP IPC endpoint: {error}"))?;
    if bytes.len() > MAX_IPC_ENDPOINT_BYTES {
        return Err(format!(
            "MCP IPC endpoint exceeds the {MAX_IPC_ENDPOINT_BYTES}-byte limit"
        ));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("failed to decode MCP IPC endpoint: {error}"))
}

pub(super) fn valid_ipc_token_ref(token_ref: &str) -> bool {
    let Some(account) = token_ref.strip_prefix("keychain:ipc-") else {
        return false;
    };
    Uuid::parse_str(account).is_ok_and(|uuid| uuid.hyphenated().to_string() == account)
}

fn ipc_endpoint_matches_store(endpoint: &IpcEndpointFile, store_path: &Path) -> bool {
    let endpoint_store_path = Path::new(&endpoint.store_path);
    match (
        fs::canonicalize(endpoint_store_path),
        fs::canonicalize(store_path),
    ) {
        (Ok(endpoint_store_path), Ok(store_path)) => endpoint_store_path == store_path,
        _ => {
            endpoint_store_path.is_absolute()
                && store_path.is_absolute()
                && endpoint_store_path == store_path
        }
    }
}

fn retirable_ipc_token_ref(endpoint: &IpcEndpointFile, store_path: &Path) -> Option<String> {
    let address = endpoint.addr.parse::<std::net::SocketAddr>().ok()?;
    if !address.ip().is_loopback() || !ipc_endpoint_matches_store(endpoint, store_path) {
        return None;
    }
    match (&endpoint.token, &endpoint.token_ref) {
        (None, Some(token_ref)) if valid_ipc_token_ref(token_ref) => Some(token_ref.clone()),
        _ => None,
    }
}

pub(super) fn publish_ipc_endpoint(
    state: &AppState,
    endpoint_path: &Path,
    endpoint: &IpcEndpointFile,
) -> Result<Option<String>, String> {
    let mut publication = state
        .ipc_publication
        .lock()
        .map_err(|error| error.to_string())?;
    if publication.shutting_down {
        return Err("application is shutting down".to_string());
    }
    let mut endpoint_lock = lock_ipc_endpoint(endpoint_path)?;
    if publication.owner_claimed {
        if !ipc_publication_owner_matches(&mut endpoint_lock, publication.owner_id)? {
            return Err(IPC_PUBLICATION_SUPERSEDED.into());
        }
    } else {
        // Claim once, including before an initial publication fails. An older
        // startup retry must not overtake a desktop launched in the meantime.
        write_ipc_publication_owner(&mut endpoint_lock, publication.owner_id)?;
        publication.owner_claimed = true;
    }
    let previous_token_ref = read_private_ipc_endpoint_file(endpoint_path)
        .ok()
        .flatten()
        .and_then(|previous| retirable_ipc_token_ref(&previous, &state.store_path));
    write_ipc_endpoint_file(endpoint_path, endpoint)?;
    publication.published = Some(PublishedIpcEndpoint {
        path: endpoint_path.to_path_buf(),
        endpoint: endpoint.clone(),
    });
    drop(endpoint_lock);
    Ok(previous_token_ref)
}

pub(super) fn retire_ipc_publication_with<DeleteSecret>(
    state: &AppState,
    mut delete_secret: DeleteSecret,
) -> Vec<String>
where
    DeleteSecret: FnMut(&str) -> Result<(), String>,
{
    let published = match state.ipc_publication.lock() {
        Ok(mut publication) => {
            publication.shutting_down = true;
            publication.published.take()
        }
        Err(error) => return vec![format!("failed to lock MCP IPC publication state: {error}")],
    };
    let Some(published) = published else {
        return Vec::new();
    };

    let mut errors = Vec::new();
    match lock_ipc_endpoint(&published.path) {
        Ok(_endpoint_lock) => match read_private_ipc_endpoint_file(&published.path) {
            Ok(Some(current)) if current == published.endpoint => {
                if let Err(error) = fs::remove_file(&published.path) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        errors.push(format!(
                            "failed to remove MCP IPC endpoint {}: {error}",
                            published.path.display()
                        ));
                    }
                }
            }
            Ok(_) => {}
            Err(error) => errors.push(format!(
                "preserved unrecognized MCP IPC endpoint {}: {error}",
                published.path.display()
            )),
        },
        Err(error) => errors.push(error),
    }
    if let Some(token_ref) = published
        .endpoint
        .token_ref
        .as_deref()
        .filter(|token_ref| valid_ipc_token_ref(token_ref))
    {
        if let Err(error) = delete_secret(token_ref) {
            errors.push(format!(
                "failed to retire MCP IPC token {token_ref}: {error}"
            ));
        }
    }
    errors
}

pub(super) fn shutdown_ipc_publication(state: &AppState) {
    for error in retire_ipc_publication_with(state, delete_secret_from_keyring) {
        eprintln!("PortMate: {error}");
    }
}

pub(super) fn write_ipc_endpoint_file(
    path: &Path,
    endpoint: &IpcEndpointFile,
) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(endpoint)
        .map_err(|error| format!("failed to encode MCP IPC endpoint: {error}"))?;
    write_private_atomic_file(path, &bytes, "MCP IPC endpoint")
}

/// Constant-time string comparison so a local process guessing the IPC token
/// can't use response-time differences to narrow down a correct byte prefix.
fn constant_time_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(super) async fn handle_ipc_client(state: AppState, token: String, mut stream: TcpStream) {
    let response = match read_ipc_payload(&mut stream, MAX_IPC_REQUEST_BYTES, IPC_IO_TIMEOUT).await
    {
        Ok(raw) => match serde_json::from_slice::<IpcRequest>(&raw) {
            Ok(mut request) if constant_time_str_eq(&request.token, &token) => {
                match bind_ipc_request_identity(&state, &mut request).map(|()| request) {
                    Ok(request) => match handle_ipc_request(state.clone(), request).await {
                        Ok(value) => IpcResponse {
                            ok: true,
                            value: Some(value),
                            error: None,
                        },
                        Err(error) => IpcResponse {
                            ok: false,
                            value: None,
                            error: Some(error),
                        },
                    },
                    Err(error) => IpcResponse {
                        ok: false,
                        value: None,
                        error: Some(error),
                    },
                }
            }
            Ok(_) => IpcResponse {
                ok: false,
                value: None,
                error: Some("invalid IPC token".to_string()),
            },
            Err(error) => IpcResponse {
                ok: false,
                value: None,
                error: Some(format!("invalid IPC request: {error}")),
            },
        },
        Err(error) => IpcResponse {
            ok: false,
            value: None,
            error: Some(error),
        },
    };

    write_ipc_response(&mut stream, &response, IPC_IO_TIMEOUT).await;
}

pub(super) fn bind_ipc_request_identity(
    state: &AppState,
    request: &mut IpcRequest,
) -> Result<(), String> {
    let expected_client_id = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .mcp_resolved_client_id(None);
    if request.client_id.trim() != expected_client_id {
        return Err(format!(
            "IPC clientId does not match the desktop-selected MCP identity `{expected_client_id}`"
        ));
    }
    // Privilege is derived exclusively from the selected stored grant.
    request.client_id = expected_client_id;
    request.trusted_write = false;
    Ok(())
}

async fn write_ipc_response(stream: &mut TcpStream, response: &IpcResponse, timeout: Duration) {
    if let Ok(bytes) = serde_json::to_vec(response) {
        let _ = tokio::time::timeout(timeout, async {
            stream.write_all(&bytes).await?;
            stream.shutdown().await
        })
        .await;
    }
}

pub(super) async fn read_ipc_payload(
    stream: &mut TcpStream,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    let read = tokio::time::timeout(timeout, async {
        (&mut *stream)
            .take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut raw)
            .await
    })
    .await
    .map_err(|_| format!("IPC request timed out after {} ms", timeout.as_millis()))?
    .map_err(|error| format!("IPC read failed: {error}"))?;
    if read > max_bytes {
        return Err(format!("IPC request exceeds the {max_bytes}-byte limit"));
    }
    Ok(raw)
}

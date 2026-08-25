use crate::keyring_store::read_secret_from_keyring;
use crate::socket_io::read_stream_chunk_before;
use anyhow::{anyhow, Context, Result};
use portmate_core::MAX_MCP_BRIDGE_REQUEST_BYTES;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use uuid::Uuid;

const MAX_IPC_REQUEST_BYTES: usize = MAX_MCP_BRIDGE_REQUEST_BYTES;
const MAX_IPC_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const MAX_IPC_ENDPOINT_BYTES: usize = 64 * 1024;
const MAX_IPC_TOKEN_BYTES: usize = 4096;
const IPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const IPC_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const IPC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) fn ipc_value_to_text(value: Value) -> Result<String> {
    if let Some(text) = value.as_str() {
        Ok(text.to_string())
    } else {
        Ok(serde_json::to_string_pretty(&value)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IpcEndpointFile {
    pub(crate) addr: String,
    #[serde(default)]
    pub(crate) token: Option<String>,
    #[serde(default)]
    pub(crate) token_ref: Option<String>,
    pub(crate) store_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IpcRequest {
    pub(crate) token: String,
    #[serde(default)]
    pub(crate) client_id: String,
    #[serde(default)]
    pub(crate) trusted_write: bool,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IpcResponse {
    ok: bool,
    value: Option<Value>,
    error: Option<String>,
}

pub(crate) fn call_ipc_value(
    endpoint: &IpcEndpointFile,
    store_path: &Path,
    client_id: &str,
    trusted_write: bool,
    command: &str,
    args: Value,
) -> Result<Option<Value>> {
    let addr = validate_ipc_endpoint(endpoint, store_path)?;
    let mut stream = match TcpStream::connect_timeout(&addr, IPC_CONNECT_TIMEOUT) {
        Ok(stream) => stream,
        Err(_) => return Ok(None),
    };
    // Some socket runtimes reject SO_SNDTIMEO with EINVAL. The request is
    // bounded and the IPC endpoint is loopback, so a blocking write is safe;
    // the response still has an explicit deadline below.
    if let Err(error) = stream.set_write_timeout(Some(IPC_WRITE_TIMEOUT)) {
        if error.kind() != io::ErrorKind::InvalidInput {
            return Err(error).context("failed to configure desktop IPC write timeout");
        }
    }
    let request = IpcRequest {
        token: endpoint_ipc_token(endpoint)?,
        client_id: client_id.to_string(),
        trusted_write,
        command: command.to_string(),
        args,
    };
    let request = encode_ipc_request(&request, MAX_IPC_REQUEST_BYTES)?;
    stream
        .write_all(&request)
        .context("failed to write desktop IPC request")?;
    stream
        .shutdown(Shutdown::Write)
        .context("failed to finish desktop IPC request")?;
    let raw =
        read_ipc_response_with_limits(&mut stream, MAX_IPC_RESPONSE_BYTES, IPC_RESPONSE_TIMEOUT)?;
    let response = serde_json::from_slice::<IpcResponse>(&raw)?;
    if response.ok {
        Ok(Some(response.value.unwrap_or(Value::Null)))
    } else {
        Err(anyhow!(
            "desktop IPC error: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        ))
    }
}

pub(crate) fn load_ipc_endpoint(store_path: &Path) -> Option<IpcEndpointFile> {
    let endpoint_path = store_path.with_file_name("portmate-ipc.json");
    let raw = match read_ipc_endpoint_file(&endpoint_path) {
        Ok(raw) => raw,
        Err(error)
            if error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::NotFound) =>
        {
            return None;
        }
        Err(error) => {
            eprintln!("PortMate MCP ignored unreadable desktop IPC endpoint: {error}");
            return None;
        }
    };
    let endpoint = serde_json::from_slice::<IpcEndpointFile>(&raw).ok()?;
    if let Err(error) = validate_ipc_endpoint(&endpoint, store_path) {
        eprintln!("PortMate MCP ignored invalid desktop IPC endpoint: {error}");
        return None;
    }
    Some(endpoint)
}

pub(crate) fn read_ipc_endpoint_file(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(anyhow!("desktop IPC endpoint must be a regular file"));
    }
    if metadata.len() > MAX_IPC_ENDPOINT_BYTES as u64 {
        return Err(anyhow!(
            "desktop IPC endpoint exceeds the {MAX_IPC_ENDPOINT_BYTES}-byte limit"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(anyhow!(
                "desktop IPC endpoint permissions must not allow group or world access"
            ));
        }
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    let opened_metadata = file.metadata()?;
    if !opened_metadata.is_file() || opened_metadata.len() > MAX_IPC_ENDPOINT_BYTES as u64 {
        return Err(anyhow!(
            "opened desktop IPC endpoint must be a bounded regular file"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if opened_metadata.permissions().mode() & 0o077 != 0 {
            return Err(anyhow!(
                "desktop IPC endpoint permissions must not allow group or world access"
            ));
        }
    }
    let mut raw = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_IPC_ENDPOINT_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut raw)?;
    if raw.len() > MAX_IPC_ENDPOINT_BYTES {
        return Err(anyhow!(
            "desktop IPC endpoint exceeds the {MAX_IPC_ENDPOINT_BYTES}-byte limit"
        ));
    }
    Ok(raw)
}

pub(crate) fn validate_ipc_endpoint(
    endpoint: &IpcEndpointFile,
    store_path: &Path,
) -> Result<SocketAddr> {
    let addr = endpoint
        .addr
        .parse::<SocketAddr>()
        .map_err(|error| anyhow!("desktop IPC address must be an IP socket address: {error}"))?;
    if !addr.ip().is_loopback() {
        return Err(anyhow!("desktop IPC address must be loopback; got {addr}"));
    }
    if !paths_refer_to_same_store(Path::new(&endpoint.store_path), store_path) {
        return Err(anyhow!(
            "desktop IPC endpoint storePath does not match PORTMATE_STORE_PATH"
        ));
    }
    match (&endpoint.token, &endpoint.token_ref) {
        (Some(token), None) if valid_inline_ipc_token(token) => {}
        (None, Some(token_ref)) if valid_ipc_token_ref(token_ref) => {}
        (Some(_), Some(_)) => {
            return Err(anyhow!(
                "desktop IPC endpoint must not contain both token and tokenRef"
            ))
        }
        (Some(_), None) => return Err(anyhow!("desktop IPC endpoint token is invalid")),
        (None, Some(_)) => return Err(anyhow!("desktop IPC endpoint tokenRef is invalid")),
        (None, None) => return Err(anyhow!("desktop IPC endpoint is missing token/tokenRef")),
    }
    Ok(addr)
}

fn paths_refer_to_same_store(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => absolute_path(left)
            .is_some_and(|left| absolute_path(right).is_some_and(|right| left == right)),
    }
}

fn absolute_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        std::env::current_dir().ok().map(|cwd| cwd.join(path))
    }
}

fn valid_inline_ipc_token(token: &str) -> bool {
    !token.trim().is_empty() && token.len() <= MAX_IPC_TOKEN_BYTES
}

fn valid_ipc_token_ref(token_ref: &str) -> bool {
    let Some(account) = token_ref.strip_prefix("keychain:ipc-") else {
        return false;
    };
    Uuid::parse_str(account).is_ok_and(|uuid| uuid.hyphenated().to_string() == account)
}

pub(crate) fn endpoint_ipc_token(endpoint: &IpcEndpointFile) -> Result<String> {
    if let Some(token_ref) = endpoint
        .token_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !valid_ipc_token_ref(token_ref) {
            return Err(anyhow!("desktop IPC endpoint tokenRef is invalid"));
        }
        return read_secret_from_keyring(token_ref);
    }
    endpoint
        .token
        .clone()
        .filter(|value| valid_inline_ipc_token(value))
        .ok_or_else(|| anyhow!("desktop IPC endpoint is missing token/tokenRef"))
}

pub(crate) fn encode_ipc_request(request: &IpcRequest, max_bytes: usize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(request)?;
    if bytes.len() > max_bytes {
        return Err(anyhow!(
            "desktop IPC request exceeds the {max_bytes}-byte limit"
        ));
    }
    Ok(bytes)
}

pub(crate) fn read_ipc_response_with_limits(
    stream: &mut TcpStream,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = read_stream_chunk_before(
            stream,
            &mut buffer,
            deadline,
            "desktop IPC response deadline exceeded",
        )?;
        if read == 0 {
            break;
        }
        if raw.len().saturating_add(read) > max_bytes {
            return Err(anyhow!(
                "desktop IPC response exceeds the {max_bytes}-byte limit"
            ));
        }
        raw.extend_from_slice(&buffer[..read]);
    }
    Ok(raw)
}

use chrono::Utc;
use keyring_core::Entry;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use portmate_core::{
    compute_ssh_sha256_fingerprint, prompt_templates, redact_secrets, resource_templates,
    tool_definitions, AuthMethod, ConnectionConfig, EventDirection, EventStream, HostKeyDecision,
    HostKeyEvaluation, HostKeyMode, HostKeyObservation, HostKeyScope, HostKeyStore, IdentityRef,
    IdentitySource, McpGrant, McpScope, SessionEvent, SessionKind, SessionProfile, SessionStatus,
    SessionStore, SessionSummary, SshConnection, SysmonSnapshot, TimelineMark, TransferProtocol,
    TransferStatus, TransferTask, TriggerAction, TrustedHostKey, TunnelMode, TunnelSpec,
};
use rusqlite::{params, Connection as SqliteConnection};
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::agent::AgentIdentity;
use russh::keys::{
    decode_secret_key, load_secret_key, ssh_key, HashAlg, PrivateKeyWithHashAlg, PublicKeyBase64,
};
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect};
use russh_sftp::{client::SftpSession, protocol::OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
struct AgentIdentityFilter {
    label: String,
    fingerprint_sha256: Option<String>,
    path: Option<String>,
}

#[derive(Clone, Default)]
struct PortMateAgentSigner;

#[derive(Debug)]
enum PortMateAgentAuthError {
    Send(russh::SendError),
    Agent(String),
}

impl From<russh::SendError> for PortMateAgentAuthError {
    fn from(error: russh::SendError) -> Self {
        Self::Send(error)
    }
}

impl std::fmt::Display for PortMateAgentAuthError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Send(error) => write!(formatter, "{error}"),
            Self::Agent(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for PortMateAgentAuthError {}

impl russh::Signer for PortMateAgentSigner {
    type Error = PortMateAgentAuthError;

    fn auth_sign(
        &mut self,
        key: &AgentIdentity,
        hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let key = key.clone();
        async move {
            sign_with_ssh_agent_on_thread(key, hash_alg, to_sign)
                .await
                .map_err(PortMateAgentAuthError::Agent)
        }
    }
}

const STORE_FILE_NAME: &str = "portmate-store.sqlite3";
const LEGACY_JSON_STORE_FILE_NAME: &str = "portmate-store.json";
const STORE_KEY: &str = "session-store";
const SQLITE_SCHEMA_VERSION: &str = "2";
const STREAM_PERSIST_INTERVAL: Duration = Duration::from_secs(2);
const MODEM_SOH: u8 = 0x01;
const MODEM_STX: u8 = 0x02;
const MODEM_EOT: u8 = 0x04;
const MODEM_ACK: u8 = 0x06;
const MODEM_NAK: u8 = 0x15;
const MODEM_CAN: u8 = 0x18;
const MODEM_CRC_REQUEST: u8 = b'C';
const MODEM_EOF: u8 = 0x1a;
const XMODEM_BLOCK_SIZE: usize = 128;
const YMODEM_BLOCK_SIZE: usize = 1024;
const TRANSFER_CANCELLED_MESSAGE: &str = "transfer cancelled";
const MCP_HTTP_TOKEN_REF: &str = "keychain:mcp-http-token";
const MCP_HTTP_DEFAULT_ADDR: &str = "127.0.0.1:8787";
const DEFAULT_LOG_QUERY_LIMIT: u64 = 100;
const MAX_LOG_QUERY_LIMIT: u64 = 1000;

#[derive(Clone)]
pub struct AppState {
    app_handle: Option<AppHandle>,
    pub store: Arc<Mutex<SessionStore>>,
    ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
    tunnels: Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    transfer_cancellations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    transfer_lanes: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    one_time_host_keys: Arc<Mutex<HashMap<String, Vec<portmate_core::TrustedHostKey>>>>,
    store_path: PathBuf,
}

struct SshRuntime {
    runtime_id: String,
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    jump_handles: Vec<Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>>,
    writer: Arc<tokio::sync::Mutex<ChannelWriteHalf<client::Msg>>>,
    tap: broadcast::Sender<Vec<u8>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    closed: Arc<AtomicBool>,
}

struct ShellRuntime {
    runtime_id: String,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    tap: broadcast::Sender<Vec<u8>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    closed: Arc<AtomicBool>,
}

struct TcpRuntime {
    runtime_id: String,
    writer: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
}

struct SerialRuntime {
    runtime_id: String,
    writer: Option<Arc<Mutex<SerialPortHandle>>>,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
}

type SerialPortHandle = Box<dyn serialport::SerialPort>;
type SerialPortPair = (SerialPortHandle, SerialPortHandle);

#[derive(Clone)]
struct TunnelRuntime {
    session_id: String,
    spec: TunnelSpec,
    metrics: Arc<TunnelMetrics>,
    closed: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
struct TunnelForwardTarget {
    spec: TunnelSpec,
    metrics: Arc<TunnelMetrics>,
}

#[derive(Debug, Default)]
struct TunnelMetrics {
    active_connections: AtomicU64,
    total_connections: AtomicU64,
    tcp_to_ssh_bytes: AtomicU64,
    ssh_to_tcp_bytes: AtomicU64,
    last_activity: Mutex<Option<String>>,
    last_error: Mutex<Option<String>>,
}

impl TunnelMetrics {
    fn connection_opened(&self) {
        self.total_connections.fetch_add(1, Ordering::SeqCst);
        self.active_connections.fetch_add(1, Ordering::SeqCst);
        self.touch();
    }

    fn connection_closed(&self) {
        let _ = self
            .active_connections
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(value.saturating_sub(1))
            });
        self.touch();
    }

    fn add_tcp_to_ssh_bytes(&self, bytes: usize) {
        self.tcp_to_ssh_bytes
            .fetch_add(bytes as u64, Ordering::SeqCst);
        self.touch();
    }

    fn add_ssh_to_tcp_bytes(&self, bytes: usize) {
        self.ssh_to_tcp_bytes
            .fetch_add(bytes as u64, Ordering::SeqCst);
        self.touch();
    }

    fn record_error(&self, error: &str) {
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = Some(error.to_string());
        }
        self.touch();
    }

    fn touch(&self) {
        if let Ok(mut last_activity) = self.last_activity.lock() {
            *last_activity = Some(Utc::now().to_rfc3339());
        }
    }

    fn snapshot(&self, spec: TunnelSpec) -> TunnelStatus {
        TunnelStatus {
            spec,
            active_connections: self.active_connections.load(Ordering::SeqCst),
            total_connections: self.total_connections.load(Ordering::SeqCst),
            tcp_to_ssh_bytes: self.tcp_to_ssh_bytes.load(Ordering::SeqCst),
            ssh_to_tcp_bytes: self.ssh_to_tcp_bytes.load(Ordering::SeqCst),
            last_activity: self
                .last_activity
                .lock()
                .ok()
                .and_then(|value| value.clone()),
            last_error: self.last_error.lock().ok().and_then(|value| value.clone()),
        }
    }
}

#[derive(Clone)]
struct RuntimeRegistry {
    ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
}

#[derive(Clone)]
struct SessionIo {
    app_handle: Option<AppHandle>,
    store: Arc<Mutex<SessionStore>>,
    runtimes: RuntimeRegistry,
    store_path: PathBuf,
}

struct SshConnectRequest<'a> {
    config: Arc<client::Config>,
    store: Arc<Mutex<SessionStore>>,
    store_path: PathBuf,
    profile: &'a SessionProfile,
    ssh: &'a SshConnection,
    host_keys: HostKeyStore,
    one_time_host_keys: Vec<TrustedHostKey>,
    observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    host_key_error: Arc<Mutex<Option<String>>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    password: Option<&'a str>,
    passphrase: Option<&'a str>,
}

struct SshHandlerParams {
    profile_id: String,
    host: String,
    port: u16,
    alias: Option<String>,
    policy: portmate_core::HostKeyPolicy,
    host_keys: HostKeyStore,
    one_time_host_key_ids: Vec<String>,
    observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    host_key_error: Arc<Mutex<Option<String>>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
}

struct JumpHostKeyScanRequest<'a> {
    state: &'a AppState,
    profile: &'a SessionProfile,
    ssh: &'a SshConnection,
    config: Arc<client::Config>,
    target_handler: HostKeyScanHandler,
    password: Option<&'a str>,
    passphrase: Option<&'a str>,
}

#[derive(Clone)]
struct TransferProgressContext {
    state: AppState,
    task_id: String,
    cancel: Arc<AtomicBool>,
    last_emit: Arc<Mutex<Instant>>,
    started: Instant,
    rate_baseline_bytes: Arc<AtomicU64>,
    rate_limit_bytes_per_second: Option<u64>,
}

struct EstablishedSshRuntime {
    runtime_id: String,
    runtime: SshRuntime,
    tap: broadcast::Sender<Vec<u8>>,
    read_half: ChannelReadHalf,
    auth_method: AuthMethod,
    closed: Arc<AtomicBool>,
}

struct SshReadTask {
    state: AppState,
    profile: SessionProfile,
    runtime_id: String,
    tap: broadcast::Sender<Vec<u8>>,
    read_half: ChannelReadHalf,
    closed: Arc<AtomicBool>,
}

impl AppState {
    fn runtimes(&self) -> RuntimeRegistry {
        RuntimeRegistry {
            ssh: Arc::clone(&self.ssh),
            shell: Arc::clone(&self.shell),
            tcp: Arc::clone(&self.tcp),
            serial: Arc::clone(&self.serial),
        }
    }

    fn session_io(&self) -> SessionIo {
        SessionIo {
            app_handle: self.app_handle.clone(),
            store: Arc::clone(&self.store),
            runtimes: self.runtimes(),
            store_path: self.store_path.clone(),
        }
    }
}

#[derive(Debug)]
struct PortMateSshHandler {
    profile_id: String,
    host: String,
    port: u16,
    alias: Option<String>,
    policy: portmate_core::HostKeyPolicy,
    host_keys: HostKeyStore,
    one_time_host_key_ids: Vec<String>,
    observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    host_key_error: Arc<Mutex<Option<String>>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
}

struct HostKeyScanHandler {
    host: String,
    port: u16,
    alias: Option<String>,
    observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
}

impl client::Handler for PortMateSshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let observation = HostKeyObservation {
            host: self.host.clone(),
            port: self.port,
            alias: self.alias.clone(),
            algorithm: server_public_key.algorithm().to_string(),
            public_key_base64: server_public_key.public_key_base64(),
        };
        *self
            .observed_key
            .lock()
            .expect("host key observation lock poisoned") = Some(observation.clone());

        let evaluation = self
            .host_keys
            .evaluate(&self.profile_id, &self.policy, &observation);
        let accepted = match evaluation {
            Ok(HostKeyEvaluation::Trusted {
                matched_key_id,
                fingerprint_sha256,
            }) if trusted_host_key_allowed(
                &self.policy,
                &matched_key_id,
                &self.one_time_host_key_ids,
            ) =>
            {
                *self
                    .host_key_error
                    .lock()
                    .expect("host key error lock poisoned") = None;
                let _ = fingerprint_sha256;
                true
            }
            Ok(HostKeyEvaluation::Trusted {
                fingerprint_sha256, ..
            }) => {
                *self
                    .host_key_error
                    .lock()
                    .expect("host key error lock poisoned") = Some(format!(
                    "SSH host key requires confirmation for this connection: {fingerprint_sha256}"
                ));
                false
            }
            Ok(HostKeyEvaluation::Unknown {
                alias,
                fingerprint_sha256,
                ..
            }) if self.policy.mode == HostKeyMode::TrustOnFirstUse => {
                *self
                    .host_key_error
                    .lock()
                    .expect("host key error lock poisoned") = None;
                let _ = (alias, fingerprint_sha256);
                true
            }
            Ok(other) => {
                *self
                    .host_key_error
                    .lock()
                    .expect("host key error lock poisoned") =
                    Some(describe_host_key_rejection(&other));
                false
            }
            Err(error) => {
                *self
                    .host_key_error
                    .lock()
                    .expect("host key error lock poisoned") =
                    Some(format!("host key fingerprint 计算失败: {error}"));
                false
            }
        };

        Ok(accepted)
    }

    fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let forwards = Arc::clone(&self.remote_forwards);
        let connected_address = connected_address.to_string();
        let originator_address = originator_address.to_string();
        async move {
            let target = {
                let forwards = forwards
                    .lock()
                    .expect("remote forward target map lock poisoned");
                let key = remote_forward_key(&connected_address, connected_port as u16);
                forwards
                    .get(&key)
                    .or_else(|| forwards.get(&remote_forward_port_key(connected_port as u16)))
                    .cloned()
            };
            if let Some(target) = target {
                tauri::async_runtime::spawn(async move {
                    target.metrics.connection_opened();
                    if let Err(error) = handle_remote_tunnel_client(
                        channel,
                        target.spec.clone(),
                        originator_address,
                        originator_port as u16,
                        Arc::clone(&target.metrics),
                    )
                    .await
                    {
                        target.metrics.record_error(&error);
                        eprintln!("PortMate: remote SSH tunnel client failed: {error}");
                    }
                    target.metrics.connection_closed();
                });
            }
            Ok(())
        }
    }
}

impl client::Handler for HostKeyScanHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        *self
            .observed_key
            .lock()
            .expect("host key scan observation lock poisoned") = Some(HostKeyObservation {
            host: self.host.clone(),
            port: self.port,
            alias: self.alias.clone(),
            algorithm: server_public_key.algorithm().to_string(),
            public_key_base64: server_public_key.public_key_base64(),
        });
        Ok(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyDecisionRequest {
    pub profile_id: String,
    pub observation: HostKeyObservation,
    pub decision: portmate_core::HostKeyDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTransferRequest {
    pub session_id: String,
    pub protocol: TransferProtocol,
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTunnelRequest {
    pub session_id: String,
    pub mode: TunnelMode,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub spec: TunnelSpec,
    pub active_connections: u64,
    pub total_connections: u64,
    pub tcp_to_ssh_bytes: u64,
    pub ssh_to_tcp_bytes: u64,
    pub last_activity: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileProperties {
    pub name: String,
    pub path: String,
    pub remote: bool,
    pub kind: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub permissions: Option<u32>,
    pub modified: Option<String>,
    pub accessed: Option<String>,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFilesRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePropertiesRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePathRequest {
    pub session_id: Option<String>,
    pub old_path: String,
    pub new_path: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChmodPathRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub mode: u32,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialLineRequest {
    pub session_id: String,
    pub dtr: Option<bool>,
    pub rts: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretWriteRequest {
    pub secret_ref: Option<String>,
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretWriteResponse {
    pub secret_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpHttpConfig {
    pub endpoint: String,
    pub token_ref: String,
    pub token_available: bool,
    pub default_origin: String,
    pub start_command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpHttpTokenResponse {
    pub config: McpHttpConfig,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxSessionInfo {
    pub name: String,
    pub windows: u32,
    pub attached: u32,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxPaneInfo {
    pub session: String,
    pub window_index: u32,
    pub pane_index: u32,
    pub pane_id: String,
    pub active: bool,
    pub command: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxState {
    pub sessions: Vec<TmuxSessionInfo>,
    pub panes: Vec<TmuxPaneInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyScanResult {
    pub label: Option<String>,
    pub observation: HostKeyObservation,
    pub evaluation: HostKeyEvaluation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostsImportRequest {
    pub profile_id: String,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyUpdateRequest {
    pub key_id: String,
    pub profile_id: Option<String>,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub scope: HostKeyScope,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustScannedHostKeyRequest {
    pub profile: SessionProfile,
    pub observation: HostKeyObservation,
    pub decision: HostKeyDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IpcEndpointFile {
    addr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token_ref: Option<String>,
    store_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IpcRequest {
    token: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    trusted_write: bool,
    command: String,
    #[serde(default)]
    args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IpcResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.summaries())
}

#[tauri::command]
fn read_screen(state: State<'_, AppState>, session_id: String) -> Result<String, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    store
        .screen(&session_id)
        .ok_or_else(|| format!("no screen data for session: {session_id}"))
}

#[tauri::command]
fn tail_log(
    state: State<'_, AppState>,
    session_id: String,
    limit: Option<u64>,
) -> Result<Vec<SessionEvent>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.tail_log(&session_id, bounded_log_query_limit(limit)))
}

#[tauri::command]
fn search_logs(
    state: State<'_, AppState>,
    query: String,
    session_id: Option<String>,
    limit: Option<u64>,
) -> Result<Vec<SessionEvent>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.search_logs(
        &query,
        session_id.as_deref(),
        bounded_log_query_limit(limit),
    ))
}

#[tauri::command]
async fn send_text(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<SessionEvent, String> {
    send_text_inner(state.inner().session_io(), session_id, text).await
}

#[tauri::command]
async fn send_bytes(
    state: State<'_, AppState>,
    session_id: String,
    bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    send_bytes_inner(state.inner().session_io(), session_id, bytes).await
}

#[tauri::command]
async fn send_key(
    state: State<'_, AppState>,
    session_id: String,
    key: String,
) -> Result<SessionEvent, String> {
    let text = terminal_key_sequence(&key)?;
    send_text_inner(state.inner().session_io(), session_id, text).await
}

#[tauri::command]
async fn run_command(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
) -> Result<SessionEvent, String> {
    let mut text = command;
    if !text.ends_with('\n') && !text.ends_with('\r') {
        text.push('\n');
    }
    send_text_inner(state.inner().session_io(), session_id, text).await
}

async fn send_text_inner(
    io: SessionIo,
    session_id: String,
    text: String,
) -> Result<SessionEvent, String> {
    let wire_text = outbound_text_for_session(&io.store, &session_id, &text)?;
    write_session_bytes(
        &io.store,
        &io.runtimes.ssh,
        &io.runtimes.shell,
        &io.runtimes.tcp,
        &io.runtimes.serial,
        &session_id,
        wire_text.as_bytes(),
    )
    .await?;

    let mut store = io.store.lock().map_err(|error| error.to_string())?;
    let event = store.send_text("desktop-user", &session_id, &text)?;
    save_store(&io.store_path, &store)?;
    Ok(event)
}

async fn send_bytes_inner(
    io: SessionIo,
    session_id: String,
    bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    let wire_bytes = outbound_bytes_for_session(&io.store, &session_id, &bytes)?;
    write_session_bytes(
        &io.store,
        &io.runtimes.ssh,
        &io.runtimes.shell,
        &io.runtimes.tcp,
        &io.runtimes.serial,
        &session_id,
        &wire_bytes,
    )
    .await?;

    let mut store = io.store.lock().map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let event = store.send_text("desktop-user", &session_id, &text)?;
    save_store(&io.store_path, &store)?;
    Ok(event)
}

async fn write_session_bytes(
    store: &Arc<Mutex<SessionStore>>,
    ssh: &Arc<Mutex<HashMap<String, SshRuntime>>>,
    shell: &Arc<Mutex<HashMap<String, ShellRuntime>>>,
    tcp: &Arc<Mutex<HashMap<String, TcpRuntime>>>,
    serial: &Arc<Mutex<HashMap<String, SerialRuntime>>>,
    session_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let writer = {
        let connections = ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    };

    if let Some(writer) = writer {
        let writer = writer.lock().await;
        writer
            .data(bytes)
            .await
            .map_err(|error| format!("SSH 写入失败: {error}"))?;
    } else {
        let writer = {
            let connections = shell.lock().map_err(|error| error.to_string())?;
            connections
                .get(session_id)
                .map(|runtime| Arc::clone(&runtime.writer))
        };
        if let Some(writer) = writer {
            let mut writer = writer.lock().map_err(|error| error.to_string())?;
            writer
                .write_all(bytes)
                .map_err(|error| format!("Shell PTY 写入失败: {error}"))?;
            writer
                .flush()
                .map_err(|error| format!("Shell PTY 刷新失败: {error}"))?;
        } else {
            let writer = {
                let connections = tcp.lock().map_err(|error| error.to_string())?;
                connections
                    .get(session_id)
                    .map(|runtime| Arc::clone(&runtime.writer))
            };
            if let Some(writer) = writer {
                let mut writer = writer.lock().await;
                writer
                    .write_all(bytes)
                    .await
                    .map_err(|error| format!("TCP/Telnet 写入失败: {error}"))?;
            } else {
                let serial_writer = {
                    let connections = serial.lock().map_err(|error| error.to_string())?;
                    connections
                        .get(session_id)
                        .map(|runtime| runtime.writer.as_ref().map(Arc::clone))
                };
                match serial_writer {
                    Some(Some(writer)) => {
                        let mut writer = writer.lock().map_err(|error| error.to_string())?;
                        writer
                            .write_all(bytes)
                            .map_err(|error| format!("串口写入失败: {error}"))?;
                        writer
                            .flush()
                            .map_err(|error| format!("串口刷新失败: {error}"))?;
                    }
                    Some(None) => return Err("串口正在重连，无法发送输入".to_string()),
                    None if profile_requires_runtime(store, session_id)? => {
                        return Err("会话尚未连接，无法发送输入".to_string());
                    }
                    None => {}
                }
            }
        }
    }
    Ok(())
}

fn outbound_text_for_session(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    text: &str,
) -> Result<String, String> {
    let is_telnet = {
        let store = store.lock().map_err(|error| error.to_string())?;
        store
            .profile(session_id)
            .is_some_and(|profile| matches!(profile.connection, ConnectionConfig::Telnet(_)))
    };
    if is_telnet {
        Ok(encode_telnet_outbound_text(text))
    } else {
        Ok(text.to_string())
    }
}

fn outbound_bytes_for_session(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    bytes: &[u8],
) -> Result<Vec<u8>, String> {
    let is_telnet = {
        let store = store.lock().map_err(|error| error.to_string())?;
        store
            .profile(session_id)
            .is_some_and(|profile| matches!(profile.connection, ConnectionConfig::Telnet(_)))
    };
    Ok(if is_telnet {
        encode_telnet_outbound_bytes(bytes)
    } else {
        bytes.to_vec()
    })
}

fn encode_telnet_outbound_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut previous = '\0';
    for ch in text.chars() {
        match ch {
            '\n' if previous != '\r' => output.push_str("\r\n"),
            _ => output.push(ch),
        }
        previous = ch;
    }
    output
}

fn encode_telnet_outbound_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    for byte in bytes {
        output.push(*byte);
        if *byte == TELNET_IAC {
            output.push(*byte);
        }
    }
    output
}

#[tauri::command]
async fn resize_session(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<SessionSummary, String> {
    if cols == 0 || rows == 0 {
        return Err("terminal size must be non-zero".to_string());
    }

    let ssh_writer = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(&session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = ssh_writer {
        let writer = writer.lock().await;
        writer
            .window_change(u32::from(cols), u32::from(rows), 0, 0)
            .await
            .map_err(|error| format!("SSH resize failed: {error}"))?;
    }

    let shell_master = {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections
            .get(&session_id)
            .map(|runtime| Arc::clone(&runtime.master))
    };
    if let Some(master) = shell_master {
        let master = master.lock().map_err(|error| error.to_string())?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Shell PTY resize failed: {error}"))?;
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    profile.terminal.cols = cols;
    profile.terminal.rows = rows;
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == session_id)
        .ok_or_else(|| format!("session summary is missing: {session_id}"))?;
    save_store(&state.store_path, &store)?;
    Ok(summary)
}

#[tauri::command]
fn save_session_profile(
    state: State<'_, AppState>,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let summary = store.upsert_profile(normalize_session_profile(profile));
    save_store(&state.store_path, &store)?;
    Ok(summary)
}

#[tauri::command]
async fn open_session(
    state: State<'_, AppState>,
    session_id: String,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<SessionSummary, String> {
    let state = state.inner().clone();
    open_session_inner(state, session_id, password, passphrase).await
}

async fn open_session_inner(
    state: AppState,
    session_id: String,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<SessionSummary, String> {
    let profile = {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        let profile = store
            .profile(&session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        let endpoint = describe_endpoint(&profile);
        store.set_runtime_status(&session_id, SessionStatus::Connecting)?;
        store.record_system_event(
            &session_id,
            format!("PortMate: connecting to {endpoint} ({:?})", profile.kind),
        );
        save_store(&state.store_path, &store)?;
        normalize_session_profile(profile)
    };

    if matches!(
        profile.connection,
        ConnectionConfig::Ssh(_) | ConnectionConfig::Tmux(_)
    ) {
        return match open_ssh_session(&state, profile, password, passphrase).await {
            Ok(summary) => Ok(summary),
            Err(error) => {
                record_connection_failure(&state, &session_id, &error);
                Err(error)
            }
        };
    }

    if matches!(
        profile.connection,
        ConnectionConfig::Tcp(_) | ConnectionConfig::Telnet(_)
    ) {
        return match open_tcp_session(&state, profile).await {
            Ok(summary) => Ok(summary),
            Err(error) => {
                record_connection_failure(&state, &session_id, &error);
                Err(error)
            }
        };
    }

    if matches!(profile.connection, ConnectionConfig::Serial(_)) {
        return match open_serial_session(&state, profile) {
            Ok(summary) => Ok(summary),
            Err(error) => {
                record_connection_failure(&state, &session_id, &error);
                Err(error)
            }
        };
    }

    if matches!(profile.connection, ConnectionConfig::Shell(_)) {
        return match open_shell_session(&state, profile) {
            Ok(summary) => Ok(summary),
            Err(error) => {
                record_connection_failure(&state, &session_id, &error);
                Err(error)
            }
        };
    }

    {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        let summary = store.open_session(&session_id)?;
        save_store(&state.store_path, &store)?;
        Ok(summary)
    }
}

#[tauri::command]
async fn close_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionSummary, String> {
    close_session_inner(state.inner(), session_id).await
}

async fn close_session_inner(
    state: &AppState,
    session_id: String,
) -> Result<SessionSummary, String> {
    let existing = {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing {
        runtime.closed.store(true, Ordering::SeqCst);
        let handle = runtime.handle.lock().await;
        let _ = handle
            .disconnect(Disconnect::ByApplication, "PortMate close_session", "en")
            .await;
        for jump_handle in runtime.jump_handles {
            let handle = jump_handle.lock().await;
            let _ = handle
                .disconnect(
                    Disconnect::ByApplication,
                    "PortMate close jump session",
                    "en",
                )
                .await;
        }
    }
    let existing_shell = {
        let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_shell {
        runtime.closed.store(true, Ordering::SeqCst);
        if let Ok(mut child) = runtime.child.lock() {
            let _ = child.kill();
        }
    }
    let existing_tcp = {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_tcp {
        runtime.closed.store(true, Ordering::SeqCst);
        let mut writer = runtime.writer.lock().await;
        let _ = writer.shutdown().await;
    }
    let existing_serial = {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_serial {
        runtime.closed.store(true, Ordering::SeqCst);
    }
    {
        let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        let ids = tunnels
            .iter()
            .filter_map(|(id, runtime)| (runtime.session_id == session_id).then_some(id.clone()))
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(runtime) = tunnels.remove(&id) {
                runtime.closed.store(true, Ordering::SeqCst);
            }
        }
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let summary = store.close_session(&session_id)?;
    save_store(&state.store_path, &store)?;
    Ok(summary)
}

#[tauri::command]
fn evaluate_host_key(
    state: State<'_, AppState>,
    profile_id: String,
    observation: HostKeyObservation,
) -> Result<HostKeyEvaluation, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    store.evaluate_host_key(&profile_id, &observation)
}

#[tauri::command]
fn apply_host_key_decision(
    state: State<'_, AppState>,
    request: HostKeyDecisionRequest,
) -> Result<Option<portmate_core::TrustedHostKey>, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if request.decision == HostKeyDecision::TrustOnce {
        let trusted =
            temporary_trusted_host_key(&store, &request.profile_id, &request.observation)?;
        drop(store);
        remember_one_time_host_key(state.inner(), &request.profile_id, trusted.clone())?;
        return Ok(Some(trusted));
    }
    let trusted = store.apply_host_key_decision(
        &request.profile_id,
        &request.observation,
        request.decision,
    )?;
    save_store(&state.store_path, &store)?;
    Ok(trusted)
}

#[tauri::command]
async fn scan_ssh_host_key(
    state: State<'_, AppState>,
    profile: SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<HostKeyScanResult, String> {
    scan_ssh_host_key_inner(
        state.inner(),
        normalize_session_profile(profile),
        password.as_deref(),
        passphrase.as_deref(),
    )
    .await
}

#[tauri::command]
fn trust_scanned_host_key(
    state: State<'_, AppState>,
    request: TrustScannedHostKeyRequest,
) -> Result<Option<portmate_core::TrustedHostKey>, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let profile = normalize_session_profile(request.profile);
    let profile_id = profile.id.clone();
    store.upsert_profile(profile);
    if request.decision == HostKeyDecision::TrustOnce {
        let trusted = temporary_trusted_host_key(&store, &profile_id, &request.observation)?;
        drop(store);
        remember_one_time_host_key(state.inner(), &profile_id, trusted.clone())?;
        return Ok(Some(trusted));
    }
    let trusted =
        store.apply_host_key_decision(&profile_id, &request.observation, request.decision)?;
    save_store(&state.store_path, &store)?;
    Ok(trusted)
}

#[tauri::command]
fn import_known_hosts(
    state: State<'_, AppState>,
    request: KnownHostsImportRequest,
) -> Result<HostKeyStore, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if store.profile(&request.profile_id).is_none() {
        return Err(format!("unknown session: {}", request.profile_id));
    }
    store
        .host_keys
        .import_known_hosts(&request.profile_id, &request.contents);
    let host_keys = store.host_keys.clone();
    save_store(&state.store_path, &store)?;
    Ok(host_keys)
}

#[tauri::command]
fn export_known_hosts(state: State<'_, AppState>) -> Result<String, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.host_keys.export_known_hosts())
}

#[tauri::command]
fn delete_host_key(state: State<'_, AppState>, key_id: String) -> Result<HostKeyStore, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let host_keys = delete_host_keys_from_store(&mut store, &[key_id]);
    save_store(&state.store_path, &store)?;
    Ok(host_keys)
}

#[tauri::command]
fn delete_host_keys(
    state: State<'_, AppState>,
    key_ids: Vec<String>,
) -> Result<HostKeyStore, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let host_keys = delete_host_keys_from_store(&mut store, &key_ids);
    save_store(&state.store_path, &store)?;
    Ok(host_keys)
}

fn delete_host_keys_from_store(store: &mut SessionStore, key_ids: &[String]) -> HostKeyStore {
    store
        .host_keys
        .keys
        .retain(|key| !key_ids.contains(&key.id));
    for profile in &mut store.profiles {
        if let ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) = &mut profile.connection {
            ssh.trusted_host_keys
                .retain(|key| !key_ids.contains(&key.id));
        }
    }
    store.host_keys.clone()
}

#[tauri::command]
fn update_host_key(
    state: State<'_, AppState>,
    request: HostKeyUpdateRequest,
) -> Result<HostKeyStore, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let host_keys = update_host_key_in_store(&mut store, request)?;
    save_store(&state.store_path, &store)?;
    Ok(host_keys)
}

fn update_host_key_in_store(
    store: &mut SessionStore,
    request: HostKeyUpdateRequest,
) -> Result<HostKeyStore, String> {
    let alias = request.alias.trim().to_string();
    if alias.is_empty() {
        return Err("host key alias 不能为空".to_string());
    }
    let host = request.host.trim().to_string();
    if host.is_empty() {
        return Err("host key host 不能为空".to_string());
    }
    if request.port == 0 {
        return Err("host key 端口必须在 1-65535 之间".to_string());
    }
    let profile_id = request
        .profile_id
        .as_deref()
        .map(str::trim)
        .filter(|profile_id| !profile_id.is_empty())
        .map(str::to_string);
    if request.scope == HostKeyScope::Profile {
        let Some(profile_id) = profile_id.as_deref() else {
            return Err("Profile scope host key 必须选择 Profile".to_string());
        };
        if store.profile(profile_id).is_none() {
            return Err(format!("unknown session: {profile_id}"));
        }
    } else if let Some(profile_id) = profile_id.as_deref() {
        if store.profile(profile_id).is_none() {
            return Err(format!("unknown session: {profile_id}"));
        }
    }
    let label = request
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string);

    let Some(key) = store
        .host_keys
        .keys
        .iter_mut()
        .find(|key| key.id == request.key_id)
    else {
        return Err(format!("unknown host key: {}", request.key_id));
    };
    key.profile_id = profile_id.clone();
    key.alias = alias.clone();
    key.host = host.clone();
    key.port = request.port;
    key.scope = request.scope;
    key.label = label.clone();

    for profile in &mut store.profiles {
        if let ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) = &mut profile.connection {
            for profile_key in &mut ssh.trusted_host_keys {
                if profile_key.id == request.key_id {
                    profile_key.profile_id = profile_id.clone();
                    profile_key.alias = alias.clone();
                    profile_key.host = host.clone();
                    profile_key.port = request.port;
                    profile_key.scope = request.scope;
                    profile_key.label = label.clone();
                }
            }
        }
    }

    Ok(store.host_keys.clone())
}

#[tauri::command]
fn list_transfers(state: State<'_, AppState>) -> Result<Vec<TransferTask>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.transfers.clone())
}

#[tauri::command]
async fn retry_transfer(
    state: State<'_, AppState>,
    transfer_id: String,
) -> Result<TransferTask, String> {
    retry_transfer_inner(state.inner(), &transfer_id).await
}

async fn retry_transfer_inner(state: &AppState, transfer_id: &str) -> Result<TransferTask, String> {
    let previous = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store
            .transfer_by_id(transfer_id)
            .ok_or_else(|| format!("unknown transfer: {transfer_id}"))?
    };
    start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: previous.session_id,
            protocol: previous.protocol,
            source: previous.source,
            destination: previous.destination,
        },
    )
    .await
}

#[tauri::command]
fn cancel_transfer(
    state: State<'_, AppState>,
    transfer_id: String,
) -> Result<TransferTask, String> {
    cancel_transfer_inner(state.inner(), &transfer_id)
}

#[tauri::command]
fn list_mcp_audit(state: State<'_, AppState>) -> Result<Vec<portmate_core::AuditRecord>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.audit.clone())
}

#[tauri::command]
fn list_mcp_grants(state: State<'_, AppState>) -> Result<Vec<McpGrant>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.grants.clone())
}

#[tauri::command]
fn save_mcp_grant(state: State<'_, AppState>, grant: McpGrant) -> Result<Vec<McpGrant>, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if let Some(existing) = store
        .grants
        .iter_mut()
        .find(|existing| existing.client_id == grant.client_id)
    {
        *existing = grant;
    } else {
        store.grants.push(grant);
    }
    save_store(&state.store_path, &store)?;
    Ok(store.grants.clone())
}

#[tauri::command]
fn revoke_mcp_grant(
    state: State<'_, AppState>,
    client_id: String,
) -> Result<Vec<McpGrant>, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.grants.retain(|grant| grant.client_id != client_id);
    save_store(&state.store_path, &store)?;
    Ok(store.grants.clone())
}

#[tauri::command]
fn mcp_http_config() -> McpHttpConfig {
    build_mcp_http_config(has_secret_ref(MCP_HTTP_TOKEN_REF))
}

#[tauri::command]
fn rotate_mcp_http_token() -> Result<McpHttpTokenResponse, String> {
    let token = Uuid::new_v4().to_string();
    write_secret_to_keyring(MCP_HTTP_TOKEN_REF, &token)?;
    Ok(McpHttpTokenResponse {
        config: build_mcp_http_config(true),
        token,
    })
}

#[tauri::command]
fn list_host_keys(state: State<'_, AppState>) -> Result<HostKeyStore, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.host_keys.clone())
}

#[tauri::command]
async fn list_ssh_agent_identities() -> Result<Vec<IdentityRef>, String> {
    let identities = list_ssh_agent_identities_on_thread().await?;
    Ok(identities
        .into_iter()
        .enumerate()
        .map(|(index, identity)| {
            let public_key = identity.public_key();
            IdentityRef {
                id: format!("agent-{index}"),
                label: if identity.comment().trim().is_empty() {
                    format!("agent key {}", index + 1)
                } else {
                    identity.comment().to_string()
                },
                source: IdentitySource::Agent,
                fingerprint_sha256: compute_ssh_sha256_fingerprint(&public_key.public_key_base64())
                    .ok(),
                path: (!identity.comment().trim().is_empty())
                    .then(|| identity.comment().to_string()),
                secret_ref: None,
            }
        })
        .collect())
}

#[tauri::command]
fn save_secret(request: SecretWriteRequest) -> Result<SecretWriteResponse, String> {
    let secret = request.secret.trim_end_matches(['\r', '\n']).to_string();
    if secret.trim().is_empty() {
        return Err("密钥内容不能为空".to_string());
    }
    let secret_ref = request
        .secret_ref
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("keychain:{}", Uuid::new_v4()));
    write_secret_to_keyring(&secret_ref, &secret)?;
    Ok(SecretWriteResponse { secret_ref })
}

#[tauri::command]
fn delete_secret(secret_ref: String) -> Result<(), String> {
    delete_secret_from_keyring(&secret_ref)
}

#[tauri::command]
fn has_secret(secret_ref: String) -> Result<bool, String> {
    match read_secret_from_keyring(&secret_ref) {
        Ok(_) => Ok(true),
        Err(error) if error.contains("NoEntry") || error.contains("No credential") => Ok(false),
        Err(error) => Err(error),
    }
}

#[tauri::command]
fn list_serial_ports() -> Result<Vec<String>, String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|port| port.port_name).collect())
        .map_err(|error| format!("串口枚举失败: {error}"))
}

#[tauri::command]
async fn list_tmux_state(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<TmuxState, String> {
    list_tmux_state_inner(state.inner(), &session_id).await
}

#[tauri::command]
async fn attach_tmux(
    state: State<'_, AppState>,
    session_id: String,
    target: String,
) -> Result<SessionEvent, String> {
    let command = format!(
        "tmux switch-client -t {} || tmux attach -t {} || tmux new-session -A -s {}\r",
        shell_quote(&target),
        shell_quote(&target),
        shell_quote(&target)
    );
    send_text_inner(state.inner().session_io(), session_id, command).await
}

#[tauri::command]
async fn list_files(
    state: State<'_, AppState>,
    request: ListFilesRequest,
) -> Result<Vec<FileEntry>, String> {
    list_files_inner(state.inner(), request).await
}

#[tauri::command]
async fn file_properties(
    state: State<'_, AppState>,
    request: FilePropertiesRequest,
) -> Result<FileProperties, String> {
    file_properties_inner(state.inner(), request).await
}

#[tauri::command]
async fn create_directory(
    state: State<'_, AppState>,
    request: FileOperationRequest,
) -> Result<(), String> {
    file_operation_inner(state.inner(), request, FileOperation::CreateDirectory).await
}

#[tauri::command]
async fn delete_path(
    state: State<'_, AppState>,
    request: FileOperationRequest,
) -> Result<(), String> {
    file_operation_inner(state.inner(), request, FileOperation::Delete).await
}

#[tauri::command]
async fn rename_path(state: State<'_, AppState>, request: RenamePathRequest) -> Result<(), String> {
    rename_path_inner(state.inner(), request).await
}

#[tauri::command]
async fn chmod_path(state: State<'_, AppState>, request: ChmodPathRequest) -> Result<(), String> {
    chmod_path_inner(state.inner(), request).await
}

#[tauri::command]
fn serial_set_lines(
    state: State<'_, AppState>,
    request: SerialLineRequest,
) -> Result<SessionSummary, String> {
    let writer = {
        let connections = state.serial.lock().map_err(|error| error.to_string())?;
        match connections.get(&request.session_id) {
            Some(runtime) => runtime
                .writer
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| "串口正在重连".to_string()),
            None => Err("串口会话尚未连接".to_string()),
        }
    }?;

    {
        let mut port = writer.lock().map_err(|error| error.to_string())?;
        if let Some(dtr) = request.dtr {
            port.write_data_terminal_ready(dtr)
                .map_err(|error| format!("设置 DTR 失败: {error}"))?;
        }
        if let Some(rts) = request.rts {
            port.write_request_to_send(rts)
                .map_err(|error| format!("设置 RTS 失败: {error}"))?;
        }
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == request.session_id)
        .ok_or_else(|| format!("unknown session: {}", request.session_id))?;
    if let ConnectionConfig::Serial(serial) = &mut profile.connection {
        if let Some(dtr) = request.dtr {
            serial.dtr = dtr;
        }
        if let Some(rts) = request.rts {
            serial.rts = rts;
        }
    }
    store.record_system_event(&request.session_id, "PortMate: serial line state updated");
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == request.session_id)
        .ok_or_else(|| format!("session summary is missing: {}", request.session_id))?;
    save_store(&state.store_path, &store)?;
    Ok(summary)
}

#[tauri::command]
fn serial_send_break(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let writer = {
        let connections = state.serial.lock().map_err(|error| error.to_string())?;
        match connections.get(&session_id) {
            Some(runtime) => runtime
                .writer
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| "串口正在重连".to_string()),
            None => Err("串口会话尚未连接".to_string()),
        }
    }?;

    {
        let port = writer.lock().map_err(|error| error.to_string())?;
        port.set_break()
            .map_err(|error| format!("发送 Break 失败: {error}"))?;
        std::thread::sleep(Duration::from_millis(250));
        port.clear_break()
            .map_err(|error| format!("清除 Break 失败: {error}"))?;
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.record_system_event(&session_id, "PortMate: serial Break sent");
    save_store(&state.store_path, &store)?;
    Ok(())
}

#[tauri::command]
async fn refresh_sysmon(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SysmonSnapshot, String> {
    let profile = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store
            .profile(&session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?
    };

    let snapshot = if matches!(
        profile.connection,
        ConnectionConfig::Ssh(_) | ConnectionConfig::Tmux(_)
    ) {
        let handle = {
            let connections = state.ssh.lock().map_err(|error| error.to_string())?;
            connections
                .get(&session_id)
                .map(|runtime| Arc::clone(&runtime.handle))
        };
        if let Some(handle) = handle {
            collect_remote_sysmon(&session_id, handle).await?
        } else {
            return Err("需要先连接 SSH/Tmux 会话才能读取远端 Sysmon".to_string());
        }
    } else {
        collect_local_sysmon(&session_id)
    };

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.sysmon.push(snapshot.clone());
    store.record_system_event(&session_id, "PortMate: sysmon snapshot refreshed");
    save_store(&state.store_path, &store)?;
    Ok(snapshot)
}

#[tauri::command]
async fn start_transfer(
    state: State<'_, AppState>,
    request: StartTransferRequest,
) -> Result<TransferTask, String> {
    start_transfer_inner(state.inner(), request).await
}

#[tauri::command]
async fn create_tunnel(
    state: State<'_, AppState>,
    request: CreateTunnelRequest,
) -> Result<TunnelSpec, String> {
    create_tunnel_inner(state.inner(), request).await
}

#[tauri::command]
fn list_tunnels(
    state: State<'_, AppState>,
    session_id: Option<String>,
) -> Result<Vec<TunnelStatus>, String> {
    list_tunnels_inner(state.inner(), session_id.as_deref())
}

#[tauri::command]
async fn stop_tunnel(
    state: State<'_, AppState>,
    tunnel_id: String,
) -> Result<TunnelStatus, String> {
    stop_tunnel_inner(state.inner(), &tunnel_id).await
}

#[tauri::command]
fn mcp_manifest() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": "2025-06-18",
        "tools": tool_definitions(),
        "resources": resource_templates(),
        "prompts": prompt_templates(),
    })
}

fn start_ipc_server(state: AppState, endpoint_path: PathBuf, token: String) {
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("PortMate: failed to bind MCP IPC server: {error}");
                return;
            }
        };
        let addr = match listener.local_addr() {
            Ok(addr) => addr.to_string(),
            Err(error) => {
                eprintln!("PortMate: failed to inspect MCP IPC server addr: {error}");
                return;
            }
        };
        if let Some(parent) = endpoint_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                eprintln!("PortMate: failed to create IPC endpoint directory: {error}");
                return;
            }
        }
        let token_ref = format!("keychain:ipc-{}", Uuid::new_v4());
        let token_written = write_secret_to_keyring(&token_ref, &token);
        let (endpoint_token, endpoint_token_ref) = match token_written {
            Ok(()) => (None, Some(token_ref)),
            Err(error) => {
                eprintln!("PortMate: failed to store MCP IPC token in keyring: {error}");
                (Some(token.clone()), None)
            }
        };
        let endpoint = IpcEndpointFile {
            addr: addr.clone(),
            token: endpoint_token,
            token_ref: endpoint_token_ref,
            store_path: state.store_path.display().to_string(),
        };
        match serde_json::to_vec_pretty(&endpoint)
            .map_err(|error| error.to_string())
            .and_then(|bytes| fs::write(&endpoint_path, bytes).map_err(|error| error.to_string()))
        {
            Ok(()) => {}
            Err(error) => {
                eprintln!("PortMate: failed to write MCP IPC endpoint: {error}");
                return;
            }
        }

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let state = state.clone();
                    let token = token.clone();
                    tauri::async_runtime::spawn(async move {
                        handle_ipc_client(state, token, stream).await;
                    });
                }
                Err(error) => {
                    eprintln!("PortMate: MCP IPC accept failed: {error}");
                    break;
                }
            }
        }
    });
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

async fn handle_ipc_client(state: AppState, token: String, mut stream: TcpStream) {
    let mut raw = Vec::new();
    let response = match stream.read_to_end(&mut raw).await {
        Ok(_) => match serde_json::from_slice::<IpcRequest>(&raw) {
            Ok(request) if constant_time_str_eq(&request.token, &token) => {
                match handle_ipc_request(state.clone(), request).await {
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
            error: Some(format!("IPC read failed: {error}")),
        },
    };

    if let Ok(bytes) = serde_json::to_vec(&response) {
        let _ = stream.write_all(&bytes).await;
        let _ = stream.shutdown().await;
    }
}

/// Independently enforces the MCP grant model against the desktop's live store.
/// The empty-store bootstrap is accepted only when the bridge explicitly declares
/// its trusted-write mode, preserving the documented local development flow while
/// preventing ordinary read-only bridge requests from inheriting write access.
fn guard_mcp_scope(
    state: &AppState,
    request: &IpcRequest,
    scope: McpScope,
    session_id: &str,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    if mcp_scope_allowed(
        &store,
        &request.client_id,
        request.trusted_write,
        scope,
        session_id,
    ) {
        Ok(())
    } else {
        Err(format!(
            "MCP grant does not permit {scope:?} for client `{}` on session `{session_id}`",
            request.client_id
        ))
    }
}

fn mcp_scope_allowed(
    store: &SessionStore,
    client_id: &str,
    trusted_write: bool,
    scope: McpScope,
    session_id: &str,
) -> bool {
    !client_id.trim().is_empty()
        && (store.mcp_can(client_id, scope, Some(session_id))
            || (trusted_write && store.grants.is_empty()))
}

async fn handle_ipc_request(
    state: AppState,
    request: IpcRequest,
) -> Result<serde_json::Value, String> {
    match request.command.as_str() {
        "list_sessions" => {
            let store = state.store.lock().map_err(|error| error.to_string())?;
            serde_json::to_value(store.summaries()).map_err(|error| error.to_string())
        }
        "read_screen" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let store = state.store.lock().map_err(|error| error.to_string())?;
            let screen = redact_secrets(&store.screen(&session_id).unwrap_or_default());
            Ok(serde_json::json!(screen))
        }
        "tail_log" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let limit = request
                .args
                .get("limit")
                .and_then(serde_json::Value::as_u64);
            let limit = bounded_log_query_limit(limit);
            let store = state.store.lock().map_err(|error| error.to_string())?;
            serde_json::to_value(redact_mcp_events(store.tail_log(&session_id, limit)))
                .map_err(|error| error.to_string())
        }
        "search_logs" => {
            let query = ipc_string_arg(&request.args, "query")?.to_string();
            let session_id = request
                .args
                .get("sessionId")
                .and_then(serde_json::Value::as_str);
            let limit = request
                .args
                .get("limit")
                .and_then(serde_json::Value::as_u64);
            let limit = bounded_log_query_limit(limit);
            let store = state.store.lock().map_err(|error| error.to_string())?;
            serde_json::to_value(redact_mcp_events(
                store.search_logs(&query, session_id, limit),
            ))
            .map_err(|error| error.to_string())
        }
        "send_text" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let text = ipc_string_arg(&request.args, "text")?.to_string();
            guard_mcp_scope(&state, &request, McpScope::WriteInput, &session_id)?;
            let event = send_text_inner(state.session_io(), session_id, text).await?;
            serde_json::to_value(event).map_err(|error| error.to_string())
        }
        "send_key" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let key = ipc_string_arg(&request.args, "key")?.to_string();
            guard_mcp_scope(&state, &request, McpScope::WriteInput, &session_id)?;
            let text = terminal_key_sequence(&key)?;
            let event = send_text_inner(state.session_io(), session_id, text).await?;
            serde_json::to_value(event).map_err(|error| error.to_string())
        }
        "run_command" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let mut text = ipc_string_arg(&request.args, "command")?.to_string();
            guard_mcp_scope(&state, &request, McpScope::WriteInput, &session_id)?;
            if !text.ends_with('\n') && !text.ends_with('\r') {
                text.push('\n');
            }
            let event = send_text_inner(state.session_io(), session_id, text).await?;
            serde_json::to_value(event).map_err(|error| error.to_string())
        }
        "open_session" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            guard_mcp_scope(&state, &request, McpScope::ManageSessions, &session_id)?;
            let password = request
                .args
                .get("password")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let passphrase = request
                .args
                .get("passphrase")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let summary =
                open_session_inner(state.clone(), session_id, password, passphrase).await?;
            serde_json::to_value(summary).map_err(|error| error.to_string())
        }
        "close_session" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            guard_mcp_scope(&state, &request, McpScope::ManageSessions, &session_id)?;
            let summary = close_session_inner(&state, session_id).await?;
            serde_json::to_value(summary).map_err(|error| error.to_string())
        }
        "start_transfer" => {
            let transfer = serde_json::from_value::<StartTransferRequest>(request.args.clone())
                .map_err(|error| format!("invalid transfer request: {error}"))?;
            guard_mcp_scope(&state, &request, McpScope::Transfer, &transfer.session_id)?;
            let task = start_transfer_inner(&state, transfer).await?;
            serde_json::to_value(task).map_err(|error| error.to_string())
        }
        "create_tunnel" => {
            let tunnel = serde_json::from_value::<CreateTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
            guard_mcp_scope(&state, &request, McpScope::Tunnel, &tunnel.session_id)?;
            let spec = create_tunnel_inner(&state, tunnel).await?;
            serde_json::to_value(spec).map_err(|error| error.to_string())
        }
        "list_files" => {
            let request = serde_json::from_value::<ListFilesRequest>(request.args)
                .map_err(|error| format!("invalid list files request: {error}"))?;
            let entries = list_files_inner(&state, request).await?;
            serde_json::to_value(entries).map_err(|error| error.to_string())
        }
        "list_tmux_state" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let tmux = list_tmux_state_inner(&state, &session_id).await?;
            serde_json::to_value(tmux).map_err(|error| error.to_string())
        }
        "attach_tmux" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let target = ipc_string_arg(&request.args, "target")?.to_string();
            guard_mcp_scope(&state, &request, McpScope::WriteInput, &session_id)?;
            let command = format!(
                "tmux switch-client -t {} || tmux attach -t {} || tmux new-session -A -s {}\r",
                shell_quote(&target),
                shell_quote(&target),
                shell_quote(&target)
            );
            let event = send_text_inner(state.session_io(), session_id, command).await?;
            serde_json::to_value(event).map_err(|error| error.to_string())
        }
        "export_session_bundle" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let store = state.store.lock().map_err(|error| error.to_string())?;
            Ok(store.export_session_bundle_redacted(&session_id))
        }
        other => Err(format!("unsupported IPC command: {other}")),
    }
}

async fn scan_ssh_host_key_inner(
    state: &AppState,
    profile: SessionProfile,
    password: Option<&str>,
    passphrase: Option<&str>,
) -> Result<HostKeyScanResult, String> {
    let ssh = match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.clone(),
        _ => return Err("profile is not SSH-backed".to_string()),
    };
    let host = ssh.endpoint.host.trim().to_string();
    if host.is_empty() {
        return Err("SSH 主机不能为空".to_string());
    }
    if ssh.endpoint.port == 0 {
        return Err("SSH 端口必须在 1-65535 之间".to_string());
    }

    let observed_key = Arc::new(Mutex::new(None));
    let alias = ssh
        .host_key_policy
        .alias
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| Some(profile.id.clone()));
    let handler = HostKeyScanHandler {
        host: host.clone(),
        port: ssh.endpoint.port,
        alias,
        observed_key: Arc::clone(&observed_key),
    };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(10)),
        keepalive_max: 1,
        nodelay: true,
        ..Default::default()
    });

    if !ssh.jumps.is_empty() {
        if let Some(result) = scan_ssh_host_key_via_jump(JumpHostKeyScanRequest {
            state,
            profile: &profile,
            ssh: &ssh,
            config,
            target_handler: handler,
            password,
            passphrase,
        })
        .await?
        {
            return Ok(result);
        }
    } else {
        let session = tokio::time::timeout(
            Duration::from_secs(12),
            client::connect(config, (host.clone(), ssh.endpoint.port), handler),
        )
        .await
        .map_err(|_| format!("SSH host key 扫描超时: {host}:{}", ssh.endpoint.port))?
        .map_err(|error| {
            format!(
                "SSH host key 扫描失败: {host}:{}: {error}",
                ssh.endpoint.port
            )
        })?;
        let _ = session
            .disconnect(Disconnect::ByApplication, "PortMate host key scan", "en")
            .await;
    }

    let observation = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "SSH host key 扫描未收到服务器 host key".to_string())?;
    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());
    host_keys
        .keys
        .extend(one_time_host_keys_snapshot(state, &profile.id)?);
    let evaluation = host_keys
        .evaluate(&profile.id, &ssh.host_key_policy, &observation)
        .map_err(|error| error.to_string())?;
    Ok(HostKeyScanResult {
        label: Some("目标 SSH".to_string()),
        observation,
        evaluation,
    })
}

async fn scan_ssh_host_key_via_jump(
    request: JumpHostKeyScanRequest<'_>,
) -> Result<Option<HostKeyScanResult>, String> {
    let JumpHostKeyScanRequest {
        state,
        profile,
        ssh,
        config,
        target_handler,
        password,
        passphrase,
    } = request;

    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());
    let one_time_host_keys = one_time_host_keys_snapshot(state, &profile.id)?;
    let one_time_host_key_ids = one_time_host_keys
        .iter()
        .map(|key| key.id.clone())
        .collect::<Vec<_>>();
    host_keys.keys.extend(one_time_host_keys);

    let mut jump_sessions: Vec<client::Handle<PortMateSshHandler>> = Vec::new();
    for (index, jump) in ssh.jumps.iter().enumerate() {
        let (jump_host, jump_port, jump_username) = jump_endpoint_details(jump, index)?;
        let jump_policy = jump_host_key_policy(ssh, jump);
        let jump_ssh = jump_ssh_connection(ssh, jump, jump_policy.clone());
        let observed_jump_key = Arc::new(Mutex::new(None));
        let jump_key_error = Arc::new(Mutex::new(None));
        let jump_handler = ssh_handler_for_endpoint(SshHandlerParams {
            profile_id: profile.id.clone(),
            host: jump_host.clone(),
            port: jump_port,
            alias: jump_policy.alias.clone(),
            policy: jump_ssh.host_key_policy.clone(),
            host_keys: host_keys.clone(),
            one_time_host_key_ids: one_time_host_key_ids.clone(),
            observed_key: Arc::clone(&observed_jump_key),
            host_key_error: Arc::clone(&jump_key_error),
            remote_forwards: Arc::new(Mutex::new(HashMap::new())),
        });
        let jump_label = format!("Jump Host 第 {} 跳", index + 1);
        let mut jump_session = if let Some(previous_jump) = jump_sessions.last_mut() {
            let jump_channel = match previous_jump
                .channel_open_direct_tcpip(jump_host.clone(), u32::from(jump_port), "127.0.0.1", 0)
                .await
            {
                Ok(channel) => channel,
                Err(error) => {
                    disconnect_jump_sessions(
                        jump_sessions,
                        "PortMate jump host key scan channel failed",
                    )
                    .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳打开 host key 扫描通道到 {jump_host}:{jump_port} 失败: {error}",
                        index + 1
                    ));
                }
            };
            match tokio::time::timeout(
                Duration::from_secs(12),
                client::connect_stream(config.clone(), jump_channel.into_stream(), jump_handler),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Err(_) => {
                    disconnect_jump_sessions(jump_sessions, "PortMate jump host key scan timeout")
                        .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳 host key 扫描连接超时: {jump_host}:{jump_port}",
                        index + 1
                    ));
                }
                Ok(Err(error)) => {
                    if let Some(result) = host_key_scan_result_for_policy(
                        profile,
                        ssh,
                        &jump_policy,
                        &observed_jump_key,
                        &jump_label,
                        state,
                    )? {
                        disconnect_jump_sessions(
                            jump_sessions,
                            "PortMate jump host key scan needs confirmation",
                        )
                        .await;
                        return Ok(Some(result));
                    }
                    let message = jump_key_error
                        .lock()
                        .ok()
                        .and_then(|reason| reason.clone())
                        .unwrap_or_else(|| {
                            format!(
                                "Jump Host 第 {} 跳 host key 扫描连接失败: {error}",
                                index + 1
                            )
                        });
                    disconnect_jump_sessions(
                        jump_sessions,
                        "PortMate jump host key scan handshake failed",
                    )
                    .await;
                    return Err(message);
                }
            }
        } else {
            match tokio::time::timeout(
                Duration::from_secs(12),
                client::connect(config.clone(), (jump_host.clone(), jump_port), jump_handler),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Err(_) => {
                    return Err(format!(
                        "Jump Host host key 扫描连接超时: {jump_host}:{jump_port}"
                    ));
                }
                Ok(Err(error)) => {
                    if let Some(result) = host_key_scan_result_for_policy(
                        profile,
                        ssh,
                        &jump_policy,
                        &observed_jump_key,
                        &jump_label,
                        state,
                    )? {
                        return Ok(Some(result));
                    }
                    return Err(jump_key_error
                        .lock()
                        .ok()
                        .and_then(|reason| reason.clone())
                        .unwrap_or_else(|| format!("Jump Host host key 扫描连接失败: {error}")));
                }
            }
        };

        if let Err(error) = authenticate_ssh(
            &mut jump_session,
            jump_ssh,
            jump_username,
            jump_runtime_credential(password, jump.password_secret_ref.as_deref()),
            jump_runtime_credential(passphrase, jump.passphrase_secret_ref.as_deref()),
        )
        .await
        {
            disconnect_jump_sessions(jump_sessions, "PortMate jump host key scan auth failed")
                .await;
            let _ = jump_session
                .disconnect(
                    Disconnect::ByApplication,
                    "PortMate jump host key scan auth failed",
                    "en",
                )
                .await;
            return Err(format!(
                "Jump Host 第 {} 跳 host key 扫描认证失败: {error}",
                index + 1
            ));
        }
        jump_sessions.push(jump_session);
    }

    let target_host = ssh.endpoint.host.trim().to_string();
    let jump_channel = match jump_sessions
        .last_mut()
        .expect("non-empty jumps should create jump sessions")
        .channel_open_direct_tcpip(
            target_host.clone(),
            u32::from(ssh.endpoint.port),
            "127.0.0.1",
            0,
        )
        .await
    {
        Ok(channel) => channel,
        Err(error) => {
            disconnect_jump_sessions(
                jump_sessions,
                "PortMate jump host key scan target channel failed",
            )
            .await;
            return Err(format!(
                "Jump Host 打开 host key 扫描通道到 {target_host}:{} 失败: {error}",
                ssh.endpoint.port
            ));
        }
    };
    let target_session = match tokio::time::timeout(
        Duration::from_secs(12),
        client::connect_stream(config, jump_channel.into_stream(), target_handler),
    )
    .await
    {
        Ok(Ok(session)) => session,
        Err(_) => {
            disconnect_jump_sessions(jump_sessions, "PortMate host key scan target timeout").await;
            return Err(format!(
                "SSH 经 Jump Host host key 扫描超时: {target_host}:{}",
                ssh.endpoint.port
            ));
        }
        Ok(Err(error)) => {
            disconnect_jump_sessions(jump_sessions, "PortMate host key scan target failed").await;
            return Err(format!(
                "SSH 经 Jump Host host key 扫描失败: {target_host}:{}: {error}",
                ssh.endpoint.port
            ));
        }
    };
    let _ = target_session
        .disconnect(Disconnect::ByApplication, "PortMate host key scan", "en")
        .await;
    disconnect_jump_sessions(jump_sessions, "PortMate jump host key scan").await;
    Ok(None)
}

fn host_key_scan_result_for_policy(
    profile: &SessionProfile,
    ssh: &SshConnection,
    policy: &portmate_core::HostKeyPolicy,
    observed_key: &Arc<Mutex<Option<HostKeyObservation>>>,
    label: &str,
    state: &AppState,
) -> Result<Option<HostKeyScanResult>, String> {
    let Some(observation) = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
    else {
        return Ok(None);
    };
    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());
    host_keys
        .keys
        .extend(one_time_host_keys_snapshot(state, &profile.id)?);
    let evaluation = host_keys
        .evaluate(&profile.id, policy, &observation)
        .map_err(|error| error.to_string())?;
    Ok(Some(HostKeyScanResult {
        label: Some(label.to_string()),
        observation,
        evaluation,
    }))
}

fn ipc_string_arg<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("missing string argument `{key}`"))
}

fn bounded_log_query_limit(limit: Option<u64>) -> usize {
    limit
        .unwrap_or(DEFAULT_LOG_QUERY_LIMIT)
        .clamp(1, MAX_LOG_QUERY_LIMIT) as usize
}

/// Redacts secrets out of events before they cross the MCP/IPC boundary to an
/// external client. Not applied inside `SessionStore` itself, so the desktop
/// UI keeps showing the human operator raw, real terminal output.
fn redact_mcp_events(events: Vec<SessionEvent>) -> Vec<SessionEvent> {
    events
        .into_iter()
        .map(|mut event| {
            event.text = event.text.map(|text| redact_secrets(&text));
            event
        })
        .collect()
}

async fn start_transfer_inner(
    state: &AppState,
    request: StartTransferRequest,
) -> Result<TransferTask, String> {
    {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if store.profile(&request.session_id).is_none() {
            return Err(format!("unknown session: {}", request.session_id));
        }
    }

    let task = TransferTask {
        id: Uuid::new_v4().to_string(),
        session_id: request.session_id.clone(),
        protocol: request.protocol.clone(),
        source: request.source.clone(),
        destination: request.destination.clone(),
        bytes_total: 0,
        bytes_done: 0,
        status: TransferStatus::Queued,
        message: Some("queued".to_string()),
        started_at: None,
        finished_at: None,
        average_bytes_per_second: None,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut cancellations = state
            .transfer_cancellations
            .lock()
            .map_err(|error| error.to_string())?;
        cancellations.insert(task.id.clone(), Arc::clone(&cancel));
    }

    {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        store.transfers.push(task.clone());
        store.record_system_event(
            &request.session_id,
            format!(
                "PortMate: transfer queued ({:?}) {} -> {}",
                request.protocol, request.source, request.destination
            ),
        );
        save_store(&state.store_path, &store)?;
    }
    emit_transfer_task(state, &task);

    let lane = transfer_lane(state, &request.session_id)?;
    let runner_state = state.clone();
    let task_id = task.id.clone();
    tauri::async_runtime::spawn(async move {
        run_queued_transfer(runner_state, request, task_id, cancel, lane).await;
    });

    Ok(task)
}

fn transfer_lane(
    state: &AppState,
    session_id: &str,
) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let mut lanes = state
        .transfer_lanes
        .lock()
        .map_err(|error| error.to_string())?;
    Ok(Arc::clone(
        lanes
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    ))
}

async fn run_queued_transfer(
    state: AppState,
    request: StartTransferRequest,
    task_id: String,
    cancel: Arc<AtomicBool>,
    lane: Arc<tokio::sync::Mutex<()>>,
) {
    let _lane_guard = lane.lock().await;
    if cancel.load(Ordering::SeqCst) {
        finish_transfer_task(
            &state,
            &task_id,
            &request.session_id,
            TransferStatus::Cancelled,
            "cancelled".to_string(),
            None,
        );
        return;
    }

    let progress = TransferProgressContext {
        state: state.clone(),
        task_id: task_id.clone(),
        cancel: Arc::clone(&cancel),
        last_emit: Arc::new(Mutex::new(Instant::now())),
        started: Instant::now(),
        rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
        rate_limit_bytes_per_second: transfer_rate_limit_bytes_per_second(
            &state,
            &request.session_id,
        ),
    };

    if let Err(error) = mark_transfer_running(&state, &task_id, &request) {
        let status = if error == TRANSFER_CANCELLED_MESSAGE {
            TransferStatus::Cancelled
        } else {
            TransferStatus::Failed
        };
        finish_transfer_task(&state, &task_id, &request.session_id, status, error, None);
        return;
    }

    let result = match request.protocol {
        TransferProtocol::Sftp => transfer_file_via_sftp(&state, &request, &progress).await,
        TransferProtocol::Scp => transfer_file_via_local_or_scp(&state, &request, &progress).await,
        TransferProtocol::Xmodem => transfer_file_via_xmodem(&state, &request, &progress).await,
        TransferProtocol::Ymodem => transfer_file_via_ymodem(&state, &request, &progress).await,
        TransferProtocol::Zmodem => transfer_file_via_zmodem(&state, &request, &progress).await,
    };

    let (status, message, bytes) = match result {
        Ok(bytes) if cancel.load(Ordering::SeqCst) => (
            TransferStatus::Cancelled,
            "cancelled".to_string(),
            Some(bytes),
        ),
        Ok(bytes) => (
            TransferStatus::Completed,
            "completed".to_string(),
            Some(bytes),
        ),
        Err(error) if error == TRANSFER_CANCELLED_MESSAGE => {
            (TransferStatus::Cancelled, "cancelled".to_string(), None)
        }
        Err(error) => (TransferStatus::Failed, error, None),
    };
    finish_transfer_task(
        &state,
        &task_id,
        &request.session_id,
        status,
        message,
        bytes,
    );
}

fn mark_transfer_running(
    state: &AppState,
    task_id: &str,
    request: &StartTransferRequest,
) -> Result<(), String> {
    let task = {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        let task = store
            .transfers
            .iter_mut()
            .find(|task| task.id == task_id)
            .ok_or_else(|| format!("unknown transfer: {task_id}"))?;
        if task.status == TransferStatus::Cancelled {
            return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
        }
        task.status = TransferStatus::Running;
        task.message = Some("running".to_string());
        task.started_at = Some(Utc::now());
        let task = task.clone();
        store.record_system_event(
            &request.session_id,
            format!(
                "PortMate: transfer started ({:?}) {} -> {}",
                request.protocol, request.source, request.destination
            ),
        );
        save_store(&state.store_path, &store)?;
        task
    };
    emit_transfer_task(state, &task);
    Ok(())
}

fn finish_transfer_task(
    state: &AppState,
    task_id: &str,
    session_id: &str,
    status: TransferStatus,
    message: String,
    bytes: Option<u64>,
) {
    let task = {
        let mut store = match state.store.lock() {
            Ok(store) => store,
            Err(error) => {
                eprintln!("PortMate: failed to lock transfer store: {error}");
                return;
            }
        };
        let task = match store.transfers.iter_mut().find(|item| item.id == task_id) {
            Some(task) => task,
            None => return,
        };
        if let Some(bytes) = bytes {
            task.bytes_total = bytes;
            task.bytes_done = bytes;
        }
        task.status = status;
        task.message = Some(message);
        task.finished_at = Some(Utc::now());
        task.average_bytes_per_second = transfer_average_bps(task);
        let task = task.clone();
        {
            let mut cancellations = match state.transfer_cancellations.lock() {
                Ok(cancellations) => cancellations,
                Err(error) => {
                    eprintln!("PortMate: failed to lock transfer cancellations: {error}");
                    return;
                }
            };
            cancellations.remove(&task.id);
        }
        store.record_system_event(
            session_id,
            format!(
                "PortMate: transfer finished ({:?}, {:?})",
                task.protocol, task.status
            ),
        );
        if let Err(error) = save_store(&state.store_path, &store) {
            eprintln!("PortMate: failed to persist transfer finish: {error}");
        }
        task
    };
    emit_transfer_task(state, &task);
}

fn transfer_task_is_active(status: &TransferStatus) -> bool {
    matches!(status, TransferStatus::Queued | TransferStatus::Running)
}

fn transfer_rate_limit_bytes_per_second(state: &AppState, session_id: &str) -> Option<u64> {
    state
        .store
        .lock()
        .ok()
        .and_then(|store| store.profile(session_id))
        .and_then(|profile| profile.transfer.rate_limit_bytes_per_second)
        .filter(|limit| *limit > 0)
}

fn transfer_throttle_delay(
    rate_limit_bytes_per_second: Option<u64>,
    bytes_done: u64,
    elapsed: Duration,
) -> Option<Duration> {
    let limit = rate_limit_bytes_per_second.filter(|limit| *limit > 0)?;
    if bytes_done == 0 {
        return None;
    }
    Duration::from_secs_f64(bytes_done as f64 / limit as f64)
        .checked_sub(elapsed)
        .filter(|delay| !delay.is_zero())
}

fn transfer_average_bps(task: &TransferTask) -> Option<f64> {
    let started = task.started_at?;
    let finished = task.finished_at?;
    let elapsed_ms = (finished - started).num_milliseconds().max(1) as f64;
    if task.bytes_done == 0 {
        return None;
    }
    Some((task.bytes_done as f64) * 1000.0 / elapsed_ms)
}

fn cancel_transfer_inner(state: &AppState, transfer_id: &str) -> Result<TransferTask, String> {
    if let Some(cancel) = state
        .transfer_cancellations
        .lock()
        .map_err(|error| error.to_string())?
        .get(transfer_id)
        .cloned()
    {
        cancel.store(true, Ordering::SeqCst);
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let task = store
        .transfers
        .iter_mut()
        .find(|task| task.id == transfer_id)
        .ok_or_else(|| format!("unknown transfer: {transfer_id}"))?;
    if transfer_task_is_active(&task.status) {
        task.status = TransferStatus::Cancelled;
        task.message = Some("cancelling".to_string());
        task.finished_at = Some(Utc::now());
        task.average_bytes_per_second = transfer_average_bps(task);
    }
    let task = task.clone();
    save_store(&state.store_path, &store)?;
    emit_transfer_task(state, &task);
    Ok(task)
}

fn emit_transfer_task(state: &AppState, task: &TransferTask) {
    if let Some(app_handle) = &state.app_handle {
        let _ = app_handle.emit("portmate-transfer-task", task.clone());
    }
}

impl TransferProgressContext {
    fn check_cancelled(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(TRANSFER_CANCELLED_MESSAGE.to_string())
        } else {
            Ok(())
        }
    }

    fn set_rate_baseline(&self, bytes_done: u64) {
        self.rate_baseline_bytes.store(bytes_done, Ordering::SeqCst);
    }

    fn throttle(&self, bytes_done: u64) -> Result<(), String> {
        let transferred_this_run =
            bytes_done.saturating_sub(self.rate_baseline_bytes.load(Ordering::SeqCst));
        if let Some(delay) = transfer_throttle_delay(
            self.rate_limit_bytes_per_second,
            transferred_this_run,
            self.started.elapsed(),
        ) {
            std::thread::sleep(delay);
            self.check_cancelled()?;
        }
        Ok(())
    }

    fn update(&self, bytes_done: u64, bytes_total: u64) -> Result<(), String> {
        self.check_cancelled()?;
        self.throttle(bytes_done)?;
        let should_emit = {
            let mut last_emit = self.last_emit.lock().map_err(|error| error.to_string())?;
            if last_emit.elapsed() < Duration::from_millis(300) && bytes_done < bytes_total {
                false
            } else {
                *last_emit = Instant::now();
                true
            }
        };
        if !should_emit {
            return Ok(());
        }
        let task = {
            let mut store = self.state.store.lock().map_err(|error| error.to_string())?;
            let task = store
                .transfers
                .iter_mut()
                .find(|task| task.id == self.task_id)
                .ok_or_else(|| format!("unknown transfer: {}", self.task_id))?;
            task.bytes_done = bytes_done;
            if bytes_total > 0 {
                task.bytes_total = bytes_total;
            }
            task.message = Some("running".to_string());
            let task = task.clone();
            save_store(&self.state.store_path, &store)?;
            task
        };
        emit_transfer_task(&self.state, &task);
        Ok(())
    }
}

async fn transfer_file_via_sftp(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);

    match (source_remote, destination_remote) {
        (None, None) => {
            copy_local_file_for_transfer(&request.source, &request.destination, progress)
        }
        (None, Some(remote_destination)) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            let sftp = open_sftp_session(handle).await?;
            let result = sftp_upload(&sftp, &request.source, remote_destination, progress).await;
            let _ = sftp.close().await;
            result
        }
        (Some(remote_source), None) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            let sftp = open_sftp_session(handle).await?;
            let result = sftp_download(&sftp, remote_source, &request.destination, progress).await;
            let _ = sftp.close().await;
            result
        }
        (Some(remote_source), Some(remote_destination)) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            let sftp = open_sftp_session(handle).await?;
            let result = sftp_remote_copy(&sftp, remote_source, remote_destination, progress).await;
            let _ = sftp.close().await;
            result
        }
    }
}

async fn transfer_file_via_local_or_scp(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);

    match (source_remote, destination_remote) {
        (None, None) => {
            copy_local_file_for_transfer(&request.source, &request.destination, progress)
        }
        (None, Some(remote_destination)) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            scp_upload(handle, &request.source, remote_destination, progress).await
        }
        (Some(remote_source), None) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            scp_download(handle, remote_source, &request.destination, progress).await
        }
        (Some(remote_source), Some(remote_destination)) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            remote_copy(handle, remote_source, remote_destination, progress).await
        }
    }
}

async fn transfer_file_via_xmodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let source_size = fs::metadata(&local_source)
                .map_err(|error| format!("读取 XModem 本地文件元数据失败: {error}"))?
                .len();
            let completion_token = Uuid::new_v4().simple().to_string();
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Xmodem,
                true,
                &remote_destination,
            )
            .await?;
            let remote_started = remote_start.is_some();
            let reader = modem_reader_after_start(receiver, remote_start.as_ref()).await?;
            let bytes = xmodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                remote_started,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(&mut completion_receiver, remote_start).await?;
                let command = xmodem_remote_finalize_command(
                    &remote_destination,
                    source_size,
                    &completion_token,
                );
                let _ = send_text_inner(state.session_io(), request.session_id.clone(), command)
                    .await?;
                wait_for_xmodem_remote_completion(&mut completion_receiver, &completion_token)
                    .await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Xmodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(receiver, remote_start.as_ref()).await?;
            let bytes = xmodem_receive_file(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(&mut completion_receiver, remote_start).await?;
            }
            Ok(bytes)
        }
    }
}

async fn transfer_file_via_ymodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Ymodem,
                true,
                &remote_destination,
            )
            .await?;
            let reader = modem_reader_after_start(receiver, remote_start.as_ref()).await?;
            let bytes = ymodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                Some(&remote_destination),
                remote_start.is_some(),
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(&mut completion_receiver, remote_start).await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Ymodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(receiver, remote_start.as_ref()).await?;
            let bytes = ymodem_receive_file(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(&mut completion_receiver, remote_start).await?;
            }
            Ok(bytes)
        }
    }
}

async fn transfer_file_via_zmodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Zmodem,
                true,
                &remote_destination,
            )
            .await?;
            let reader = modem_reader_after_start(receiver, remote_start.as_ref()).await?;
            let bytes = zmodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                Some(&remote_destination),
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(&mut completion_receiver, remote_start).await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Zmodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(receiver, remote_start.as_ref()).await?;
            let bytes = zmodem_receive_files(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(&mut completion_receiver, remote_start).await?;
            }
            Ok(bytes)
        }
    }
}

enum ModemDirection {
    Upload {
        local_source: String,
        remote_destination: String,
    },
    Download {
        remote_source: String,
        local_destination: String,
    },
}

fn modem_direction(request: &StartTransferRequest) -> Result<ModemDirection, String> {
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);

    match (source_remote, destination_remote) {
        (None, Some(remote_destination)) => Ok(ModemDirection::Upload {
            local_source: request.source.clone(),
            remote_destination: remote_destination.to_string(),
        }),
        (Some(remote_source), None) => Ok(ModemDirection::Download {
            remote_source: remote_source.to_string(),
            local_destination: request.destination.clone(),
        }),
        (None, None) if Path::new(&request.source).is_file() => Ok(ModemDirection::Upload {
            local_source: request.source.clone(),
            remote_destination: request.destination.clone(),
        }),
        _ => Err(
            "Modem transfer expects local -> remote:path upload or remote:path -> local download"
                .to_string(),
        ),
    }
}

struct RemoteModemStart {
    token: String,
    ready_marker: String,
}

impl RemoteModemStart {
    fn success_marker(&self) -> String {
        format!("__PORTMATE_MODEM_{}_DONE__", self.token)
    }

    fn failure_marker(&self) -> String {
        format!("__PORTMATE_MODEM_{}_FAIL__", self.token)
    }
}

async fn maybe_start_remote_modem(
    state: &AppState,
    session_id: &str,
    protocol: TransferProtocol,
    upload: bool,
    remote_path: &str,
) -> Result<Option<RemoteModemStart>, String> {
    if !remote_modem_auto_start_enabled(state, session_id)? {
        return Ok(None);
    }

    let readiness_token = Uuid::new_v4().simple().to_string();
    let command = modem_remote_command(protocol, upload, remote_path, &readiness_token);
    let _ = send_text_inner(state.session_io(), session_id.to_string(), command).await?;
    Ok(Some(RemoteModemStart {
        ready_marker: format!("__PORTMATE_MODEM_{readiness_token}_READY__"),
        token: readiness_token,
    }))
}

fn remote_modem_auto_start_enabled(state: &AppState, session_id: &str) -> Result<bool, String> {
    let profile = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.profile(session_id)
    }
    .ok_or_else(|| format!("unknown session: {session_id}"))?;
    Ok(matches!(
        profile.kind,
        SessionKind::Ssh | SessionKind::Tmux | SessionKind::Shell | SessionKind::Telnet
    ))
}

fn xmodem_remote_finalize_command(
    remote_path: &str,
    source_size: u64,
    completion_token: &str,
) -> String {
    format!(
        concat!(
            "target={}; portmate_status=0; ",
            "if command -v truncate >/dev/null 2>&1; then ",
            "truncate -s {} -- \"$target\"; portmate_status=$?; ",
            "else part=\"$target.portmate-trim\"; ",
            "dd if=\"$target\" of=\"$part\" bs=1 count={} 2>/dev/null ",
            "&& mv -f -- \"$part\" \"$target\"; portmate_status=$?; ",
            "if [ \"$portmate_status\" -ne 0 ]; then rm -f -- \"$part\"; fi; fi; ",
            "if [ \"$portmate_status\" -eq 0 ]; then ",
            "printf '\\n__PORTMATE_XMODEM_%s_DONE__\\n' {}; ",
            "else printf '\\n__PORTMATE_XMODEM_%s_FAIL__%s\\n' {} \"$portmate_status\"; fi\r"
        ),
        shell_quote(remote_path),
        source_size,
        source_size,
        shell_quote(completion_token),
        shell_quote(completion_token),
    )
}

async fn wait_for_xmodem_remote_completion(
    receiver: &mut broadcast::Receiver<Vec<u8>>,
    completion_token: &str,
) -> Result<(), String> {
    let success = format!("__PORTMATE_XMODEM_{completion_token}_DONE__");
    let failure = format!("__PORTMATE_XMODEM_{completion_token}_FAIL__");
    let started = Instant::now();
    let mut output = Vec::new();
    loop {
        let remaining = Duration::from_secs(15).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("XModem remote finalize timed out".to_string());
        }
        match tokio::time::timeout(remaining.min(Duration::from_secs(2)), receiver.recv()).await {
            Ok(Ok(bytes)) => {
                output.extend_from_slice(&bytes);
                if output
                    .windows(success.len())
                    .any(|window| window == success.as_bytes())
                {
                    return Ok(());
                }
                if output
                    .windows(failure.len())
                    .any(|window| window == failure.as_bytes())
                {
                    return Err(format!(
                        "XModem remote finalize failed: {}",
                        String::from_utf8_lossy(&output)
                    ));
                }
                if output.len() > 64 * 1024 {
                    let keep = success.len().max(failure.len()).saturating_sub(1);
                    output.drain(..output.len().saturating_sub(keep));
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err("XModem remote finalize stream closed".to_string())
            }
            Err(_) => {}
        }
    }
}

async fn wait_for_remote_modem_completion(
    receiver: &mut broadcast::Receiver<Vec<u8>>,
    remote_start: &RemoteModemStart,
) -> Result<(), String> {
    let success = remote_start.success_marker();
    let failure = remote_start.failure_marker();
    let started = Instant::now();
    let mut output = Vec::new();
    loop {
        let remaining = Duration::from_secs(15).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("remote modem command completion timed out".to_string());
        }
        match tokio::time::timeout(remaining.min(Duration::from_secs(2)), receiver.recv()).await {
            Ok(Ok(bytes)) => {
                output.extend_from_slice(&bytes);
                if output
                    .windows(success.len())
                    .any(|window| window == success.as_bytes())
                {
                    return Ok(());
                }
                if output
                    .windows(failure.len())
                    .any(|window| window == failure.as_bytes())
                {
                    return Err(format!(
                        "remote modem command failed: {}",
                        String::from_utf8_lossy(&output)
                    ));
                }
                if output.len() > 64 * 1024 {
                    let keep = success.len().max(failure.len()).saturating_sub(1);
                    output.drain(..output.len().saturating_sub(keep));
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err("remote modem command completion stream closed".to_string())
            }
            Err(_) => {}
        }
    }
}

fn modem_remote_command(
    protocol: TransferProtocol,
    upload: bool,
    remote_path: &str,
    readiness_token: &str,
) -> String {
    match (protocol, upload) {
        (TransferProtocol::Xmodem, true) => format!(
            "{}\r",
            modem_raw_tty_shell_command(
                &format!("rx {}", shell_quote(remote_path)),
                readiness_token,
            )
        ),
        (TransferProtocol::Xmodem, false) => format!(
            "{}\r",
            modem_raw_tty_shell_command(
                &format!("sx {}", shell_quote(remote_path)),
                readiness_token,
            )
        ),
        (TransferProtocol::Ymodem, true) => {
            let (parent, _) = remote_parent_and_file_name(remote_path);
            if parent.is_empty() {
                format!(
                    "{}\r",
                    modem_raw_tty_shell_command("rb -y", readiness_token)
                )
            } else {
                format!(
                    "mkdir -p {} && cd {} && {}\r",
                    shell_quote(&parent),
                    shell_quote(&parent),
                    modem_raw_tty_shell_command("rb -y", readiness_token)
                )
            }
        }
        (TransferProtocol::Ymodem, false) => format!(
            "{}\r",
            modem_raw_tty_shell_command(
                &format!("sb {}", shell_quote(remote_path)),
                readiness_token,
            )
        ),
        (TransferProtocol::Zmodem, true) => {
            let (parent, _) = remote_parent_and_file_name(remote_path);
            if parent.is_empty() {
                format!(
                    "{}\r",
                    modem_raw_tty_shell_command("rz -y", readiness_token)
                )
            } else {
                format!(
                    "mkdir -p {} && cd {} && {}\r",
                    shell_quote(&parent),
                    shell_quote(&parent),
                    modem_raw_tty_shell_command("rz -y", readiness_token)
                )
            }
        }
        (TransferProtocol::Zmodem, false) => format!(
            "{}\r",
            modem_raw_tty_shell_command(
                &format!("sz {}", shell_quote(remote_path)),
                readiness_token,
            )
        ),
        _ => String::new(),
    }
}

fn modem_raw_tty_shell_command(command: &str, readiness_token: &str) -> String {
    format!(
        concat!(
            "{{ portmate_stty=0; ",
            "if command -v stty >/dev/null 2>&1; then ",
            "stty raw -echo; portmate_stty=1; fi; ",
            "printf '__PORTMATE_MODEM_%s_READY__' {}; ",
            "{}; portmate_modem_status=$?; ",
            "if [ \"$portmate_stty\" -eq 1 ]; then stty sane; fi; ",
            "if [ \"$portmate_modem_status\" -eq 0 ]; then ",
            "printf '\\n__PORTMATE_MODEM_%s_DONE__\\n' {}; ",
            "else printf '\\n__PORTMATE_MODEM_%s_FAIL__%s\\n' {} ",
            "\"$portmate_modem_status\"; fi; ",
            "(exit \"$portmate_modem_status\"); }}"
        ),
        shell_quote(readiness_token),
        command,
        shell_quote(readiness_token),
        shell_quote(readiness_token),
    )
}

fn runtime_tap_receiver(
    state: &AppState,
    session_id: &str,
) -> Result<broadcast::Receiver<Vec<u8>>, String> {
    if let Some(tap) = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    Err("需要先连接会话才能执行 X/Y/ZModem 传输".to_string())
}

async fn write_runtime_bytes(
    state: &AppState,
    session_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let ssh_writer = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = ssh_writer {
        let writer = writer.lock().await;
        writer
            .data(bytes)
            .await
            .map_err(|error| format!("SSH modem 写入失败: {error}"))?;
        return Ok(());
    }

    let shell_writer = {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = shell_writer {
        let mut writer = writer.lock().map_err(|error| error.to_string())?;
        writer
            .write_all(bytes)
            .map_err(|error| format!("Shell modem 写入失败: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("Shell modem 刷新失败: {error}"))?;
        return Ok(());
    }

    let tcp_writer = {
        let connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = tcp_writer {
        let mut writer = writer.lock().await;
        writer
            .write_all(bytes)
            .await
            .map_err(|error| format!("TCP/Telnet modem 写入失败: {error}"))?;
        return Ok(());
    }

    let serial_writer = {
        let connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.writer.as_ref().map(Arc::clone))
    };
    match serial_writer {
        Some(Some(writer)) => {
            let mut writer = writer.lock().map_err(|error| error.to_string())?;
            writer
                .write_all(bytes)
                .map_err(|error| format!("串口 modem 写入失败: {error}"))?;
            writer
                .flush()
                .map_err(|error| format!("串口 modem 刷新失败: {error}"))?;
            return Ok(());
        }
        Some(None) => return Err("串口正在重连，无法执行 modem 写入".to_string()),
        None => {}
    }

    Err("会话尚未连接，无法执行 modem 写入".to_string())
}

async fn check_modem_cancelled(
    state: &AppState,
    session_id: &str,
    progress: &TransferProgressContext,
) -> Result<(), String> {
    if progress.cancel.load(Ordering::SeqCst) {
        let _ = write_runtime_bytes(state, session_id, &[MODEM_CAN, MODEM_CAN, MODEM_CAN]).await;
        Err(TRANSFER_CANCELLED_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

struct ModemByteReader {
    receiver: broadcast::Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
}

impl ModemByteReader {
    fn new(receiver: broadcast::Receiver<Vec<u8>>) -> Self {
        Self {
            receiver,
            pending: VecDeque::new(),
        }
    }

    async fn after_marker(
        mut receiver: broadcast::Receiver<Vec<u8>>,
        marker: &str,
    ) -> Result<Self, String> {
        let started = Instant::now();
        let marker = marker.as_bytes();
        let mut buffered = Vec::new();
        loop {
            let remaining = Duration::from_secs(15).saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("remote modem readiness marker timed out".to_string());
            }
            match tokio::time::timeout(remaining.min(Duration::from_secs(2)), receiver.recv()).await
            {
                Ok(Ok(bytes)) => {
                    buffered.extend_from_slice(&bytes);
                    if let Some(offset) = buffered
                        .windows(marker.len())
                        .position(|window| window == marker)
                    {
                        return Ok(Self {
                            receiver,
                            pending: buffered[offset + marker.len()..].iter().copied().collect(),
                        });
                    }
                    if buffered.len() > 64 * 1024 {
                        let keep = marker.len().saturating_sub(1);
                        buffered.drain(..buffered.len().saturating_sub(keep));
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("remote modem readiness stream closed".to_string())
                }
                Err(_) => {}
            }
        }
    }

    async fn next_byte(&mut self, timeout: Duration) -> Result<u8, String> {
        loop {
            if let Some(byte) = self.pending.pop_front() {
                return Ok(byte);
            }
            match tokio::time::timeout(timeout, self.receiver.recv()).await {
                Ok(Ok(bytes)) => self.pending.extend(bytes),
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("modem byte stream closed".to_string())
                }
                Err(_) => return Err("modem byte timeout".to_string()),
            }
        }
    }

    async fn read_exact(&mut self, len: usize, timeout: Duration) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(len);
        while bytes.len() < len {
            bytes.push(self.next_byte(timeout).await?);
        }
        Ok(bytes)
    }

    async fn next_chunk(&mut self, timeout: Duration, max_len: usize) -> Result<Vec<u8>, String> {
        if !self.pending.is_empty() {
            let take = self.pending.len().min(max_len);
            return Ok(self.pending.drain(..take).collect());
        }

        loop {
            match tokio::time::timeout(timeout, self.receiver.recv()).await {
                Ok(Ok(bytes)) => {
                    if bytes.len() <= max_len {
                        return Ok(bytes);
                    }
                    self.pending.extend(bytes[max_len..].iter().copied());
                    return Ok(bytes[..max_len].to_vec());
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("modem byte stream closed".to_string())
                }
                Err(_) => return Err("modem byte timeout".to_string()),
            }
        }
    }
}

async fn modem_reader_after_start(
    receiver: broadcast::Receiver<Vec<u8>>,
    remote_start: Option<&RemoteModemStart>,
) -> Result<ModemByteReader, String> {
    match remote_start {
        Some(start) => ModemByteReader::after_marker(receiver, &start.ready_marker).await,
        None => Ok(ModemByteReader::new(receiver)),
    }
}

async fn zmodem_send_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    remote_destination: Option<&str>,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let metadata = fs::metadata(local_source)
        .map_err(|error| format!("读取 ZModem 本地文件元数据失败: {error}"))?;
    if !metadata.is_file() {
        return Err("ZModem upload only supports regular local files".to_string());
    }
    let size = u32::try_from(metadata.len())
        .map_err(|_| "ZModem 当前状态机只支持 4 GiB 以内的单文件".to_string())?;
    let mut file = fs::File::open(local_source)
        .map_err(|error| format!("打开 ZModem 本地文件失败: {error}"))?;
    let (_, remote_name) = remote_destination
        .map(remote_parent_and_file_name)
        .unwrap_or_else(|| ("".to_string(), local_file_name(local_source)));
    let file_name = if remote_name.is_empty() {
        local_file_name(local_source)
    } else {
        remote_name
    };

    let mut sender =
        zmodem2::Sender::new().map_err(|error| format!("ZModem sender 初始化失败: {error}"))?;
    sender
        .start_file(file_name.as_bytes(), size)
        .map_err(|error| format!("ZModem 文件发送启动失败: {error}"))?;

    let mut input_buf = Vec::<u8>::new();
    let mut input_offset = 0_usize;
    let mut file_buf = vec![0_u8; 1024];
    let mut session_done = false;
    let mut last_progress = Instant::now();
    let mut bytes_done = 0_u64;

    while !session_done || !sender.drain_outgoing().is_empty() {
        check_modem_cancelled(state, session_id, progress).await?;
        let mut progressed = false;

        let outgoing = sender.drain_outgoing().to_vec();
        if !outgoing.is_empty() {
            write_runtime_bytes(state, session_id, &outgoing).await?;
            sender.advance_outgoing(outgoing.len());
            progressed = true;
        }

        if let Some(request) = sender.poll_file() {
            file.seek(std::io::SeekFrom::Start(u64::from(request.offset)))
                .map_err(|error| format!("ZModem 本地文件 seek 失败: {error}"))?;
            let read_len = request.len.min(file_buf.len());
            let read = file
                .read(&mut file_buf[..read_len])
                .map_err(|error| format!("ZModem 读取本地文件失败: {error}"))?;
            if read == 0 && request.len > 0 {
                return Err("ZModem 本地文件提前结束".to_string());
            }
            sender
                .feed_file(&file_buf[..read])
                .map_err(|error| format!("ZModem 发送文件块失败: {error}"))?;
            bytes_done = bytes_done.max(u64::from(request.offset) + read as u64);
            progress.update(bytes_done.min(u64::from(size)), u64::from(size))?;
            progressed = true;
        }

        match reader.next_chunk(Duration::from_millis(30), 4096).await {
            Ok(bytes) if !bytes.is_empty() => {
                input_buf.extend_from_slice(&bytes);
                progressed = true;
            }
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }

        if sender.drain_outgoing().is_empty() && input_offset < input_buf.len() {
            let consumed = sender
                .feed_incoming(&input_buf[input_offset..])
                .map_err(|error| format!("ZModem 接收远端响应失败: {error}"))?;
            if consumed > 0 {
                input_offset += consumed;
                progressed = true;
                if input_offset == input_buf.len() {
                    input_buf.clear();
                    input_offset = 0;
                } else if input_offset > 4096 {
                    input_buf.drain(..input_offset);
                    input_offset = 0;
                }
            }
        }

        if let Some(event) = sender.poll_event() {
            match event {
                zmodem2::SenderEvent::FileComplete => {
                    sender
                        .finish_session()
                        .map_err(|error| format!("ZModem 结束会话失败: {error}"))?;
                }
                zmodem2::SenderEvent::SessionComplete => {
                    session_done = true;
                }
            }
            progressed = true;
        }

        if progressed {
            last_progress = Instant::now();
        } else if last_progress.elapsed() > Duration::from_secs(90) {
            return Err("ZModem upload idle timeout".to_string());
        } else {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(u64::from(size))
}

async fn zmodem_receive_files(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut modem_receiver =
        zmodem2::Receiver::new().map_err(|error| format!("ZModem receiver 初始化失败: {error}"))?;
    let mut input_buf = Vec::<u8>::new();
    let mut input_offset = 0_usize;
    let mut current_file: Option<(fs::File, PathBuf)> = None;
    let mut received_files = 0_usize;
    let mut bytes_done = 0_u64;
    let mut session_done = false;
    let mut last_progress = Instant::now();

    while !session_done || !modem_receiver.drain_outgoing().is_empty() {
        check_modem_cancelled(state, session_id, progress).await?;
        let mut progressed = false;

        let outgoing = modem_receiver.drain_outgoing().to_vec();
        if !outgoing.is_empty() {
            write_runtime_bytes(state, session_id, &outgoing).await?;
            modem_receiver.advance_outgoing(outgoing.len());
            progressed = true;
        }

        while let Some(event) = modem_receiver.poll_event() {
            match event {
                zmodem2::ReceiverEvent::FileStart => {
                    let incoming = String::from_utf8_lossy(modem_receiver.file_name()).to_string();
                    let target =
                        zmodem_local_target_path(local_destination, &incoming, received_files)?;
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(|error| {
                            format!("创建 ZModem 本地目录失败 {}: {error}", parent.display())
                        })?;
                    }
                    let file = fs::File::create(&target).map_err(|error| {
                        format!("创建 ZModem 本地文件失败 {}: {error}", target.display())
                    })?;
                    current_file = Some((file, target));
                }
                zmodem2::ReceiverEvent::FileComplete => {
                    if let Some((mut file, _)) = current_file.take() {
                        file.flush()
                            .map_err(|error| format!("刷新 ZModem 本地文件失败: {error}"))?;
                    }
                    received_files += 1;
                }
                zmodem2::ReceiverEvent::SessionComplete => {
                    session_done = true;
                }
            }
            progressed = true;
        }

        let file_data = modem_receiver.drain_file().to_vec();
        if !file_data.is_empty() {
            let Some((file, path)) = current_file.as_mut() else {
                return Err("ZModem 收到文件数据但还没有文件头".to_string());
            };
            file.write_all(&file_data)
                .map_err(|error| format!("写入 ZModem 本地文件失败 {}: {error}", path.display()))?;
            modem_receiver
                .advance_file(file_data.len())
                .map_err(|error| format!("ZModem 文件写入确认失败: {error}"))?;
            bytes_done += file_data.len() as u64;
            progress.update(bytes_done, 0)?;
            progressed = true;
        }

        match reader.next_chunk(Duration::from_millis(30), 4096).await {
            Ok(bytes) if !bytes.is_empty() => {
                input_buf.extend_from_slice(&bytes);
                progressed = true;
            }
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }

        if modem_receiver.drain_outgoing().is_empty()
            && modem_receiver.drain_file().is_empty()
            && input_offset < input_buf.len()
        {
            let consumed = modem_receiver
                .feed_incoming(&input_buf[input_offset..])
                .map_err(|error| format!("ZModem 接收远端数据失败: {error}"))?;
            if consumed > 0 {
                input_offset += consumed;
                progressed = true;
                if input_offset == input_buf.len() {
                    input_buf.clear();
                    input_offset = 0;
                } else if input_offset > 4096 {
                    input_buf.drain(..input_offset);
                    input_offset = 0;
                }
            }
        }

        if progressed {
            last_progress = Instant::now();
        } else if last_progress.elapsed() > Duration::from_secs(90) {
            return Err("ZModem download idle timeout".to_string());
        } else {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    if let Some((mut file, _)) = current_file.take() {
        file.flush()
            .map_err(|error| format!("刷新 ZModem 本地文件失败: {error}"))?;
    }

    Ok(bytes_done)
}

fn zmodem_local_target_path(
    local_destination: &str,
    incoming_name: &str,
    received_files: usize,
) -> Result<PathBuf, String> {
    let destination = local_destination.trim();
    if destination.is_empty() {
        return Err("ZModem 本地目标路径不能为空".to_string());
    }
    let incoming = Path::new(incoming_name)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("zmodem-file.bin");
    let base = expand_identity_path(destination);
    let ends_with_separator = destination.ends_with('/') || destination.ends_with('\\');

    if base.is_dir() || ends_with_separator {
        return Ok(base.join(incoming));
    }
    if received_files == 0 {
        return Ok(base);
    }
    Ok(base
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(incoming))
}

async fn xmodem_send_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    auto_remote_receiver: bool,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let data =
        fs::read(local_source).map_err(|error| format!("读取 XModem 本地文件失败: {error}"))?;
    let crc = modem_wait_for_receiver(&mut reader).await?;
    let mut block_no = 1_u8;
    let total = data.len() as u64;
    let mut bytes_done = 0_u64;

    for chunk in data.chunks(XMODEM_BLOCK_SIZE) {
        check_modem_cancelled(state, session_id, progress).await?;
        modem_send_packet_with_retries(
            state,
            session_id,
            &mut reader,
            MODEM_SOH,
            block_no,
            chunk,
            crc,
        )
        .await
        .map_err(|error| format!("XModem data block {block_no} failed: {error}"))?;
        bytes_done += chunk.len() as u64;
        progress.update(bytes_done, total)?;
        block_no = block_no.wrapping_add(1);
    }
    if auto_remote_receiver {
        modem_finish_auto_remote_xmodem(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("XModem EOT handshake failed: {error}"))?;
    } else {
        modem_finish_eot(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("XModem EOT handshake failed: {error}"))?;
    }
    Ok(data.len() as u64)
}

async fn modem_finish_auto_remote_xmodem(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    for _ in 0..3 {
        write_runtime_bytes(state, session_id, &[MODEM_EOT]).await?;
        match modem_wait_for_ack(reader, Duration::from_secs(2)).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Err("remote did not ACK modem EOT".to_string())
}

async fn xmodem_receive_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut expected = 1_u8;
    let mut output = Vec::new();
    let mut first_packet = true;

    loop {
        check_modem_cancelled(state, session_id, progress).await?;
        let marker = if first_packet {
            first_packet = false;
            modem_wait_for_packet_marker(state, session_id, &mut reader).await?
        } else {
            modem_wait_for_next_marker(&mut reader, Duration::from_secs(15)).await?
        };
        if marker == MODEM_EOT {
            write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
            break;
        }
        let packet = match modem_read_packet(&mut reader, marker).await {
            Ok(packet) => packet,
            Err(error) => {
                write_runtime_bytes(state, session_id, &[MODEM_NAK]).await?;
                return Err(error);
            }
        };
        if packet.block_no == expected {
            output.extend_from_slice(&packet.data);
            progress.update(output.len() as u64, 0)?;
            expected = expected.wrapping_add(1);
        }
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
    }

    while output.last().copied() == Some(MODEM_EOF) {
        output.pop();
    }
    write_local_transfer_file(local_destination, &output)?;
    Ok(output.len() as u64)
}

async fn ymodem_send_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    remote_destination: Option<&str>,
    auto_remote_receiver: bool,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let data =
        fs::read(local_source).map_err(|error| format!("读取 YModem 本地文件失败: {error}"))?;
    if !modem_wait_for_receiver(&mut reader).await? {
        return Err("YModem receiver did not request CRC mode".to_string());
    }

    let (_, remote_name) = remote_destination
        .map(remote_parent_and_file_name)
        .unwrap_or_else(|| ("".to_string(), local_file_name(local_source)));
    let name = if remote_name.is_empty() {
        local_file_name(local_source)
    } else {
        remote_name
    };
    let mut metadata = vec![0_u8; XMODEM_BLOCK_SIZE];
    let metadata_text = format!("{}\0{} ", name, data.len());
    let metadata_bytes = metadata_text.as_bytes();
    let metadata_len = metadata_bytes.len().min(metadata.len());
    metadata[..metadata_len].copy_from_slice(&metadata_bytes[..metadata_len]);
    modem_send_packet_with_retries(
        state,
        session_id,
        &mut reader,
        MODEM_SOH,
        0,
        &metadata,
        true,
    )
    .await
    .map_err(|error| format!("YModem metadata block failed: {error}"))?;
    let _ = modem_wait_for_crc_request(&mut reader, Duration::from_secs(10)).await;

    let mut block_no = 1_u8;
    let total = data.len() as u64;
    let mut bytes_done = 0_u64;
    for chunk in data.chunks(YMODEM_BLOCK_SIZE) {
        check_modem_cancelled(state, session_id, progress).await?;
        modem_send_packet_with_retries(
            state,
            session_id,
            &mut reader,
            MODEM_STX,
            block_no,
            chunk,
            true,
        )
        .await
        .map_err(|error| format!("YModem data block {block_no} failed: {error}"))?;
        bytes_done += chunk.len() as u64;
        progress.update(bytes_done, total)?;
        block_no = block_no.wrapping_add(1);
    }
    modem_finish_eot(state, session_id, &mut reader)
        .await
        .map_err(|error| format!("YModem EOT handshake failed: {error}"))?;
    let _ = modem_wait_for_crc_request(&mut reader, Duration::from_secs(10)).await;
    if auto_remote_receiver {
        modem_finish_auto_remote_ymodem_batch(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("YModem final empty block failed: {error}"))?;
    } else {
        let empty = vec![0_u8; XMODEM_BLOCK_SIZE];
        modem_send_packet_with_retries(state, session_id, &mut reader, MODEM_SOH, 0, &empty, true)
            .await
            .map_err(|error| format!("YModem final empty block failed: {error}"))?;
    }
    Ok(data.len() as u64)
}

async fn modem_finish_auto_remote_ymodem_batch(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    let empty = vec![0_u8; XMODEM_BLOCK_SIZE];
    let packet = modem_packet_bytes(MODEM_SOH, 0, &empty, XMODEM_BLOCK_SIZE, true);
    for _ in 0..3 {
        write_runtime_bytes(state, session_id, &packet).await?;
        match modem_wait_for_ack(reader, Duration::from_secs(2)).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Err("remote rejected final YModem empty block".to_string())
}

async fn ymodem_receive_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let marker = modem_wait_for_packet_marker(state, session_id, &mut reader).await?;
    if marker == MODEM_EOT {
        return Err("YModem sender ended before metadata block".to_string());
    }
    let metadata = modem_read_packet(&mut reader, marker).await?;
    if metadata.block_no != 0 {
        return Err("YModem metadata block missing".to_string());
    }
    let (name, expected_size) = parse_ymodem_metadata(&metadata.data);
    if name.is_empty() {
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
        return Err("YModem sender sent empty batch".to_string());
    }
    write_runtime_bytes(state, session_id, &[MODEM_ACK, MODEM_CRC_REQUEST]).await?;

    let mut expected = 1_u8;
    let mut output = Vec::new();
    let total = expected_size.unwrap_or(0) as u64;
    loop {
        check_modem_cancelled(state, session_id, progress).await?;
        let marker = modem_wait_for_next_marker(&mut reader, Duration::from_secs(15)).await?;
        if marker == MODEM_EOT {
            write_runtime_bytes(state, session_id, &[MODEM_ACK, MODEM_CRC_REQUEST]).await?;
            if let Ok(final_marker) =
                modem_wait_for_next_marker(&mut reader, Duration::from_secs(5)).await
            {
                if final_marker != MODEM_EOT {
                    let final_packet = modem_read_packet(&mut reader, final_marker).await?;
                    if final_packet.block_no == 0 {
                        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
                    }
                }
            }
            break;
        }
        let packet = modem_read_packet(&mut reader, marker).await?;
        if packet.block_no == expected {
            output.extend_from_slice(&packet.data);
            progress.update(output.len() as u64, total)?;
            expected = expected.wrapping_add(1);
        }
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
    }

    if let Some(size) = expected_size {
        output.truncate(size.min(output.len()));
    } else {
        while output.last().copied() == Some(MODEM_EOF) {
            output.pop();
        }
    }
    let destination = if Path::new(local_destination).is_dir() {
        let safe_name = Path::new(&name)
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("ymodem-file.bin");
        Path::new(local_destination)
            .join(safe_name)
            .display()
            .to_string()
    } else {
        local_destination.to_string()
    };
    write_local_transfer_file(&destination, &output)?;
    Ok(output.len() as u64)
}

struct ModemPacket {
    block_no: u8,
    data: Vec<u8>,
}

async fn modem_wait_for_receiver(reader: &mut ModemByteReader) -> Result<bool, String> {
    let started = Instant::now();
    loop {
        let remaining = Duration::from_secs(45).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("modem receiver did not send NAK/C within 45s".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(3)))
            .await
        {
            Ok(MODEM_CRC_REQUEST) => return Ok(true),
            Ok(MODEM_NAK) => return Ok(false),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

async fn modem_wait_for_crc_request(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("timed out waiting for YModem CRC request".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(2)))
            .await
        {
            Ok(MODEM_CRC_REQUEST) => return Ok(()),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

async fn modem_send_packet_with_retries(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
    marker: u8,
    block_no: u8,
    payload: &[u8],
    crc: bool,
) -> Result<(), String> {
    let size = if marker == MODEM_STX {
        YMODEM_BLOCK_SIZE
    } else {
        XMODEM_BLOCK_SIZE
    };
    let packet = modem_packet_bytes(marker, block_no, payload, size, crc);
    for _ in 0..10 {
        write_runtime_bytes(state, session_id, &packet).await?;
        match modem_wait_for_ack(reader, Duration::from_secs(12)).await? {
            ModemAck::Ack => return Ok(()),
            ModemAck::Nak => {}
        }
    }
    Err(format!(
        "modem block {block_no} was rejected too many times"
    ))
}

fn modem_packet_bytes(marker: u8, block_no: u8, payload: &[u8], size: usize, crc: bool) -> Vec<u8> {
    let mut data = vec![MODEM_EOF; size];
    data[..payload.len().min(size)].copy_from_slice(&payload[..payload.len().min(size)]);
    let mut packet = Vec::with_capacity(3 + size + if crc { 2 } else { 1 });
    packet.push(marker);
    packet.push(block_no);
    packet.push(255_u8.wrapping_sub(block_no));
    packet.extend_from_slice(&data);
    if crc {
        let crc = crc16_xmodem(&data);
        packet.extend_from_slice(&crc.to_be_bytes());
    } else {
        packet.push(data.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte)));
    }
    packet
}

enum ModemAck {
    Ack,
    Nak,
}

async fn modem_wait_for_ack(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<ModemAck, String> {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("timed out waiting for modem ACK".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(2)))
            .await
        {
            Ok(MODEM_ACK) => return Ok(ModemAck::Ack),
            Ok(MODEM_NAK) => return Ok(ModemAck::Nak),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

async fn modem_finish_eot(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    for _ in 0..10 {
        write_runtime_bytes(state, session_id, &[MODEM_EOT]).await?;
        match modem_wait_for_ack(reader, Duration::from_secs(12)).await? {
            ModemAck::Ack => return Ok(()),
            ModemAck::Nak => {}
        }
    }
    Err("remote did not ACK modem EOT".to_string())
}

async fn modem_wait_for_packet_marker(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<u8, String> {
    for _ in 0..24 {
        write_runtime_bytes(state, session_id, &[MODEM_CRC_REQUEST]).await?;
        match modem_wait_for_next_marker(reader, Duration::from_secs(3)).await {
            Ok(marker) => return Ok(marker),
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Err("timed out waiting for modem packet".to_string())
}

async fn modem_wait_for_next_marker(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<u8, String> {
    loop {
        match reader.next_byte(timeout).await? {
            MODEM_SOH => return Ok(MODEM_SOH),
            MODEM_STX => return Ok(MODEM_STX),
            MODEM_EOT => return Ok(MODEM_EOT),
            MODEM_CAN => return Err("modem transfer cancelled by remote".to_string()),
            _ => {}
        }
    }
}

async fn modem_read_packet(
    reader: &mut ModemByteReader,
    marker: u8,
) -> Result<ModemPacket, String> {
    let size = match marker {
        MODEM_SOH => XMODEM_BLOCK_SIZE,
        MODEM_STX => YMODEM_BLOCK_SIZE,
        _ => return Err(format!("unexpected modem packet marker: {marker}")),
    };
    let header = reader.read_exact(2, Duration::from_secs(5)).await?;
    let block_no = header[0];
    let inverse = header[1];
    if block_no != 255_u8.wrapping_sub(inverse) {
        return Err("modem packet block number check failed".to_string());
    }
    let mut data = reader.read_exact(size + 2, Duration::from_secs(8)).await?;
    let received_crc = u16::from_be_bytes([data[size], data[size + 1]]);
    data.truncate(size);
    let actual_crc = crc16_xmodem(&data);
    if received_crc != actual_crc {
        return Err(format!(
            "modem packet CRC mismatch: received={received_crc:04x} actual={actual_crc:04x}"
        ));
    }
    Ok(ModemPacket { block_no, data })
}

fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn write_local_transfer_file(path: &str, data: &[u8]) -> Result<(), String> {
    let destination = Path::new(path);
    if let Some(parent) = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| format!("创建本地目录失败: {error}"))?;
    }
    fs::write(destination, data).map_err(|error| format!("写入本地文件失败: {error}"))
}

fn parse_ymodem_metadata(data: &[u8]) -> (String, Option<usize>) {
    let name_end = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let name = String::from_utf8_lossy(&data[..name_end])
        .trim()
        .to_string();
    let rest = if name_end < data.len() {
        &data[name_end + 1..]
    } else {
        &[]
    };
    let rest = String::from_utf8_lossy(rest);
    let size = rest
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<usize>().ok());
    (name, size)
}

fn local_file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-transfer.bin")
        .to_string()
}

fn remote_parent_and_file_name(path: &str) -> (String, String) {
    let normalized = path.trim_end_matches('/');
    let Some((parent, name)) = normalized.rsplit_once('/') else {
        return (String::new(), normalized.to_string());
    };
    (parent.to_string(), name.to_string())
}

fn is_modem_timeout(error: &str) -> bool {
    error.contains("timeout") || error.contains("timed out")
}

fn remote_path(value: &str) -> Option<&str> {
    value
        .strip_prefix("remote:")
        .or_else(|| value.strip_prefix("ssh:"))
        .filter(|path| !path.trim().is_empty())
}

fn ssh_handle_for_transfer(
    state: &AppState,
    session_id: &str,
) -> Result<Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>, String> {
    let connections = state.ssh.lock().map_err(|error| error.to_string())?;
    connections
        .get(session_id)
        .map(|runtime| Arc::clone(&runtime.handle))
        .ok_or_else(|| "需要先连接 SSH/Tmux 会话才能执行 remote: 传输".to_string())
}

fn copy_local_file_for_transfer(
    source: &str,
    destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let source = PathBuf::from(source);
    let destination = PathBuf::from(destination);
    if !source.is_file() {
        return Err(
            "only local file copy is available for this protocol path right now".to_string(),
        );
    }
    let total = fs::metadata(&source)
        .map_err(|error| format!("failed to read transfer source metadata: {error}"))?
        .len();
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create transfer destination: {error}"))?;
    }
    let temp_destination = local_resume_part_path(&destination);
    let mut input =
        fs::File::open(&source).map_err(|error| format!("local transfer open failed: {error}"))?;
    let mut copied = local_resume_offset(&temp_destination, total)?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        input
            .seek(std::io::SeekFrom::Start(copied))
            .map_err(|error| format!("local transfer seek failed: {error}"))?;
        progress.update(copied, total)?;
    }
    if copied == total {
        finalize_local_resume_file(&temp_destination, &destination)?;
        return Ok(copied);
    }
    let mut output = open_local_resume_writer(&temp_destination, copied)
        .map_err(|error| format!("local transfer create failed: {error}"))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        progress.check_cancelled()?;
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("local transfer read failed: {error}"))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("local transfer write failed: {error}"))?;
        copied += read as u64;
        progress.update(copied, total)?;
    }
    output
        .flush()
        .map_err(|error| format!("local transfer flush failed: {error}"))?;
    drop(output);
    finalize_local_resume_file(&temp_destination, &destination)?;
    Ok(copied)
}

async fn open_sftp_session(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
) -> Result<SftpSession, String> {
    let channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|error| format!("SFTP 打开 SSH channel 失败: {error}"))?
    };
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| format!("SFTP subsystem 启动失败: {error}"))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| format!("SFTP 初始化失败: {error}"))?;
    sftp.set_timeout(20);
    Ok(sftp)
}

/// A transfer writes to a temp sibling of the real destination and is only
/// renamed onto it after a full success; on any error the temp is best-effort
/// removed. Otherwise a mid-transfer failure leaves a partial file at the real
/// destination path with nothing distinguishing it from a complete one.
fn local_resume_part_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("portmate-transfer");
    target.with_file_name(format!("{name}.portmate-part"))
}

fn local_resume_offset(path: &Path, total: u64) -> Result<u64, String> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(0);
    };
    let size = metadata.len();
    if size <= total {
        Ok(size)
    } else {
        fs::remove_file(path)
            .map_err(|error| format!("删除过大的断点文件失败 {}: {error}", path.display()))?;
        Ok(0)
    }
}

fn open_local_resume_writer(path: &Path, offset: u64) -> std::io::Result<fs::File> {
    if offset == 0 {
        OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
    } else {
        OpenOptions::new().create(true).append(true).open(path)
    }
}

fn finalize_local_resume_file(temp: &Path, target: &Path) -> Result<(), String> {
    if target.exists() && target.is_file() {
        fs::remove_file(target)
            .map_err(|error| format!("删除旧目标文件失败 {}: {error}", target.display()))?;
    }
    fs::rename(temp, target).map_err(|error| {
        format!(
            "重命名本地目标文件失败 {} -> {}: {error}",
            temp.display(),
            target.display()
        )
    })
}

fn remote_resume_part_path(target: &str) -> String {
    match target.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/{name}.portmate-part"),
        None => format!("{target}.portmate-part"),
    }
}

async fn sftp_resume_offset(sftp: &SftpSession, path: &str, total: u64) -> Result<u64, String> {
    let Some(size) = sftp
        .metadata(path.to_string())
        .await
        .ok()
        .and_then(|metadata| metadata.size)
    else {
        return Ok(0);
    };
    if size <= total {
        Ok(size)
    } else {
        sftp.remove_file(path.to_string())
            .await
            .map_err(|error| format!("SFTP 删除过大的断点文件失败 {path}: {error}"))?;
        Ok(0)
    }
}

async fn sftp_open_resume_writer(
    sftp: &SftpSession,
    path: &str,
    offset: u64,
) -> Result<russh_sftp::client::fs::File, String> {
    let flags = if offset == 0 {
        OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE
    } else {
        OpenFlags::CREATE | OpenFlags::WRITE
    };
    let mut file = sftp
        .open_with_flags(path.to_string(), flags)
        .await
        .map_err(|error| format!("SFTP 打开断点文件失败 {path}: {error}"))?;
    if offset > 0 {
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|error| format!("SFTP 断点文件 seek 失败 {path}: {error}"))?;
    }
    Ok(file)
}

async fn sftp_finalize_resume_file(
    sftp: &SftpSession,
    temp: &str,
    target: &str,
) -> Result<(), String> {
    let _ = sftp.remove_file(target.to_string()).await;
    sftp.rename(temp.to_string(), target.to_string())
        .await
        .map_err(|error| format!("SFTP 重命名断点文件失败 {temp} -> {target}: {error}"))
}

async fn sftp_upload(
    sftp: &SftpSession,
    local_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let metadata = fs::metadata(local_source)
        .map_err(|error| format!("读取本地文件元数据失败 {local_source}: {error}"))?;
    if !metadata.is_file() {
        return Err("SFTP upload only supports regular local files".to_string());
    }
    let mut local_file = fs::File::open(local_source)
        .map_err(|error| format!("打开本地文件失败 {local_source}: {error}"))?;
    let file_name = Path::new(local_source)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-upload.bin");
    let target = sftp_destination_file_path(sftp, remote_destination, file_name).await?;
    let temp_target = remote_resume_part_path(&target);
    let mut copied = sftp_resume_offset(sftp, &temp_target, metadata.len()).await?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        local_file
            .seek(std::io::SeekFrom::Start(copied))
            .map_err(|error| format!("SFTP 本地文件 seek 失败 {local_source}: {error}"))?;
        progress.update(copied, metadata.len())?;
    }
    if copied == metadata.len() {
        sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
        return Ok(copied);
    }
    let mut remote_file = sftp_open_resume_writer(sftp, &temp_target, copied).await?;

    let copy_result: Result<u64, String> = async {
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            progress.check_cancelled()?;
            let read = local_file
                .read(&mut buffer)
                .map_err(|error| format!("读取本地文件失败 {local_source}: {error}"))?;
            if read == 0 {
                break;
            }
            remote_file
                .write_all(&buffer[..read])
                .await
                .map_err(|error| format!("SFTP 写入远端文件失败 {temp_target}: {error}"))?;
            copied += read as u64;
            progress.update(copied, metadata.len())?;
        }
        remote_file
            .flush()
            .await
            .map_err(|error| format!("SFTP 刷新远端文件失败 {temp_target}: {error}"))?;
        remote_file
            .shutdown()
            .await
            .map_err(|error| format!("SFTP 关闭远端文件失败 {temp_target}: {error}"))?;
        Ok(copied)
    }
    .await;

    match copy_result {
        Ok(copied) => {
            sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
            Ok(copied)
        }
        Err(error) => Err(error),
    }
}

async fn sftp_download(
    sftp: &SftpSession,
    remote_source: &str,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut remote_file = sftp
        .open(remote_source.to_string())
        .await
        .map_err(|error| format!("SFTP 打开远端文件失败 {remote_source}: {error}"))?;
    let total = sftp
        .metadata(remote_source.to_string())
        .await
        .ok()
        .and_then(|metadata| metadata.size)
        .unwrap_or(0);
    let target = local_destination_file_path(local_destination, remote_source)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建本地目录失败 {}: {error}", parent.display()))?;
    }
    let temp_target = local_resume_part_path(&target);
    let mut copied = local_resume_offset(&temp_target, total)?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        remote_file
            .seek(std::io::SeekFrom::Start(copied))
            .await
            .map_err(|error| format!("SFTP 远端文件 seek 失败 {remote_source}: {error}"))?;
        progress.update(copied, total)?;
    }
    if total > 0 && copied == total {
        finalize_local_resume_file(&temp_target, &target)?;
        let _ = remote_file.shutdown().await;
        return Ok(copied);
    }
    let mut local_file = open_local_resume_writer(&temp_target, copied)
        .map_err(|error| format!("创建本地目标文件失败 {}: {error}", temp_target.display()))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        progress.check_cancelled()?;
        let read = remote_file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("SFTP 读取远端文件失败 {remote_source}: {error}"))?;
        if read == 0 {
            break;
        }
        local_file
            .write_all(&buffer[..read])
            .map_err(|error| format!("写入本地目标文件失败 {}: {error}", temp_target.display()))?;
        copied += read as u64;
        progress.update(copied, total)?;
    }
    local_file
        .flush()
        .map_err(|error| format!("刷新本地目标文件失败 {}: {error}", temp_target.display()))?;
    drop(local_file);
    finalize_local_resume_file(&temp_target, &target)?;
    let _ = remote_file.shutdown().await;
    Ok(copied)
}

async fn sftp_remote_copy(
    sftp: &SftpSession,
    remote_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut source_file = sftp
        .open(remote_source.to_string())
        .await
        .map_err(|error| format!("SFTP 打开远端源文件失败 {remote_source}: {error}"))?;
    let total = sftp
        .metadata(remote_source.to_string())
        .await
        .ok()
        .and_then(|metadata| metadata.size)
        .unwrap_or(0);
    let file_name = remote_file_name(remote_source);
    let target = sftp_destination_file_path(sftp, remote_destination, &file_name).await?;
    let temp_target = remote_resume_part_path(&target);
    let mut copied = sftp_resume_offset(sftp, &temp_target, total).await?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        source_file
            .seek(std::io::SeekFrom::Start(copied))
            .await
            .map_err(|error| format!("SFTP 远端源文件 seek 失败 {remote_source}: {error}"))?;
        progress.update(copied, total)?;
    }
    if total > 0 && copied == total {
        sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
        let _ = source_file.shutdown().await;
        return Ok(copied);
    }
    let mut target_file = sftp_open_resume_writer(sftp, &temp_target, copied).await?;

    let copy_result: Result<u64, String> = async {
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            progress.check_cancelled()?;
            let read = source_file
                .read(&mut buffer)
                .await
                .map_err(|error| format!("SFTP 读取远端源文件失败 {remote_source}: {error}"))?;
            if read == 0 {
                break;
            }
            target_file
                .write_all(&buffer[..read])
                .await
                .map_err(|error| format!("SFTP 写入远端目标文件失败 {temp_target}: {error}"))?;
            copied += read as u64;
            progress.update(copied, total)?;
        }
        target_file
            .flush()
            .await
            .map_err(|error| format!("SFTP 刷新远端目标文件失败 {temp_target}: {error}"))?;
        target_file
            .shutdown()
            .await
            .map_err(|error| format!("SFTP 关闭远端目标文件失败 {temp_target}: {error}"))?;
        Ok(copied)
    }
    .await;
    let _ = source_file.shutdown().await;

    match copy_result {
        Ok(copied) => {
            sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
            Ok(copied)
        }
        Err(error) => Err(error),
    }
}

async fn sftp_destination_file_path(
    sftp: &SftpSession,
    remote_destination: &str,
    source_name: &str,
) -> Result<String, String> {
    let destination = remote_destination.trim();
    if destination.is_empty() {
        return Err("SFTP 远端目标路径不能为空".to_string());
    }

    if destination.ends_with('/') {
        sftp_create_dir_all(sftp, destination).await?;
        return Ok(remote_join_path(destination, source_name));
    }

    if let Ok(metadata) = sftp.metadata(destination.to_string()).await {
        if metadata.is_dir() {
            return Ok(remote_join_path(destination, source_name));
        }
    }

    if let Some(parent) = remote_parent_path(destination) {
        if parent != "." && parent != "/" {
            sftp_create_dir_all(sftp, &parent).await?;
        }
    }
    Ok(destination.to_string())
}

fn local_destination_file_path(
    local_destination: &str,
    remote_source: &str,
) -> Result<PathBuf, String> {
    let destination = expand_identity_path(local_destination.trim());
    if local_destination.trim().is_empty() {
        return Err("本地目标路径不能为空".to_string());
    }
    let source_name = remote_file_name(remote_source);
    let ends_with_separator = local_destination.ends_with('/') || local_destination.ends_with('\\');
    if destination.is_dir() || ends_with_separator {
        Ok(destination.join(source_name))
    } else {
        Ok(destination)
    }
}

async fn sftp_create_dir_all(sftp: &SftpSession, path: &str) -> Result<(), String> {
    let path = path.trim().trim_end_matches('/');
    if path.is_empty() || path == "." || path == "/" {
        return Ok(());
    }

    let mut current = if path.starts_with('/') {
        "/".to_string()
    } else {
        String::new()
    };
    for part in path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        current = remote_join_path(&current, part);
        match sftp.create_dir(current.clone()).await {
            Ok(()) => {}
            Err(error) => match sftp.metadata(current.clone()).await {
                Ok(metadata) if metadata.is_dir() => {}
                _ => return Err(format!("SFTP 创建远端目录失败 {current}: {error}")),
            },
        }
    }
    Ok(())
}

fn remote_parent_path(path: &str) -> Option<String> {
    let path = path.trim().trim_end_matches('/');
    let index = path.rfind('/')?;
    if index == 0 {
        Some("/".to_string())
    } else {
        Some(path[..index].to_string())
    }
}

fn remote_file_name(path: &str) -> String {
    path.trim()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-file.bin")
        .to_string()
}

fn remote_join_path(parent: &str, name: &str) -> String {
    let name = name.trim_matches('/');
    if parent.is_empty() || parent == "." {
        name.to_string()
    } else if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

async fn remote_copy(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    remote_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let command = remote_copy_command(remote_source, remote_destination);
    let mut channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|error| format!("SSH remote copy 打开 channel 失败: {error}"))?
    };
    channel
        .exec(true, command)
        .await
        .map_err(|error| format!("SSH remote copy 启动失败: {error}"))?;

    let mut output = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    let mut reported_total = None;
    let mut reported_resume = None;
    let mut reported_progress = None;
    let mut reported_done = None;
    let started = Instant::now();

    loop {
        if progress.cancel.load(Ordering::SeqCst) {
            let _ = channel.eof().await;
            let _ = channel.close().await;
            return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
        }
        if started.elapsed() > Duration::from_secs(300) {
            let _ = channel.close().await;
            return Err("SSH remote copy 超时".to_string());
        }

        match tokio::time::timeout(Duration::from_millis(250), channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                output.extend_from_slice(&data);
                let markers = remote_copy_markers(&output);
                if markers.total.is_some() && markers.total != reported_total {
                    let total = markers.total.unwrap_or_default();
                    progress.update(0, total)?;
                    reported_total = Some(total);
                }
                if markers.resume.is_some() && markers.resume != reported_resume {
                    let mut resume_bytes = markers.resume.unwrap_or_default();
                    if let Some(total) = markers.total.or(reported_total) {
                        resume_bytes = resume_bytes.min(total);
                    }
                    progress.set_rate_baseline(resume_bytes);
                    progress.update(resume_bytes, markers.total.or(reported_total).unwrap_or(0))?;
                    reported_resume = Some(markers.resume.unwrap_or_default());
                }
                if markers.progress.is_some() && markers.progress != reported_progress {
                    let mut progress_bytes = markers.progress.unwrap_or_default();
                    if let Some(total) = markers.total.or(reported_total) {
                        progress_bytes = progress_bytes.min(total);
                    }
                    progress.update(
                        progress_bytes,
                        markers.total.or(reported_total).unwrap_or(0),
                    )?;
                    reported_progress = Some(markers.progress.unwrap_or_default());
                }
                if markers.done.is_some() && markers.done != reported_done {
                    let done = markers.done.unwrap_or_default();
                    progress.update(done, reported_total.unwrap_or(done))?;
                    reported_done = Some(done);
                }
            }
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => stderr.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExitStatus { exit_status: code })) => exit_status = Some(code),
            Ok(Some(ChannelMsg::Eof | ChannelMsg::Close)) | Ok(None) => break,
            Ok(Some(_)) => {}
            Err(_) => {}
        }
    }

    if exit_status.is_some_and(|code| code != 0) {
        return Err(format!(
            "SSH remote copy 返回非零状态 {:?}: {}",
            exit_status,
            String::from_utf8_lossy(&stderr)
        ));
    }

    let markers = remote_copy_markers(&output);
    let bytes = markers.done.ok_or_else(|| {
        format!(
            "remote copy completed but done marker was missing: {}",
            String::from_utf8_lossy(&output)
        )
    })?;
    progress.update(bytes, reported_total.unwrap_or(bytes))?;
    Ok(bytes)
}

fn remote_copy_command(remote_source: &str, remote_destination: &str) -> String {
    format!(
        concat!(
            "src={}; dst={}; target=; part=; pid=; ",
            "remote_name=${{src##*/}}; if [ -z \"$remote_name\" ]; then remote_name=portmate-file.bin; fi; ",
            "case \"$dst\" in */) target=\"${{dst%/}}/$remote_name\" ;; ",
            "*) if [ -d \"$dst\" ]; then target=\"${{dst%/}}/$remote_name\"; else target=\"$dst\"; fi ;; esac; ",
            "case \"$target\" in */*) part=\"${{target%/*}}/${{target##*/}}.portmate-part\" ;; ",
            "*) part=\"$target.portmate-part\" ;; esac; ",
            "cleanup() {{ if [ -n \"$pid\" ]; then kill \"$pid\" 2>/dev/null || :; fi; }}; ",
            "trap cleanup INT TERM HUP EXIT; ",
            "if ! total=$(stat -c %s -- \"$src\"); then exit 1; fi; ",
            "printf '__PORTMATE_SIZE__%s\\n' \"$total\"; ",
            "offset=0; ",
            "if [ -e \"$part\" ]; then ",
            "if current=$(stat -c %s -- \"$part\" 2>/dev/null); then ",
            "if [ \"$current\" -le \"$total\" ]; then offset=$current; else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "printf '__PORTMATE_RESUME__%s\\n' \"$offset\"; ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$offset\"; ",
            "if [ \"$offset\" -lt \"$total\" ]; then ",
            "tail -c +$((offset + 1)) -- \"$src\" >> \"$part\" & pid=$!; ",
            "while kill -0 \"$pid\" 2>/dev/null; do ",
            "if current=$(stat -c %s -- \"$part\" 2>/dev/null); then ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$current\"; ",
            "fi; sleep 0.25; done; ",
            "wait \"$pid\"; status=$?; pid=; ",
            "if [ \"$status\" -ne 0 ]; then exit \"$status\"; fi; ",
            "fi; ",
            "final=$(stat -c %s -- \"$part\") || exit 1; ",
            "if [ \"$final\" -ne \"$total\" ]; then ",
            "printf 'PortMate remote copy size mismatch: %s of %s\\n' \"$final\" \"$total\" >&2; exit 1; ",
            "fi; ",
            "mv -f -- \"$part\" \"$target\" || exit 1; ",
            "stat -c '__PORTMATE_DONE__%s' -- \"$target\""
        ),
        shell_quote(remote_source),
        shell_quote(remote_destination)
    )
}

#[derive(Debug, Default, PartialEq, Eq)]
struct RemoteCopyMarkers {
    total: Option<u64>,
    resume: Option<u64>,
    progress: Option<u64>,
    done: Option<u64>,
}

fn remote_copy_markers(output: &[u8]) -> RemoteCopyMarkers {
    let text = String::from_utf8_lossy(output);
    let mut markers = RemoteCopyMarkers::default();
    for line in text.lines() {
        if let Some(value) = line.trim().strip_prefix("__PORTMATE_SIZE__") {
            markers.total = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_RESUME__") {
            markers.resume = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_PROGRESS__") {
            markers.progress = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_DONE__") {
            markers.done = value.trim().parse::<u64>().ok();
        }
    }
    markers
}

enum FileOperation {
    CreateDirectory,
    Delete,
}

async fn list_files_inner(
    state: &AppState,
    request: ListFilesRequest,
) -> Result<Vec<FileEntry>, String> {
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote file list requires sessionId".to_string())?;
        let handle = ssh_handle_for_transfer(state, session_id)?;
        list_remote_files(handle, &request.path).await
    } else {
        list_local_files(&request.path)
    }
}

async fn file_properties_inner(
    state: &AppState,
    request: FilePropertiesRequest,
) -> Result<FileProperties, String> {
    if request.path.trim().is_empty() {
        return Err("属性路径不能为空".to_string());
    }
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote file properties require sessionId".to_string())?;
        let handle = ssh_handle_for_transfer(state, session_id)?;
        let sftp = open_sftp_session(handle).await?;
        let result = remote_file_properties(&sftp, request.path.trim()).await;
        let _ = sftp.close().await;
        result
    } else {
        local_file_properties(request.path.trim())
    }
}

async fn file_operation_inner(
    state: &AppState,
    request: FileOperationRequest,
    operation: FileOperation,
) -> Result<(), String> {
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote file operation requires sessionId".to_string())?;
        let handle = ssh_handle_for_transfer(state, session_id)?;
        let sftp = open_sftp_session(handle).await?;
        let result = match operation {
            FileOperation::CreateDirectory => sftp_create_dir_all(&sftp, &request.path).await,
            FileOperation::Delete => sftp_remove_recursive(&sftp, &request.path).await,
        };
        let _ = sftp.close().await;
        result
    } else {
        match operation {
            FileOperation::CreateDirectory => {
                let path = PathBuf::from(&request.path);
                fs::create_dir_all(&path)
                    .map_err(|error| format!("创建本地目录失败 {}: {error}", path.display()))
            }
            FileOperation::Delete => {
                let path = validate_local_mutating_path(&request.path)?;
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|error| format!("读取本地路径失败 {}: {error}", path.display()))?;
                if metadata.is_dir() {
                    fs::remove_dir_all(&path)
                        .map_err(|error| format!("删除本地目录失败 {}: {error}", path.display()))
                } else {
                    fs::remove_file(&path)
                        .map_err(|error| format!("删除本地文件失败 {}: {error}", path.display()))
                }
            }
        }
    }
}

async fn rename_path_inner(state: &AppState, request: RenamePathRequest) -> Result<(), String> {
    if request.old_path.trim().is_empty() || request.new_path.trim().is_empty() {
        return Err("重命名路径不能为空".to_string());
    }
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote rename requires sessionId".to_string())?;
        let handle = ssh_handle_for_transfer(state, session_id)?;
        let sftp = open_sftp_session(handle).await?;
        let result = sftp
            .rename(request.old_path.clone(), request.new_path.clone())
            .await
            .map_err(|error| {
                format!(
                    "SFTP 重命名失败 {} -> {}: {error}",
                    request.old_path, request.new_path
                )
            });
        let _ = sftp.close().await;
        result
    } else {
        let old_path = validate_local_mutating_path(&request.old_path)?;
        let new_path = validate_local_mutating_path(&request.new_path)?;
        fs::rename(&old_path, &new_path).map_err(|error| {
            format!(
                "本地重命名失败 {} -> {}: {error}",
                old_path.display(),
                new_path.display()
            )
        })
    }
}

async fn chmod_path_inner(state: &AppState, request: ChmodPathRequest) -> Result<(), String> {
    if request.path.trim().is_empty() {
        return Err("权限路径不能为空".to_string());
    }
    if request.mode > 0o7777 {
        return Err("权限模式必须是 0000-7777 八进制范围".to_string());
    }
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote chmod requires sessionId".to_string())?;
        let handle = ssh_handle_for_transfer(state, session_id)?;
        let sftp = open_sftp_session(handle).await?;
        let mut metadata = sftp
            .symlink_metadata(request.path.clone())
            .await
            .map_err(|error| format!("SFTP 读取权限失败 {}: {error}", request.path))?;
        let file_type_bits = metadata.permissions.unwrap_or(0) & 0o170000;
        metadata.permissions = Some(file_type_bits | request.mode);
        let result = sftp
            .set_metadata(request.path.clone(), metadata)
            .await
            .map_err(|error| format!("SFTP 设置权限失败 {}: {error}", request.path));
        let _ = sftp.close().await;
        result
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = PathBuf::from(&request.path);
            let mut permissions = fs::metadata(&path)
                .map_err(|error| format!("读取本地权限失败 {}: {error}", path.display()))?
                .permissions();
            permissions.set_mode(request.mode);
            fs::set_permissions(&path, permissions)
                .map_err(|error| format!("设置本地权限失败 {}: {error}", path.display()))
        }
        #[cfg(not(unix))]
        {
            let _ = state;
            Err("当前平台不支持本地 chmod".to_string())
        }
    }
}

async fn list_tmux_state_inner(state: &AppState, session_id: &str) -> Result<TmuxState, String> {
    let handle = ssh_handle_for_transfer(state, session_id)?;
    let sessions_output = exec_ssh_command_capture(
        Arc::clone(&handle),
        "tmux list-sessions -F '#{session_name}\t#{session_windows}\t#{session_attached}\t#{session_created}' 2>/dev/null || true",
        Duration::from_secs(8),
    )
    .await?;
    let panes_output = exec_ssh_command_capture(
        handle,
        "tmux list-panes -a -F '#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_id}\t#{pane_active}\t#{pane_current_command}\t#{pane_title}' 2>/dev/null || true",
        Duration::from_secs(8),
    )
    .await?;

    let sessions = sessions_output
        .lines()
        .filter_map(parse_tmux_session)
        .collect::<Vec<_>>();
    let panes = panes_output
        .lines()
        .filter_map(parse_tmux_pane)
        .collect::<Vec<_>>();
    Ok(TmuxState { sessions, panes })
}

fn parse_tmux_session(line: &str) -> Option<TmuxSessionInfo> {
    let mut parts = line.split('\t');
    let name = parts.next()?.to_string();
    if name.is_empty() {
        return None;
    }
    let windows = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default();
    let attached = parts
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default();
    let created = parts
        .next()
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(|timestamp| chrono::DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|value| value.to_rfc3339());
    Some(TmuxSessionInfo {
        name,
        windows,
        attached,
        created,
    })
}

fn parse_tmux_pane(line: &str) -> Option<TmuxPaneInfo> {
    let mut parts = line.split('\t');
    let session = parts.next()?.to_string();
    if session.is_empty() {
        return None;
    }
    Some(TmuxPaneInfo {
        session,
        window_index: parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_default(),
        pane_index: parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or_default(),
        pane_id: parts.next().unwrap_or_default().to_string(),
        active: parts.next().unwrap_or_default() == "1",
        command: parts.next().unwrap_or_default().to_string(),
        title: parts.next().unwrap_or_default().to_string(),
    })
}

fn list_local_files(path: &str) -> Result<Vec<FileEntry>, String> {
    let path = expand_identity_path(if path.trim().is_empty() {
        "."
    } else {
        path.trim()
    });
    let mut entries = Vec::new();
    for entry in fs::read_dir(&path)
        .map_err(|error| format!("读取本地目录失败 {}: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| format!("读取本地目录项失败: {error}"))?;
        let metadata = entry
            .metadata()
            .map_err(|error| format!("读取本地文件元数据失败: {error}"))?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| {
                chrono::DateTime::<Utc>::from(std::time::UNIX_EPOCH + duration).to_rfc3339()
            });
        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path().display().to_string(),
            is_dir: metadata.is_dir(),
            size: if metadata.is_file() {
                metadata.len()
            } else {
                0
            },
            modified,
        });
    }
    sort_file_entries(&mut entries);
    Ok(entries)
}

fn local_file_properties(path: &str) -> Result<FileProperties, String> {
    let path = expand_identity_path(path);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("读取本地路径属性失败 {}: {error}", path.display()))?;
    let file_type = metadata.file_type();
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode())
    };
    #[cfg(not(unix))]
    let permissions = None;
    Ok(FileProperties {
        name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_else(|| path.to_str().unwrap_or(""))
            .to_string(),
        path: path.display().to_string(),
        remote: false,
        kind: file_kind_label(
            metadata.is_dir(),
            metadata.is_file(),
            file_type.is_symlink(),
        ),
        is_dir: metadata.is_dir(),
        is_file: metadata.is_file(),
        is_symlink: file_type.is_symlink(),
        size: if metadata.is_file() {
            metadata.len()
        } else {
            0
        },
        permissions,
        modified: metadata.modified().ok().and_then(system_time_to_rfc3339),
        accessed: metadata.accessed().ok().and_then(system_time_to_rfc3339),
        created: metadata.created().ok().and_then(system_time_to_rfc3339),
    })
}

async fn list_remote_files(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    path: &str,
) -> Result<Vec<FileEntry>, String> {
    let path = if path.trim().is_empty() {
        "."
    } else {
        path.trim()
    };
    let sftp = open_sftp_session(handle).await?;
    let result = list_remote_files_via_sftp(&sftp, path).await;
    let _ = sftp.close().await;
    result
}

async fn list_remote_files_via_sftp(
    sftp: &SftpSession,
    path: &str,
) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    for entry in sftp
        .read_dir(path.to_string())
        .await
        .map_err(|error| format!("SFTP 读取远端目录失败 {path}: {error}"))?
    {
        let metadata = entry.metadata();
        let name = entry.file_name();
        let modified = metadata
            .mtime
            .and_then(|timestamp| chrono::DateTime::<Utc>::from_timestamp(timestamp.into(), 0))
            .map(|value| value.to_rfc3339());
        entries.push(FileEntry {
            path: remote_join_path(path, &name),
            name,
            is_dir: metadata.is_dir(),
            size: if metadata.is_regular() {
                metadata.len()
            } else {
                0
            },
            modified,
        });
    }
    sort_file_entries(&mut entries);
    Ok(entries)
}

async fn remote_file_properties(sftp: &SftpSession, path: &str) -> Result<FileProperties, String> {
    let metadata = sftp
        .symlink_metadata(path.to_string())
        .await
        .map_err(|error| format!("SFTP 读取远端属性失败 {path}: {error}"))?;
    let is_dir = metadata.is_dir();
    let is_file = metadata.is_regular();
    let is_symlink = metadata.is_symlink();
    Ok(FileProperties {
        name: remote_file_name(path),
        path: path.to_string(),
        remote: true,
        kind: file_kind_label(is_dir, is_file, is_symlink),
        is_dir,
        is_file,
        is_symlink,
        size: if is_file { metadata.len() } else { 0 },
        permissions: metadata.permissions,
        modified: metadata
            .mtime
            .and_then(|timestamp| chrono::DateTime::<Utc>::from_timestamp(timestamp.into(), 0))
            .map(|value| value.to_rfc3339()),
        accessed: None,
        created: None,
    })
}

fn file_kind_label(is_dir: bool, is_file: bool, is_symlink: bool) -> String {
    if is_symlink {
        "symlink"
    } else if is_dir {
        "directory"
    } else if is_file {
        "file"
    } else {
        "other"
    }
    .to_string()
}

fn system_time_to_rfc3339(time: std::time::SystemTime) -> Option<String> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| {
            chrono::DateTime::<Utc>::from(std::time::UNIX_EPOCH + duration).to_rfc3339()
        })
}

async fn sftp_remove_recursive(sftp: &SftpSession, path: &str) -> Result<(), String> {
    let path = validate_remote_delete_path(path)?;
    let mut stack = vec![(path.to_string(), false)];

    while let Some((current, visited)) = stack.pop() {
        let metadata = sftp
            .symlink_metadata(current.clone())
            .await
            .map_err(|error| format!("SFTP 读取远端路径失败 {current}: {error}"))?;
        let is_directory = metadata.is_dir() && !metadata.is_symlink();
        if is_directory && !visited {
            stack.push((current.clone(), true));
            let entries = sftp
                .read_dir(current.clone())
                .await
                .map_err(|error| format!("SFTP 读取远端目录失败 {current}: {error}"))?;
            for entry in entries {
                stack.push((remote_join_path(&current, &entry.file_name()), false));
            }
            continue;
        }

        if is_directory {
            sftp.remove_dir(current.clone())
                .await
                .map_err(|error| format!("SFTP 删除远端目录失败 {current}: {error}"))?;
        } else {
            sftp.remove_file(current.clone())
                .await
                .map_err(|error| format!("SFTP 删除远端文件失败 {current}: {error}"))?;
        }
    }

    Ok(())
}

fn validate_remote_delete_path(path: &str) -> Result<&str, String> {
    let path = path.trim();
    let trimmed = path.trim_end_matches('/');
    if path.is_empty()
        || trimmed.is_empty()
        || matches!(trimmed, "." | "~" | "/" | "//")
        || path.contains('\0')
    {
        return Err("拒绝删除空路径、根目录或当前目录".to_string());
    }
    Ok(path)
}

/// Guards local delete/rename endpoints against the two most catastrophic
/// fat-finger paths: an empty/`.`/`~` path, and a path that resolves to a
/// filesystem root or to the user's home directory itself.
fn validate_local_mutating_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    let trimmed_slashes = trimmed.trim_end_matches(['/', '\\']);
    if trimmed.is_empty()
        || trimmed_slashes.is_empty()
        || matches!(trimmed_slashes, "." | ".." | "~")
        || trimmed.contains('\0')
    {
        return Err("拒绝操作空路径、根目录或当前目录".to_string());
    }

    let candidate = expand_identity_path(trimmed);
    if candidate.parent().is_none() {
        return Err("拒绝操作文件系统根目录".to_string());
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        let candidate_check = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        let home_check = home.canonicalize().unwrap_or(home);
        if candidate_check == home_check {
            return Err("拒绝操作用户主目录".to_string());
        }
    }
    Ok(candidate)
}

fn sort_file_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

async fn scp_upload(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    local_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut file =
        fs::File::open(local_source).map_err(|error| format!("读取本地文件失败: {error}"))?;
    let size = file
        .metadata()
        .map_err(|error| format!("读取本地文件元数据失败: {error}"))?
        .len();
    let file_name = Path::new(local_source)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-upload.bin");
    let command = scp_upload_command(remote_destination, file_name, size);
    let mut channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|error| format!("SCP 打开 SSH channel 失败: {error}"))?
    };
    channel
        .exec(true, command)
        .await
        .map_err(|error| format!("SCP 启动远端接收失败: {error}"))?;

    let mut output = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    let mut reported_total = None;
    let started = Instant::now();
    let mut copied = loop {
        if progress.cancel.load(Ordering::SeqCst) {
            let _ = channel.eof().await;
            let _ = channel.close().await;
            return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
        }
        if started.elapsed() > Duration::from_secs(30) {
            let _ = channel.close().await;
            return Err("SCP upload 等待远端续传状态超时".to_string());
        }
        match tokio::time::timeout(Duration::from_millis(250), channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => {
                output.extend_from_slice(&data);
                let markers = remote_copy_markers(&output);
                if markers.total.is_some() && markers.total != reported_total {
                    let total = markers.total.unwrap_or_default();
                    progress.update(0, total)?;
                    reported_total = Some(total);
                }
                if let Some(resume) = markers.resume {
                    let resume = resume.min(size);
                    progress.set_rate_baseline(resume);
                    if resume > 0 {
                        progress.update(resume, size)?;
                    }
                    break resume;
                }
            }
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => stderr.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExitStatus { exit_status: code })) => exit_status = Some(code),
            Ok(Some(ChannelMsg::Eof | ChannelMsg::Close)) | Ok(None) => {
                return Err(format!(
                    "SCP upload remote closed before resume marker: {}{}",
                    String::from_utf8_lossy(&output),
                    String::from_utf8_lossy(&stderr)
                ));
            }
            Ok(Some(_)) => {}
            Err(_) => {}
        }
        if exit_status.is_some_and(|code| code != 0) {
            return Err(format!(
                "SCP upload remote returned non-zero before upload {:?}: {}",
                exit_status,
                String::from_utf8_lossy(&stderr)
            ));
        }
    };

    if copied < size {
        file.seek(std::io::SeekFrom::Start(copied))
            .map_err(|error| format!("SCP 定位本地续传偏移失败: {error}"))?;
    }
    let mut buffer = vec![0_u8; 64 * 1024];
    while copied < size {
        if progress.cancel.load(Ordering::SeqCst) {
            let _ = channel.eof().await;
            let _ = channel.close().await;
            return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
        }
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("读取本地文件失败: {error}"))?;
        if read == 0 {
            break;
        }
        channel
            .data(&buffer[..read])
            .await
            .map_err(|error| format!("SCP 写入文件内容失败: {error}"))?;
        copied += read as u64;
        progress.update(copied, size)?;
    }
    let _ = channel.eof().await;

    let started = Instant::now();
    loop {
        if progress.cancel.load(Ordering::SeqCst) {
            let _ = channel.eof().await;
            let _ = channel.close().await;
            return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
        }
        if started.elapsed() > Duration::from_secs(300) {
            let _ = channel.close().await;
            return Err("SCP upload 等待远端完成超时".to_string());
        }
        match tokio::time::timeout(Duration::from_millis(250), channel.wait()).await {
            Ok(Some(ChannelMsg::Data { data })) => output.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExtendedData { data, .. })) => stderr.extend_from_slice(&data),
            Ok(Some(ChannelMsg::ExitStatus { exit_status: code })) => exit_status = Some(code),
            Ok(Some(ChannelMsg::Eof | ChannelMsg::Close)) | Ok(None) => break,
            Ok(Some(_)) => {}
            Err(_) => {}
        }
    }

    if exit_status.is_some_and(|code| code != 0) {
        return Err(format!(
            "SCP upload remote returned non-zero {:?}: {}",
            exit_status,
            String::from_utf8_lossy(&stderr)
        ));
    }
    let markers = remote_copy_markers(&output);
    let done = markers.done.ok_or_else(|| {
        format!(
            "SCP upload completed but done marker was missing: {}",
            String::from_utf8_lossy(&output)
        )
    })?;
    if done != size {
        return Err(format!(
            "SCP upload size mismatch: remote done {done}, expected {size}"
        ));
    }
    progress.update(done, size)?;
    Ok(done)
}

fn scp_upload_command(remote_destination: &str, file_name: &str, total: u64) -> String {
    format!(
        concat!(
            "dst={}; source_name={}; total={}; target=; part=; ",
            "if [ -z \"$source_name\" ]; then source_name=portmate-upload.bin; fi; ",
            "case \"$dst\" in */) target=\"${{dst%/}}/$source_name\" ;; ",
            "*) if [ -d \"$dst\" ]; then target=\"${{dst%/}}/$source_name\"; else target=\"$dst\"; fi ;; esac; ",
            "case \"$target\" in */*) part=\"${{target%/*}}/${{target##*/}}.portmate-part\" ;; ",
            "*) part=\"$target.portmate-part\" ;; esac; ",
            "printf '__PORTMATE_SIZE__%s\\n' \"$total\"; ",
            "offset=0; ",
            "if [ -e \"$part\" ]; then ",
            "if current=$(stat -c %s -- \"$part\" 2>/dev/null); then ",
            "if [ \"$current\" -le \"$total\" ]; then offset=$current; else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "printf '__PORTMATE_RESUME__%s\\n' \"$offset\"; ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$offset\"; ",
            "if [ \"$offset\" -lt \"$total\" ]; then ",
            "cat >> \"$part\" || exit 1; ",
            "if current=$(stat -c %s -- \"$part\" 2>/dev/null); then ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$current\"; ",
            "fi; ",
            "fi; ",
            "final=$(stat -c %s -- \"$part\") || exit 1; ",
            "if [ \"$final\" -ne \"$total\" ]; then ",
            "printf 'PortMate SCP upload size mismatch: %s of %s\\n' \"$final\" \"$total\" >&2; exit 1; ",
            "fi; ",
            "mv -f -- \"$part\" \"$target\" || exit 1; ",
            "stat -c '__PORTMATE_DONE__%s' -- \"$target\""
        ),
        shell_quote(remote_destination),
        shell_quote(file_name),
        total
    )
}

async fn scp_download(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    remote_source: &str,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|error| format!("SCP 打开 SSH channel 失败: {error}"))?
    };
    channel
        .exec(true, format!("scp -f {}", shell_quote(remote_source)))
        .await
        .map_err(|error| format!("SCP 启动远端发送失败: {error}"))?;
    let mut pending = VecDeque::new();
    channel
        .data(&[0_u8][..])
        .await
        .map_err(|error| format!("SCP 写入初始确认失败: {error}"))?;

    let first = scp_next_byte(&mut channel, &mut pending)
        .await?
        .ok_or_else(|| "SCP remote closed before header".to_string())?;
    if first == 1 || first == 2 {
        let message = scp_read_line(&mut channel, &mut pending).await?;
        return Err(format!("SCP remote error: {message}"));
    }
    if first != b'C' {
        return Err(format!("SCP unexpected header byte: {first}"));
    }

    let header = scp_read_line(&mut channel, &mut pending).await?;
    let parts = header.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 2 {
        return Err(format!("SCP invalid file header: C{header}"));
    }
    let size = parts[1]
        .parse::<u64>()
        .map_err(|error| format!("SCP invalid file size: {error}"))?;

    if let Some(parent) = Path::new(local_destination).parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建本地目录失败: {error}"))?;
    }
    let target = Path::new(local_destination);
    let temp_target = local_resume_part_path(target);
    let copied = local_resume_offset(&temp_target, size)?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, size)?;
    }
    if copied == size {
        finalize_local_resume_file(&temp_target, target)?;
        let _ = channel.close().await;
        return Ok(size);
    }
    let mut file = open_local_resume_writer(&temp_target, copied)
        .map_err(|error| format!("创建本地目标文件失败: {error}"))?;
    channel
        .data(&[0_u8][..])
        .await
        .map_err(|error| format!("SCP 写入文件头确认失败: {error}"))?;

    let mut received = 0_u64;
    while received < size {
        progress.check_cancelled()?;
        if pending.is_empty() {
            match channel.wait().await {
                Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                    pending.extend(data.iter().copied());
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => {
                    return Err("SCP remote closed during file body".to_string());
                }
                _ => {}
            }
            continue;
        }
        let take = pending.len().min((size - received) as usize);
        let chunk = pending.drain(..take).collect::<Vec<_>>();
        let chunk_start = received;
        received += take as u64;
        if received <= copied {
            continue;
        }
        let write_from = copied.saturating_sub(chunk_start) as usize;
        file.write_all(&chunk[write_from..])
            .map_err(|error| format!("写入本地目标文件失败: {error}"))?;
        progress.update(received, size)?;
    }
    file.flush()
        .map_err(|error| format!("刷新本地目标文件失败: {error}"))?;
    drop(file);
    scp_wait_ack(&mut channel, &mut pending).await?;
    channel
        .data(&[0_u8][..])
        .await
        .map_err(|error| format!("SCP 写入完成确认失败: {error}"))?;
    finalize_local_resume_file(&temp_target, target)?;
    let _ = channel.close().await;
    Ok(size)
}

async fn scp_wait_ack(
    channel: &mut Channel<client::Msg>,
    pending: &mut VecDeque<u8>,
) -> Result<(), String> {
    match scp_next_byte(channel, pending).await? {
        Some(0) => Ok(()),
        Some(1) | Some(2) => {
            let message = scp_read_line(channel, pending).await?;
            Err(format!("SCP remote error: {message}"))
        }
        Some(byte) => Err(format!("SCP unexpected ack byte: {byte}")),
        None => Err("SCP remote closed while waiting for ack".to_string()),
    }
}

async fn scp_read_line(
    channel: &mut Channel<client::Msg>,
    pending: &mut VecDeque<u8>,
) -> Result<String, String> {
    let mut bytes = Vec::new();
    while let Some(byte) = scp_next_byte(channel, pending).await? {
        if byte == b'\n' {
            break;
        }
        bytes.push(byte);
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

async fn scp_next_byte(
    channel: &mut Channel<client::Msg>,
    pending: &mut VecDeque<u8>,
) -> Result<Option<u8>, String> {
    loop {
        if let Some(byte) = pending.pop_front() {
            return Ok(Some(byte));
        }
        match channel.wait().await {
            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                pending.extend(data.iter().copied());
            }
            Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => return Ok(None),
            _ => {}
        }
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

async fn create_tunnel_inner(
    state: &AppState,
    request: CreateTunnelRequest,
) -> Result<TunnelSpec, String> {
    let request = normalize_tunnel_request(request)?;
    let (handle, remote_forwards) = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(&request.session_id)
            .map(|runtime| {
                (
                    Arc::clone(&runtime.handle),
                    Arc::clone(&runtime.remote_forwards),
                )
            })
            .ok_or_else(|| "需要先连接 SSH/Tmux 会话才能创建 tunnel".to_string())?
    };

    let mut tunnel = TunnelSpec {
        id: Uuid::new_v4().to_string(),
        label: request.label.clone().unwrap_or_else(|| {
            tunnel_label(
                request.mode,
                &request.bind_host,
                request.bind_port,
                &request.target_host,
                request.target_port,
            )
        }),
        mode: request.mode,
        bind_host: request.bind_host.clone(),
        bind_port: request.bind_port,
        target_host: request.target_host.clone(),
        target_port: request.target_port,
        enabled: true,
    };

    if request.mode == TunnelMode::Remote {
        if request.target_host.trim().is_empty() || request.target_port == 0 {
            return Err("remote tunnel requires a local target host and port".to_string());
        }
        let returned_port = {
            let handle = handle.lock().await;
            handle
                .tcpip_forward(request.bind_host.clone(), u32::from(request.bind_port))
                .await
                .map_err(|error| {
                    format!(
                        "remote SSH tunnel request failed {}:{}: {error}",
                        request.bind_host, request.bind_port
                    )
                })?
        };
        if request.bind_port == 0 && returned_port > 0 {
            tunnel.bind_port = returned_port as u16;
            tunnel.label = format!(
                "remote {}:{} -> {}:{}",
                tunnel.bind_host, tunnel.bind_port, tunnel.target_host, tunnel.target_port
            );
        }
        let metrics = Arc::new(TunnelMetrics::default());
        {
            let mut forwards = remote_forwards.lock().map_err(|error| error.to_string())?;
            let target = TunnelForwardTarget {
                spec: tunnel.clone(),
                metrics: Arc::clone(&metrics),
            };
            forwards.insert(
                remote_forward_key(&tunnel.bind_host, tunnel.bind_port),
                target.clone(),
            );
            forwards.insert(remote_forward_port_key(tunnel.bind_port), target);
        }
        let closed = Arc::new(AtomicBool::new(false));
        {
            let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
            tunnels.insert(
                tunnel.id.clone(),
                TunnelRuntime {
                    session_id: request.session_id.clone(),
                    spec: tunnel.clone(),
                    metrics,
                    closed,
                },
            );
        }
        persist_tunnel_to_profile_and_log(state, &request.session_id, &tunnel, None)?;
        return Ok(tunnel);
    }

    let listener = TcpListener::bind((request.bind_host.clone(), request.bind_port))
        .await
        .map_err(|error| {
            format!(
                "SSH tunnel bind failed {}:{}: {error}",
                request.bind_host, request.bind_port
            )
        })?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("SSH tunnel local addr failed: {error}"))?;
    if request.bind_port == 0 {
        tunnel.bind_port = local_addr.port();
        if request.label.is_none() {
            tunnel.label = tunnel_label(
                tunnel.mode,
                &tunnel.bind_host,
                tunnel.bind_port,
                &tunnel.target_host,
                tunnel.target_port,
            );
        }
    }
    let closed = Arc::new(AtomicBool::new(false));
    let metrics = Arc::new(TunnelMetrics::default());
    {
        let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels.insert(
            tunnel.id.clone(),
            TunnelRuntime {
                session_id: request.session_id.clone(),
                spec: tunnel.clone(),
                metrics: Arc::clone(&metrics),
                closed: Arc::clone(&closed),
            },
        );
    }

    let session_id = request.session_id.clone();
    let store = Arc::clone(&state.store);
    let store_path = state.store_path.clone();
    let tunnel_for_task = tunnel.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if closed.load(Ordering::SeqCst) {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(500), listener.accept()).await {
                Ok(Ok((stream, peer))) => {
                    let handle = handle.clone();
                    let spec = tunnel_for_task.clone();
                    let metrics = Arc::clone(&metrics);
                    let store = Arc::clone(&store);
                    let store_path = store_path.clone();
                    let session_id = session_id.clone();
                    tauri::async_runtime::spawn(async move {
                        metrics.connection_opened();
                        let result = if spec.mode == TunnelMode::Dynamic {
                            handle_dynamic_tunnel_client(handle, stream, peer, Arc::clone(&metrics))
                                .await
                        } else {
                            handle_local_tunnel_client(
                                handle,
                                spec,
                                stream,
                                peer,
                                Arc::clone(&metrics),
                            )
                            .await
                        };
                        if let Err(error) = result {
                            metrics.record_error(&error);
                            if let Ok(mut store) = store.lock() {
                                store.record_system_event(
                                    &session_id,
                                    format!("PortMate: SSH tunnel client failed: {error}"),
                                );
                                if let Err(error) = save_store(&store_path, &store) {
                                    eprintln!(
                                        "PortMate: failed to persist tunnel client error: {error}"
                                    );
                                }
                            }
                        }
                        metrics.connection_closed();
                    });
                }
                Ok(Err(error)) => {
                    if let Ok(mut store) = store.lock() {
                        store.record_system_event(
                            &session_id,
                            format!("PortMate: SSH tunnel accept failed: {error}"),
                        );
                        let _ = save_store(&store_path, &store);
                    }
                    break;
                }
                Err(_) => {}
            }
        }
    });

    persist_tunnel_to_profile_and_log(state, &request.session_id, &tunnel, Some(local_addr))?;
    Ok(tunnel)
}

fn normalize_tunnel_request(
    mut request: CreateTunnelRequest,
) -> Result<CreateTunnelRequest, String> {
    request.session_id = request.session_id.trim().to_string();
    request.bind_host = request.bind_host.trim().to_string();
    request.target_host = request.target_host.trim().to_string();
    request.label = request
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned);

    if request.session_id.is_empty() {
        return Err("tunnel requires a session id".to_string());
    }
    if request.mode != TunnelMode::Remote && request.bind_host.is_empty() {
        return Err("local and dynamic tunnels require a bind host".to_string());
    }
    if request.mode != TunnelMode::Dynamic
        && (request.target_host.is_empty() || request.target_port == 0)
    {
        return Err("local and remote tunnels require a target host and port".to_string());
    }
    if request.mode == TunnelMode::Dynamic {
        request.target_host.clear();
        request.target_port = 0;
    }
    Ok(request)
}

fn list_tunnels_inner(
    state: &AppState,
    session_id: Option<&str>,
) -> Result<Vec<TunnelStatus>, String> {
    let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
    let mut statuses = tunnels
        .values()
        .filter(|runtime| {
            !runtime.closed.load(Ordering::SeqCst)
                && match session_id {
                    Some(expected) => runtime.session_id == expected,
                    None => true,
                }
        })
        .map(tunnel_status_from_runtime)
        .collect::<Vec<_>>();
    statuses.sort_by(|left, right| {
        left.spec
            .label
            .cmp(&right.spec.label)
            .then_with(|| left.spec.id.cmp(&right.spec.id))
    });
    Ok(statuses)
}

async fn stop_tunnel_inner(state: &AppState, tunnel_id: &str) -> Result<TunnelStatus, String> {
    let runtime = {
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels
            .get(tunnel_id)
            .cloned()
            .ok_or_else(|| format!("tunnel not found: {tunnel_id}"))?
    };

    let mut stopped = runtime.spec.clone();
    stopped.enabled = false;

    if runtime.spec.mode == TunnelMode::Remote {
        let remote_forward = {
            let connections = state.ssh.lock().map_err(|error| error.to_string())?;
            connections
                .get(&runtime.session_id)
                .map(|ssh| (Arc::clone(&ssh.handle), Arc::clone(&ssh.remote_forwards)))
        };
        if let Some((handle, remote_forwards)) = remote_forward {
            {
                let handle = handle.lock().await;
                handle
                    .cancel_tcpip_forward(
                        runtime.spec.bind_host.clone(),
                        u32::from(runtime.spec.bind_port),
                    )
                    .await
                    .map_err(|error| {
                        format!(
                            "remote SSH tunnel cancel failed {}:{}: {error}",
                            runtime.spec.bind_host, runtime.spec.bind_port
                        )
                    })?;
            }
            let mut forwards = remote_forwards.lock().map_err(|error| error.to_string())?;
            forwards.remove(&remote_forward_key(
                &runtime.spec.bind_host,
                runtime.spec.bind_port,
            ));
            forwards.remove(&remote_forward_port_key(runtime.spec.bind_port));
        }
    }

    {
        let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels
            .remove(tunnel_id)
            .ok_or_else(|| format!("tunnel not found: {tunnel_id}"))?;
    }
    runtime.closed.store(true, Ordering::SeqCst);

    persist_stopped_tunnel_to_profile_and_log(state, &runtime.session_id, &stopped)?;
    Ok(runtime.metrics.snapshot(stopped))
}

fn tunnel_status_from_runtime(runtime: &TunnelRuntime) -> TunnelStatus {
    runtime.metrics.snapshot(runtime.spec.clone())
}

fn tunnel_label(
    mode: TunnelMode,
    bind_host: &str,
    bind_port: u16,
    target_host: &str,
    target_port: u16,
) -> String {
    match mode {
        TunnelMode::Local => {
            format!("{bind_host}:{bind_port} -> {target_host}:{target_port}")
        }
        TunnelMode::Dynamic => format!("SOCKS5 {bind_host}:{bind_port}"),
        TunnelMode::Remote => {
            format!("remote {bind_host}:{bind_port} -> {target_host}:{target_port}")
        }
    }
}

fn persist_tunnel_to_profile_and_log(
    state: &AppState,
    session_id: &str,
    tunnel: &TunnelSpec,
    local_addr: Option<std::net::SocketAddr>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if let Some(mut profile) = store.profile(session_id) {
        match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
                ssh.tunnels.retain(|item| item.id != tunnel.id);
                ssh.tunnels.push(tunnel.clone());
                let _ = store.upsert_profile(profile);
            }
            _ => {}
        }
    }
    let listen = local_addr
        .map(|addr| addr.to_string())
        .unwrap_or_else(|| format!("{}:{}", tunnel.bind_host, tunnel.bind_port));
    store.record_system_event(
        session_id,
        format!(
            "PortMate: SSH {:?} tunnel listening on {} -> {}:{}",
            tunnel.mode, listen, tunnel.target_host, tunnel.target_port
        ),
    );
    save_store(&state.store_path, &store)
}

fn persist_stopped_tunnel_to_profile_and_log(
    state: &AppState,
    session_id: &str,
    stopped: &TunnelSpec,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    mark_tunnel_stopped_in_store(&mut store, session_id, stopped);
    store.record_system_event(
        session_id,
        format!(
            "PortMate: SSH {:?} tunnel stopped on {}:{}",
            stopped.mode, stopped.bind_host, stopped.bind_port
        ),
    );
    save_store(&state.store_path, &store)
}

fn mark_tunnel_stopped_in_store(store: &mut SessionStore, session_id: &str, stopped: &TunnelSpec) {
    if let Some(mut profile) = store.profile(session_id) {
        match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
                if let Some(saved) = ssh.tunnels.iter_mut().find(|item| item.id == stopped.id) {
                    saved.enabled = false;
                } else {
                    ssh.tunnels.push(stopped.clone());
                }
                let _ = store.upsert_profile(profile);
            }
            _ => {}
        }
    }
}

fn remote_forward_key(host: &str, port: u16) -> String {
    format!("{}:{}", host, port)
}

fn remote_forward_port_key(port: u16) -> String {
    format!("*:{}", port)
}

async fn handle_local_tunnel_client(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    tunnel: TunnelSpec,
    local_stream: TcpStream,
    peer: std::net::SocketAddr,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_direct_tcpip(
                tunnel.target_host.clone(),
                u32::from(tunnel.target_port),
                peer.ip().to_string(),
                u32::from(peer.port()),
            )
            .await
            .map_err(|error| format!("direct-tcpip open failed: {error}"))?
    };
    let (mut remote_read, remote_write) = channel.split();
    let (mut local_read, mut local_write) = local_stream.into_split();

    let upload_metrics = Arc::clone(&metrics);
    let local_to_remote = async move {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            let size = local_read
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if size == 0 {
                remote_write
                    .eof()
                    .await
                    .map_err(|error| error.to_string())?;
                break;
            }
            upload_metrics.add_tcp_to_ssh_bytes(size);
            remote_write
                .data(&buffer[..size])
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok::<(), String>(())
    };

    let download_metrics = Arc::clone(&metrics);
    let remote_to_local = async move {
        while let Some(message) = remote_read.wait().await {
            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    download_metrics.add_ssh_to_tcp_bytes(data.len());
                    local_write
                        .write_all(&data)
                        .await
                        .map_err(|error| error.to_string())?;
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok::<(), String>(())
    };

    tokio::try_join!(local_to_remote, remote_to_local)
        .map(|_| ())
        .map_err(|error| format!("local tunnel pipe failed ({}): {error}", tunnel.label))
}

async fn handle_remote_tunnel_client(
    channel: Channel<client::Msg>,
    tunnel: TunnelSpec,
    originator_address: String,
    originator_port: u16,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let local_stream = TcpStream::connect((tunnel.target_host.clone(), tunnel.target_port))
        .await
        .map_err(|error| {
            format!(
                "remote tunnel target connect failed {}:{} for {}:{}: {error}",
                tunnel.target_host, tunnel.target_port, originator_address, originator_port
            )
        })?;
    pipe_ssh_channel_to_tcp(channel, local_stream, tunnel, metrics).await
}

async fn handle_dynamic_tunnel_client(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    mut local_stream: TcpStream,
    peer: std::net::SocketAddr,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let (target_host, target_port) = read_socks5_connect_request(&mut local_stream).await?;

    let channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_direct_tcpip(
                target_host.clone(),
                u32::from(target_port),
                peer.ip().to_string(),
                u32::from(peer.port()),
            )
            .await
    };
    let channel = match channel {
        Ok(channel) => channel,
        Err(error) => {
            let _ = local_stream.write_all(&socks5_reply(5)).await;
            return Err(format!("dynamic direct-tcpip open failed: {error}"));
        }
    };

    local_stream
        .write_all(&socks5_reply(0))
        .await
        .map_err(|error| format!("SOCKS5 success response failed: {error}"))?;

    let spec = TunnelSpec {
        id: "dynamic-client".to_string(),
        label: format!("SOCKS5 -> {target_host}:{target_port}"),
        mode: TunnelMode::Dynamic,
        bind_host: String::new(),
        bind_port: 0,
        target_host,
        target_port,
        enabled: true,
    };
    pipe_ssh_channel_to_tcp(channel, local_stream, spec, metrics).await
}

async fn read_socks5_connect_request(
    local_stream: &mut TcpStream,
) -> Result<(String, u16), String> {
    let mut header = [0_u8; 2];
    local_stream
        .read_exact(&mut header)
        .await
        .map_err(|error| format!("SOCKS5 handshake read failed: {error}"))?;
    if header[0] != 5 {
        let _ = local_stream.write_all(&[5, 0xff]).await;
        return Err("only SOCKS5 is supported for dynamic tunnel".to_string());
    }
    let mut methods = vec![0_u8; header[1] as usize];
    local_stream
        .read_exact(&mut methods)
        .await
        .map_err(|error| format!("SOCKS5 methods read failed: {error}"))?;
    if !methods.contains(&0) {
        local_stream
            .write_all(&[5, 0xff])
            .await
            .map_err(|error| format!("SOCKS5 method rejection failed: {error}"))?;
        return Err("SOCKS5 client did not offer no-authentication method".to_string());
    }
    local_stream
        .write_all(&[5, 0])
        .await
        .map_err(|error| format!("SOCKS5 method response failed: {error}"))?;

    let mut request = [0_u8; 4];
    local_stream
        .read_exact(&mut request)
        .await
        .map_err(|error| format!("SOCKS5 request read failed: {error}"))?;
    if request[0] != 5 || request[2] != 0 {
        let _ = local_stream.write_all(&socks5_reply(1)).await;
        return Err("invalid SOCKS5 CONNECT request header".to_string());
    }
    if request[1] != 1 {
        local_stream.write_all(&socks5_reply(7)).await.ok();
        return Err("only SOCKS5 CONNECT is supported".to_string());
    }

    let target_host = match request[3] {
        1 => {
            let mut addr = [0_u8; 4];
            local_stream
                .read_exact(&mut addr)
                .await
                .map_err(|error| format!("SOCKS5 IPv4 read failed: {error}"))?;
            std::net::Ipv4Addr::from(addr).to_string()
        }
        3 => {
            let mut len = [0_u8; 1];
            local_stream
                .read_exact(&mut len)
                .await
                .map_err(|error| format!("SOCKS5 domain length read failed: {error}"))?;
            if len[0] == 0 {
                let _ = local_stream.write_all(&socks5_reply(8)).await;
                return Err("SOCKS5 domain name cannot be empty".to_string());
            }
            let mut name = vec![0_u8; len[0] as usize];
            local_stream
                .read_exact(&mut name)
                .await
                .map_err(|error| format!("SOCKS5 domain read failed: {error}"))?;
            match String::from_utf8(name) {
                Ok(name) => name,
                Err(_) => {
                    let _ = local_stream.write_all(&socks5_reply(8)).await;
                    return Err("SOCKS5 domain name is not valid UTF-8".to_string());
                }
            }
        }
        4 => {
            let mut addr = [0_u8; 16];
            local_stream
                .read_exact(&mut addr)
                .await
                .map_err(|error| format!("SOCKS5 IPv6 read failed: {error}"))?;
            std::net::Ipv6Addr::from(addr).to_string()
        }
        other => {
            let _ = local_stream.write_all(&socks5_reply(8)).await;
            return Err(format!("unsupported SOCKS5 address type: {other}"));
        }
    };
    let mut port_bytes = [0_u8; 2];
    local_stream
        .read_exact(&mut port_bytes)
        .await
        .map_err(|error| format!("SOCKS5 port read failed: {error}"))?;
    let target_port = u16::from_be_bytes(port_bytes);
    if target_port == 0 {
        let _ = local_stream.write_all(&socks5_reply(1)).await;
        return Err("SOCKS5 target port cannot be zero".to_string());
    }
    Ok((target_host, target_port))
}

fn socks5_reply(code: u8) -> [u8; 10] {
    [5, code, 0, 1, 0, 0, 0, 0, 0, 0]
}

async fn pipe_ssh_channel_to_tcp(
    channel: Channel<client::Msg>,
    local_stream: TcpStream,
    tunnel: TunnelSpec,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let (mut remote_read, remote_write) = channel.split();
    let (mut local_read, mut local_write) = local_stream.into_split();

    let upload_metrics = Arc::clone(&metrics);
    let local_to_remote = async move {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            let size = local_read
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if size == 0 {
                remote_write
                    .eof()
                    .await
                    .map_err(|error| error.to_string())?;
                break;
            }
            upload_metrics.add_tcp_to_ssh_bytes(size);
            remote_write
                .data(&buffer[..size])
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok::<(), String>(())
    };

    let download_metrics = Arc::clone(&metrics);
    let remote_to_local = async move {
        while let Some(message) = remote_read.wait().await {
            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    download_metrics.add_ssh_to_tcp_bytes(data.len());
                    local_write
                        .write_all(&data)
                        .await
                        .map_err(|error| error.to_string())?;
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok::<(), String>(())
    };

    tokio::try_join!(local_to_remote, remote_to_local)
        .map(|_| ())
        .map_err(|error| format!("tunnel pipe failed ({}): {error}", tunnel.label))
}

async fn open_ssh_session(
    state: &AppState,
    profile: SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<SessionSummary, String> {
    if let Some(existing) = {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        existing.closed.store(true, Ordering::SeqCst);
        let handle = existing.handle.lock().await;
        let _ = handle
            .disconnect(Disconnect::ByApplication, "PortMate reconnect", "en")
            .await;
        for jump_handle in existing.jump_handles {
            let handle = jump_handle.lock().await;
            let _ = handle
                .disconnect(Disconnect::ByApplication, "PortMate reconnect jump", "en")
                .await;
        }
    }

    let established = establish_ssh_runtime(state, &profile, password, passphrase).await?;
    let one_time_cleanup_error = take_one_time_host_keys(state, &profile.id).err();
    let EstablishedSshRuntime {
        runtime_id,
        runtime,
        tap,
        read_half,
        auth_method,
        closed,
    } = established;
    {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.insert(profile.id.clone(), runtime);
    }

    tauri::async_runtime::spawn(read_ssh_channel(SshReadTask {
        state: state.clone(),
        profile: profile.clone(),
        runtime_id,
        tap,
        read_half,
        closed,
    }));

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let _ = store.record_auth_success(&profile.id, auth_method);
    store.record_system_event(
        &profile.id,
        format!("PortMate: SSH authentication succeeded via {auth_method:?}"),
    );
    if let Some(error) = one_time_cleanup_error {
        store.record_system_event(
            &profile.id,
            format!("PortMate: failed to consume one-time host key trust: {error}"),
        );
    }
    let summary = store.open_session(&profile.id)?;
    save_store(&state.store_path, &store)?;
    Ok(summary)
}

async fn establish_ssh_runtime(
    state: &AppState,
    profile: &SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<EstablishedSshRuntime, String> {
    let ssh = match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.clone(),
        _ => return Err("profile is not SSH-backed".to_string()),
    };

    let host = ssh.endpoint.host.trim().to_string();
    if host.is_empty() {
        return Err("SSH 主机不能为空".to_string());
    }
    let username = ssh.username.trim().to_string();
    if username.is_empty() {
        return Err("SSH 用户名不能为空；PortMate 不读取系统 ssh_config 的默认用户名".to_string());
    }

    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());
    let one_time_host_keys = one_time_host_keys_snapshot(state, &profile.id)?;
    host_keys.keys.extend(one_time_host_keys.clone());

    let observed_key = Arc::new(Mutex::new(None));
    let host_key_error = Arc::new(Mutex::new(None));
    let remote_forwards = Arc::new(Mutex::new(HashMap::new()));

    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    });

    let (mut session, jump_handles) = connect_ssh_target(SshConnectRequest {
        config: Arc::clone(&config),
        store: Arc::clone(&state.store),
        store_path: state.store_path.clone(),
        profile,
        ssh: &ssh,
        host_keys,
        one_time_host_keys: one_time_host_keys.clone(),
        observed_key: Arc::clone(&observed_key),
        host_key_error: Arc::clone(&host_key_error),
        remote_forwards: Arc::clone(&remote_forwards),
        password: password.as_deref(),
        passphrase: passphrase.as_deref(),
    })
    .await?;

    let auth_method = authenticate_ssh(
        &mut session,
        ssh.clone(),
        username.clone(),
        password,
        passphrase,
    )
    .await?;

    persist_observed_host_key(
        &state.store,
        &profile.id,
        &observed_key,
        &one_time_host_keys,
    )?;
    persist_store_arc(&state.store_path, &state.store)?;

    let channel = session
        .channel_open_session()
        .await
        .map_err(|error| format!("SSH 打开 session channel 失败: {error}"))?;
    channel
        .request_pty(
            true,
            &profile.terminal.term,
            u32::from(profile.terminal.cols),
            u32::from(profile.terminal.rows),
            0,
            0,
            &[],
        )
        .await
        .map_err(|error| format!("SSH 请求 PTY 失败: {error}"))?;
    apply_ssh_terminal_color_env(&channel).await;
    if ssh.agent_policy.forwarding {
        let _ = channel.agent_forward(false).await;
    }
    channel
        .request_shell(true)
        .await
        .map_err(|error| format!("SSH 请求 shell 失败: {error}"))?;

    let runtime_id = Uuid::new_v4().to_string();
    let (read_half, write_half) = channel.split();
    let writer = Arc::new(tokio::sync::Mutex::new(write_half));
    let (tap, _) = broadcast::channel(1024);
    let closed = Arc::new(AtomicBool::new(false));

    if matches!(profile.connection, ConnectionConfig::Tmux(_)) {
        let writer = writer.lock().await;
        writer
            .data(&b"tmux new-session -A -s portmate\r"[..])
            .await
            .map_err(|error| format!("Tmux attach 命令发送失败: {error}"))?;
    }

    Ok(EstablishedSshRuntime {
        runtime_id: runtime_id.clone(),
        runtime: SshRuntime {
            runtime_id: runtime_id.clone(),
            handle: Arc::new(tokio::sync::Mutex::new(session)),
            jump_handles,
            writer,
            tap: tap.clone(),
            remote_forwards,
            closed: Arc::clone(&closed),
        },
        tap,
        read_half,
        auth_method,
        closed,
    })
}

async fn connect_ssh_target(
    request: SshConnectRequest<'_>,
) -> Result<
    (
        client::Handle<PortMateSshHandler>,
        Vec<Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>>,
    ),
    String,
> {
    let SshConnectRequest {
        config,
        store,
        store_path,
        profile,
        ssh,
        host_keys,
        one_time_host_keys,
        observed_key,
        host_key_error,
        remote_forwards,
        password,
        passphrase,
    } = request;
    let one_time_host_key_ids = one_time_host_keys
        .iter()
        .map(|key| key.id.clone())
        .collect::<Vec<_>>();

    let target_host = ssh.endpoint.host.trim().to_string();
    if target_host.is_empty() {
        return Err("SSH 主机不能为空".to_string());
    }
    if ssh.endpoint.port == 0 {
        return Err("SSH 端口必须在 1-65535 之间".to_string());
    }

    let target_handler = ssh_handler_for_endpoint(SshHandlerParams {
        profile_id: profile.id.clone(),
        host: target_host.clone(),
        port: ssh.endpoint.port,
        alias: ssh
            .host_key_policy
            .alias
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some(profile.id.clone())),
        policy: ssh.host_key_policy.clone(),
        host_keys: host_keys.clone(),
        one_time_host_key_ids: one_time_host_key_ids.clone(),
        observed_key: Arc::clone(&observed_key),
        host_key_error: Arc::clone(&host_key_error),
        remote_forwards: Arc::clone(&remote_forwards),
    });

    if ssh.jumps.is_empty() {
        let session = tokio::time::timeout(
            Duration::from_secs(20),
            client::connect(
                config,
                (target_host.clone(), ssh.endpoint.port),
                target_handler,
            ),
        )
        .await
        .map_err(|_| format!("SSH 连接超时: {target_host}:{}", ssh.endpoint.port))?
        .map_err(|error| {
            host_key_error
                .lock()
                .ok()
                .and_then(|reason| reason.clone())
                .unwrap_or_else(|| format!("SSH 握手失败: {error}"))
        })?;
        return Ok((session, Vec::new()));
    }

    let mut jump_sessions: Vec<client::Handle<PortMateSshHandler>> = Vec::new();
    for (index, jump) in ssh.jumps.iter().enumerate() {
        let (jump_host, jump_port, jump_username) = jump_endpoint_details(jump, index)?;
        let jump_policy = jump_host_key_policy(ssh, jump);
        let observed_jump_key = Arc::new(Mutex::new(None));
        let jump_key_error = Arc::new(Mutex::new(None));
        let jump_ssh = jump_ssh_connection(ssh, jump, jump_policy.clone());
        let jump_handler = ssh_handler_for_endpoint(SshHandlerParams {
            profile_id: profile.id.clone(),
            host: jump_host.clone(),
            port: jump_port,
            alias: jump_policy.alias.clone(),
            policy: jump_ssh.host_key_policy.clone(),
            host_keys: host_keys.clone(),
            one_time_host_key_ids: one_time_host_key_ids.clone(),
            observed_key: Arc::clone(&observed_jump_key),
            host_key_error: Arc::clone(&jump_key_error),
            remote_forwards: Arc::new(Mutex::new(HashMap::new())),
        });
        let mut jump_session = if let Some(previous_jump) = jump_sessions.last_mut() {
            let jump_channel = match previous_jump
                .channel_open_direct_tcpip(jump_host.clone(), u32::from(jump_port), "127.0.0.1", 0)
                .await
            {
                Ok(channel) => channel,
                Err(error) => {
                    disconnect_jump_sessions(jump_sessions, "PortMate jump chain channel failed")
                        .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳打开 direct-tcpip 到 {jump_host}:{jump_port} 失败: {error}",
                        index + 1
                    ));
                }
            };
            match tokio::time::timeout(
                Duration::from_secs(20),
                client::connect_stream(config.clone(), jump_channel.into_stream(), jump_handler),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Err(_) => {
                    disconnect_jump_sessions(jump_sessions, "PortMate jump chain connect timeout")
                        .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳连接超时: {jump_host}:{jump_port}",
                        index + 1
                    ));
                }
                Ok(Err(error)) => {
                    let message = jump_key_error
                        .lock()
                        .ok()
                        .and_then(|reason| reason.clone())
                        .unwrap_or_else(|| {
                            format!("Jump Host 第 {} 跳 SSH 握手失败: {error}", index + 1)
                        });
                    disconnect_jump_sessions(jump_sessions, "PortMate jump chain handshake failed")
                        .await;
                    return Err(message);
                }
            }
        } else {
            tokio::time::timeout(
                Duration::from_secs(20),
                client::connect(config.clone(), (jump_host.clone(), jump_port), jump_handler),
            )
            .await
            .map_err(|_| format!("Jump Host 连接超时: {jump_host}:{jump_port}"))?
            .map_err(|error| {
                jump_key_error
                    .lock()
                    .ok()
                    .and_then(|reason| reason.clone())
                    .unwrap_or_else(|| format!("Jump Host SSH 握手失败: {error}"))
            })?
        };

        if let Err(error) = authenticate_ssh(
            &mut jump_session,
            jump_ssh,
            jump_username,
            jump_runtime_credential(password, jump.password_secret_ref.as_deref()),
            jump_runtime_credential(passphrase, jump.passphrase_secret_ref.as_deref()),
        )
        .await
        {
            disconnect_jump_sessions(jump_sessions, "PortMate jump authentication failed").await;
            let _ = jump_session
                .disconnect(
                    Disconnect::ByApplication,
                    "PortMate jump authentication failed",
                    "en",
                )
                .await;
            return Err(format!("Jump Host 第 {} 跳认证失败: {error}", index + 1));
        }
        if let Err(error) = persist_observed_host_key_with_policy(
            &store,
            &store_path,
            &profile.id,
            &jump_policy,
            &observed_jump_key,
            &one_time_host_keys,
            &format!("Jump Host #{}", index + 1),
        ) {
            disconnect_jump_sessions(jump_sessions, "PortMate jump host key rejected").await;
            let _ = jump_session
                .disconnect(
                    Disconnect::ByApplication,
                    "PortMate jump host key rejected",
                    "en",
                )
                .await;
            return Err(error);
        }
        jump_sessions.push(jump_session);
    }

    let jump_channel = match jump_sessions
        .last_mut()
        .expect("non-empty jumps should create jump sessions")
        .channel_open_direct_tcpip(
            target_host.clone(),
            u32::from(ssh.endpoint.port),
            "127.0.0.1",
            0,
        )
        .await
    {
        Ok(channel) => channel,
        Err(error) => {
            disconnect_jump_sessions(jump_sessions, "PortMate jump target channel failed").await;
            return Err(format!(
                "Jump Host 打开 direct-tcpip 到 {target_host}:{} 失败: {error}",
                ssh.endpoint.port
            ));
        }
    };
    let target_session = match tokio::time::timeout(
        Duration::from_secs(20),
        client::connect_stream(config, jump_channel.into_stream(), target_handler),
    )
    .await
    {
        Ok(Ok(session)) => session,
        Err(_) => {
            disconnect_jump_sessions(jump_sessions, "PortMate jump target connect timeout").await;
            return Err(format!(
                "SSH 经 Jump Host 连接超时: {target_host}:{}",
                ssh.endpoint.port
            ));
        }
        Ok(Err(error)) => {
            disconnect_jump_sessions(jump_sessions, "PortMate jump target handshake failed").await;
            return Err(host_key_error
                .lock()
                .ok()
                .and_then(|reason| reason.clone())
                .unwrap_or_else(|| format!("SSH 经 Jump Host 握手失败: {error}")));
        }
    };

    Ok((
        target_session,
        jump_sessions
            .into_iter()
            .map(|session| Arc::new(tokio::sync::Mutex::new(session)))
            .collect(),
    ))
}

fn jump_endpoint_details(
    jump: &portmate_core::JumpHop,
    index: usize,
) -> Result<(String, u16, String), String> {
    let label = format!("Jump Host 第 {} 跳", index + 1);
    let host = jump.host.trim().to_string();
    if host.is_empty() {
        return Err(format!("{label} 主机不能为空"));
    }
    if jump.port == 0 {
        return Err(format!("{label} 端口必须在 1-65535 之间"));
    }
    let username = jump.username.trim().to_string();
    if username.is_empty() {
        return Err(format!("{label} 用户名不能为空"));
    }
    Ok((host, jump.port, username))
}

async fn disconnect_jump_sessions(
    jump_sessions: Vec<client::Handle<PortMateSshHandler>>,
    reason: &str,
) {
    for session in jump_sessions {
        let _ = session
            .disconnect(Disconnect::ByApplication, reason, "en")
            .await;
    }
}

fn ssh_handler_for_endpoint(params: SshHandlerParams) -> PortMateSshHandler {
    PortMateSshHandler {
        profile_id: params.profile_id,
        host: params.host,
        port: params.port,
        alias: params.alias,
        policy: params.policy,
        host_keys: params.host_keys,
        one_time_host_key_ids: params.one_time_host_key_ids,
        observed_key: params.observed_key,
        host_key_error: params.host_key_error,
        remote_forwards: params.remote_forwards,
    }
}

fn trusted_host_key_allowed(
    policy: &portmate_core::HostKeyPolicy,
    matched_key_id: &str,
    one_time_host_key_ids: &[String],
) -> bool {
    policy.mode != HostKeyMode::AskEveryTime
        || one_time_host_key_ids
            .iter()
            .any(|key_id| key_id == matched_key_id)
}

fn jump_host_key_policy(
    ssh: &SshConnection,
    jump: &portmate_core::JumpHop,
) -> portmate_core::HostKeyPolicy {
    let default_alias = format!("jump:{}:{}", jump.host.trim(), jump.port);
    let mut policy = if let Some(custom) = jump.host_key_policy.clone() {
        custom
    } else {
        let mut inherited = ssh.host_key_policy.clone();
        inherited.alias = Some(default_alias.clone());
        inherited.trust_scope = HostKeyScope::Profile;
        inherited
    };
    policy.alias = policy
        .alias
        .as_deref()
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
        .map(str::to_string)
        .or(Some(default_alias));
    policy
}

fn jump_ssh_connection(
    ssh: &SshConnection,
    jump: &portmate_core::JumpHop,
    host_key_policy: portmate_core::HostKeyPolicy,
) -> SshConnection {
    let mut identity_refs = ssh.identity_refs.clone();
    if let Some(identity_ref) = jump
        .identity_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        identity_refs.retain(|identity| identity.id == identity_ref);
    }
    SshConnection {
        endpoint: portmate_core::HostEndpoint {
            host: jump.host.trim().to_string(),
            port: jump.port,
        },
        username: jump.username.trim().to_string(),
        reconnect: ssh.reconnect,
        password_secret_ref: jump
            .password_secret_ref
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| ssh.password_secret_ref.clone()),
        passphrase_secret_ref: jump
            .passphrase_secret_ref
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| ssh.passphrase_secret_ref.clone()),
        host_key_policy,
        trusted_host_keys: Vec::new(),
        identity_policy: ssh.identity_policy.clone(),
        identity_refs,
        agent_policy: ssh.agent_policy.clone(),
        jumps: Vec::new(),
        tunnels: Vec::new(),
    }
}

fn jump_runtime_credential(
    inherited: Option<&str>,
    jump_secret_ref: Option<&str>,
) -> Option<String> {
    if jump_secret_ref.is_some_and(|value| !value.trim().is_empty()) {
        None
    } else {
        inherited
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

fn open_shell_session(state: &AppState, profile: SessionProfile) -> Result<SessionSummary, String> {
    let shell = match &profile.connection {
        ConnectionConfig::Shell(shell) => shell.clone(),
        _ => return Err("profile is not shell-backed".to_string()),
    };
    let program = if shell.program.trim().is_empty() {
        default_shell_program()
    } else {
        shell.program.trim().to_string()
    };

    if let Some(existing) = {
        let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        existing.closed.store(true, Ordering::SeqCst);
        if let Ok(mut child) = existing.child.lock() {
            let _ = child.kill();
        }
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: profile.terminal.rows,
            cols: profile.terminal.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("Shell PTY 打开失败: {error}"))?;

    let mut command = CommandBuilder::new(&program);
    command.args(shell.args.iter());
    apply_shell_terminal_color_env(&mut command, profile.terminal.term.as_str());
    if let Some(cwd) = shell
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        command.cwd(cwd);
    }

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Shell 启动失败 {program}: {error}"))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("Shell PTY reader 创建失败: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("Shell PTY writer 创建失败: {error}"))?;

    let runtime_id = Uuid::new_v4().to_string();
    let closed = Arc::new(AtomicBool::new(false));
    let (tap, _) = broadcast::channel(1024);
    let child = Arc::new(Mutex::new(child));
    let master = Arc::new(Mutex::new(pair.master));
    let writer = Arc::new(Mutex::new(writer));
    {
        let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            ShellRuntime {
                runtime_id: runtime_id.clone(),
                master,
                writer,
                tap: tap.clone(),
                child: Arc::clone(&child),
                closed: Arc::clone(&closed),
            },
        );
    }

    if let Err(error) = std::thread::Builder::new()
        .name(format!("portmate-shell-{}", profile.id))
        .spawn(read_shell_pty(ShellReadTask {
            io: state.session_io(),
            session_id: profile.id.clone(),
            runtime_id,
            program: program.clone(),
            tap,
            closed,
            child,
            reader,
        }))
    {
        let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id);
        return Err(format!("Shell PTY 读取线程启动失败: {error}"));
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.record_system_event(&profile.id, format!("PortMate: shell started ({program})"));
    let summary = store.open_session(&profile.id)?;
    save_store(&state.store_path, &store)?;
    Ok(summary)
}

fn tcp_connection_details(profile: &SessionProfile) -> Result<(String, u16, &'static str), String> {
    let (host, port, label) = match &profile.connection {
        ConnectionConfig::Tcp(tcp) => (tcp.host.trim().to_string(), tcp.port, "TCP"),
        ConnectionConfig::Telnet(tcp) => (tcp.host.trim().to_string(), tcp.port, "Telnet"),
        _ => return Err("profile is not TCP/Telnet-backed".to_string()),
    };
    if host.is_empty() {
        return Err(format!("{label} 主机不能为空"));
    }
    if port == 0 {
        return Err(format!("{label} 端口不能为空"));
    }
    Ok((host, port, label))
}

fn tcp_reconnect_enabled(profile: &SessionProfile) -> bool {
    match &profile.connection {
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => tcp.reconnect,
        _ => false,
    }
}

async fn connect_tcp_socket(host: &str, port: u16, label: &str) -> Result<TcpStream, String> {
    let stream = tokio::time::timeout(Duration::from_secs(15), TcpStream::connect((host, port)))
        .await
        .map_err(|_| format!("{label} 连接超时: {host}:{port}"))?
        .map_err(|error| format!("{label} 连接失败: {host}:{port}: {error}"))?;
    stream
        .set_nodelay(true)
        .map_err(|error| format!("{label} 设置 TCP_NODELAY 失败: {error}"))?;
    Ok(stream)
}

async fn open_tcp_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    let (host, port, label) = tcp_connection_details(&profile)?;
    if let Some(existing) = {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        existing.closed.store(true, Ordering::SeqCst);
        let mut writer = existing.writer.lock().await;
        let _ = writer.shutdown().await;
    }

    let stream = connect_tcp_socket(&host, port, label).await?;

    let runtime_id = Uuid::new_v4().to_string();
    let (read_half, write_half) = stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(write_half));
    let (tap, _) = broadcast::channel(1024);
    let closed = Arc::new(AtomicBool::new(false));
    {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: runtime_id.clone(),
                writer: Arc::clone(&writer),
                tap: tap.clone(),
                closed: Arc::clone(&closed),
            },
        );
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.record_system_event(&profile.id, format!("PortMate: {label} socket connected"));
    let summary = store.open_session(&profile.id)?;
    save_store(&state.store_path, &store)?;
    drop(store);

    tauri::async_runtime::spawn(read_tcp_stream(TcpReadTask {
        state: state.clone(),
        profile,
        runtime_id,
        label: label.to_string(),
        tap,
        writer,
        read_half,
        closed,
    }));
    Ok(summary)
}

fn serial_connection_details(
    profile: &SessionProfile,
) -> Result<(portmate_core::SerialConnection, String), String> {
    let serial = match &profile.connection {
        ConnectionConfig::Serial(serial) => serial.clone(),
        _ => return Err("profile is not serial-backed".to_string()),
    };
    let port_name = serial.port.trim().to_string();
    if port_name.is_empty() {
        return Err("串口不能为空".to_string());
    }
    Ok((serial, port_name))
}

fn serial_reconnect_enabled(profile: &SessionProfile) -> bool {
    match &profile.connection {
        ConnectionConfig::Serial(serial) => serial.reconnect,
        _ => false,
    }
}

fn open_configured_serial_port(
    serial: &portmate_core::SerialConnection,
    port_name: &str,
) -> Result<SerialPortPair, String> {
    let mut port = serialport::new(port_name, serial.baud_rate)
        .data_bits(serial_data_bits(serial.data_bits))
        .stop_bits(serial_stop_bits(serial.stop_bits))
        .parity(serial_parity(&serial.parity))
        .flow_control(serial_flow_control(&serial.flow_control))
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|error| format!("串口打开失败 {port_name}: {error}"))?;
    if let Err(error) = port.write_data_terminal_ready(serial.dtr) {
        if serial.dtr {
            return Err(format!("设置 DTR 失败: {error}"));
        }
        eprintln!("PortMate: serial device does not support clearing DTR: {error}");
    }
    if let Err(error) = port.write_request_to_send(serial.rts) {
        if serial.rts {
            return Err(format!("设置 RTS 失败: {error}"));
        }
        eprintln!("PortMate: serial device does not support clearing RTS: {error}");
    }

    let reader = port
        .try_clone()
        .map_err(|error| format!("串口 reader 克隆失败: {error}"))?;
    Ok((port, reader))
}

fn open_serial_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    let (serial, port_name) = serial_connection_details(&profile)?;

    if let Some(existing) = {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        existing.closed.store(true, Ordering::SeqCst);
    }

    let (port, reader) = open_configured_serial_port(&serial, &port_name)?;
    let runtime_id = Uuid::new_v4().to_string();
    let closed = Arc::new(AtomicBool::new(false));
    let (tap, _) = broadcast::channel(1024);
    let writer = Arc::new(Mutex::new(port));
    {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            SerialRuntime {
                runtime_id: runtime_id.clone(),
                writer: Some(writer),
                tap: tap.clone(),
                closed: Arc::clone(&closed),
            },
        );
    }

    if let Err(error) = spawn_serial_reader(SerialReadTask {
        io: state.session_io(),
        profile: profile.clone(),
        runtime_id,
        port_name: port_name.clone(),
        tap,
        closed,
        reader,
    }) {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id);
        return Err(format!("串口读取线程启动失败: {error}"));
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.record_system_event(
        &profile.id,
        format!(
            "PortMate: serial port connected ({port_name}, {} baud)",
            serial.baud_rate
        ),
    );
    let summary = store.open_session(&profile.id)?;
    save_store(&state.store_path, &store)?;
    Ok(summary)
}

async fn authenticate_ssh(
    session: &mut client::Handle<PortMateSshHandler>,
    ssh: SshConnection,
    username: String,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<AuthMethod, String> {
    let auth_order = ordered_auth_methods(&ssh);
    let mut attempted = Vec::new();
    let mut key_errors = Vec::new();
    let saved_password = if password
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        read_optional_secret_ref(ssh.password_secret_ref.as_deref(), "SSH password")?
    } else {
        None
    };
    let saved_passphrase = if passphrase
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        read_optional_secret_ref(
            ssh.passphrase_secret_ref.as_deref(),
            "SSH private-key passphrase",
        )?
    } else {
        None
    };
    let effective_password = password
        .filter(|value| !value.is_empty())
        .or(saved_password);
    let effective_passphrase = passphrase
        .filter(|value| !value.is_empty())
        .or(saved_passphrase);
    let mut agent_attempted = false;

    for method in auth_order {
        match method {
            AuthMethod::PublicKey => {
                if ssh.agent_policy.enabled
                    && !agent_attempted
                    && !ssh.identity_policy.identities_only
                    && ssh.agent_policy.offer_mode
                        == portmate_core::AgentOfferMode::BeforeProfileKeys
                {
                    attempted.push("agent(before-profile-keys)");
                    agent_attempted = true;
                    match authenticate_with_agent(
                        session,
                        username.clone(),
                        ssh.identity_policy.identities_only,
                        ssh.agent_policy.offer_mode,
                        ssh.identity_refs.clone(),
                    )
                    .await
                    {
                        Ok(true) => return Ok(AuthMethod::PublicKey),
                        Ok(false) => {}
                        Err(error) => key_errors.push(error),
                    }
                }

                let identities = ssh
                    .identity_refs
                    .iter()
                    .filter(|identity| {
                        matches!(
                            identity.source,
                            IdentitySource::SystemFile | IdentitySource::ProfileVault
                        )
                    })
                    .collect::<Vec<_>>();
                if !identities.is_empty() {
                    attempted.push("publickey");
                    let rsa_hash = session
                        .best_supported_rsa_hash()
                        .await
                        .map_err(|error| {
                            format!("SSH publickey 认证准备失败，无法查询 RSA 签名算法: {error}")
                        })?
                        .flatten();
                    for identity in identities {
                        let label = identity.label.clone();
                        let key = match load_identity_private_key(
                            identity,
                            effective_passphrase.as_deref(),
                        ) {
                            Ok(Some(key)) => key,
                            Ok(None) => continue,
                            Err(error) => {
                                key_errors.push(format!("{label}: {error}"));
                                continue;
                            }
                        };
                        let result = match session
                            .authenticate_publickey(
                                username.clone(),
                                PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
                            )
                            .await
                        {
                            Ok(result) => result,
                            Err(error) => {
                                key_errors.push(format!("{label}: 认证请求失败: {error}"));
                                break;
                            }
                        };
                        if result.success() {
                            return Ok(AuthMethod::PublicKey);
                        }
                        key_errors.push(format!("{label}: 被服务器拒绝"));
                    }
                }

                if ssh.agent_policy.enabled
                    && !agent_attempted
                    && (ssh.agent_policy.offer_mode
                        == portmate_core::AgentOfferMode::AfterProfileKeys
                        || ssh
                            .identity_refs
                            .iter()
                            .any(|identity| identity.source == IdentitySource::Agent))
                    && (!ssh.identity_policy.identities_only
                        || ssh
                            .identity_refs
                            .iter()
                            .any(|identity| identity.source == IdentitySource::Agent))
                {
                    attempted.push("agent(after-profile-keys)");
                    agent_attempted = true;
                    match authenticate_with_agent(
                        session,
                        username.clone(),
                        ssh.identity_policy.identities_only,
                        ssh.agent_policy.offer_mode,
                        ssh.identity_refs.clone(),
                    )
                    .await
                    {
                        Ok(true) => return Ok(AuthMethod::PublicKey),
                        Ok(false) => {}
                        Err(error) => key_errors.push(error),
                    }
                }
            }
            AuthMethod::KeyboardInteractive => {
                let Some(password) = effective_password.clone() else {
                    continue;
                };
                attempted.push("keyboard-interactive");
                if authenticate_keyboard_interactive(session, username.clone(), password).await? {
                    return Ok(AuthMethod::KeyboardInteractive);
                }
            }
            AuthMethod::Password => {
                let Some(password) = effective_password.clone() else {
                    continue;
                };
                attempted.push("password");
                let result = session
                    .authenticate_password(username.clone(), password)
                    .await
                    .map_err(|error| format!("SSH password 认证失败: {error}"))?;
                if result.success() {
                    return Ok(AuthMethod::Password);
                }
            }
            AuthMethod::None => {
                attempted.push("none");
                let result = session
                    .authenticate_none(username.clone())
                    .await
                    .map_err(|error| format!("SSH none 认证失败: {error}"))?;
                if result.success() {
                    return Ok(AuthMethod::None);
                }
            }
            AuthMethod::GssapiWithMic => {
                attempted.push("gssapi-with-mic(unsupported)");
            }
        }
    }

    let mut message = if attempted.is_empty() {
        "SSH 认证失败：没有可尝试的认证方式。请配置 identityRefs 或在连接时输入密码。".to_string()
    } else {
        format!("SSH 认证失败，已尝试: {}", attempted.join(", "))
    };
    if !key_errors.is_empty() {
        message.push_str(&format!("；密钥详情: {}", key_errors.join(" | ")));
    }
    if ssh.agent_policy.enabled && ssh.identity_policy.identities_only {
        message.push_str("；当前按 IdentitiesOnly 处理，不会遍历系统 ssh-agent 的全部密钥");
    }
    Err(message)
}

async fn authenticate_with_agent(
    session: &mut client::Handle<PortMateSshHandler>,
    username: String,
    identities_only: bool,
    offer_mode: portmate_core::AgentOfferMode,
    identity_refs: Vec<IdentityRef>,
) -> Result<bool, String> {
    if offer_mode == portmate_core::AgentOfferMode::Disabled {
        return Ok(false);
    }

    let agent_refs = identity_refs
        .into_iter()
        .filter(|identity| identity.source == IdentitySource::Agent)
        .map(|identity| AgentIdentityFilter {
            label: identity.label,
            fingerprint_sha256: identity.fingerprint_sha256,
            path: identity.path,
        })
        .collect::<Vec<_>>();
    let allow_unfiltered_agent = !identities_only && agent_refs.is_empty();
    let identities = list_ssh_agent_identities_on_thread().await?;
    if identities.is_empty() {
        return Ok(false);
    }

    let rsa_hash = session
        .best_supported_rsa_hash()
        .await
        .map_err(|error| format!("SSH 查询 RSA 签名算法失败: {error}"))?
        .flatten();
    let mut tried = 0_usize;
    let max_agent_attempts = if allow_unfiltered_agent {
        6
    } else {
        usize::MAX
    };
    let mut signer = PortMateAgentSigner;

    for identity in identities {
        if !allow_unfiltered_agent && !agent_identity_matches(&identity, &agent_refs) {
            continue;
        }
        if tried >= max_agent_attempts {
            break;
        }
        tried += 1;
        let public_key = identity.public_key().into_owned();
        let result = session
            .authenticate_publickey_with(username.clone(), public_key, rsa_hash, &mut signer)
            .await
            .map_err(|error| format!("ssh-agent 认证失败: {error}"))?;
        if result.success() {
            return Ok(true);
        }
    }

    Ok(false)
}

async fn list_ssh_agent_identities_on_thread() -> Result<Vec<AgentIdentity>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("portmate-ssh-agent-list".to_string())
        .spawn(move || {
            let result = run_agent_runtime(async {
                let mut agent = connect_ssh_agent().await?;
                agent
                    .request_identities()
                    .await
                    .map_err(|error| format!("读取 ssh-agent identities 失败: {error}"))
            });
            let _ = sender.send(result);
        })
        .map_err(|error| format!("启动 ssh-agent 查询线程失败: {error}"))?;
    receiver
        .await
        .map_err(|error| format!("ssh-agent 查询线程未返回: {error}"))?
}

async fn sign_with_ssh_agent_on_thread(
    identity: AgentIdentity,
    hash_alg: Option<HashAlg>,
    data: Vec<u8>,
) -> Result<Vec<u8>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("portmate-ssh-agent-sign".to_string())
        .spawn(move || {
            let result = run_agent_runtime(async {
                let mut agent = connect_ssh_agent().await?;
                agent
                    .sign_request(&identity, hash_alg, data)
                    .await
                    .map_err(|error| format!("ssh-agent 签名失败: {error}"))
            });
            let _ = sender.send(result);
        })
        .map_err(|error| format!("启动 ssh-agent 签名线程失败: {error}"))?;
    receiver
        .await
        .map_err(|error| format!("ssh-agent 签名线程未返回: {error}"))?
}

fn run_agent_runtime<T>(
    future: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("创建 ssh-agent runtime 失败: {error}"))?
        .block_on(future)
}

fn agent_identity_matches(identity: &AgentIdentity, refs: &[AgentIdentityFilter]) -> bool {
    let comment = identity.comment();
    let public_key = identity.public_key();
    let fingerprint = compute_ssh_sha256_fingerprint(&public_key.public_key_base64()).ok();
    refs.iter().any(|identity_ref| {
        identity_ref
            .fingerprint_sha256
            .as_deref()
            .is_some_and(|expected| fingerprint.as_deref() == Some(expected))
            || (!identity_ref.label.trim().is_empty() && identity_ref.label == comment)
            || identity_ref
                .path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty() && path == comment)
    })
}

async fn connect_ssh_agent(
) -> Result<AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>, String> {
    #[cfg(unix)]
    {
        AgentClient::connect_env()
            .await
            .map(|client| client.dynamic())
            .map_err(|error| format!("无法连接 SSH_AUTH_SOCK: {error}"))
    }

    #[cfg(windows)]
    {
        if let Ok(path) = std::env::var("SSH_AUTH_SOCK") {
            if !path.trim().is_empty() {
                return AgentClient::connect_named_pipe(path)
                    .await
                    .map(|client| client.dynamic())
                    .map_err(|error| format!("无法连接 Windows OpenSSH agent: {error}"));
            }
        }
        AgentClient::connect_pageant()
            .await
            .map(|client| client.dynamic())
            .map_err(|error| format!("无法连接 Pageant: {error}"))
    }

    #[cfg(not(any(unix, windows)))]
    {
        Err("当前平台不支持 ssh-agent".to_string())
    }
}

async fn authenticate_keyboard_interactive(
    session: &mut client::Handle<PortMateSshHandler>,
    username: String,
    password: String,
) -> Result<bool, String> {
    let mut response = session
        .authenticate_keyboard_interactive_start(username, None::<String>)
        .await
        .map_err(|error| format!("SSH keyboard-interactive 启动失败: {error}"))?;

    for _ in 0..8 {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let responses = prompts
                    .iter()
                    .map(|prompt| {
                        if prompt.echo {
                            String::new()
                        } else {
                            password.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                response = session
                    .authenticate_keyboard_interactive_respond(responses)
                    .await
                    .map_err(|error| format!("SSH keyboard-interactive 响应失败: {error}"))?;
            }
        }
    }

    Err("SSH keyboard-interactive 认证轮次过多，已中止".to_string())
}

fn ordered_auth_methods(ssh: &SshConnection) -> Vec<AuthMethod> {
    let mut ordered = Vec::new();
    if let Some(last) = ssh.identity_policy.last_successful {
        ordered.push(last);
    }
    for method in &ssh.identity_policy.auth_order {
        if !ordered.contains(method) {
            ordered.push(*method);
        }
    }
    if ordered.is_empty() {
        ordered.extend([
            AuthMethod::PublicKey,
            AuthMethod::KeyboardInteractive,
            AuthMethod::Password,
        ]);
    }
    ordered
}

fn load_identity_private_key(
    identity: &IdentityRef,
    passphrase: Option<&str>,
) -> Result<Option<ssh_key::PrivateKey>, String> {
    match identity.source {
        IdentitySource::SystemFile => {
            let Some(path) = identity
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            else {
                return Ok(None);
            };
            load_secret_key(expand_identity_path(path), passphrase)
                .map(Some)
                .map_err(|error| format!("system-file {}: {error}", path))
        }
        IdentitySource::ProfileVault => {
            let Some(secret_ref) = identity
                .secret_ref
                .as_deref()
                .map(str::trim)
                .filter(|secret_ref| !secret_ref.is_empty())
            else {
                return Err("profile-vault identity 缺少 secretRef".to_string());
            };
            let private_key = read_secret_from_keyring(secret_ref)?;
            decode_secret_key(&private_key, passphrase)
                .map(Some)
                .map_err(|error| format!("profile-vault {secret_ref}: {error}"))
        }
        IdentitySource::Agent | IdentitySource::PublicKeyOnly => Ok(None),
    }
}

fn ensure_keyring_store() -> Result<(), String> {
    static KEYRING_INIT: OnceLock<Result<(), String>> = OnceLock::new();
    KEYRING_INIT
        .get_or_init(|| {
            keyring::use_native_store(true)
                .or_else(|_| keyring::use_native_store(false))
                .map_err(|error| format!("系统密钥库初始化失败: {error}"))
        })
        .clone()
}

fn keyring_entry(secret_ref: &str) -> Result<Entry, String> {
    ensure_keyring_store()?;
    let account = secret_ref
        .trim()
        .strip_prefix("keychain:")
        .unwrap_or_else(|| secret_ref.trim());
    if account.is_empty() || account.contains('\0') {
        return Err("secretRef 无效".to_string());
    }
    Entry::new("PortMate", account).map_err(|error| format!("创建系统密钥库条目失败: {error}"))
}

fn write_secret_to_keyring(secret_ref: &str, secret: &str) -> Result<(), String> {
    let entry = keyring_entry(secret_ref)?;
    entry
        .set_password(secret)
        .map_err(|error| format!("写入系统密钥库失败: {error}"))
}

fn read_secret_from_keyring(secret_ref: &str) -> Result<String, String> {
    let entry = keyring_entry(secret_ref)?;
    entry
        .get_password()
        .map_err(|error| format!("读取系统密钥库失败: {error:?}"))
}

fn has_secret_ref(secret_ref: &str) -> bool {
    read_secret_from_keyring(secret_ref).is_ok()
}

fn build_mcp_http_config(token_available: bool) -> McpHttpConfig {
    McpHttpConfig {
        endpoint: format!("http://{MCP_HTTP_DEFAULT_ADDR}/mcp"),
        token_ref: MCP_HTTP_TOKEN_REF.to_string(),
        token_available,
        default_origin: format!("http://{MCP_HTTP_DEFAULT_ADDR}"),
        start_command: format!(
            "PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR={MCP_HTTP_DEFAULT_ADDR} PORTMATE_MCP_HTTP_ORIGINS=http://{MCP_HTTP_DEFAULT_ADDR} cargo run -p portmate-mcp -- --http"
        ),
    }
}

fn read_optional_secret_ref(
    secret_ref: Option<&str>,
    label: &str,
) -> Result<Option<String>, String> {
    let Some(secret_ref) = secret_ref.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    read_secret_from_keyring(secret_ref)
        .map(Some)
        .map_err(|error| format!("{label} 已配置 secretRef 但读取失败: {error}"))
}

fn delete_secret_from_keyring(secret_ref: &str) -> Result<(), String> {
    let entry = keyring_entry(secret_ref)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("删除系统密钥库条目失败: {error}")),
    }
}

fn expand_identity_path(path: &str) -> PathBuf {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            let rest = path.strip_prefix("~/").unwrap_or_default();
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

fn persist_observed_host_key(
    store: &Arc<Mutex<SessionStore>>,
    profile_id: &str,
    observed_key: &Arc<Mutex<Option<HostKeyObservation>>>,
    one_time_host_keys: &[TrustedHostKey],
) -> Result<(), String> {
    let observation = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "SSH 未收到服务器 host key".to_string())?;
    let mut store = store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profile(profile_id)
        .ok_or_else(|| format!("unknown session: {profile_id}"))?;
    let policy = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.host_key_policy,
        _ => return Err(format!("profile is not SSH-backed: {profile_id}")),
    };

    if one_time_trusts_observation(one_time_host_keys, profile_id, &policy, &observation) {
        let fingerprint = observation
            .fingerprint_sha256()
            .map_err(|error| error.to_string())?;
        store.record_system_event(
            profile_id,
            format!(
                "PortMate: SSH host key trusted for this connection only ({}, {})",
                observation.algorithm, fingerprint
            ),
        );
        return Ok(());
    }

    if profile_trusts_observation(&store, profile_id, &observation) {
        let fingerprint = observation
            .fingerprint_sha256()
            .map_err(|error| error.to_string())?;
        store.record_system_event(
            profile_id,
            format!(
                "PortMate: SSH host key verified by profile trust ({}, {})",
                observation.algorithm, fingerprint
            ),
        );
        return Ok(());
    }

    match store.evaluate_host_key(profile_id, &observation)? {
        HostKeyEvaluation::Trusted {
            fingerprint_sha256, ..
        } => {
            store.record_system_event(
                profile_id,
                format!(
                    "PortMate: SSH host key verified ({}, {})",
                    observation.algorithm, fingerprint_sha256
                ),
            );
            Ok(())
        }
        HostKeyEvaluation::Unknown {
            fingerprint_sha256, ..
        } => {
            if policy.mode != HostKeyMode::TrustOnFirstUse {
                return Err(format!(
                    "SSH host key 未受信任: {} {}",
                    observation.algorithm, fingerprint_sha256
                ));
            }
            store.apply_host_key_decision(
                profile_id,
                &observation,
                HostKeyDecision::AppendToProfile,
            )?;
            store.record_system_event(
                profile_id,
                format!(
                    "PortMate: SSH host key trusted for this profile ({}, {})",
                    observation.algorithm, fingerprint_sha256
                ),
            );
            Ok(())
        }
        mismatch @ HostKeyEvaluation::Mismatch { .. } => {
            Err(describe_host_key_rejection(&mismatch))
        }
    }
}

fn temporary_trusted_host_key(
    store: &SessionStore,
    profile_id: &str,
    observation: &HostKeyObservation,
) -> Result<portmate_core::TrustedHostKey, String> {
    let profile = store
        .profile(profile_id)
        .ok_or_else(|| format!("unknown session: {profile_id}"))?;
    let policy = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.host_key_policy,
        _ => return Err(format!("profile is not SSH-backed: {profile_id}")),
    };
    Ok(portmate_core::TrustedHostKey {
        id: Uuid::new_v4().to_string(),
        profile_id: Some(profile_id.to_string()),
        alias: observation.target_alias(&policy).to_string(),
        host: observation.host.clone(),
        port: observation.port,
        algorithm: observation.algorithm.clone(),
        fingerprint_sha256: observation
            .fingerprint_sha256()
            .map_err(|error| error.to_string())?,
        public_key_base64: observation.public_key_base64.clone(),
        scope: HostKeyScope::Profile,
        label: Some("trust once".to_string()),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    })
}

fn remember_one_time_host_key(
    state: &AppState,
    profile_id: &str,
    key: portmate_core::TrustedHostKey,
) -> Result<(), String> {
    remember_one_time_host_key_in(&state.one_time_host_keys, profile_id, key)
}

fn take_one_time_host_keys(
    state: &AppState,
    profile_id: &str,
) -> Result<Vec<portmate_core::TrustedHostKey>, String> {
    take_one_time_host_keys_from(&state.one_time_host_keys, profile_id)
}

fn one_time_host_keys_snapshot(
    state: &AppState,
    profile_id: &str,
) -> Result<Vec<portmate_core::TrustedHostKey>, String> {
    one_time_host_keys_snapshot_from(&state.one_time_host_keys, profile_id)
}

fn remember_one_time_host_key_in(
    one_time: &Arc<Mutex<HashMap<String, Vec<portmate_core::TrustedHostKey>>>>,
    profile_id: &str,
    key: portmate_core::TrustedHostKey,
) -> Result<(), String> {
    let mut one_time = one_time.lock().map_err(|error| error.to_string())?;
    one_time
        .entry(profile_id.to_string())
        .or_default()
        .push(key);
    Ok(())
}

fn take_one_time_host_keys_from(
    one_time: &Arc<Mutex<HashMap<String, Vec<portmate_core::TrustedHostKey>>>>,
    profile_id: &str,
) -> Result<Vec<portmate_core::TrustedHostKey>, String> {
    let mut one_time = one_time.lock().map_err(|error| error.to_string())?;
    Ok(one_time.remove(profile_id).unwrap_or_default())
}

fn one_time_host_keys_snapshot_from(
    one_time: &Arc<Mutex<HashMap<String, Vec<portmate_core::TrustedHostKey>>>>,
    profile_id: &str,
) -> Result<Vec<portmate_core::TrustedHostKey>, String> {
    let one_time = one_time.lock().map_err(|error| error.to_string())?;
    Ok(one_time.get(profile_id).cloned().unwrap_or_default())
}

fn persist_observed_host_key_with_policy(
    store: &Arc<Mutex<SessionStore>>,
    store_path: &Path,
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    observed_key: &Arc<Mutex<Option<HostKeyObservation>>>,
    one_time_host_keys: &[TrustedHostKey],
    label: &str,
) -> Result<(), String> {
    let observation = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| format!("{label} 未收到服务器 host key"))?;
    let mut store = store.lock().map_err(|error| error.to_string())?;
    if one_time_trusts_observation(one_time_host_keys, profile_id, policy, &observation) {
        let fingerprint = observation
            .fingerprint_sha256()
            .map_err(|error| error.to_string())?;
        store.record_system_event(
            profile_id,
            format!(
                "PortMate: {label} host key trusted for this connection only ({}, {})",
                observation.algorithm, fingerprint
            ),
        );
        return save_store(store_path, &store);
    }
    match store.host_keys.evaluate(profile_id, policy, &observation) {
        Ok(HostKeyEvaluation::Trusted {
            fingerprint_sha256, ..
        }) => {
            store.record_system_event(
                profile_id,
                format!(
                    "PortMate: {label} host key verified ({}, {})",
                    observation.algorithm, fingerprint_sha256
                ),
            );
        }
        Ok(HostKeyEvaluation::Unknown {
            fingerprint_sha256, ..
        }) if policy.mode == HostKeyMode::TrustOnFirstUse => {
            store
                .host_keys
                .apply_decision(
                    profile_id,
                    policy,
                    &observation,
                    HostKeyDecision::AppendToProfile,
                )
                .map_err(|error| error.to_string())?;
            store.record_system_event(
                profile_id,
                format!(
                    "PortMate: {label} host key trusted for this profile ({}, {})",
                    observation.algorithm, fingerprint_sha256
                ),
            );
        }
        Ok(other) => return Err(describe_host_key_rejection(&other)),
        Err(error) => return Err(error.to_string()),
    }
    save_store(store_path, &store)
}

fn one_time_trusts_observation(
    one_time_host_keys: &[TrustedHostKey],
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    observation: &HostKeyObservation,
) -> bool {
    let Ok(fingerprint) = observation.fingerprint_sha256() else {
        return false;
    };
    let alias = observation.target_alias(policy);
    one_time_host_keys.iter().any(|key| {
        key.profile_id.as_deref() == Some(profile_id)
            && key.alias == alias
            && key.host == observation.host
            && key.port == observation.port
            && key.algorithm == observation.algorithm
            && key.fingerprint_sha256 == fingerprint
    })
}

fn profile_trusts_observation(
    store: &SessionStore,
    profile_id: &str,
    observation: &HostKeyObservation,
) -> bool {
    let Some(profile) = store.profile(profile_id) else {
        return false;
    };
    let (policy, trusted_host_keys) = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            (ssh.host_key_policy, ssh.trusted_host_keys)
        }
        _ => return false,
    };
    let Ok(fingerprint) = observation.fingerprint_sha256() else {
        return false;
    };
    let alias = observation.target_alias(&policy);
    trusted_host_keys.iter().any(|key| {
        key.alias == alias
            && key.port == observation.port
            && key.algorithm == observation.algorithm
            && key.fingerprint_sha256 == fingerprint
    })
}

fn read_ssh_channel(
    task: SshReadTask,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        let SshReadTask {
            state,
            profile,
            runtime_id,
            tap,
            mut read_half,
            closed,
        } = task;
        let io = state.session_io();
        let session_id = profile.id.clone();
        let mut last_persist = Instant::now();
        let mut has_unpersisted_stream = false;

        while let Some(message) = read_half.wait().await {
            match message {
                ChannelMsg::Data { data } => {
                    let bytes = data.to_vec();
                    let _ = tap.send(bytes.clone());
                    record_channel_text(
                        &io,
                        &session_id,
                        EventStream::Stdout,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                ChannelMsg::ExtendedData { data, ext } => {
                    let bytes = data.to_vec();
                    let _ = tap.send(bytes.clone());
                    let stream = if ext == 1 {
                        EventStream::Stderr
                    } else {
                        EventStream::Stdout
                    };
                    record_channel_text(
                        &io,
                        &session_id,
                        stream,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    if let Ok(mut store) = io.store.lock() {
                        store.record_system_event(
                            &session_id,
                            format!(
                                "PortMate: SSH remote process exited with status {exit_status}"
                            ),
                        );
                        if let Err(error) = save_store(&io.store_path, &store) {
                            eprintln!("PortMate: failed to persist SSH exit status: {error}");
                        }
                    }
                }
                ChannelMsg::ExitSignal {
                    signal_name,
                    error_message,
                    ..
                } => {
                    if let Ok(mut store) = io.store.lock() {
                        store.record_system_event(
                            &session_id,
                            format!(
                                "PortMate: SSH remote process exited by signal {signal_name:?} {error_message}"
                            ),
                        );
                        if let Err(error) = save_store(&io.store_path, &store) {
                            eprintln!("PortMate: failed to persist SSH exit signal: {error}");
                        }
                    }
                }
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                    eprintln!("PortMate: failed to persist SSH stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                eprintln!("PortMate: failed to persist final SSH stream data: {error}");
            }
        }

        let mut should_reconnect = false;
        let removed_current = {
            let mut connections = match io.runtimes.ssh.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                if ssh_reconnect_enabled(&profile) && !closed.load(Ordering::SeqCst) {
                    should_reconnect = true;
                    false
                } else {
                    connections.remove(&session_id);
                    true
                }
            } else {
                false
            }
        };

        if should_reconnect {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Reconnecting,
                    Some("SSH channel closed".to_string()),
                );
                store.record_system_event(
                    &session_id,
                    "PortMate: SSH channel closed; reconnecting in 1000ms",
                );
                if let Err(error) = save_store(&io.store_path, &store) {
                    eprintln!("PortMate: failed to persist SSH reconnect event: {error}");
                }
            }
            tauri::async_runtime::spawn(reconnect_ssh_session(state, profile, runtime_id, closed));
            return;
        }

        if removed_current {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some("SSH channel closed".to_string()),
                );
                store.record_system_event(&session_id, "PortMate: SSH channel closed");
                if let Err(error) = save_store(&io.store_path, &store) {
                    eprintln!("PortMate: failed to persist SSH close event: {error}");
                }
            }
        }
    })
}

fn ssh_reconnect_enabled(profile: &SessionProfile) -> bool {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.reconnect,
        _ => false,
    }
}

fn ssh_reconnect_pending(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    if closed.load(Ordering::SeqCst) {
        return false;
    }
    state
        .ssh
        .lock()
        .ok()
        .and_then(|connections| {
            connections
                .get(session_id)
                .map(|runtime| runtime.runtime_id == runtime_id)
        })
        .unwrap_or(false)
}

async fn disconnect_ssh_runtime(runtime: SshRuntime, reason: &str) {
    runtime.closed.store(true, Ordering::SeqCst);
    let handle = runtime.handle.lock().await;
    let _ = handle
        .disconnect(Disconnect::ByApplication, reason, "en")
        .await;
    drop(handle);
    for jump_handle in runtime.jump_handles {
        let handle = jump_handle.lock().await;
        let _ = handle
            .disconnect(Disconnect::ByApplication, reason, "en")
            .await;
    }
}

async fn reconnect_ssh_session(
    state: AppState,
    profile: SessionProfile,
    previous_runtime_id: String,
    closed: Arc<AtomicBool>,
) {
    let session_id = profile.id.clone();
    let reconnect_delay = Duration::from_millis(1000);

    loop {
        tokio::time::sleep(reconnect_delay).await;
        if !ssh_reconnect_pending(&state, &session_id, &previous_runtime_id, &closed) {
            return;
        }

        let established = match establish_ssh_runtime(&state, &profile, None, None).await {
            Ok(established) => established,
            Err(error) => {
                if !ssh_reconnect_pending(&state, &session_id, &previous_runtime_id, &closed) {
                    return;
                }
                if let Ok(mut store) = state.store.lock() {
                    let _ = store.set_runtime_status_with_reason(
                        &session_id,
                        SessionStatus::Reconnecting,
                        Some(format!("SSH reconnect failed: {error}")),
                    );
                    store.record_system_event(
                        &session_id,
                        format!("PortMate: SSH reconnect failed: {error}; retrying in 1000ms"),
                    );
                    if let Err(error) = save_store(&state.store_path, &store) {
                        eprintln!("PortMate: failed to persist SSH reconnect failure: {error}");
                    }
                }
                continue;
            }
        };
        let EstablishedSshRuntime {
            runtime_id,
            runtime,
            tap,
            read_half,
            auth_method,
            closed: next_closed,
        } = established;
        let mut runtime = Some(runtime);
        let inserted = {
            let mut connections = match state.ssh.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == previous_runtime_id)
                && !closed.load(Ordering::SeqCst)
            {
                connections.insert(session_id.clone(), runtime.take().expect("runtime present"));
                true
            } else {
                false
            }
        };

        if !inserted {
            if let Some(runtime) = runtime {
                disconnect_ssh_runtime(runtime, "PortMate SSH reconnect superseded").await;
            }
            return;
        }

        let one_time_cleanup_error = take_one_time_host_keys(&state, &session_id).err();

        tauri::async_runtime::spawn(read_ssh_channel(SshReadTask {
            state: state.clone(),
            profile: profile.clone(),
            runtime_id,
            tap,
            read_half,
            closed: next_closed,
        }));

        if let Ok(mut store) = state.store.lock() {
            let _ = store.record_auth_success(&session_id, auth_method);
            store.record_system_event(
                &session_id,
                format!("PortMate: SSH session reconnected via {auth_method:?}"),
            );
            if let Some(error) = one_time_cleanup_error {
                store.record_system_event(
                    &session_id,
                    format!("PortMate: failed to consume one-time host key trust: {error}"),
                );
            }
            if let Err(error) = store.open_session(&session_id) {
                store.record_system_event(
                    &session_id,
                    format!("PortMate: SSH reconnect status update failed: {error}"),
                );
            }
            if let Err(error) = save_store(&state.store_path, &store) {
                eprintln!("PortMate: failed to persist SSH reconnect success: {error}");
            }
        }
        return;
    }
}

const TELNET_IAC: u8 = 255;
const TELNET_SE: u8 = 240;
const TELNET_SB: u8 = 250;
const TELNET_WILL: u8 = 251;
const TELNET_WONT: u8 = 252;
const TELNET_DO: u8 = 253;
const TELNET_DONT: u8 = 254;
const TELNET_OPT_ECHO: u8 = 1;
const TELNET_OPT_SUPPRESS_GO_AHEAD: u8 = 3;
const TELNET_OPT_TERMINAL_TYPE: u8 = 24;
const TELNET_TTYPE_IS: u8 = 0;
const TELNET_TTYPE_SEND: u8 = 1;

enum TelnetState {
    Data,
    Iac,
    Command(u8),
    Subnegotiation,
    SubnegotiationIac,
}

struct TelnetNegotiator {
    state: TelnetState,
    subnegotiation: Vec<u8>,
}

impl TelnetNegotiator {
    fn new() -> Self {
        Self {
            state: TelnetState::Data,
            subnegotiation: Vec::new(),
        }
    }

    fn filter(&mut self, input: &[u8]) -> (Vec<u8>, Vec<Vec<u8>>) {
        let mut output = Vec::with_capacity(input.len());
        let mut replies = Vec::new();
        for byte in input {
            match self.state {
                TelnetState::Data => {
                    if *byte == TELNET_IAC {
                        self.state = TelnetState::Iac;
                    } else {
                        output.push(*byte);
                    }
                }
                TelnetState::Iac => match *byte {
                    TELNET_IAC => {
                        output.push(TELNET_IAC);
                        self.state = TelnetState::Data;
                    }
                    TELNET_DO | TELNET_DONT | TELNET_WILL | TELNET_WONT => {
                        self.state = TelnetState::Command(*byte);
                    }
                    TELNET_SB => {
                        self.subnegotiation.clear();
                        self.state = TelnetState::Subnegotiation;
                    }
                    _ => {
                        self.state = TelnetState::Data;
                    }
                },
                TelnetState::Command(command) => {
                    if let Some(reply) = telnet_option_reply(command, *byte) {
                        replies.push(reply);
                    }
                    self.state = TelnetState::Data;
                }
                TelnetState::Subnegotiation => {
                    if *byte == TELNET_IAC {
                        self.state = TelnetState::SubnegotiationIac;
                    } else {
                        self.subnegotiation.push(*byte);
                    }
                }
                TelnetState::SubnegotiationIac => {
                    if *byte == TELNET_SE {
                        if let Some(reply) = telnet_subnegotiation_reply(&self.subnegotiation) {
                            replies.push(reply);
                        }
                        self.subnegotiation.clear();
                        self.state = TelnetState::Data;
                    } else {
                        self.subnegotiation.push(TELNET_IAC);
                        self.subnegotiation.push(*byte);
                        self.state = TelnetState::Subnegotiation;
                    }
                }
            }
        }
        (output, replies)
    }
}

fn telnet_option_reply(command: u8, option: u8) -> Option<Vec<u8>> {
    let response = match command {
        TELNET_DO => match option {
            TELNET_OPT_SUPPRESS_GO_AHEAD | TELNET_OPT_TERMINAL_TYPE => TELNET_WILL,
            _ => TELNET_WONT,
        },
        TELNET_DONT => TELNET_WONT,
        TELNET_WILL => match option {
            TELNET_OPT_ECHO | TELNET_OPT_SUPPRESS_GO_AHEAD => TELNET_DO,
            _ => TELNET_DONT,
        },
        TELNET_WONT => TELNET_DONT,
        _ => return None,
    };
    Some(vec![TELNET_IAC, response, option])
}

fn telnet_subnegotiation_reply(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.first().copied() == Some(TELNET_OPT_TERMINAL_TYPE)
        && payload.get(1).copied() == Some(TELNET_TTYPE_SEND)
    {
        let mut reply = vec![
            TELNET_IAC,
            TELNET_SB,
            TELNET_OPT_TERMINAL_TYPE,
            TELNET_TTYPE_IS,
        ];
        reply.extend_from_slice(b"xterm-256color");
        reply.extend_from_slice(&[TELNET_IAC, TELNET_SE]);
        return Some(reply);
    }
    None
}

struct TcpReadTask {
    state: AppState,
    profile: SessionProfile,
    runtime_id: String,
    label: String,
    tap: broadcast::Sender<Vec<u8>>,
    writer: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    read_half: OwnedReadHalf,
    closed: Arc<AtomicBool>,
}

fn read_tcp_stream(
    task: TcpReadTask,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        let TcpReadTask {
            state,
            profile,
            runtime_id,
            label,
            tap,
            writer,
            mut read_half,
            closed,
        } = task;
        let io = state.session_io();
        let session_id = profile.id.clone();
        let mut buffer = vec![0_u8; 8192];
        let mut last_persist = Instant::now();
        let mut has_unpersisted_stream = false;
        let mut telnet = (label == "Telnet").then(TelnetNegotiator::new);

        loop {
            match read_half.read(&mut buffer).await {
                Ok(0) => break,
                Ok(size) => {
                    let (bytes, replies) = if let Some(negotiator) = telnet.as_mut() {
                        negotiator.filter(&buffer[..size])
                    } else {
                        (buffer[..size].to_vec(), Vec::new())
                    };
                    for reply in replies {
                        let mut writer = writer.lock().await;
                        if let Err(error) = writer.write_all(&reply).await {
                            if let Ok(mut store) = io.store.lock() {
                                store.record_system_event(
                                    &session_id,
                                    format!("PortMate: Telnet negotiation reply failed: {error}"),
                                );
                            }
                            break;
                        }
                    }
                    if bytes.is_empty() {
                        continue;
                    }
                    let _ = tap.send(bytes.clone());
                    record_channel_text(
                        &io,
                        &session_id,
                        EventStream::Stdout,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                Err(error) => {
                    if let Ok(mut store) = io.store.lock() {
                        store.record_system_event(
                            &session_id,
                            format!("PortMate: {label} read failed: {error}"),
                        );
                        if let Err(error) = save_store(&io.store_path, &store) {
                            eprintln!("PortMate: failed to persist {label} read error: {error}");
                        }
                    }
                    break;
                }
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                    eprintln!("PortMate: failed to persist {label} stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                eprintln!("PortMate: failed to persist final {label} stream data: {error}");
            }
        }

        let mut should_reconnect = false;
        let removed_current = {
            let mut connections = match io.runtimes.tcp.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                if tcp_reconnect_enabled(&profile) && !closed.load(Ordering::SeqCst) {
                    should_reconnect = true;
                    false
                } else {
                    connections.remove(&session_id);
                    true
                }
            } else {
                false
            }
        };

        if should_reconnect {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Reconnecting,
                    Some(format!("{label} socket closed")),
                );
                store.record_system_event(
                    &session_id,
                    format!("PortMate: {label} socket closed; reconnecting in 1000ms"),
                );
                if let Err(error) = save_store(&io.store_path, &store) {
                    eprintln!("PortMate: failed to persist {label} reconnect event: {error}");
                }
            }
            tauri::async_runtime::spawn(reconnect_tcp_session(
                state, profile, runtime_id, label, closed,
            ));
            return;
        }

        if removed_current {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some(format!("{label} socket closed")),
                );
                store.record_system_event(&session_id, format!("PortMate: {label} socket closed"));
                if let Err(error) = save_store(&io.store_path, &store) {
                    eprintln!("PortMate: failed to persist {label} close event: {error}");
                }
            }
        }
    })
}

fn tcp_reconnect_pending(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    if closed.load(Ordering::SeqCst) {
        return false;
    }
    state
        .tcp
        .lock()
        .ok()
        .and_then(|connections| {
            connections
                .get(session_id)
                .map(|runtime| runtime.runtime_id == runtime_id)
        })
        .unwrap_or(false)
}

async fn reconnect_tcp_session(
    state: AppState,
    profile: SessionProfile,
    previous_runtime_id: String,
    label: String,
    closed: Arc<AtomicBool>,
) {
    let session_id = profile.id.clone();
    let (host, port, connect_label) = match tcp_connection_details(&profile) {
        Ok(details) => details,
        Err(error) => {
            record_connection_failure(&state, &session_id, &error);
            return;
        }
    };
    let reconnect_delay = Duration::from_millis(1000);

    loop {
        tokio::time::sleep(reconnect_delay).await;
        if !tcp_reconnect_pending(&state, &session_id, &previous_runtime_id, &closed) {
            return;
        }

        let stream = match connect_tcp_socket(&host, port, connect_label).await {
            Ok(stream) => stream,
            Err(error) => {
                if !tcp_reconnect_pending(&state, &session_id, &previous_runtime_id, &closed) {
                    return;
                }
                if let Ok(mut store) = state.store.lock() {
                    let _ = store.set_runtime_status_with_reason(
                        &session_id,
                        SessionStatus::Reconnecting,
                        Some(format!("{label} reconnect failed: {error}")),
                    );
                    store.record_system_event(
                        &session_id,
                        format!("PortMate: {label} reconnect failed: {error}; retrying in 1000ms"),
                    );
                    if let Err(error) = save_store(&state.store_path, &store) {
                        eprintln!("PortMate: failed to persist {label} reconnect failure: {error}");
                    }
                }
                continue;
            }
        };

        let runtime_id = Uuid::new_v4().to_string();
        let (read_half, write_half) = stream.into_split();
        let writer = Arc::new(tokio::sync::Mutex::new(write_half));
        let (tap, _) = broadcast::channel(1024);
        let next_closed = Arc::new(AtomicBool::new(false));
        let inserted = {
            let mut connections = match state.tcp.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == previous_runtime_id)
                && !closed.load(Ordering::SeqCst)
            {
                connections.insert(
                    session_id.clone(),
                    TcpRuntime {
                        runtime_id: runtime_id.clone(),
                        writer: Arc::clone(&writer),
                        tap: tap.clone(),
                        closed: Arc::clone(&next_closed),
                    },
                );
                true
            } else {
                false
            }
        };

        if !inserted {
            let mut writer = writer.lock().await;
            let _ = writer.shutdown().await;
            return;
        }

        if let Ok(mut store) = state.store.lock() {
            store.record_system_event(&session_id, format!("PortMate: {label} socket reconnected"));
            if let Err(error) = store.open_session(&session_id) {
                store.record_system_event(
                    &session_id,
                    format!("PortMate: reconnect status update failed: {error}"),
                );
            }
            if let Err(error) = save_store(&state.store_path, &store) {
                eprintln!("PortMate: failed to persist {label} reconnect success: {error}");
            }
        }

        tauri::async_runtime::spawn(read_tcp_stream(TcpReadTask {
            state: state.clone(),
            profile: profile.clone(),
            runtime_id,
            label: label.clone(),
            tap,
            writer,
            read_half,
            closed: next_closed,
        }));
        return;
    }
}

struct ShellReadTask {
    io: SessionIo,
    session_id: String,
    runtime_id: String,
    program: String,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    reader: Box<dyn Read + Send>,
}

fn read_shell_pty(task: ShellReadTask) -> impl FnOnce() + Send + 'static {
    move || {
        let ShellReadTask {
            io,
            session_id,
            runtime_id,
            program,
            tap,
            closed,
            child,
            mut reader,
        } = task;
        let mut buffer = vec![0_u8; 8192];
        let mut last_persist = Instant::now();
        let mut has_unpersisted_stream = false;

        while !closed.load(Ordering::SeqCst) {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    if let Ok(mut child) = child.lock() {
                        if child.try_wait().ok().flatten().is_some() {
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(size) => {
                    let bytes = buffer[..size].to_vec();
                    let _ = tap.send(bytes.clone());
                    record_channel_text(
                        &io,
                        &session_id,
                        EventStream::Stdout,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    if let Ok(mut store) = io.store.lock() {
                        store.record_system_event(
                            &session_id,
                            format!("PortMate: shell read failed on {program}: {error}"),
                        );
                        if let Err(error) = save_store(&io.store_path, &store) {
                            eprintln!("PortMate: failed to persist shell read error: {error}");
                        }
                    }
                    break;
                }
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                    eprintln!("PortMate: failed to persist shell stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                eprintln!("PortMate: failed to persist final shell stream data: {error}");
            }
        }

        let removed_current = {
            let mut connections = match io.runtimes.shell.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                connections.remove(&session_id);
                true
            } else {
                false
            }
        };

        if removed_current {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some(format!("shell closed ({program})")),
                );
                store.record_system_event(
                    &session_id,
                    format!("PortMate: shell closed ({program})"),
                );
                if let Err(error) = save_store(&io.store_path, &store) {
                    eprintln!("PortMate: failed to persist shell close event: {error}");
                }
            }
        }
    }
}

struct SerialReadTask {
    io: SessionIo,
    profile: SessionProfile,
    runtime_id: String,
    port_name: String,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
    reader: SerialPortHandle,
}

fn spawn_serial_reader(task: SerialReadTask) -> std::io::Result<std::thread::JoinHandle<()>> {
    let name = format!("portmate-serial-{}", task.profile.id);
    std::thread::Builder::new()
        .name(name)
        .spawn(read_serial_port(task))
}

fn read_serial_port(task: SerialReadTask) -> impl FnOnce() + Send + 'static {
    move || {
        let SerialReadTask {
            io,
            profile,
            runtime_id,
            port_name,
            tap,
            closed,
            mut reader,
        } = task;
        let session_id = profile.id.clone();
        let mut buffer = vec![0_u8; 8192];
        let mut last_persist = Instant::now();
        let mut has_unpersisted_stream = false;

        while !closed.load(Ordering::SeqCst) {
            match reader.read(&mut buffer) {
                Ok(0) => {}
                Ok(size) => {
                    let bytes = buffer[..size].to_vec();
                    let _ = tap.send(bytes.clone());
                    record_channel_text(
                        &io,
                        &session_id,
                        EventStream::Stdout,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    if let Ok(mut store) = io.store.lock() {
                        store.record_system_event(
                            &session_id,
                            format!("PortMate: serial read failed on {port_name}: {error}"),
                        );
                        if let Err(error) = save_store(&io.store_path, &store) {
                            eprintln!("PortMate: failed to persist serial read error: {error}");
                        }
                    }
                    break;
                }
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                    eprintln!("PortMate: failed to persist serial stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                eprintln!("PortMate: failed to persist final serial stream data: {error}");
            }
        }

        let mut should_reconnect = false;
        let removed_current = {
            let mut connections = match io.runtimes.serial.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                if serial_reconnect_enabled(&profile) && !closed.load(Ordering::SeqCst) {
                    if let Some(runtime) = connections.get_mut(&session_id) {
                        runtime.writer = None;
                    }
                    should_reconnect = true;
                    false
                } else {
                    connections.remove(&session_id);
                    true
                }
            } else {
                false
            }
        };

        if should_reconnect {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Reconnecting,
                    Some(format!("serial port closed ({port_name})")),
                );
                store.record_system_event(
                    &session_id,
                    format!("PortMate: serial port closed ({port_name}); reconnecting in 1000ms"),
                );
                if let Err(error) = save_store(&io.store_path, &store) {
                    eprintln!("PortMate: failed to persist serial reconnect event: {error}");
                }
            }
            spawn_serial_reconnect(io, profile, runtime_id, port_name, closed);
            return;
        }

        if removed_current {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some(format!("serial port closed ({port_name})")),
                );
                store.record_system_event(
                    &session_id,
                    format!("PortMate: serial port closed ({port_name})"),
                );
                if let Err(error) = save_store(&io.store_path, &store) {
                    eprintln!("PortMate: failed to persist serial close event: {error}");
                }
            }
        }
    }
}

fn serial_reconnect_pending(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    if closed.load(Ordering::SeqCst) {
        return false;
    }
    io.runtimes
        .serial
        .lock()
        .ok()
        .and_then(|connections| {
            connections
                .get(session_id)
                .map(|runtime| runtime.runtime_id == runtime_id)
        })
        .unwrap_or(false)
}

fn spawn_serial_reconnect(
    io: SessionIo,
    profile: SessionProfile,
    previous_runtime_id: String,
    port_name: String,
    closed: Arc<AtomicBool>,
) {
    let thread_name = format!("portmate-serial-reconnect-{}", profile.id);
    if let Err(error) = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            reconnect_serial_session(io, profile, previous_runtime_id, port_name, closed)
        })
    {
        eprintln!("PortMate: failed to start serial reconnect thread: {error}");
    }
}

fn reconnect_serial_session(
    io: SessionIo,
    profile: SessionProfile,
    previous_runtime_id: String,
    port_name: String,
    closed: Arc<AtomicBool>,
) {
    let session_id = profile.id.clone();
    let (serial, _) = match serial_connection_details(&profile) {
        Ok(details) => details,
        Err(error) => {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Error,
                    Some(format!("serial reconnect cannot start: {error}")),
                );
                store.record_system_event(
                    &session_id,
                    format!("PortMate: serial reconnect cannot start: {error}"),
                );
                let _ = save_store(&io.store_path, &store);
            }
            return;
        }
    };
    let reconnect_delay = Duration::from_millis(1000);

    loop {
        std::thread::sleep(reconnect_delay);
        if !serial_reconnect_pending(&io, &session_id, &previous_runtime_id, &closed) {
            return;
        }

        let (port, reader) = match open_configured_serial_port(&serial, &port_name) {
            Ok(port) => port,
            Err(error) => {
                if !serial_reconnect_pending(&io, &session_id, &previous_runtime_id, &closed) {
                    return;
                }
                if let Ok(mut store) = io.store.lock() {
                    let _ = store.set_runtime_status_with_reason(
                        &session_id,
                        SessionStatus::Reconnecting,
                        Some(format!("serial reconnect failed on {port_name}: {error}")),
                    );
                    store.record_system_event(
                        &session_id,
                        format!(
                            "PortMate: serial reconnect failed on {port_name}: {error}; retrying in 1000ms"
                        ),
                    );
                    if let Err(error) = save_store(&io.store_path, &store) {
                        eprintln!("PortMate: failed to persist serial reconnect failure: {error}");
                    }
                }
                continue;
            }
        };

        let runtime_id = Uuid::new_v4().to_string();
        let writer = Arc::new(Mutex::new(port));
        let (tap, _) = broadcast::channel(1024);
        let next_closed = Arc::new(AtomicBool::new(false));
        let inserted = {
            let mut connections = match io.runtimes.serial.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == previous_runtime_id)
                && !closed.load(Ordering::SeqCst)
            {
                connections.insert(
                    session_id.clone(),
                    SerialRuntime {
                        runtime_id: runtime_id.clone(),
                        writer: Some(Arc::clone(&writer)),
                        tap: tap.clone(),
                        closed: Arc::clone(&next_closed),
                    },
                );
                true
            } else {
                false
            }
        };

        if !inserted {
            return;
        }

        if let Err(error) = spawn_serial_reader(SerialReadTask {
            io: io.clone(),
            profile: profile.clone(),
            runtime_id: runtime_id.clone(),
            port_name: port_name.clone(),
            tap,
            closed: next_closed,
            reader,
        }) {
            if let Ok(mut connections) = io.runtimes.serial.lock() {
                if connections
                    .get(&session_id)
                    .is_some_and(|runtime| runtime.runtime_id == runtime_id)
                {
                    connections.remove(&session_id);
                }
            }
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Error,
                    Some(format!("serial read thread restart failed: {error}")),
                );
                store.record_system_event(
                    &session_id,
                    format!("PortMate: serial read thread restart failed: {error}"),
                );
                let _ = save_store(&io.store_path, &store);
            }
            return;
        }

        if let Ok(mut store) = io.store.lock() {
            store.record_system_event(
                &session_id,
                format!(
                    "PortMate: serial port reconnected ({port_name}, {} baud)",
                    serial.baud_rate
                ),
            );
            if let Err(error) = store.open_session(&session_id) {
                store.record_system_event(
                    &session_id,
                    format!("PortMate: serial reconnect status update failed: {error}"),
                );
            }
            if let Err(error) = save_store(&io.store_path, &store) {
                eprintln!("PortMate: failed to persist serial reconnect success: {error}");
            }
        }
        return;
    }
}

fn record_channel_text(io: &SessionIo, session_id: &str, stream: EventStream, text: String) {
    if text.is_empty() {
        return;
    }
    let bytes_ref = append_raw_and_text_log_shards(io, session_id, stream, &text);
    let mut live_event = None;
    let local_commands = if let Ok(mut store) = io.store.lock() {
        live_event = store
            .record_stream_event_with_bytes_ref(
                session_id,
                EventDirection::Inbound,
                stream,
                text.clone(),
                bytes_ref,
            )
            .ok();
        let (trigger_dispatch, trigger_changed_store) =
            apply_trigger_actions_locked(&mut store, session_id, &text);
        if trigger_changed_store {
            if let Err(error) = save_store(&io.store_path, &store) {
                eprintln!("PortMate: failed to persist trigger actions: {error}");
            }
        }
        trigger_dispatch
    } else {
        eprintln!(
            "PortMate: session store lock poisoned; dropping event for {session_id} \
             (live push and persistence degraded until the app restarts)"
        );
        TriggerDispatch::default()
    };
    if let Some(event) = live_event {
        append_jsonl_log_shard(io, session_id, &event);
        if let Some(app_handle) = &io.app_handle {
            let _ = app_handle.emit("portmate-session-event", event);
        }
    }
    for command in local_commands.local_commands {
        spawn_trigger_command(
            Arc::clone(&io.store),
            io.store_path.clone(),
            session_id.to_string(),
            command,
        );
    }
    for text in local_commands.send_texts {
        spawn_trigger_send_text(io.clone(), session_id.to_string(), text);
    }
}

fn append_raw_and_text_log_shards(
    io: &SessionIo,
    session_id: &str,
    stream: EventStream,
    text: &str,
) -> Option<String> {
    let profile = logging_profile(io, session_id)?;
    if !profile.logging.enabled {
        return None;
    }

    let mut raw_ref = None;
    if profile.logging.raw {
        match append_log_bytes(&io.store_path, &profile, "raw", text.as_bytes()) {
            Ok(reference) => raw_ref = Some(reference),
            Err(error) => eprintln!("PortMate: failed to append raw log shard: {error}"),
        }
    }

    if profile.logging.text {
        let mut line = if profile.logging.redact_secrets {
            redact_secrets(text)
        } else {
            text.to_string()
        };
        if !line.ends_with('\n') {
            line.push('\n');
        }
        if let Err(error) = append_log_bytes(&io.store_path, &profile, "txt", line.as_bytes()) {
            eprintln!("PortMate: failed to append text log shard: {error}");
        }
    }

    if raw_ref.is_none() && matches!(stream, EventStream::Stdout | EventStream::Stderr) {
        // `bytesRef` points at the raw shard when enabled; when raw logging is off,
        // the store event still carries text/jsonl without pretending a byte shard exists.
        return None;
    }
    raw_ref
}

fn append_jsonl_log_shard(io: &SessionIo, session_id: &str, event: &SessionEvent) {
    let Some(profile) = logging_profile(io, session_id) else {
        return;
    };
    if !profile.logging.enabled || !profile.logging.jsonl {
        return;
    }
    let mut event = event.clone();
    if profile.logging.redact_secrets {
        event.text = event.text.map(|text| redact_secrets(&text));
    }
    let Ok(mut line) = serde_json::to_vec(&event) else {
        return;
    };
    line.push(b'\n');
    if let Err(error) = append_log_bytes(&io.store_path, &profile, "jsonl", &line) {
        eprintln!("PortMate: failed to append jsonl log shard: {error}");
    }
}

fn logging_profile(io: &SessionIo, session_id: &str) -> Option<SessionProfile> {
    io.store
        .lock()
        .ok()
        .and_then(|store| store.profile(session_id))
}

fn append_log_bytes(
    store_path: &Path,
    profile: &SessionProfile,
    extension: &str,
    bytes: &[u8],
) -> Result<String, String> {
    let path = log_shard_path(store_path, profile, extension)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create log dir {}: {error}", parent.display()))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(&path)
        .map_err(|error| format!("failed to open log shard {}: {error}", path.display()))?;
    let offset = file
        .seek(std::io::SeekFrom::End(0))
        .map_err(|error| format!("failed to seek log shard {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to append log shard {}: {error}", path.display()))?;
    file.flush()
        .map_err(|error| format!("failed to flush log shard {}: {error}", path.display()))?;
    let relative = path
        .strip_prefix(log_root(store_path))
        .unwrap_or(path.as_path())
        .display()
        .to_string();
    Ok(format!("{relative}:{offset}:{}", bytes.len()))
}

fn log_shard_path(
    store_path: &Path,
    profile: &SessionProfile,
    extension: &str,
) -> Result<PathBuf, String> {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let template = profile
        .logging
        .path_template
        .trim()
        .trim_start_matches('/')
        .trim_start_matches('\\');
    let template = if template.is_empty() {
        "{profile}/{date}/{session}.jsonl"
    } else {
        template
    };
    let rendered = template
        .replace("{profile}", &profile.name)
        .replace("{group}", &profile.group)
        .replace("{session}", &profile.id)
        .replace("{date}", &date);

    let mut path = log_root(store_path);
    for segment in rendered.replace('\\', "/").split('/') {
        let clean = sanitize_log_path_segment(segment);
        if !clean.is_empty() && clean != "." && clean != ".." {
            path.push(clean);
        }
    }
    if path == log_root(store_path) {
        path.push(sanitize_log_path_segment(&profile.id));
    }
    path.set_extension(extension);
    Ok(path)
}

fn log_root(store_path: &Path) -> PathBuf {
    store_path
        .parent()
        .map(|parent| parent.join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

fn sanitize_log_path_segment(segment: &str) -> String {
    let cleaned = segment
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    cleaned.trim_matches('_').to_string()
}

#[derive(Default)]
struct TriggerDispatch {
    local_commands: Vec<String>,
    send_texts: Vec<String>,
}

fn apply_trigger_actions_locked(
    store: &mut SessionStore,
    session_id: &str,
    text: &str,
) -> (TriggerDispatch, bool) {
    let Some(profile) = store.profile(session_id) else {
        return (TriggerDispatch::default(), false);
    };

    let matches = portmate_core::triggers::evaluate_triggers(&profile.triggers, text);
    let mut dispatch = TriggerDispatch::default();
    let mut changed = false;
    for trigger_match in matches {
        changed = true;
        store.record_system_event(
            session_id,
            format!("PortMate: trigger matched ({})", trigger_match.label),
        );
        for action in trigger_match.actions {
            match action {
                TriggerAction::Highlight { color } => {
                    store.record_system_event(
                        session_id,
                        format!(
                            "PortMate: trigger highlight action ({}, color={color})",
                            trigger_match.label
                        ),
                    );
                }
                TriggerAction::SendText { text } => {
                    store.record_system_event(
                        session_id,
                        format!(
                            "PortMate: trigger send_text action queued ({}) bytes={}",
                            trigger_match.label,
                            text.len()
                        ),
                    );
                    dispatch.send_texts.push(text);
                }
                TriggerAction::LocalCommand { command } => dispatch.local_commands.push(command),
                TriggerAction::Notification { message } => {
                    store.record_system_event(
                        session_id,
                        format!("PortMate notification: {message}"),
                    );
                }
                TriggerAction::TimelineMark { label } => {
                    store.timeline.push(TimelineMark {
                        id: Uuid::new_v4().to_string(),
                        session_id: session_id.to_string(),
                        ts: Utc::now(),
                        label,
                        details: Some(format!("trigger: {}", trigger_match.label)),
                    });
                }
                TriggerAction::CustomLink { url_template } => {
                    let url = url_template.replace("{text}", text.trim());
                    store.record_system_event(
                        session_id,
                        format!("PortMate: trigger custom link ({url})"),
                    );
                }
            }
        }
    }
    (dispatch, changed)
}

fn spawn_trigger_command(
    store: Arc<Mutex<SessionStore>>,
    store_path: PathBuf,
    session_id: String,
    command: String,
) {
    std::thread::spawn(move || {
        let output = run_shell_command(&command);
        let message = match output {
            Ok((code, stdout, stderr)) => format!(
                "PortMate: trigger command exited code={code}: {}{}",
                truncate_for_log(&stdout, 1600),
                if stderr.trim().is_empty() {
                    String::new()
                } else {
                    format!(" stderr={}", truncate_for_log(&stderr, 1600))
                }
            ),
            Err(error) => format!("PortMate: trigger command failed: {error}"),
        };
        if let Ok(mut store) = store.lock() {
            store.record_system_event(&session_id, message);
            if let Err(error) = save_store(&store_path, &store) {
                eprintln!("PortMate: failed to persist trigger command output: {error}");
            }
        }
    });
}

fn spawn_trigger_send_text(io: SessionIo, session_id: String, text: String) {
    tauri::async_runtime::spawn(async move {
        let result = send_text_inner(io.clone(), session_id.clone(), text).await;

        if let Err(error) = result {
            if let Ok(mut store) = io.store.lock() {
                store.record_system_event(
                    &session_id,
                    format!("PortMate: trigger send_text failed: {error}"),
                );
                if let Err(error) = save_store(&io.store_path, &store) {
                    eprintln!("PortMate: failed to persist trigger send_text error: {error}");
                }
            }
        }
    });
}

fn run_shell_command(command: &str) -> Result<(i32, String, String), String> {
    let output = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", command])
            .output()
            .map_err(|error| error.to_string())?
    } else {
        Command::new("sh")
            .args(["-lc", command])
            .output()
            .map_err(|error| error.to_string())?
    };
    Ok((
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    ))
}

fn truncate_for_log(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= limit {
        return trimmed.to_string();
    }
    format!("{}...", &trimmed[..limit])
}

fn profile_requires_runtime(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<bool, String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    Ok(matches!(
        store.profile(session_id).map(|profile| profile.connection),
        Some(
            ConnectionConfig::Ssh(_)
                | ConnectionConfig::Tmux(_)
                | ConnectionConfig::Tcp(_)
                | ConnectionConfig::Telnet(_)
                | ConnectionConfig::Serial(_)
                | ConnectionConfig::Shell(_)
        )
    ))
}

fn serial_data_bits(value: u8) -> serialport::DataBits {
    match value {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        _ => serialport::DataBits::Eight,
    }
}

fn serial_stop_bits(value: u8) -> serialport::StopBits {
    match value {
        2 => serialport::StopBits::Two,
        _ => serialport::StopBits::One,
    }
}

fn serial_parity(value: &str) -> serialport::Parity {
    match value {
        "odd" => serialport::Parity::Odd,
        "even" => serialport::Parity::Even,
        _ => serialport::Parity::None,
    }
}

fn serial_flow_control(value: &str) -> serialport::FlowControl {
    match value {
        "software" => serialport::FlowControl::Software,
        "hardware" => serialport::FlowControl::Hardware,
        _ => serialport::FlowControl::None,
    }
}

fn collect_local_sysmon(session_id: &str) -> SysmonSnapshot {
    let uptime_seconds = read_uptime_seconds().unwrap_or_default();
    let (cpu_percent, rx_kbps, tx_kbps) = sample_cpu_and_network();
    let memory_percent = read_memory_percent().unwrap_or_default();
    SysmonSnapshot {
        session_id: session_id.to_string(),
        ts: Utc::now(),
        uptime_seconds,
        cpu_percent,
        memory_percent,
        rx_kbps,
        tx_kbps,
    }
}

async fn collect_remote_sysmon(
    session_id: &str,
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
) -> Result<SysmonSnapshot, String> {
    let command = r#"sh -lc 'cat /proc/uptime 2>/dev/null; echo __PORTMATE_MEMINFO__; cat /proc/meminfo 2>/dev/null; echo __PORTMATE_STAT1__; head -n 1 /proc/stat 2>/dev/null; echo __PORTMATE_NET1__; cat /proc/net/dev 2>/dev/null; sleep 0.2; echo __PORTMATE_STAT2__; head -n 1 /proc/stat 2>/dev/null; echo __PORTMATE_NET2__; cat /proc/net/dev 2>/dev/null'"#;
    let output = exec_ssh_command_capture(handle, command, Duration::from_secs(8)).await?;
    parse_remote_sysmon_output(session_id, &output)
}

async fn exec_ssh_command_capture(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    command: &str,
    timeout: Duration,
) -> Result<String, String> {
    let mut channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|error| format!("SSH exec 打开 channel 失败: {error}"))?
    };
    channel
        .exec(true, command)
        .await
        .map_err(|error| format!("SSH exec 启动失败: {error}"))?;

    let mut output = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    tokio::time::timeout(timeout, async {
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => output.extend_from_slice(&data),
                ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                ChannelMsg::ExitStatus { exit_status: code } => exit_status = Some(code),
                ChannelMsg::Eof | ChannelMsg::Close => break,
                _ => {}
            }
        }
    })
    .await
    .map_err(|_| "SSH exec 超时".to_string())?;

    if exit_status.is_some_and(|code| code != 0) && output.is_empty() {
        return Err(format!(
            "SSH exec 返回非零状态 {:?}: {}",
            exit_status,
            String::from_utf8_lossy(&stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&output).to_string())
}

fn parse_remote_sysmon_output(session_id: &str, output: &str) -> Result<SysmonSnapshot, String> {
    let uptime_raw = output
        .split("__PORTMATE_MEMINFO__")
        .next()
        .unwrap_or_default();
    let meminfo = section_between(output, "__PORTMATE_MEMINFO__", "__PORTMATE_STAT1__");
    let stat1 = section_between(output, "__PORTMATE_STAT1__", "__PORTMATE_NET1__");
    let net1 = section_between(output, "__PORTMATE_NET1__", "__PORTMATE_STAT2__");
    let stat2 = section_between(output, "__PORTMATE_STAT2__", "__PORTMATE_NET2__");
    let net2 = output.split("__PORTMATE_NET2__").nth(1).unwrap_or_default();

    let uptime_seconds = parse_uptime_seconds(uptime_raw).unwrap_or_default();
    let memory_percent = parse_memory_percent(meminfo).unwrap_or_default();
    let cpu_percent = match (parse_cpu_times(stat1), parse_cpu_times(stat2)) {
        (Some((idle_a, total_a)), Some((idle_b, total_b))) if total_b > total_a => {
            let idle_delta = idle_b.saturating_sub(idle_a) as f32;
            let total_delta = total_b.saturating_sub(total_a) as f32;
            if total_delta > 0.0 {
                ((1.0 - idle_delta / total_delta) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            }
        }
        _ => 0.0,
    };
    let (rx_kbps, tx_kbps) = match (parse_network_bytes(net1), parse_network_bytes(net2)) {
        (Some((rx_a, tx_a)), Some((rx_b, tx_b))) => {
            let seconds = 0.2_f32;
            (
                rx_b.saturating_sub(rx_a) as f32 / 1024.0 / seconds,
                tx_b.saturating_sub(tx_a) as f32 / 1024.0 / seconds,
            )
        }
        _ => (0.0, 0.0),
    };

    if uptime_seconds == 0 && memory_percent == 0.0 && cpu_percent == 0.0 {
        return Err("远端未提供 Linux /proc Sysmon 数据".to_string());
    }

    Ok(SysmonSnapshot {
        session_id: session_id.to_string(),
        ts: Utc::now(),
        uptime_seconds,
        cpu_percent,
        memory_percent,
        rx_kbps,
        tx_kbps,
    })
}

fn section_between<'a>(value: &'a str, start: &str, end: &str) -> &'a str {
    value
        .split(start)
        .nth(1)
        .and_then(|tail| tail.split(end).next())
        .unwrap_or_default()
}

fn sample_cpu_and_network() -> (f32, f32, f32) {
    let cpu_a = read_cpu_times();
    let net_a = read_network_bytes();
    std::thread::sleep(Duration::from_millis(120));
    let cpu_b = read_cpu_times();
    let net_b = read_network_bytes();

    let cpu_percent = match (cpu_a, cpu_b) {
        (Some((idle_a, total_a)), Some((idle_b, total_b))) if total_b > total_a => {
            let idle_delta = idle_b.saturating_sub(idle_a) as f32;
            let total_delta = total_b.saturating_sub(total_a) as f32;
            if total_delta > 0.0 {
                ((1.0 - idle_delta / total_delta) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            }
        }
        _ => 0.0,
    };

    let (rx_kbps, tx_kbps) = match (net_a, net_b) {
        (Some((rx_a, tx_a)), Some((rx_b, tx_b))) => {
            let seconds = 0.12_f32;
            (
                rx_b.saturating_sub(rx_a) as f32 / 1024.0 / seconds,
                tx_b.saturating_sub(tx_a) as f32 / 1024.0 / seconds,
            )
        }
        _ => (0.0, 0.0),
    };

    (cpu_percent, rx_kbps, tx_kbps)
}

#[cfg(target_os = "linux")]
fn read_uptime_seconds() -> Option<u64> {
    let raw = fs::read_to_string("/proc/uptime").ok()?;
    parse_uptime_seconds(&raw)
}

#[cfg(not(target_os = "linux"))]
fn read_uptime_seconds() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn read_memory_percent() -> Option<f32> {
    let raw = fs::read_to_string("/proc/meminfo").ok()?;
    parse_memory_percent(&raw)
}

fn parse_memory_percent(raw: &str) -> Option<f32> {
    let mut total = None;
    let mut available = None;
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        match parts.next()? {
            "MemTotal:" => total = parts.next()?.parse::<f32>().ok(),
            "MemAvailable:" => available = parts.next()?.parse::<f32>().ok(),
            _ => {}
        }
    }
    let total = total?;
    let available = available?;
    (total > 0.0).then_some(((total - available) / total * 100.0).clamp(0.0, 100.0))
}

#[cfg(not(target_os = "linux"))]
fn read_memory_percent() -> Option<f32> {
    None
}

#[cfg(target_os = "linux")]
fn read_cpu_times() -> Option<(u64, u64)> {
    let raw = fs::read_to_string("/proc/stat").ok()?;
    parse_cpu_times(&raw)
}

fn parse_cpu_times(raw: &str) -> Option<(u64, u64)> {
    let line = raw.lines().find(|line| line.starts_with("cpu "))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .filter_map(|value| value.parse::<u64>().ok())
        .collect::<Vec<_>>();
    if values.len() < 4 {
        return None;
    }
    let idle =
        values.get(3).copied().unwrap_or_default() + values.get(4).copied().unwrap_or_default();
    let total = values.iter().sum();
    Some((idle, total))
}

#[cfg(not(target_os = "linux"))]
fn read_cpu_times() -> Option<(u64, u64)> {
    None
}

#[cfg(target_os = "linux")]
fn read_network_bytes() -> Option<(u64, u64)> {
    let raw = fs::read_to_string("/proc/net/dev").ok()?;
    parse_network_bytes(&raw)
}

fn parse_network_bytes(raw: &str) -> Option<(u64, u64)> {
    let mut rx = 0_u64;
    let mut tx = 0_u64;
    for line in raw.lines().skip(2) {
        let Some((_, values)) = line.split_once(':') else {
            continue;
        };
        let parts = values.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 16 {
            rx = rx.saturating_add(parts[0].parse::<u64>().unwrap_or_default());
            tx = tx.saturating_add(parts[8].parse::<u64>().unwrap_or_default());
        }
    }
    Some((rx, tx))
}

fn parse_uptime_seconds(raw: &str) -> Option<u64> {
    raw.split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .map(|value| value as u64)
}

#[cfg(not(target_os = "linux"))]
fn read_network_bytes() -> Option<(u64, u64)> {
    None
}

fn terminal_key_sequence(key: &str) -> Result<String, String> {
    let normalized = key.trim().to_ascii_lowercase().replace('_', "-");
    let sequence = match normalized.as_str() {
        "" => return Err("key must not be empty".to_string()),
        "enter" | "return" => "\r".to_string(),
        "linefeed" | "lf" => "\n".to_string(),
        "tab" => "\t".to_string(),
        "backspace" | "bs" => "\u{0008}".to_string(),
        "delete" | "del" => "\x1b[3~".to_string(),
        "escape" | "esc" => "\x1b".to_string(),
        "up" | "arrow-up" => "\x1b[A".to_string(),
        "down" | "arrow-down" => "\x1b[B".to_string(),
        "right" | "arrow-right" => "\x1b[C".to_string(),
        "left" | "arrow-left" => "\x1b[D".to_string(),
        "home" => "\x1b[H".to_string(),
        "end" => "\x1b[F".to_string(),
        "pageup" | "page-up" => "\x1b[5~".to_string(),
        "pagedown" | "page-down" => "\x1b[6~".to_string(),
        "insert" | "ins" => "\x1b[2~".to_string(),
        "f1" => "\x1bOP".to_string(),
        "f2" => "\x1bOQ".to_string(),
        "f3" => "\x1bOR".to_string(),
        "f4" => "\x1bOS".to_string(),
        "f5" => "\x1b[15~".to_string(),
        "f6" => "\x1b[17~".to_string(),
        "f7" => "\x1b[18~".to_string(),
        "f8" => "\x1b[19~".to_string(),
        "f9" => "\x1b[20~".to_string(),
        "f10" => "\x1b[21~".to_string(),
        "f11" => "\x1b[23~".to_string(),
        "f12" => "\x1b[24~".to_string(),
        "space" => " ".to_string(),
        value if value.starts_with("ctrl+") || value.starts_with("ctrl-") => {
            let key = value
                .trim_start_matches("ctrl+")
                .trim_start_matches("ctrl-");
            let byte = match key {
                "space" | "@" => 0,
                "[" | "escape" | "esc" => 27,
                "\\" => 28,
                "]" => 29,
                "^" => 30,
                "_" => 31,
                value if value.len() == 1 => {
                    let ch = value.as_bytes()[0];
                    if ch.is_ascii_alphabetic() {
                        ch.to_ascii_uppercase() - b'@'
                    } else {
                        return Err(format!("unsupported control key: {key}"));
                    }
                }
                _ => return Err(format!("unsupported control key: {key}")),
            };
            String::from_utf8(vec![byte]).map_err(|error| error.to_string())?
        }
        value if value.chars().count() == 1 => value.to_string(),
        _ => return Err(format!("unsupported key sequence: {key}")),
    };
    Ok(sequence)
}

fn default_shell_program() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        if let Ok(shell) = std::env::var("SHELL") {
            let shell = shell.trim();
            if !shell.is_empty() {
                return shell.to_string();
            }
        }
        [
            "/bin/zsh",
            "/usr/bin/zsh",
            "/usr/local/bin/zsh",
            "/opt/homebrew/bin/zsh",
            "/bin/bash",
            "/usr/bin/bash",
        ]
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
        .unwrap_or("/bin/sh")
        .to_string()
    }
}

fn apply_shell_terminal_color_env(command: &mut CommandBuilder, term: &str) {
    command.env("TERM", normalized_terminal_name(term));
    command.env("COLORTERM", "truecolor");
    command.env("CLICOLOR", "1");
    command.env("CLICOLOR_FORCE", "1");
    command.env("FORCE_COLOR", "1");
    command.env("TERM_PROGRAM", "PortMate");
    command.env_remove("NO_COLOR");
}

async fn apply_ssh_terminal_color_env(channel: &Channel<client::Msg>) {
    for (name, value) in [
        ("COLORTERM", "truecolor"),
        ("CLICOLOR", "1"),
        ("CLICOLOR_FORCE", "1"),
        ("FORCE_COLOR", "1"),
        ("TERM_PROGRAM", "PortMate"),
    ] {
        let _ = channel.set_env(false, name, value).await;
    }
}

fn normalized_terminal_name(term: &str) -> &str {
    let term = term.trim();
    if term.is_empty() {
        "xterm-256color"
    } else {
        term
    }
}

fn record_connection_failure(state: &AppState, session_id: &str, error: &str) {
    if let Ok(mut store) = state.store.lock() {
        let _ = store.set_runtime_status_with_reason(
            session_id,
            SessionStatus::Error,
            Some(error.to_string()),
        );
        store.record_system_event(session_id, format!("PortMate: connection failed: {error}"));
        if let Err(error) = save_store(&state.store_path, &store) {
            eprintln!("PortMate: failed to persist connection failure: {error}");
        }
    }
}

fn load_store(path: &Path) -> SessionStore {
    if let Some(store) = load_store_sqlite(path) {
        return normalize_loaded_store(store);
    }
    let legacy_path = path.with_file_name(LEGACY_JSON_STORE_FILE_NAME);
    if legacy_path.exists() {
        return load_store_json(&legacy_path);
    }
    SessionStore::default()
}

fn load_store_sqlite(path: &Path) -> Option<SessionStore> {
    let connection = SqliteConnection::open(path).ok()?;
    ensure_store_schema(&connection).ok()?;
    let raw = connection
        .query_row(
            "select value from kv where key = ?1",
            params![STORE_KEY],
            |row| row.get::<_, String>(0),
        )
        .ok()?;
    match serde_json::from_str::<SessionStore>(&raw) {
        Ok(store) => Some(store),
        Err(error) => {
            eprintln!(
                "PortMate: failed to parse SQLite store {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn load_store_json(path: &Path) -> SessionStore {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SessionStore::default();
        }
        Err(error) => {
            eprintln!("PortMate: failed to read store {}: {error}", path.display());
            return SessionStore::default();
        }
    };

    match serde_json::from_str::<SessionStore>(&raw) {
        Ok(store) => normalize_loaded_store(store),
        Err(error) => {
            eprintln!(
                "PortMate: failed to parse store {}: {error}",
                path.display()
            );
            SessionStore::default()
        }
    }
}

fn normalize_session_profile(mut profile: SessionProfile) -> SessionProfile {
    profile.id = profile.id.trim().to_string();
    if profile.id.is_empty() {
        profile.id = format!("session-{}", Uuid::new_v4());
    }
    profile.name = profile.name.trim().to_string();
    profile.group = profile.group.trim().to_string();
    profile.tags = profile
        .tags
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect();
    profile.kind = session_kind_for_connection(&profile.connection);
    profile.terminal.term = normalized_terminal_name(&profile.terminal.term).to_string();

    match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            ssh.endpoint.host = ssh.endpoint.host.trim().to_string();
            if ssh.endpoint.port == 0 {
                ssh.endpoint.port = 22;
            }
            ssh.username = ssh.username.trim().to_string();
            let alias = ssh
                .host_key_policy
                .alias
                .as_deref()
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| profile.id.clone());
            ssh.host_key_policy.alias = Some(alias);
            for key in &mut ssh.trusted_host_keys {
                if key.scope == HostKeyScope::Profile && key.profile_id.is_none() {
                    key.profile_id = Some(profile.id.clone());
                }
                key.alias = key.alias.trim().to_string();
            }
            ssh.trusted_host_keys.retain(|key| {
                key.scope != HostKeyScope::Profile
                    || key.profile_id.as_deref() == Some(profile.id.as_str())
            });
            for jump in &mut ssh.jumps {
                jump.host = jump.host.trim().to_string();
                if jump.port == 0 {
                    jump.port = 22;
                }
                jump.username = jump.username.trim().to_string();
                if jump.username.is_empty() {
                    jump.username = ssh.username.clone();
                }
                jump.identity_ref = jump
                    .identity_ref
                    .as_deref()
                    .map(str::trim)
                    .filter(|identity_ref| !identity_ref.is_empty())
                    .map(ToOwned::to_owned);
            }
            ssh.jumps.retain(|jump| !jump.host.is_empty());
            let mut normalized_auth_order = Vec::new();
            for method in ssh.identity_policy.auth_order.drain(..) {
                if !normalized_auth_order.contains(&method) {
                    normalized_auth_order.push(method);
                }
            }
            if normalized_auth_order.is_empty() {
                normalized_auth_order = vec![
                    AuthMethod::PublicKey,
                    AuthMethod::KeyboardInteractive,
                    AuthMethod::Password,
                ];
            }
            ssh.identity_policy.auth_order = normalized_auth_order;
        }
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => {
            tcp.host = tcp.host.trim().to_string();
        }
        ConnectionConfig::Serial(serial) => {
            serial.port = serial.port.trim().to_string();
        }
        ConnectionConfig::Shell(shell) => {
            shell.program = shell.program.trim().to_string();
        }
    }

    profile
}

fn session_kind_for_connection(connection: &ConnectionConfig) -> SessionKind {
    match connection {
        ConnectionConfig::Ssh(_) => SessionKind::Ssh,
        ConnectionConfig::Serial(_) => SessionKind::Serial,
        ConnectionConfig::Shell(_) => SessionKind::Shell,
        ConnectionConfig::Telnet(_) => SessionKind::Telnet,
        ConnectionConfig::Tcp(_) => SessionKind::Tcp,
        ConnectionConfig::Tmux(_) => SessionKind::Tmux,
    }
}

fn normalize_loaded_store(mut store: SessionStore) -> SessionStore {
    let profiles = std::mem::take(&mut store.profiles);
    let saved_runtimes = std::mem::take(&mut store.runtimes)
        .into_iter()
        .map(|mut runtime| {
            runtime.session_id = runtime.session_id.trim().to_string();
            (runtime.session_id.clone(), runtime)
        })
        .collect::<HashMap<_, _>>();
    for profile in profiles {
        let _ = store.upsert_profile(normalize_session_profile(profile));
    }
    for runtime in &mut store.runtimes {
        if let Some(saved) = saved_runtimes.get(&runtime.session_id) {
            runtime.pane_id = saved.pane_id.clone();
            runtime.title = saved.title.clone();
            runtime.cwd = saved.cwd.clone();
            runtime.last_activity = saved.last_activity;
            runtime.last_disconnect = saved.last_disconnect;
            runtime.last_disconnect_reason = saved.last_disconnect_reason.clone();
        }
        runtime.status = SessionStatus::Disconnected;
        runtime.connected_since = None;
    }
    store
}

fn save_store(path: &Path, store: &SessionStore) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()) == Some("sqlite3") {
        save_store_sqlite(path, store)?;
        let legacy_path = path.with_file_name(LEGACY_JSON_STORE_FILE_NAME);
        if let Err(error) = save_store_json(&legacy_path, store) {
            eprintln!("PortMate: failed to update JSON compatibility store: {error}");
        }
        return Ok(());
    }
    save_store_json(path, store)
}

fn save_store_sqlite(path: &Path, store: &SessionStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create PortMate data directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let connection = SqliteConnection::open(path).map_err(|error| {
        format!(
            "failed to open PortMate SQLite store {}: {error}",
            path.display()
        )
    })?;
    ensure_store_schema(&connection)?;
    let bytes = serde_json::to_string_pretty(store)
        .map_err(|error| format!("failed to serialize PortMate store: {error}"))?;

    // Everything below is one transaction: a mid-write crash or error leaves the
    // connection with an open (uncommitted) transaction, and since this connection
    // is local to this call and gets dropped right after, SQLite rolls it back on
    // close instead of leaving the per-table mirror partially deleted/reinserted.
    connection
        .execute_batch("BEGIN IMMEDIATE;")
        .map_err(|error| format!("failed to start PortMate SQLite transaction: {error}"))?;
    connection
        .execute(
            "insert into kv (key, value, updated_at) values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
             on conflict(key) do update set value = excluded.value, updated_at = excluded.updated_at",
            params![STORE_KEY, bytes],
        )
        .map_err(|error| format!("failed to save PortMate SQLite store: {error}"))?;
    save_store_sqlite_tables(&connection, store)?;
    connection
        .execute_batch("COMMIT;")
        .map_err(|error| format!("failed to commit PortMate SQLite transaction: {error}"))?;
    Ok(())
}

fn ensure_store_schema(connection: &SqliteConnection) -> Result<(), String> {
    connection
        .execute_batch(
            "create table if not exists kv (
                key text primary key not null,
                value text not null,
                updated_at text not null
            );
            create table if not exists metadata (
                key text primary key not null,
                value text not null
            );
            create table if not exists profiles (
                id text primary key not null,
                name text not null,
                kind text not null,
                group_name text not null,
                tags_json text not null,
                connection_json text not null,
                terminal_json text not null,
                logging_json text not null,
                triggers_json text not null,
                transfer_json text not null,
                updated_at text not null
            );
            create table if not exists runtimes (
                session_id text primary key not null,
                pane_id text not null,
                status text not null,
                title text not null,
                cwd text,
                connected_since text,
                last_activity text not null,
                last_disconnect text,
                last_disconnect_reason text,
                active_transport text not null,
                raw_json text not null
            );
            create table if not exists events (
                id text primary key not null,
                session_id text not null,
                pane_id text not null,
                ts text not null,
                direction text not null,
                stream text not null,
                bytes_ref text,
                text text,
                annotations_json text not null,
                raw_json text not null
            );
            create table if not exists transfers (
                id text primary key not null,
                session_id text not null,
                protocol text not null,
                source text not null,
                destination text not null,
                bytes_total integer not null,
                bytes_done integer not null,
                status text not null,
                message text,
                raw_json text not null
            );
            create table if not exists trusted_host_keys (
                id text primary key not null,
                profile_id text,
                alias text not null,
                host text not null,
                port integer not null,
                algorithm text not null,
                fingerprint_sha256 text not null,
                public_key_base64 text not null,
                scope text not null,
                label text,
                first_seen text not null,
                last_seen text not null,
                raw_json text not null
            );
            create table if not exists mcp_grants (
                client_id text primary key not null,
                name text not null,
                scopes_json text not null,
                allowed_sessions_json text not null,
                expires_at text,
                revoked_at text,
                raw_json text not null
            );
            create table if not exists mcp_audit (
                id text primary key not null,
                ts text not null,
                actor text not null,
                action text not null,
                session_id text,
                decision text not null,
                details_json text not null,
                raw_json text not null
            );
            create table if not exists timeline_marks (
                id text primary key not null,
                session_id text not null,
                ts text not null,
                label text not null,
                details text,
                raw_json text not null
            );
            create table if not exists sysmon_snapshots (
                session_id text not null,
                ts text not null,
                uptime_seconds integer not null,
                cpu_percent real not null,
                memory_percent real not null,
                rx_kbps real not null,
                tx_kbps real not null,
                raw_json text not null,
                primary key (session_id, ts)
            );
            create index if not exists idx_events_session_ts on events(session_id, ts);
            create index if not exists idx_events_text on events(text);
            create index if not exists idx_transfers_session on transfers(session_id);
            create index if not exists idx_host_keys_alias on trusted_host_keys(alias, port, algorithm);
            create index if not exists idx_audit_session_ts on mcp_audit(session_id, ts);
            create index if not exists idx_timeline_session_ts on timeline_marks(session_id, ts);
            create index if not exists idx_sysmon_session_ts on sysmon_snapshots(session_id, ts);
            insert into metadata (key, value) values ('schemaVersion', '2')
                on conflict(key) do update set value = excluded.value;",
        )
        .map_err(|error| format!("failed to initialize PortMate SQLite schema: {error}"))?;
    let _ = connection.execute("alter table runtimes add column last_disconnect text", []);
    let _ = connection.execute(
        "alter table runtimes add column last_disconnect_reason text",
        [],
    );
    Ok(())
}

fn save_store_sqlite_tables(
    connection: &SqliteConnection,
    store: &SessionStore,
) -> Result<(), String> {
    connection
        .execute_batch(
            "delete from profiles;
             delete from runtimes;
             delete from events;
             delete from transfers;
             delete from trusted_host_keys;
             delete from mcp_grants;
             delete from mcp_audit;
             delete from timeline_marks;
             delete from sysmon_snapshots;",
        )
        .map_err(|error| format!("failed to clear PortMate SQLite mirror tables: {error}"))?;

    for profile in &store.profiles {
        connection
            .execute(
                "insert into profiles (
                    id, name, kind, group_name, tags_json, connection_json, terminal_json,
                    logging_json, triggers_json, transfer_json, updated_at
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                params![
                    profile.id,
                    profile.name,
                    enum_text(&profile.kind)?,
                    profile.group,
                    json_text(&profile.tags)?,
                    json_text(&profile.connection)?,
                    json_text(&profile.terminal)?,
                    json_text(&profile.logging)?,
                    json_text(&profile.triggers)?,
                    json_text(&profile.transfer)?,
                ],
            )
            .map_err(|error| format!("failed to mirror profile {}: {error}", profile.id))?;
    }

    for runtime in &store.runtimes {
        connection
            .execute(
                "insert into runtimes (
                    session_id, pane_id, status, title, cwd, connected_since, last_activity,
                    last_disconnect, last_disconnect_reason, active_transport, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    runtime.session_id,
                    runtime.pane_id,
                    enum_text(&runtime.status)?,
                    runtime.title,
                    runtime.cwd,
                    runtime.connected_since.map(|value| value.to_rfc3339()),
                    runtime.last_activity.to_rfc3339(),
                    runtime.last_disconnect.map(|value| value.to_rfc3339()),
                    runtime.last_disconnect_reason,
                    enum_text(&runtime.active_transport)?,
                    json_text(runtime)?,
                ],
            )
            .map_err(|error| format!("failed to mirror runtime {}: {error}", runtime.session_id))?;
    }

    for event in &store.events {
        connection
            .execute(
                "insert into events (
                    id, session_id, pane_id, ts, direction, stream, bytes_ref, text,
                    annotations_json, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    event.id,
                    event.session_id,
                    event.pane_id,
                    event.ts.to_rfc3339(),
                    enum_text(&event.direction)?,
                    enum_text(&event.stream)?,
                    event.bytes_ref,
                    event.text,
                    json_text(&event.annotations)?,
                    json_text(event)?,
                ],
            )
            .map_err(|error| format!("failed to mirror event {}: {error}", event.id))?;
    }

    for transfer in &store.transfers {
        connection
            .execute(
                "insert into transfers (
                    id, session_id, protocol, source, destination, bytes_total, bytes_done,
                    status, message, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    transfer.id,
                    transfer.session_id,
                    enum_text(&transfer.protocol)?,
                    transfer.source,
                    transfer.destination,
                    transfer.bytes_total as i64,
                    transfer.bytes_done as i64,
                    enum_text(&transfer.status)?,
                    transfer.message,
                    json_text(transfer)?,
                ],
            )
            .map_err(|error| format!("failed to mirror transfer {}: {error}", transfer.id))?;
    }

    for key in &store.host_keys.keys {
        connection
            .execute(
                "insert into trusted_host_keys (
                    id, profile_id, alias, host, port, algorithm, fingerprint_sha256,
                    public_key_base64, scope, label, first_seen, last_seen, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    key.id,
                    key.profile_id,
                    key.alias,
                    key.host,
                    i64::from(key.port),
                    key.algorithm,
                    key.fingerprint_sha256,
                    key.public_key_base64,
                    enum_text(&key.scope)?,
                    key.label,
                    key.first_seen.to_rfc3339(),
                    key.last_seen.to_rfc3339(),
                    json_text(key)?,
                ],
            )
            .map_err(|error| format!("failed to mirror host key {}: {error}", key.id))?;
    }

    for grant in &store.grants {
        connection
            .execute(
                "insert into mcp_grants (
                    client_id, name, scopes_json, allowed_sessions_json, expires_at, revoked_at, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    grant.client_id,
                    grant.name,
                    json_text(&grant.scopes)?,
                    json_text(&grant.allowed_sessions)?,
                    grant.expires_at.map(|value| value.to_rfc3339()),
                    grant.revoked_at.map(|value| value.to_rfc3339()),
                    json_text(grant)?,
                ],
            )
            .map_err(|error| format!("failed to mirror MCP grant {}: {error}", grant.client_id))?;
    }

    for record in &store.audit {
        connection
            .execute(
                "insert into mcp_audit (
                    id, ts, actor, action, session_id, decision, details_json, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    record.id,
                    record.ts.to_rfc3339(),
                    record.actor,
                    record.action,
                    record.session_id,
                    record.decision,
                    json_text(&record.details)?,
                    json_text(record)?,
                ],
            )
            .map_err(|error| format!("failed to mirror MCP audit {}: {error}", record.id))?;
    }

    for mark in &store.timeline {
        connection
            .execute(
                "insert into timeline_marks (
                    id, session_id, ts, label, details, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    mark.id,
                    mark.session_id,
                    mark.ts.to_rfc3339(),
                    mark.label,
                    mark.details,
                    json_text(mark)?,
                ],
            )
            .map_err(|error| format!("failed to mirror timeline mark {}: {error}", mark.id))?;
    }

    for snapshot in &store.sysmon {
        connection
            .execute(
                "insert into sysmon_snapshots (
                    session_id, ts, uptime_seconds, cpu_percent, memory_percent, rx_kbps, tx_kbps, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    snapshot.session_id,
                    snapshot.ts.to_rfc3339(),
                    snapshot.uptime_seconds as i64,
                    snapshot.cpu_percent,
                    snapshot.memory_percent,
                    snapshot.rx_kbps,
                    snapshot.tx_kbps,
                    json_text(snapshot)?,
                ],
            )
            .map_err(|error| {
                format!(
                    "failed to mirror sysmon snapshot {} {}: {error}",
                    snapshot.session_id, snapshot.ts
                )
            })?;
    }

    connection
        .execute(
            "insert into metadata (key, value) values ('schemaVersion', ?1)
                on conflict(key) do update set value = excluded.value",
            params![SQLITE_SCHEMA_VERSION],
        )
        .map_err(|error| format!("failed to update SQLite schema version: {error}"))?;
    Ok(())
}

fn json_text<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value)
        .map_err(|error| format!("failed to encode SQLite JSON mirror: {error}"))
}

fn enum_text<T: Serialize>(value: &T) -> Result<String, String> {
    let value = serde_json::to_value(value)
        .map_err(|error| format!("failed to encode enum for SQLite mirror: {error}"))?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "expected enum to serialize as a string".to_string())
}

fn save_store_json(path: &Path, store: &SessionStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create PortMate data directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let tmp_path = path.with_file_name(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(LEGACY_JSON_STORE_FILE_NAME)
    ));
    let bytes = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("failed to serialize PortMate store: {error}"))?;
    fs::write(&tmp_path, bytes).map_err(|error| {
        format!(
            "failed to write PortMate store {}: {error}",
            tmp_path.display()
        )
    })?;

    if cfg!(windows) && path.exists() {
        fs::remove_file(path).map_err(|error| {
            format!(
                "failed to replace existing PortMate store {}: {error}",
                path.display()
            )
        })?;
    }
    fs::rename(&tmp_path, path).map_err(|error| {
        format!(
            "failed to commit PortMate store {} -> {}: {error}",
            tmp_path.display(),
            path.display()
        )
    })
}

fn persist_store_arc(path: &Path, store: &Arc<Mutex<SessionStore>>) -> Result<(), String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    save_store(path, &store)
}

fn describe_endpoint(profile: &SessionProfile) -> String {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            if ssh.username.is_empty() {
                format!("{}:{}", ssh.endpoint.host, ssh.endpoint.port)
            } else {
                format!(
                    "{}@{}:{}",
                    ssh.username, ssh.endpoint.host, ssh.endpoint.port
                )
            }
        }
        ConnectionConfig::Serial(serial) => serial.port.clone(),
        ConnectionConfig::Shell(shell) => shell.program.clone(),
        ConnectionConfig::Telnet(tcp) | ConnectionConfig::Tcp(tcp) => {
            format!("{}:{}", tcp.host, tcp.port)
        }
    }
}

fn describe_host_key_rejection(evaluation: &HostKeyEvaluation) -> String {
    match evaluation {
        HostKeyEvaluation::Trusted { .. } => "SSH host key 已受信任".to_string(),
        HostKeyEvaluation::Unknown {
            alias,
            port,
            algorithm,
            fingerprint_sha256,
            ..
        } => format!(
            "SSH host key 未受信任: alias={alias}:{port}, algorithm={algorithm}, fingerprint={fingerprint_sha256}"
        ),
        HostKeyEvaluation::Mismatch {
            alias,
            port,
            algorithm,
            expected,
            observed_fingerprint_sha256,
            ..
        } => {
            let expected = expected
                .iter()
                .map(|key| key.fingerprint_sha256.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "SSH host key 已变化，已阻断: alias={alias}:{port}, algorithm={algorithm}, observed={observed_fingerprint_sha256}, expected=[{expected}]"
            )
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            let store_path = data_dir.join(STORE_FILE_NAME);
            let store = load_store(&store_path);
            if let Err(error) = save_store(&store_path, &store) {
                eprintln!("PortMate: failed to initialize persistent store: {error}");
            }
            let state = AppState {
                app_handle: Some(app.handle().clone()),
                store: Arc::new(Mutex::new(store)),
                ssh: Arc::new(Mutex::new(HashMap::new())),
                shell: Arc::new(Mutex::new(HashMap::new())),
                tcp: Arc::new(Mutex::new(HashMap::new())),
                serial: Arc::new(Mutex::new(HashMap::new())),
                tunnels: Arc::new(Mutex::new(HashMap::new())),
                transfer_cancellations: Arc::new(Mutex::new(HashMap::new())),
                transfer_lanes: Arc::new(Mutex::new(HashMap::new())),
                one_time_host_keys: Arc::new(Mutex::new(HashMap::new())),
                store_path,
            };
            start_ipc_server(
                state.clone(),
                data_dir.join("portmate-ipc.json"),
                Uuid::new_v4().to_string(),
            );
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_sessions,
            read_screen,
            tail_log,
            search_logs,
            send_text,
            send_bytes,
            send_key,
            run_command,
            resize_session,
            save_session_profile,
            open_session,
            close_session,
            evaluate_host_key,
            apply_host_key_decision,
            scan_ssh_host_key,
            trust_scanned_host_key,
            import_known_hosts,
            export_known_hosts,
            delete_host_key,
            delete_host_keys,
            update_host_key,
            list_transfers,
            retry_transfer,
            cancel_transfer,
            list_mcp_audit,
            list_mcp_grants,
            save_mcp_grant,
            revoke_mcp_grant,
            mcp_http_config,
            rotate_mcp_http_token,
            list_host_keys,
            list_ssh_agent_identities,
            save_secret,
            delete_secret,
            has_secret,
            list_serial_ports,
            list_tmux_state,
            attach_tmux,
            list_files,
            file_properties,
            create_directory,
            delete_path,
            rename_path,
            chmod_path,
            serial_set_lines,
            serial_send_break,
            refresh_sysmon,
            start_transfer,
            create_tunnel,
            list_tunnels,
            stop_tunnel,
            mcp_manifest
        ])
        .run(tauri::generate_context!())
        .expect("error while running PortMate");
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ChildGuard(Option<std::process::Child>);

    impl ChildGuard {
        fn stop(&mut self) {
            if let Some(mut child) = self.0.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            self.stop();
        }
    }

    #[cfg(unix)]
    fn openssh_test_server_path() -> Option<&'static Path> {
        ["/usr/sbin/sshd", "/usr/local/sbin/sshd"]
            .into_iter()
            .map(Path::new)
            .find(|path| path.exists())
    }

    #[cfg(unix)]
    fn generate_ed25519_test_key(path: &Path) {
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(path)
            .status()
            .unwrap();
        assert!(status.success(), "ssh-keygen failed for {}", path.display());
    }

    #[cfg(unix)]
    fn openssh_test_username() -> String {
        std::env::var("USER").unwrap_or_else(|_| {
            String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout)
                .unwrap()
                .trim()
                .to_string()
        })
    }

    #[cfg(unix)]
    fn write_openssh_test_config(
        config_path: &Path,
        host_key: &Path,
        pid_file: &Path,
        authorized_keys: &Path,
        port: u16,
    ) {
        write_openssh_test_config_with_extra(
            config_path,
            host_key,
            pid_file,
            authorized_keys,
            port,
            "",
        );
    }

    #[cfg(unix)]
    fn write_openssh_test_config_with_extra(
        config_path: &Path,
        host_key: &Path,
        pid_file: &Path,
        authorized_keys: &Path,
        port: u16,
        extra_config: &str,
    ) {
        fs::write(
            config_path,
            format!(
                "AddressFamily inet\nListenAddress 127.0.0.1\nPort {port}\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\nAuthenticationMethods publickey\nPubkeyAuthentication yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nPermitRootLogin prohibit-password\nStrictModes no\nAllowTcpForwarding yes\nLogLevel ERROR\nSubsystem sftp internal-sftp\n{extra_config}",
                host_key.display(),
                pid_file.display(),
                authorized_keys.display(),
            ),
        )
        .unwrap();
    }

    #[cfg(unix)]
    fn spawn_openssh_test_server(sshd_path: &Path, config_path: &Path) -> ChildGuard {
        let child = Command::new(sshd_path)
            .args(["-D", "-e", "-f"])
            .arg(config_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        ChildGuard(Some(child))
    }

    #[cfg(unix)]
    async fn wait_for_openssh_test_server(server: &mut ChildGuard, port: u16, label: &str) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                    break;
                }
                if let Some(status) = server.0.as_mut().unwrap().try_wait().unwrap() {
                    let mut stderr = String::new();
                    server
                        .0
                        .as_mut()
                        .unwrap()
                        .stderr
                        .as_mut()
                        .unwrap()
                        .read_to_string(&mut stderr)
                        .unwrap();
                    panic!("{label} exited early with {status}: {stderr}");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{label} did not start"));
    }

    #[test]
    fn telnet_negotiator_filters_iac_and_replies() {
        let mut negotiator = TelnetNegotiator::new();
        let (output, replies) = negotiator.filter(&[
            b'h',
            b'i',
            TELNET_IAC,
            TELNET_WILL,
            TELNET_OPT_ECHO,
            TELNET_IAC,
            TELNET_DO,
            TELNET_OPT_TERMINAL_TYPE,
        ]);
        assert_eq!(output, b"hi");
        assert_eq!(
            replies,
            vec![
                vec![TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO],
                vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_TERMINAL_TYPE],
            ]
        );
    }

    #[test]
    fn telnet_outbound_text_uses_crlf() {
        assert_eq!(encode_telnet_outbound_text("show\n"), "show\r\n");
        assert_eq!(encode_telnet_outbound_text("show\r\n"), "show\r\n");
        assert_eq!(encode_telnet_outbound_text("ÿ\n"), "ÿ\r\n");
        assert_eq!(
            encode_telnet_outbound_bytes(&[0x01, TELNET_IAC]),
            vec![0x01, TELNET_IAC, TELNET_IAC]
        );
    }

    #[test]
    fn tcp_telnet_loopback_negotiates_and_round_trips_wire_bytes() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                socket
                    .write_all(&[
                        TELNET_IAC,
                        TELNET_WILL,
                        TELNET_OPT_ECHO,
                        b'l',
                        b'o',
                        b'g',
                        b'i',
                        b'n',
                        b':',
                        b' ',
                    ])
                    .await
                    .unwrap();

                let mut negotiation_reply = [0_u8; 3];
                socket.read_exact(&mut negotiation_reply).await.unwrap();
                assert_eq!(negotiation_reply, [TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO]);

                let mut command = [0_u8; 6];
                socket.read_exact(&mut command).await.unwrap();
                assert_eq!(&command, b"show\r\n");

                let mut raw = [0_u8; 3];
                socket.read_exact(&mut raw).await.unwrap();
                assert_eq!(raw, [0x01, TELNET_IAC, TELNET_IAC]);
            });

            let mut client = connect_tcp_socket("127.0.0.1", address.port(), "Telnet")
                .await
                .unwrap();
            let mut incoming = [0_u8; 10];
            client.read_exact(&mut incoming).await.unwrap();
            let mut negotiator = TelnetNegotiator::new();
            let (text, replies) = negotiator.filter(&incoming);
            assert_eq!(text, b"login: ");
            assert_eq!(replies.len(), 1);
            client.write_all(&replies[0]).await.unwrap();
            client
                .write_all(encode_telnet_outbound_text("show\n").as_bytes())
                .await
                .unwrap();
            client
                .write_all(&encode_telnet_outbound_bytes(&[0x01, TELNET_IAC]))
                .await
                .unwrap();

            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("Telnet loopback server timed out")
                .expect("Telnet loopback server task failed");
        });
    }

    #[test]
    fn tcp_loopback_reconnects_after_remote_disconnect() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (second_connected_tx, second_connected_rx) = tokio::sync::oneshot::channel();
            let (release_server_tx, release_server_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (first, _) = listener.accept().await.unwrap();
                drop(first);
                let (second, _) = listener.accept().await.unwrap();
                let _ = second_connected_tx.send(());
                let _ = release_server_rx.await;
                drop(second);
            });

            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: true,
            }));
            let root = std::env::temp_dir().join(format!("portmate-tcp-test-{}", Uuid::new_v4()));
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

            let opened = open_tcp_session(&state, profile.clone()).await.unwrap();
            assert_eq!(opened.runtime.status, SessionStatus::Connected);
            let first_runtime_id = state
                .tcp
                .lock()
                .unwrap()
                .get(&profile.id)
                .unwrap()
                .runtime_id
                .clone();

            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let status = state
                        .store
                        .lock()
                        .unwrap()
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == profile.id)
                        .unwrap()
                        .runtime
                        .status;
                    if status == SessionStatus::Reconnecting {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("TCP runtime never entered reconnecting state");

            tokio::time::timeout(Duration::from_secs(3), second_connected_rx)
                .await
                .expect("TCP runtime did not reconnect")
                .expect("TCP mock server dropped reconnect signal");
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let connected = {
                        let store = state.store.lock().unwrap();
                        store
                            .summaries()
                            .into_iter()
                            .find(|summary| summary.profile.id == profile.id)
                            .is_some_and(|summary| {
                                summary.runtime.status == SessionStatus::Connected
                            })
                    };
                    if connected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("TCP runtime did not return to connected state");

            let runtime = state.tcp.lock().unwrap().remove(&profile.id).unwrap();
            assert_ne!(runtime.runtime_id, first_runtime_id);
            runtime.closed.store(true, Ordering::SeqCst);
            runtime.writer.lock().await.shutdown().await.unwrap();
            let _ = release_server_tx.send(());
            server.await.unwrap();

            let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
            assert!(screen.contains("socket closed; reconnecting"));
            assert!(screen.contains("socket reconnected"));
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn tcp_loopback_round_trips_raw_bytes_without_telnet_escaping() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut raw = [0_u8; 2];
                socket.read_exact(&mut raw).await.unwrap();
                assert_eq!(raw, [0x01, TELNET_IAC]);
            });

            let mut client = connect_tcp_socket("127.0.0.1", address.port(), "TCP")
                .await
                .unwrap();
            client.write_all(&[0x01, TELNET_IAC]).await.unwrap();

            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("TCP loopback server timed out")
                .expect("TCP loopback server task failed");
        });
    }

    #[test]
    fn normalize_loaded_store_preserves_runtime_diagnostics() {
        let mut store = SessionStore::default();
        let profile = test_shell_profile();
        let session_id = profile.id.clone();
        store.upsert_profile(profile);
        let last_activity = Utc::now() - chrono::Duration::minutes(5);
        let last_disconnect = Utc::now() - chrono::Duration::minutes(4);
        let runtime = store
            .runtimes
            .iter_mut()
            .find(|runtime| runtime.session_id == session_id)
            .unwrap();
        runtime.status = SessionStatus::Connected;
        runtime.connected_since = Some(Utc::now() - chrono::Duration::hours(1));
        runtime.pane_id = "custom-pane".to_string();
        runtime.title = "dynamic shell title".to_string();
        runtime.cwd = Some("/tmp/worktree".to_string());
        runtime.last_activity = last_activity;
        runtime.last_disconnect = Some(last_disconnect);
        runtime.last_disconnect_reason = Some("network timeout".to_string());

        let normalized = normalize_loaded_store(store);
        let runtime = normalized
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == session_id)
            .unwrap();

        assert_eq!(runtime.status, SessionStatus::Disconnected);
        assert!(runtime.connected_since.is_none());
        assert_eq!(runtime.active_transport, SessionKind::Shell);
        assert_eq!(runtime.pane_id, "custom-pane");
        assert_eq!(runtime.title, "dynamic shell title");
        assert_eq!(runtime.cwd.as_deref(), Some("/tmp/worktree"));
        assert_eq!(runtime.last_activity, last_activity);
        assert_eq!(runtime.last_disconnect, Some(last_disconnect));
        assert_eq!(
            runtime.last_disconnect_reason.as_deref(),
            Some("network timeout")
        );
    }

    #[test]
    fn tcp_connection_details_validate_endpoint_and_reconnect_flag() {
        let mut profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: " 127.0.0.1 ".to_string(),
            port: 2323,
            reconnect: true,
        }));
        assert_eq!(
            tcp_connection_details(&profile).unwrap(),
            ("127.0.0.1".to_string(), 2323, "TCP")
        );
        assert!(tcp_reconnect_enabled(&profile));

        profile.connection = ConnectionConfig::Telnet(portmate_core::TcpConnection {
            host: "console.lab".to_string(),
            port: 23,
            reconnect: false,
        });
        assert_eq!(
            tcp_connection_details(&profile).unwrap(),
            ("console.lab".to_string(), 23, "Telnet")
        );
        assert!(!tcp_reconnect_enabled(&profile));

        profile.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: " ".to_string(),
            port: 23,
            reconnect: true,
        });
        assert!(tcp_connection_details(&profile)
            .unwrap_err()
            .contains("主机不能为空"));

        profile.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: 0,
            reconnect: true,
        });
        assert!(tcp_connection_details(&profile)
            .unwrap_err()
            .contains("端口不能为空"));
    }

    #[test]
    fn serial_connection_details_validate_port_and_reconnect_flag() {
        let mut profile = test_serial_profile(portmate_core::SerialConnection {
            port: " /dev/ttyUSB0 ".to_string(),
            baud_rate: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".to_string(),
            flow_control: "none".to_string(),
            dtr: true,
            rts: false,
            reconnect: true,
        });
        let (serial, port_name) = serial_connection_details(&profile).unwrap();
        assert_eq!(serial.baud_rate, 115200);
        assert_eq!(port_name, "/dev/ttyUSB0");
        assert!(serial_reconnect_enabled(&profile));

        if let ConnectionConfig::Serial(serial) = &mut profile.connection {
            serial.port = " ".to_string();
            serial.reconnect = false;
        }
        assert!(serial_connection_details(&profile)
            .unwrap_err()
            .contains("串口不能为空"));
        assert!(!serial_reconnect_enabled(&profile));
    }

    #[test]
    fn ssh_reconnect_enabled_reads_ssh_and_tmux_profiles() {
        let mut profile = test_ssh_profile();
        assert!(ssh_reconnect_enabled(&profile));

        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.reconnect = false;
        }
        assert!(!ssh_reconnect_enabled(&profile));

        let ssh = match profile.connection {
            ConnectionConfig::Ssh(ssh) => ssh,
            _ => panic!("expected SSH profile"),
        };
        profile.connection = ConnectionConfig::Tmux(SshConnection {
            reconnect: true,
            ..ssh
        });
        profile.kind = SessionKind::Tmux;
        assert!(ssh_reconnect_enabled(&profile));
    }

    #[test]
    fn ask_every_time_only_accepts_a_one_time_key_id() {
        let mut policy = portmate_core::HostKeyPolicy::profile_alias("bench-device");
        assert!(trusted_host_key_allowed(&policy, "persistent-key", &[]));

        policy.mode = HostKeyMode::AskEveryTime;
        assert!(!trusted_host_key_allowed(
            &policy,
            "persistent-key",
            &["one-time-key".to_string()]
        ));
        assert!(trusted_host_key_allowed(
            &policy,
            "one-time-key",
            &["one-time-key".to_string()]
        ));
    }

    #[test]
    fn jump_endpoint_details_validate_each_hop() {
        let jump = portmate_core::JumpHop {
            host: " bastion-2 ".to_string(),
            port: 2222,
            username: " deploy ".to_string(),
            password_secret_ref: None,
            passphrase_secret_ref: None,
            identity_ref: Some("jump-key".to_string()),
            host_key_policy: None,
        };
        assert_eq!(
            jump_endpoint_details(&jump, 1).unwrap(),
            ("bastion-2".to_string(), 2222, "deploy".to_string())
        );

        let mut invalid = jump.clone();
        invalid.host = " ".to_string();
        assert!(jump_endpoint_details(&invalid, 0)
            .unwrap_err()
            .contains("第 1 跳 主机不能为空"));

        invalid = jump.clone();
        invalid.port = 0;
        assert!(jump_endpoint_details(&invalid, 2)
            .unwrap_err()
            .contains("第 3 跳 端口必须"));

        invalid = jump;
        invalid.username = " ".to_string();
        assert!(jump_endpoint_details(&invalid, 3)
            .unwrap_err()
            .contains("第 4 跳 用户名不能为空"));
    }

    #[test]
    fn jump_ssh_connection_uses_independent_credentials_and_policy() {
        let mut profile = test_ssh_profile();
        let ssh = match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) => ssh,
            _ => panic!("expected SSH profile"),
        };
        ssh.password_secret_ref = Some("keychain:target-password".to_string());
        ssh.passphrase_secret_ref = Some("keychain:target-passphrase".to_string());
        ssh.identity_refs.push(portmate_core::IdentityRef {
            id: "target-key".to_string(),
            label: "target".to_string(),
            source: IdentitySource::SystemFile,
            fingerprint_sha256: None,
            path: Some("~/.ssh/id_target".to_string()),
            secret_ref: None,
        });
        ssh.identity_refs.push(portmate_core::IdentityRef {
            id: "jump-key".to_string(),
            label: "jump".to_string(),
            source: IdentitySource::SystemFile,
            fingerprint_sha256: None,
            path: Some("~/.ssh/id_jump".to_string()),
            secret_ref: None,
        });

        let jump_policy = portmate_core::HostKeyPolicy {
            mode: HostKeyMode::AskEveryTime,
            alias: Some(" bastion-a ".to_string()),
            trust_scope: HostKeyScope::User,
            allow_rotation: true,
            check_ip: true,
        };
        let jump = portmate_core::JumpHop {
            host: " bastion.example ".to_string(),
            port: 2222,
            username: " jumpuser ".to_string(),
            password_secret_ref: Some(" keychain:jump-password ".to_string()),
            passphrase_secret_ref: Some(" keychain:jump-passphrase ".to_string()),
            identity_ref: Some("jump-key".to_string()),
            host_key_policy: Some(jump_policy),
        };

        let policy = jump_host_key_policy(ssh, &jump);
        assert_eq!(policy.mode, HostKeyMode::AskEveryTime);
        assert_eq!(policy.alias.as_deref(), Some("bastion-a"));
        assert_eq!(policy.trust_scope, HostKeyScope::User);
        assert!(policy.allow_rotation);
        assert!(policy.check_ip);

        let jump_ssh = jump_ssh_connection(ssh, &jump, policy.clone());
        assert_eq!(jump_ssh.endpoint.host, "bastion.example");
        assert_eq!(jump_ssh.username, "jumpuser");
        assert_eq!(
            jump_ssh.password_secret_ref.as_deref(),
            Some("keychain:jump-password")
        );
        assert_eq!(
            jump_ssh.passphrase_secret_ref.as_deref(),
            Some("keychain:jump-passphrase")
        );
        assert_eq!(jump_ssh.host_key_policy, policy);
        assert_eq!(jump_ssh.identity_refs.len(), 1);
        assert_eq!(jump_ssh.identity_refs[0].id, "jump-key");
    }

    #[test]
    fn jump_ssh_connection_falls_back_to_parent_credentials_and_policy() {
        let mut profile = test_ssh_profile();
        let ssh = match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) => ssh,
            _ => panic!("expected SSH profile"),
        };
        ssh.password_secret_ref = Some("keychain:target-password".to_string());
        ssh.passphrase_secret_ref = Some("keychain:target-passphrase".to_string());
        ssh.host_key_policy = portmate_core::HostKeyPolicy {
            mode: HostKeyMode::TrustOnFirstUse,
            alias: Some("target-alias".to_string()),
            trust_scope: HostKeyScope::Project,
            allow_rotation: true,
            check_ip: true,
        };

        let jump = portmate_core::JumpHop {
            host: "bastion.example".to_string(),
            port: 22,
            username: "jumpuser".to_string(),
            password_secret_ref: None,
            passphrase_secret_ref: None,
            identity_ref: None,
            host_key_policy: None,
        };

        let policy = jump_host_key_policy(ssh, &jump);
        assert_eq!(policy.mode, HostKeyMode::TrustOnFirstUse);
        assert_eq!(policy.alias.as_deref(), Some("jump:bastion.example:22"));
        assert_eq!(policy.trust_scope, HostKeyScope::Profile);
        assert!(policy.allow_rotation);
        assert!(policy.check_ip);

        let jump_ssh = jump_ssh_connection(ssh, &jump, policy);
        assert_eq!(
            jump_ssh.password_secret_ref.as_deref(),
            Some("keychain:target-password")
        );
        assert_eq!(
            jump_ssh.passphrase_secret_ref.as_deref(),
            Some("keychain:target-passphrase")
        );
    }

    #[test]
    fn jump_runtime_credentials_do_not_override_independent_secret_refs() {
        assert_eq!(
            jump_runtime_credential(Some("target-password"), Some("keychain:jump-password")),
            None
        );
        assert_eq!(
            jump_runtime_credential(Some("target-passphrase"), Some(" keychain:jump-key ")),
            None
        );
        assert_eq!(
            jump_runtime_credential(Some("shared-password"), None).as_deref(),
            Some("shared-password")
        );
        assert_eq!(
            jump_runtime_credential(Some("shared-password"), Some(" ")).as_deref(),
            Some("shared-password")
        );
        assert_eq!(jump_runtime_credential(Some(""), None), None);
    }

    #[test]
    fn empty_mcp_grant_store_requires_trusted_bootstrap() {
        let store = SessionStore::default();
        assert!(!mcp_scope_allowed(
            &store,
            "portmate-local",
            false,
            McpScope::WriteInput,
            "session-1",
        ));
        assert!(mcp_scope_allowed(
            &store,
            "portmate-local",
            true,
            McpScope::WriteInput,
            "session-1",
        ));
        assert!(!mcp_scope_allowed(
            &store,
            "",
            true,
            McpScope::WriteInput,
            "session-1",
        ));
    }

    #[test]
    fn log_query_limit_matches_mcp_schema_bounds() {
        assert_eq!(bounded_log_query_limit(None), 100);
        assert_eq!(bounded_log_query_limit(Some(0)), 1);
        assert_eq!(bounded_log_query_limit(Some(600)), 600);
        assert_eq!(bounded_log_query_limit(Some(u64::MAX)), 1000);
    }

    #[test]
    fn transfer_average_bps_uses_elapsed_time() {
        let started = Utc::now();
        let finished = started + chrono::Duration::seconds(2);
        let task = TransferTask {
            id: "transfer-1".to_string(),
            session_id: "session-1".to_string(),
            protocol: TransferProtocol::Sftp,
            source: "a.bin".to_string(),
            destination: "b.bin".to_string(),
            bytes_total: 2048,
            bytes_done: 2048,
            status: TransferStatus::Completed,
            message: None,
            started_at: Some(started),
            finished_at: Some(finished),
            average_bytes_per_second: None,
        };

        assert_eq!(transfer_average_bps(&task), Some(1024.0));
    }

    #[test]
    fn transfer_active_statuses_cover_queued_and_running_tasks() {
        assert!(transfer_task_is_active(&TransferStatus::Queued));
        assert!(transfer_task_is_active(&TransferStatus::Running));
        assert!(!transfer_task_is_active(&TransferStatus::Completed));
        assert!(!transfer_task_is_active(&TransferStatus::Failed));
        assert!(!transfer_task_is_active(&TransferStatus::Cancelled));
    }

    #[test]
    fn transfer_throttle_delay_respects_rate_limit() {
        assert!(transfer_throttle_delay(None, 1024, Duration::ZERO).is_none());
        assert!(transfer_throttle_delay(Some(0), 1024, Duration::ZERO).is_none());
        assert!(transfer_throttle_delay(Some(1024), 0, Duration::ZERO).is_none());

        assert_eq!(
            transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(1)),
            Some(Duration::from_secs(1))
        );
        assert!(transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(2)).is_none());
        assert!(transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(3)).is_none());
    }

    #[test]
    fn local_resume_part_helpers_keep_stable_offsets() {
        let root = std::env::temp_dir().join(format!("portmate-resume-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("image.bin");
        let part = local_resume_part_path(&target);
        assert_eq!(
            part.file_name().unwrap().to_string_lossy(),
            "image.bin.portmate-part"
        );

        fs::write(&part, b"abc").unwrap();
        assert_eq!(local_resume_offset(&part, 10).unwrap(), 3);
        assert!(part.exists());

        fs::write(&part, b"too-long").unwrap();
        assert_eq!(local_resume_offset(&part, 3).unwrap(), 0);
        assert!(!part.exists());

        fs::write(&part, b"complete").unwrap();
        fs::write(&target, b"old").unwrap();
        finalize_local_resume_file(&part, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"complete");
        assert!(!part.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn local_file_properties_reports_file_metadata() {
        let root = std::env::temp_dir().join(format!("portmate-file-props-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("payload.bin");
        fs::write(&target, b"payload").unwrap();

        let properties = local_file_properties(target.to_str().unwrap()).unwrap();
        assert_eq!(properties.name, "payload.bin");
        assert_eq!(properties.path, target.display().to_string());
        assert!(!properties.remote);
        assert_eq!(properties.kind, "file");
        assert!(properties.is_file);
        assert!(!properties.is_dir);
        assert_eq!(properties.size, 7);
        assert!(properties.modified.is_some());
        #[cfg(unix)]
        assert!(properties.permissions.is_some());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn append_log_bytes_returns_stable_byte_refs() {
        let root = std::env::temp_dir().join(format!("portmate-log-test-{}", Uuid::new_v4()));
        let store_path = root.join("portmate-store.sqlite3");
        let mut profile = test_shell_profile();
        profile.logging.path_template = "../bad/{profile}/{date}/{session}.jsonl".to_string();

        let first = append_log_bytes(&store_path, &profile, "raw", b"abc").unwrap();
        let second = append_log_bytes(&store_path, &profile, "raw", b"de").unwrap();
        let raw_path = log_shard_path(&store_path, &profile, "raw").unwrap();
        let raw = fs::read(&raw_path).unwrap();

        assert_eq!(raw, b"abcde");
        assert!(first.ends_with(":0:3"));
        assert!(second.ends_with(":3:2"));
        assert!(raw_path.starts_with(log_root(&store_path)));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn temporary_host_key_trust_matches_without_persisting() {
        let mut store = SessionStore::default();
        let profile = test_ssh_profile();
        let profile_id = profile.id.clone();
        store.upsert_profile(profile);
        let observation = HostKeyObservation {
            host: "192.0.2.10".to_string(),
            port: 22,
            alias: Some("bench-device".to_string()),
            algorithm: "ssh-ed25519".to_string(),
            public_key_base64: "YWJj".to_string(),
        };

        let key = temporary_trusted_host_key(&store, &profile_id, &observation).unwrap();
        assert!(store.host_keys.keys.is_empty());

        let mut host_keys = store.host_keys.clone();
        host_keys.keys.push(key);
        let policy = match store.profile(&profile_id).unwrap().connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.host_key_policy,
            _ => panic!("expected SSH profile"),
        };
        assert!(matches!(
            host_keys
                .evaluate(&profile_id, &policy, &observation)
                .unwrap(),
            HostKeyEvaluation::Trusted { .. }
        ));
        assert!(one_time_trusts_observation(
            &host_keys.keys,
            &profile_id,
            &policy,
            &observation
        ));

        let mut different_host = observation.clone();
        different_host.host = "192.0.2.11".to_string();
        assert!(!one_time_trusts_observation(
            &host_keys.keys,
            &profile_id,
            &policy,
            &different_host
        ));
    }

    #[test]
    fn update_host_key_edits_store_and_profile_copies() {
        let mut store = SessionStore::default();
        let mut profile = test_ssh_profile();
        let key = portmate_core::TrustedHostKey {
            id: "host-key-1".to_string(),
            profile_id: Some(profile.id.clone()),
            alias: "old-alias".to_string(),
            host: "old-host".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:test".to_string(),
            public_key_base64: "YWJj".to_string(),
            scope: HostKeyScope::Profile,
            label: Some("old label".to_string()),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        };
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.trusted_host_keys.push(key.clone());
        }
        let profile_id = profile.id.clone();
        store.upsert_profile(profile);
        store.host_keys.keys.push(key);

        let next = update_host_key_in_store(
            &mut store,
            HostKeyUpdateRequest {
                key_id: "host-key-1".to_string(),
                profile_id: Some(profile_id.clone()),
                alias: " new-alias ".to_string(),
                host: " new-host ".to_string(),
                port: 2222,
                scope: HostKeyScope::Profile,
                label: Some(" new label ".to_string()),
            },
        )
        .unwrap();

        let edited = next
            .keys
            .iter()
            .find(|key| key.id == "host-key-1")
            .expect("edited host key should remain in store");
        assert_eq!(edited.alias, "new-alias");
        assert_eq!(edited.host, "new-host");
        assert_eq!(edited.port, 2222);
        assert_eq!(edited.profile_id.as_deref(), Some(profile_id.as_str()));
        assert_eq!(edited.label.as_deref(), Some("new label"));

        let saved_profile = store.profile(&profile_id).unwrap();
        let profile_copy = match &saved_profile.connection {
            ConnectionConfig::Ssh(ssh) => ssh.trusted_host_keys.first().unwrap(),
            _ => panic!("expected SSH profile"),
        };
        assert_eq!(profile_copy.alias, "new-alias");
        assert_eq!(profile_copy.host, "new-host");
        assert_eq!(profile_copy.port, 2222);
        assert_eq!(profile_copy.label.as_deref(), Some("new label"));
    }

    #[test]
    fn update_host_key_rejects_invalid_profile_scope() {
        let mut store = SessionStore::default();
        store.host_keys.keys.push(portmate_core::TrustedHostKey {
            id: "host-key-1".to_string(),
            profile_id: None,
            alias: "alias".to_string(),
            host: "host".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:test".to_string(),
            public_key_base64: "YWJj".to_string(),
            scope: HostKeyScope::User,
            label: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        });

        let error = update_host_key_in_store(
            &mut store,
            HostKeyUpdateRequest {
                key_id: "host-key-1".to_string(),
                profile_id: None,
                alias: "alias".to_string(),
                host: "host".to_string(),
                port: 22,
                scope: HostKeyScope::Profile,
                label: None,
            },
        )
        .unwrap_err();
        assert!(error.contains("必须选择 Profile"));
    }

    #[test]
    fn delete_host_keys_removes_global_and_profile_copies() {
        let mut store = SessionStore::default();
        let mut profile = test_ssh_profile();
        let profile_id = profile.id.clone();
        let key_a = portmate_core::TrustedHostKey {
            id: "host-key-a".to_string(),
            profile_id: Some(profile_id.clone()),
            alias: "alias-a".to_string(),
            host: "host-a".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:a".to_string(),
            public_key_base64: "YQ==".to_string(),
            scope: HostKeyScope::Profile,
            label: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        };
        let key_b = portmate_core::TrustedHostKey {
            id: "host-key-b".to_string(),
            profile_id: Some(profile_id.clone()),
            alias: "alias-b".to_string(),
            host: "host-b".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:b".to_string(),
            public_key_base64: "Yg==".to_string(),
            scope: HostKeyScope::Profile,
            label: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        };
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.trusted_host_keys.push(key_a.clone());
            ssh.trusted_host_keys.push(key_b.clone());
        }
        store.upsert_profile(profile);
        store.host_keys.keys.push(key_a);
        store.host_keys.keys.push(key_b);

        let next = delete_host_keys_from_store(&mut store, &["host-key-a".to_string()]);
        assert_eq!(next.keys.len(), 1);
        assert_eq!(next.keys[0].id, "host-key-b");

        let saved_profile = store.profile(&profile_id).unwrap();
        let profile_keys = match &saved_profile.connection {
            ConnectionConfig::Ssh(ssh) => &ssh.trusted_host_keys,
            _ => panic!("expected SSH profile"),
        };
        assert_eq!(profile_keys.len(), 1);
        assert_eq!(profile_keys[0].id, "host-key-b");
    }

    #[test]
    fn one_time_host_key_snapshot_keeps_multi_hop_trust_until_success() {
        let one_time = Arc::new(Mutex::new(HashMap::new()));
        let key = portmate_core::TrustedHostKey {
            id: "one-time-key".to_string(),
            profile_id: Some("ssh-session-1".to_string()),
            alias: "jump:bastion:22".to_string(),
            host: "bastion".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:test".to_string(),
            public_key_base64: "YWJj".to_string(),
            scope: HostKeyScope::Profile,
            label: Some("trust once".to_string()),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        };
        let mut target_key = key.clone();
        target_key.id = "one-time-target-key".to_string();
        target_key.alias = "target:22".to_string();
        target_key.host = "target".to_string();
        remember_one_time_host_key_in(&one_time, "ssh-session-1", key.clone()).unwrap();
        remember_one_time_host_key_in(&one_time, "ssh-session-1", target_key.clone()).unwrap();

        assert_eq!(
            one_time_host_keys_snapshot_from(&one_time, "ssh-session-1").unwrap(),
            vec![key.clone(), target_key.clone()]
        );
        assert_eq!(
            one_time_host_keys_snapshot_from(&one_time, "ssh-session-1").unwrap(),
            vec![key.clone(), target_key.clone()]
        );
        assert_eq!(
            take_one_time_host_keys_from(&one_time, "ssh-session-1").unwrap(),
            vec![key, target_key]
        );
        assert!(one_time_host_keys_snapshot_from(&one_time, "ssh-session-1")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn remote_copy_markers_parse_latest_size_and_done() {
        let output = b"noise\n__PORTMATE_SIZE__1024\nother\n__PORTMATE_DONE__1024\n";
        assert_eq!(
            remote_copy_markers(output),
            RemoteCopyMarkers {
                total: Some(1024),
                resume: None,
                progress: None,
                done: Some(1024)
            }
        );
    }

    #[test]
    fn remote_copy_markers_parse_latest_progress() {
        let output = b"__PORTMATE_SIZE__4096\n__PORTMATE_RESUME__512\n__PORTMATE_PROGRESS__512\n__PORTMATE_PROGRESS__2048\n__PORTMATE_DONE__4096\n";
        assert_eq!(
            remote_copy_markers(output),
            RemoteCopyMarkers {
                total: Some(4096),
                resume: Some(512),
                progress: Some(2048),
                done: Some(4096)
            }
        );
    }

    #[test]
    fn remote_copy_command_polls_progress_and_cleans_background_copy() {
        let command = remote_copy_command("/tmp/source file.bin", "/tmp/o'clock.bin");
        assert!(command.contains("__PORTMATE_RESUME__%s"));
        assert!(command.contains("__PORTMATE_PROGRESS__%s"));
        assert!(command.contains("trap cleanup INT TERM HUP EXIT"));
        assert!(command.contains("kill \"$pid\""));
        assert!(command.contains("remote_name=${src##*/}"));
        assert!(command.contains("case \"$dst\" in */)"));
        assert!(command.contains("part=\"${target%/*}/${target##*/}.portmate-part\""));
        assert!(command.contains("tail -c +$((offset + 1)) -- \"$src\" >> \"$part\""));
        assert!(command.contains("mv -f -- \"$part\" \"$target\""));
        assert!(command.contains("src='/tmp/source file.bin'"));
        assert!(command.contains("dst='/tmp/o'\\''clock.bin'"));
    }

    #[test]
    fn scp_upload_command_uses_resume_receiver() {
        let command = scp_upload_command("/tmp/upload dir/", "local o'clock.bin", 8192);
        assert!(command.contains("dst='/tmp/upload dir/'"));
        assert!(command.contains("source_name='local o'\\''clock.bin'"));
        assert!(command.contains("total=8192"));
        assert!(command.contains("__PORTMATE_RESUME__%s"));
        assert!(command.contains("__PORTMATE_PROGRESS__%s"));
        assert!(command.contains("case \"$dst\" in */)"));
        assert!(command.contains("part=\"${target%/*}/${target##*/}.portmate-part\""));
        assert!(command.contains("cat >> \"$part\" || exit 1"));
        assert!(command.contains("mv -f -- \"$part\" \"$target\""));
        assert!(command.contains("stat -c '__PORTMATE_DONE__%s' -- \"$target\""));
    }

    #[test]
    fn scp_upload_command_resumes_existing_part_file() {
        let root = std::env::temp_dir().join(format!("portmate-scp-upload-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("upload.bin");
        let part = root.join("upload.bin.portmate-part");
        fs::write(&part, b"abc").unwrap();

        let mut destination = root.to_string_lossy().to_string();
        destination.push('/');
        let command = scp_upload_command(&destination, "upload.bin", 6);
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.as_mut().unwrap().write_all(b"def").unwrap();
        drop(child.stdin.take());

        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(&target).unwrap(), b"abcdef");
        assert!(!part.exists());
        let markers = remote_copy_markers(&output.stdout);
        assert_eq!(markers.total, Some(6));
        assert_eq!(markers.resume, Some(3));
        assert_eq!(markers.done, Some(6));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_http_config_uses_bridge_token_ref_and_loopback_endpoint() {
        let config = build_mcp_http_config(true);
        assert_eq!(config.token_ref, MCP_HTTP_TOKEN_REF);
        assert_eq!(config.endpoint, "http://127.0.0.1:8787/mcp");
        assert!(config.token_available);
        assert!(config.start_command.contains("PORTMATE_MCP_HTTP=1"));
    }

    #[test]
    fn tunnel_label_reflects_assigned_local_port() {
        assert_eq!(
            tunnel_label(TunnelMode::Local, "127.0.0.1", 4567, "10.0.0.5", 22),
            "127.0.0.1:4567 -> 10.0.0.5:22"
        );
        assert_eq!(
            tunnel_label(TunnelMode::Dynamic, "127.0.0.1", 1080, "", 0),
            "SOCKS5 127.0.0.1:1080"
        );
    }

    #[test]
    fn tunnel_requests_are_normalized_and_validate_targets_early() {
        let local = normalize_tunnel_request(CreateTunnelRequest {
            session_id: " ssh-session-1 ".to_string(),
            mode: TunnelMode::Local,
            bind_host: " 127.0.0.1 ".to_string(),
            bind_port: 0,
            target_host: " device.internal ".to_string(),
            target_port: 22,
            label: Some("  ".to_string()),
        })
        .unwrap();
        assert_eq!(local.session_id, "ssh-session-1");
        assert_eq!(local.bind_host, "127.0.0.1");
        assert_eq!(local.target_host, "device.internal");
        assert!(local.label.is_none());

        let error = normalize_tunnel_request(CreateTunnelRequest {
            target_host: " ".to_string(),
            target_port: 0,
            ..local.clone()
        })
        .unwrap_err();
        assert!(error.contains("require a target host and port"));

        let dynamic = normalize_tunnel_request(CreateTunnelRequest {
            mode: TunnelMode::Dynamic,
            target_host: "ignored".to_string(),
            target_port: 443,
            ..local
        })
        .unwrap();
        assert!(dynamic.target_host.is_empty());
        assert_eq!(dynamic.target_port, 0);
    }

    #[test]
    fn tunnel_metrics_snapshot_tracks_connections_bytes_and_errors() {
        let metrics = TunnelMetrics::default();
        let spec = TunnelSpec {
            id: "tunnel-1".to_string(),
            label: "127.0.0.1:10022 -> 127.0.0.1:22".to_string(),
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10022,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            enabled: true,
        };

        metrics.connection_opened();
        metrics.add_tcp_to_ssh_bytes(128);
        metrics.add_ssh_to_tcp_bytes(256);
        metrics.record_error("direct-tcpip open failed");
        let active = metrics.snapshot(spec.clone());
        assert_eq!(active.spec.id, spec.id);
        assert_eq!(active.active_connections, 1);
        assert_eq!(active.total_connections, 1);
        assert_eq!(active.tcp_to_ssh_bytes, 128);
        assert_eq!(active.ssh_to_tcp_bytes, 256);
        assert!(active.last_activity.is_some());
        assert_eq!(
            active.last_error.as_deref(),
            Some("direct-tcpip open failed")
        );

        metrics.connection_closed();
        metrics.connection_closed();
        let closed = metrics.snapshot(spec);
        assert_eq!(closed.active_connections, 0);
        assert_eq!(closed.total_connections, 1);
    }

    #[test]
    fn remote_modem_command_uses_raw_tty_and_non_echoing_markers() {
        let token = "modem-token-1";
        let command = modem_remote_command(
            TransferProtocol::Ymodem,
            true,
            "/tmp/transfers/file.bin",
            token,
        );
        assert!(command.contains("stty raw -echo"));
        assert!(command.contains("stty sane"));
        assert!(command.contains("rb -y"));
        assert!(command.contains(token));
        assert!(!command.contains("__PORTMATE_MODEM_modem-token-1_READY__"));
        assert!(!command.contains("__PORTMATE_MODEM_modem-token-1_DONE__"));

        let finalize = xmodem_remote_finalize_command("/tmp/file.bin", 37, token);
        assert!(finalize.contains("truncate -s 37"));
        assert!(finalize.contains("count=37"));
        assert!(finalize.contains("portmate_status"));
        assert!(!finalize.contains(" status="));
        assert!(is_modem_timeout("modem byte timeout"));
        assert!(is_modem_timeout("timed out waiting for modem ACK"));
    }

    #[test]
    fn modem_ready_marker_discards_stale_bytes_and_preserves_handshake() {
        tauri::async_runtime::block_on(async {
            let (tap, receiver) = broadcast::channel(8);
            tap.send(b"old-C-old-__PORTMATE_MODEM_token_REA".to_vec())
                .unwrap();
            tap.send([b"DY__".as_slice(), &[MODEM_CRC_REQUEST]].concat())
                .unwrap();

            let mut reader =
                ModemByteReader::after_marker(receiver, "__PORTMATE_MODEM_token_READY__")
                    .await
                    .unwrap();
            assert_eq!(
                reader.next_byte(Duration::from_millis(10)).await.unwrap(),
                MODEM_CRC_REQUEST
            );
        });
    }

    #[test]
    fn socks5_loopback_parses_domain_connect_request() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_socks5_connect_request(&mut socket).await.unwrap()
            });

            let mut client = TcpStream::connect(address).await.unwrap();
            client.write_all(&[5, 1, 0]).await.unwrap();
            let mut method = [0_u8; 2];
            client.read_exact(&mut method).await.unwrap();
            assert_eq!(method, [5, 0]);

            let domain = b"example.com";
            let mut request = vec![5, 1, 0, 3, domain.len() as u8];
            request.extend_from_slice(domain);
            request.extend_from_slice(&443_u16.to_be_bytes());
            client.write_all(&request).await.unwrap();

            let target = tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("SOCKS5 parser timed out")
                .expect("SOCKS5 parser task failed");
            assert_eq!(target, ("example.com".to_string(), 443));
        });
    }

    #[test]
    fn socks5_loopback_rejects_clients_without_no_auth_method() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_socks5_connect_request(&mut socket).await.unwrap_err()
            });

            let mut client = TcpStream::connect(address).await.unwrap();
            client.write_all(&[5, 1, 2]).await.unwrap();
            let mut method = [0_u8; 2];
            client.read_exact(&mut method).await.unwrap();
            assert_eq!(method, [5, 0xff]);

            let error = tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("SOCKS5 rejection timed out")
                .expect("SOCKS5 rejection task failed");
            assert!(error.contains("did not offer no-authentication"));
        });
    }

    #[test]
    fn socks5_loopback_rejects_non_connect_commands_with_reply() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                read_socks5_connect_request(&mut socket).await.unwrap_err()
            });

            let mut client = TcpStream::connect(address).await.unwrap();
            client.write_all(&[5, 1, 0]).await.unwrap();
            let mut method = [0_u8; 2];
            client.read_exact(&mut method).await.unwrap();
            assert_eq!(method, [5, 0]);
            client.write_all(&[5, 2, 0, 1]).await.unwrap();

            let mut reply = [0_u8; 10];
            client.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply, socks5_reply(7));
            let error = tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("SOCKS5 command rejection timed out")
                .expect("SOCKS5 command rejection task failed");
            assert!(error.contains("only SOCKS5 CONNECT"));
        });
    }

    #[cfg(unix)]
    #[test]
    fn openssh_sftp_scp_and_tunnels_end_to_end() {
        let Some(sshd_path) = openssh_test_server_path() else {
            eprintln!("skipping OpenSSH integration test: sshd is not installed");
            return;
        };
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping OpenSSH integration test: ssh-keygen is not installed");
            return;
        }
        let modem_tools_available = ["rx", "sx", "rb", "sb", "rz", "sz"]
            .into_iter()
            .all(|command| Command::new(command).arg("--version").output().is_ok());

        let root = std::env::temp_dir().join(format!("portmate-sshd-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let host_key = root.join("ssh_host_ed25519_key");
        let replacement_host_key = root.join("ssh_host_ed25519_key_replacement");
        let client_key = root.join("id_ed25519");
        for key_path in [&host_key, &replacement_host_key, &client_key] {
            generate_ed25519_test_key(key_path);
        }
        let authorized_keys = root.join("authorized_keys");
        fs::copy(client_key.with_extension("pub"), &authorized_keys).unwrap();

        let port = {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let username = openssh_test_username();
        let config_path = root.join("sshd_config");
        write_openssh_test_config(
            &config_path,
            &host_key,
            &root.join("sshd.pid"),
            &authorized_keys,
            port,
        );

        let mut sshd = spawn_openssh_test_server(sshd_path, &config_path);

        tauri::async_runtime::block_on(async {
            wait_for_openssh_test_server(&mut sshd, port, "sshd").await;

            let mut profile = test_ssh_profile();
            if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
                ssh.endpoint.host = "127.0.0.1".to_string();
                ssh.endpoint.port = port;
                ssh.username = username.clone();
                ssh.reconnect = false;
                ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
                ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
                ssh.identity_refs = vec![IdentityRef {
                    id: "integration-client-key".to_string(),
                    label: "integration client key".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(client_key.display().to_string()),
                    secret_ref: None,
                }];
                ssh.agent_policy.enabled = false;
                ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
            }
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            let summary = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap();
            assert_eq!(summary.runtime.status, SessionStatus::Connected);
            assert_eq!(summary.profile.connection.kind(), SessionKind::Ssh);
            assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

            send_text_inner(
                state.session_io(),
                profile.id.clone(),
                "printf '__PORTMATE_SSH_OK__\\n'\n".to_string(),
            )
            .await
            .unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if state
                        .store
                        .lock()
                        .unwrap()
                        .screen(&profile.id)
                        .is_some_and(|screen| screen.contains("__PORTMATE_SSH_OK__"))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("SSH PTY command output was not recorded");

            let ssh_handle = state
                .ssh
                .lock()
                .unwrap()
                .get(&profile.id)
                .map(|runtime| Arc::clone(&runtime.handle))
                .unwrap();
            let entries = list_remote_files(ssh_handle, ".").await.unwrap();
            assert!(entries.iter().all(|entry| !entry.name.is_empty()));

            let sftp_root = root.join("sftp-workspace");
            let sftp_nested = sftp_root.join("nested");
            file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: sftp_nested.display().to_string(),
                    remote: true,
                },
                FileOperation::CreateDirectory,
            )
            .await
            .unwrap();
            assert!(sftp_nested.is_dir());

            let sftp_source = root.join("sftp-upload-source.bin");
            let sftp_payload = b"PortMate OpenSSH SFTP integration payload\n";
            fs::write(&sftp_source, sftp_payload).unwrap();
            let uploaded_sftp_file = sftp_nested.join("sftp-upload-source.bin");
            let uploaded_sftp_part = PathBuf::from(remote_resume_part_path(
                uploaded_sftp_file.to_str().unwrap(),
            ));
            fs::write(&uploaded_sftp_part, &sftp_payload[..11]).unwrap();
            let sftp_upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Sftp,
                    source: sftp_source.display().to_string(),
                    destination: format!("remote:{}/", sftp_nested.display()),
                },
            )
            .await
            .unwrap();
            let sftp_upload = wait_for_transfer_terminal_state(&state, &sftp_upload.id).await;
            assert_eq!(
                sftp_upload.status,
                TransferStatus::Completed,
                "SFTP upload failed: {:?}",
                sftp_upload.message
            );
            assert_eq!(sftp_upload.bytes_done, sftp_payload.len() as u64);
            assert!(!uploaded_sftp_part.exists());

            let renamed_sftp_file = sftp_nested.join("renamed.bin");
            rename_path_inner(
                &state,
                RenamePathRequest {
                    session_id: Some(profile.id.clone()),
                    old_path: uploaded_sftp_file.display().to_string(),
                    new_path: renamed_sftp_file.display().to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap();
            chmod_path_inner(
                &state,
                ChmodPathRequest {
                    session_id: Some(profile.id.clone()),
                    path: renamed_sftp_file.display().to_string(),
                    mode: 0o640,
                    remote: true,
                },
            )
            .await
            .unwrap();
            let properties = file_properties_inner(
                &state,
                FilePropertiesRequest {
                    session_id: Some(profile.id.clone()),
                    path: renamed_sftp_file.display().to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap();
            assert!(properties.is_file);
            assert_eq!(properties.size, sftp_payload.len() as u64);
            assert_eq!(properties.permissions.unwrap() & 0o777, 0o640);

            let copied_sftp_file = sftp_root.join("copied.bin");
            let copied_sftp_part =
                PathBuf::from(remote_resume_part_path(copied_sftp_file.to_str().unwrap()));
            fs::write(&copied_sftp_part, &sftp_payload[..13]).unwrap();
            let sftp_copy = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Sftp,
                    source: format!("remote:{}", renamed_sftp_file.display()),
                    destination: format!("remote:{}", copied_sftp_file.display()),
                },
            )
            .await
            .unwrap();
            let sftp_copy = wait_for_transfer_terminal_state(&state, &sftp_copy.id).await;
            assert_eq!(
                sftp_copy.status,
                TransferStatus::Completed,
                "SFTP remote copy failed: {:?}",
                sftp_copy.message
            );
            assert_eq!(sftp_copy.bytes_done, sftp_payload.len() as u64);
            assert_eq!(fs::read(&copied_sftp_file).unwrap(), sftp_payload);
            assert!(!copied_sftp_part.exists());

            let sftp_download_target = root.join("sftp-download-target.bin");
            let sftp_download_part = local_resume_part_path(&sftp_download_target);
            fs::write(&sftp_download_part, &sftp_payload[..17]).unwrap();
            let sftp_download = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Sftp,
                    source: format!("remote:{}", renamed_sftp_file.display()),
                    destination: sftp_download_target.display().to_string(),
                },
            )
            .await
            .unwrap();
            let sftp_download = wait_for_transfer_terminal_state(&state, &sftp_download.id).await;
            assert_eq!(
                sftp_download.status,
                TransferStatus::Completed,
                "SFTP download failed: {:?}",
                sftp_download.message
            );
            assert_eq!(sftp_download.bytes_done, sftp_payload.len() as u64);
            assert_eq!(fs::read(&sftp_download_target).unwrap(), sftp_payload);
            assert!(!sftp_download_part.exists());

            file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: sftp_root.display().to_string(),
                    remote: true,
                },
                FileOperation::Delete,
            )
            .await
            .unwrap();
            assert!(!sftp_root.exists());

            let upload_source = root.join("scp-upload-source.bin");
            let remote_file = root.join("scp-remote.bin");
            let download_target = root.join("scp-download-target.bin");
            let payload = b"PortMate OpenSSH SCP integration payload\n";
            fs::write(&upload_source, payload).unwrap();
            let remote_part = PathBuf::from(remote_resume_part_path(remote_file.to_str().unwrap()));
            fs::write(&remote_part, &payload[..9]).unwrap();
            let upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Scp,
                    source: upload_source.display().to_string(),
                    destination: format!("remote:{}", remote_file.display()),
                },
            )
            .await
            .unwrap();
            let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
            assert_eq!(
                upload.status,
                TransferStatus::Completed,
                "SCP upload failed: {:?}",
                upload.message
            );
            assert_eq!(upload.bytes_done, payload.len() as u64);
            assert_eq!(fs::read(&remote_file).unwrap(), payload);
            assert!(!remote_part.exists());

            let download_part = local_resume_part_path(&download_target);
            fs::write(&download_part, &payload[..15]).unwrap();
            let download = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Scp,
                    source: format!("remote:{}", remote_file.display()),
                    destination: download_target.display().to_string(),
                },
            )
            .await
            .unwrap();
            let download = wait_for_transfer_terminal_state(&state, &download.id).await;
            assert_eq!(
                download.status,
                TransferStatus::Completed,
                "SCP download failed: {:?}",
                download.message
            );
            assert_eq!(download.bytes_done, payload.len() as u64);
            assert_eq!(fs::read(&download_target).unwrap(), payload);
            assert!(!download_part.exists());

            {
                let mut store = state.store.lock().unwrap();
                let mut limited = store.profile(&profile.id).unwrap();
                limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
                store.upsert_profile(limited);
            }
            let cancel_source = root.join("sftp-cancel-source.bin");
            let cancel_remote = root.join("sftp-cancel-remote.bin");
            let cancel_remote_part =
                PathBuf::from(remote_resume_part_path(cancel_remote.to_str().unwrap()));
            let cancel_payload = (0..256 * 1024)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>();
            fs::write(&cancel_source, &cancel_payload).unwrap();
            let cancelled_upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Sftp,
                    source: cancel_source.display().to_string(),
                    destination: format!("remote:{}", cancel_remote.display()),
                },
            )
            .await
            .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let task = state
                        .store
                        .lock()
                        .unwrap()
                        .transfer_by_id(&cancelled_upload.id)
                        .unwrap();
                    if task.status == TransferStatus::Running && task.bytes_done > 0 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("limited SFTP upload did not report progress");
            let cancelling = cancel_transfer_inner(&state, &cancelled_upload.id).unwrap();
            assert_eq!(cancelling.status, TransferStatus::Cancelled);
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if !state
                        .transfer_cancellations
                        .lock()
                        .unwrap()
                        .contains_key(&cancelled_upload.id)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("cancelled SFTP worker did not stop");
            let cancelled = state
                .store
                .lock()
                .unwrap()
                .transfer_by_id(&cancelled_upload.id)
                .unwrap();
            assert_eq!(cancelled.status, TransferStatus::Cancelled);
            assert!(!cancel_remote.exists());
            let partial_size = fs::metadata(&cancel_remote_part).unwrap().len();
            assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

            {
                let mut store = state.store.lock().unwrap();
                let mut unlimited = store.profile(&profile.id).unwrap();
                unlimited.transfer.rate_limit_bytes_per_second = None;
                store.upsert_profile(unlimited);
            }
            let retried = retry_transfer_inner(&state, &cancelled_upload.id)
                .await
                .unwrap();
            let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
            assert_eq!(
                retried.status,
                TransferStatus::Completed,
                "SFTP retry failed: {:?}",
                retried.message
            );
            assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
            assert_eq!(fs::read(&cancel_remote).unwrap(), cancel_payload);
            assert!(!cancel_remote_part.exists());

            {
                let mut store = state.store.lock().unwrap();
                let mut limited = store.profile(&profile.id).unwrap();
                limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
                store.upsert_profile(limited);
            }
            let scp_cancel_source = root.join("scp-cancel-source.bin");
            let scp_cancel_remote = root.join("scp-cancel-remote.bin");
            let scp_cancel_remote_part =
                PathBuf::from(remote_resume_part_path(scp_cancel_remote.to_str().unwrap()));
            fs::write(&scp_cancel_source, &cancel_payload).unwrap();
            let cancelled_scp_upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Scp,
                    source: scp_cancel_source.display().to_string(),
                    destination: format!("remote:{}", scp_cancel_remote.display()),
                },
            )
            .await
            .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let task = state
                        .store
                        .lock()
                        .unwrap()
                        .transfer_by_id(&cancelled_scp_upload.id)
                        .unwrap();
                    if task.status == TransferStatus::Running && task.bytes_done > 0 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("limited SCP upload did not report progress");
            let cancelling = cancel_transfer_inner(&state, &cancelled_scp_upload.id).unwrap();
            assert_eq!(cancelling.status, TransferStatus::Cancelled);
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if !state
                        .transfer_cancellations
                        .lock()
                        .unwrap()
                        .contains_key(&cancelled_scp_upload.id)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("cancelled SCP worker did not stop");
            let cancelled = state
                .store
                .lock()
                .unwrap()
                .transfer_by_id(&cancelled_scp_upload.id)
                .unwrap();
            assert_eq!(cancelled.status, TransferStatus::Cancelled);
            assert!(!scp_cancel_remote.exists());
            let partial_size = fs::metadata(&scp_cancel_remote_part).unwrap().len();
            assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

            {
                let mut store = state.store.lock().unwrap();
                let mut unlimited = store.profile(&profile.id).unwrap();
                unlimited.transfer.rate_limit_bytes_per_second = None;
                store.upsert_profile(unlimited);
            }
            let retried = retry_transfer_inner(&state, &cancelled_scp_upload.id)
                .await
                .unwrap();
            let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
            assert_eq!(
                retried.status,
                TransferStatus::Completed,
                "SCP retry failed: {:?}",
                retried.message
            );
            assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
            assert_eq!(fs::read(&scp_cancel_remote).unwrap(), cancel_payload);
            assert!(!scp_cancel_remote_part.exists());

            if modem_tools_available {
                let zmodem_source = root.join("zmodem-upload-source.bin");
                let zmodem_remote = root.join("zmodem-remote.bin");
                let zmodem_download = root.join("zmodem-download-target.bin");
                let zmodem_payload = b"PortMate ZModem\x00binary\xffpayload\n";
                fs::write(&zmodem_source, zmodem_payload).unwrap();

                let upload = start_transfer_inner(
                    &state,
                    StartTransferRequest {
                        session_id: profile.id.clone(),
                        protocol: TransferProtocol::Zmodem,
                        source: zmodem_source.display().to_string(),
                        destination: format!("remote:{}", zmodem_remote.display()),
                    },
                )
                .await
                .unwrap();
                let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
                assert_eq!(
                    upload.status,
                    TransferStatus::Completed,
                    "ZModem upload failed: {:?}",
                    upload.message
                );
                assert_eq!(upload.bytes_done, zmodem_payload.len() as u64);
                assert_eq!(fs::read(&zmodem_remote).unwrap(), zmodem_payload);

                let download = start_transfer_inner(
                    &state,
                    StartTransferRequest {
                        session_id: profile.id.clone(),
                        protocol: TransferProtocol::Zmodem,
                        source: format!("remote:{}", zmodem_remote.display()),
                        destination: zmodem_download.display().to_string(),
                    },
                )
                .await
                .unwrap();
                let download = wait_for_transfer_terminal_state(&state, &download.id).await;
                assert_eq!(
                    download.status,
                    TransferStatus::Completed,
                    "ZModem download failed: {:?}",
                    download.message
                );
                assert_eq!(download.bytes_done, zmodem_payload.len() as u64);
                assert_eq!(fs::read(&zmodem_download).unwrap(), zmodem_payload);

                let xmodem_source = root.join("xmodem-upload-source.bin");
                let xmodem_remote = root.join("xmodem-remote.bin");
                let xmodem_download = root.join("xmodem-download-target.bin");
                let xmodem_payload = b"PortMate XModem integration payload\n";
                fs::write(&xmodem_source, xmodem_payload).unwrap();
                let upload = start_transfer_inner(
                    &state,
                    StartTransferRequest {
                        session_id: profile.id.clone(),
                        protocol: TransferProtocol::Xmodem,
                        source: xmodem_source.display().to_string(),
                        destination: format!("remote:{}", xmodem_remote.display()),
                    },
                )
                .await
                .unwrap();
                let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
                let xmodem_screen = state
                    .store
                    .lock()
                    .unwrap()
                    .screen(&profile.id)
                    .unwrap_or_default();
                assert_eq!(
                    upload.status,
                    TransferStatus::Completed,
                    "XModem upload failed: {:?}; screen={xmodem_screen:?}",
                    upload.message,
                );
                assert_eq!(upload.bytes_done, xmodem_payload.len() as u64);
                assert_eq!(fs::read(&xmodem_remote).unwrap(), xmodem_payload);

                let download = start_transfer_inner(
                    &state,
                    StartTransferRequest {
                        session_id: profile.id.clone(),
                        protocol: TransferProtocol::Xmodem,
                        source: format!("remote:{}", xmodem_remote.display()),
                        destination: xmodem_download.display().to_string(),
                    },
                )
                .await
                .unwrap();
                let download = wait_for_transfer_terminal_state(&state, &download.id).await;
                assert_eq!(
                    download.status,
                    TransferStatus::Completed,
                    "XModem download failed: {:?}",
                    download.message
                );
                assert_eq!(download.bytes_done, xmodem_payload.len() as u64);
                assert_eq!(fs::read(&xmodem_download).unwrap(), xmodem_payload);

                let ymodem_source = root.join("ymodem-upload-source.bin");
                let ymodem_remote = root.join("ymodem-remote.bin");
                let ymodem_download = root.join("ymodem-download-target.bin");
                let ymodem_payload = b"PortMate YModem\x00binary\xffpayload\n";
                fs::write(&ymodem_source, ymodem_payload).unwrap();
                let upload = start_transfer_inner(
                    &state,
                    StartTransferRequest {
                        session_id: profile.id.clone(),
                        protocol: TransferProtocol::Ymodem,
                        source: ymodem_source.display().to_string(),
                        destination: format!("remote:{}", ymodem_remote.display()),
                    },
                )
                .await
                .unwrap();
                let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
                let ymodem_screen = state
                    .store
                    .lock()
                    .unwrap()
                    .screen(&profile.id)
                    .unwrap_or_default();
                assert_eq!(
                    upload.status,
                    TransferStatus::Completed,
                    "YModem upload failed: {:?}; screen={ymodem_screen:?}",
                    upload.message,
                );
                assert_eq!(upload.bytes_done, ymodem_payload.len() as u64);
                assert_eq!(fs::read(&ymodem_remote).unwrap(), ymodem_payload);

                let download = start_transfer_inner(
                    &state,
                    StartTransferRequest {
                        session_id: profile.id.clone(),
                        protocol: TransferProtocol::Ymodem,
                        source: format!("remote:{}", ymodem_remote.display()),
                        destination: ymodem_download.display().to_string(),
                    },
                )
                .await
                .unwrap();
                let download = wait_for_transfer_terminal_state(&state, &download.id).await;
                assert_eq!(
                    download.status,
                    TransferStatus::Completed,
                    "YModem download failed: {:?}",
                    download.message
                );
                assert_eq!(download.bytes_done, ymodem_payload.len() as u64);
                assert_eq!(fs::read(&ymodem_download).unwrap(), ymodem_payload);
            } else {
                eprintln!("skipping modem OpenSSH coverage: lrzsz tools are not installed");
            }

            let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let echo_address = echo_listener.local_addr().unwrap();
            let echo = tokio::spawn(async move {
                let (mut socket, _) = echo_listener.accept().await.unwrap();
                let mut request = [0_u8; 4];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                socket.write_all(b"pong").await.unwrap();
            });
            let tunnel = create_tunnel_inner(
                &state,
                CreateTunnelRequest {
                    session_id: profile.id.clone(),
                    mode: TunnelMode::Local,
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 0,
                    target_host: "127.0.0.1".to_string(),
                    target_port: echo_address.port(),
                    label: None,
                },
            )
            .await
            .unwrap();
            assert_ne!(tunnel.bind_port, 0);

            let mut tunnel_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
                .await
                .unwrap();
            tunnel_client.write_all(b"ping").await.unwrap();
            let mut response = [0_u8; 4];
            tunnel_client.read_exact(&mut response).await.unwrap();
            assert_eq!(&response, b"pong");
            drop(tunnel_client);
            echo.await.unwrap();

            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let status = list_tunnels_inner(&state, Some(&profile.id))
                        .unwrap()
                        .into_iter()
                        .find(|status| status.spec.id == tunnel.id)
                        .unwrap();
                    if status.active_connections == 0 && status.total_connections == 1 {
                        assert_eq!(status.tcp_to_ssh_bytes, 4);
                        assert_eq!(status.ssh_to_tcp_bytes, 4);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("local tunnel metrics did not settle");
            let stopped = stop_tunnel_inner(&state, &tunnel.id).await.unwrap();
            assert!(!stopped.spec.enabled);

            let dynamic_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let dynamic_echo_address = dynamic_echo_listener.local_addr().unwrap();
            let dynamic_echo = tokio::spawn(async move {
                let (mut socket, _) = dynamic_echo_listener.accept().await.unwrap();
                let mut request = [0_u8; 4];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                socket.write_all(b"pong").await.unwrap();
            });
            let dynamic_tunnel = create_tunnel_inner(
                &state,
                CreateTunnelRequest {
                    session_id: profile.id.clone(),
                    mode: TunnelMode::Dynamic,
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 0,
                    target_host: String::new(),
                    target_port: 0,
                    label: None,
                },
            )
            .await
            .unwrap();
            assert_ne!(dynamic_tunnel.bind_port, 0);

            let mut socks_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
                .await
                .unwrap();
            socks_client.write_all(&[5, 1, 0]).await.unwrap();
            let mut method = [0_u8; 2];
            socks_client.read_exact(&mut method).await.unwrap();
            assert_eq!(method, [5, 0]);
            let [port_high, port_low] = dynamic_echo_address.port().to_be_bytes();
            socks_client
                .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
                .await
                .unwrap();
            let mut socks_reply = [0_u8; 10];
            socks_client.read_exact(&mut socks_reply).await.unwrap();
            assert_eq!(socks_reply, super::socks5_reply(0));
            socks_client.write_all(b"ping").await.unwrap();
            let mut socks_response = [0_u8; 4];
            socks_client.read_exact(&mut socks_response).await.unwrap();
            assert_eq!(&socks_response, b"pong");
            drop(socks_client);
            dynamic_echo.await.unwrap();

            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let status = list_tunnels_inner(&state, Some(&profile.id))
                        .unwrap()
                        .into_iter()
                        .find(|status| status.spec.id == dynamic_tunnel.id)
                        .unwrap();
                    if status.active_connections == 0 && status.total_connections == 1 {
                        assert_eq!(status.tcp_to_ssh_bytes, 4);
                        assert_eq!(status.ssh_to_tcp_bytes, 4);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("dynamic tunnel metrics did not settle");
            let stopped = stop_tunnel_inner(&state, &dynamic_tunnel.id).await.unwrap();
            assert!(!stopped.spec.enabled);

            let remote_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let remote_echo_address = remote_echo_listener.local_addr().unwrap();
            let remote_echo = tokio::spawn(async move {
                let (mut socket, _) = remote_echo_listener.accept().await.unwrap();
                let mut request = [0_u8; 4];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                socket.write_all(b"pong").await.unwrap();
            });
            let remote_tunnel = create_tunnel_inner(
                &state,
                CreateTunnelRequest {
                    session_id: profile.id.clone(),
                    mode: TunnelMode::Remote,
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 0,
                    target_host: "127.0.0.1".to_string(),
                    target_port: remote_echo_address.port(),
                    label: None,
                },
            )
            .await
            .unwrap();
            assert_ne!(remote_tunnel.bind_port, 0);
            assert!(remote_tunnel
                .label
                .contains(&remote_tunnel.bind_port.to_string()));

            let mut remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
                .await
                .unwrap();
            remote_client.write_all(b"ping").await.unwrap();
            let mut remote_response = [0_u8; 4];
            remote_client
                .read_exact(&mut remote_response)
                .await
                .unwrap();
            assert_eq!(&remote_response, b"pong");
            drop(remote_client);
            remote_echo.await.unwrap();

            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let status = list_tunnels_inner(&state, Some(&profile.id))
                        .unwrap()
                        .into_iter()
                        .find(|status| status.spec.id == remote_tunnel.id)
                        .unwrap();
                    if status.active_connections == 0 && status.total_connections == 1 {
                        assert_eq!(status.tcp_to_ssh_bytes, 4);
                        assert_eq!(status.ssh_to_tcp_bytes, 4);
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("remote tunnel metrics did not settle");
            let stopped = stop_tunnel_inner(&state, &remote_tunnel.id).await.unwrap();
            assert!(!stopped.spec.enabled);

            let closed = close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
            assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
            tokio::time::sleep(Duration::from_millis(200)).await;

            sshd.stop();
            write_openssh_test_config(
                &config_path,
                &replacement_host_key,
                &root.join("sshd.pid"),
                &authorized_keys,
                port,
            );
            sshd = spawn_openssh_test_server(sshd_path, &config_path);
            wait_for_openssh_test_server(&mut sshd, port, "replacement sshd").await;

            let trusted_before = state.store.lock().unwrap().host_keys.keys.clone();
            let mismatch = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap_err();
            assert!(mismatch.contains("alias=bench-device"), "{mismatch}");
            assert!(mismatch.contains("observed="), "{mismatch}");
            assert!(mismatch.contains("expected=["), "{mismatch}");
            assert_eq!(state.store.lock().unwrap().host_keys.keys, trusted_before);

            if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
                ssh.host_key_policy.allow_rotation = true;
            }
            state.store.lock().unwrap().upsert_profile(profile.clone());
            let rotated = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap();
            assert_eq!(rotated.runtime.status, SessionStatus::Connected);
            let trusted_after_rotation = state.store.lock().unwrap().host_keys.keys.clone();
            assert_eq!(trusted_after_rotation.len(), 2);
            assert!(trusted_after_rotation
                .iter()
                .all(|key| key.alias == "bench-device" && key.port == port));
            assert_ne!(
                trusted_after_rotation[0].fingerprint_sha256,
                trusted_after_rotation[1].fingerprint_sha256
            );
            close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
        });

        sshd.stop();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn openssh_multi_hop_chain_and_key_mismatch_end_to_end() {
        let Some(sshd_path) = openssh_test_server_path() else {
            eprintln!("skipping OpenSSH Jump Host test: sshd is not installed");
            return;
        };
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping OpenSSH Jump Host test: ssh-keygen is not installed");
            return;
        }

        let root = std::env::temp_dir().join(format!("portmate-jump-sshd-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let jump_one_host_key = root.join("jump_one_host_ed25519_key");
        let jump_two_host_key = root.join("jump_two_host_ed25519_key");
        let replacement_jump_two_host_key = root.join("jump_two_host_ed25519_key_replacement");
        let target_host_key = root.join("target_host_ed25519_key");
        let client_key = root.join("id_ed25519");
        for key_path in [
            &jump_one_host_key,
            &jump_two_host_key,
            &replacement_jump_two_host_key,
            &target_host_key,
            &client_key,
        ] {
            generate_ed25519_test_key(key_path);
        }
        let authorized_keys = root.join("authorized_keys");
        fs::copy(client_key.with_extension("pub"), &authorized_keys).unwrap();

        let jump_one_reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let jump_two_reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let target_reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let jump_one_port = jump_one_reservation.local_addr().unwrap().port();
        let jump_two_port = jump_two_reservation.local_addr().unwrap().port();
        let target_port = target_reservation.local_addr().unwrap().port();
        drop(jump_one_reservation);
        drop(jump_two_reservation);
        drop(target_reservation);

        let jump_one_config = root.join("jump_one_sshd_config");
        let jump_two_config = root.join("jump_two_sshd_config");
        let target_config = root.join("target_sshd_config");
        write_openssh_test_config(
            &jump_one_config,
            &jump_one_host_key,
            &root.join("jump_one_sshd.pid"),
            &authorized_keys,
            jump_one_port,
        );
        write_openssh_test_config(
            &jump_two_config,
            &jump_two_host_key,
            &root.join("jump_two_sshd.pid"),
            &authorized_keys,
            jump_two_port,
        );
        write_openssh_test_config(
            &target_config,
            &target_host_key,
            &root.join("target_sshd.pid"),
            &authorized_keys,
            target_port,
        );
        let mut jump_one_sshd = spawn_openssh_test_server(sshd_path, &jump_one_config);
        let mut jump_two_sshd = spawn_openssh_test_server(sshd_path, &jump_two_config);
        let mut target_sshd = spawn_openssh_test_server(sshd_path, &target_config);

        tauri::async_runtime::block_on(async {
            wait_for_openssh_test_server(&mut jump_one_sshd, jump_one_port, "jump one sshd").await;
            wait_for_openssh_test_server(&mut jump_two_sshd, jump_two_port, "jump two sshd").await;
            wait_for_openssh_test_server(&mut target_sshd, target_port, "target sshd").await;

            let username = openssh_test_username();
            let mut profile = test_ssh_profile();
            if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
                ssh.endpoint.host = "127.0.0.1".to_string();
                ssh.endpoint.port = target_port;
                ssh.username = username.clone();
                ssh.reconnect = false;
                ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
                ssh.host_key_policy.alias = Some("integration-target".to_string());
                ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
                ssh.identity_refs = vec![
                    IdentityRef {
                        id: "target-client-key".to_string(),
                        label: "target client key".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(client_key.display().to_string()),
                        secret_ref: None,
                    },
                    IdentityRef {
                        id: "jump-one-client-key".to_string(),
                        label: "jump one client key".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(client_key.display().to_string()),
                        secret_ref: None,
                    },
                    IdentityRef {
                        id: "jump-two-client-key".to_string(),
                        label: "jump two client key".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(client_key.display().to_string()),
                        secret_ref: None,
                    },
                ];
                ssh.agent_policy.enabled = false;
                ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
                let mut jump_one_policy =
                    portmate_core::HostKeyPolicy::profile_alias("integration-jump-1");
                jump_one_policy.mode = HostKeyMode::TrustOnFirstUse;
                let mut jump_two_policy =
                    portmate_core::HostKeyPolicy::profile_alias("integration-jump-2");
                jump_two_policy.mode = HostKeyMode::TrustOnFirstUse;
                ssh.jumps = vec![
                    portmate_core::JumpHop {
                        host: "127.0.0.1".to_string(),
                        port: jump_one_port,
                        username: username.clone(),
                        password_secret_ref: None,
                        passphrase_secret_ref: None,
                        identity_ref: Some("jump-one-client-key".to_string()),
                        host_key_policy: Some(jump_one_policy),
                    },
                    portmate_core::JumpHop {
                        host: "127.0.0.1".to_string(),
                        port: jump_two_port,
                        username: username.clone(),
                        password_secret_ref: None,
                        passphrase_secret_ref: None,
                        identity_ref: Some("jump-two-client-key".to_string()),
                        host_key_policy: Some(jump_two_policy),
                    },
                ];
            }

            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            let summary = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap();
            assert_eq!(summary.runtime.status, SessionStatus::Connected);
            assert_eq!(
                state
                    .ssh
                    .lock()
                    .unwrap()
                    .get(&profile.id)
                    .unwrap()
                    .jump_handles
                    .len(),
                2
            );
            let trusted = state.store.lock().unwrap().host_keys.keys.clone();
            assert_eq!(trusted.len(), 3);
            assert!(trusted
                .iter()
                .any(|key| key.alias == "integration-jump-1" && key.port == jump_one_port));
            assert!(trusted
                .iter()
                .any(|key| key.alias == "integration-jump-2" && key.port == jump_two_port));
            assert!(trusted
                .iter()
                .any(|key| key.alias == "integration-target" && key.port == target_port));

            send_text_inner(
                state.session_io(),
                profile.id.clone(),
                "printf '__PORTMATE_JUMP_OK__\\n'\n".to_string(),
            )
            .await
            .unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if state
                        .store
                        .lock()
                        .unwrap()
                        .screen(&profile.id)
                        .is_some_and(|screen| screen.contains("__PORTMATE_JUMP_OK__"))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("Jump Host PTY command output was not recorded");

            close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
            jump_two_sshd.stop();
            write_openssh_test_config(
                &jump_two_config,
                &replacement_jump_two_host_key,
                &root.join("jump_two_sshd.pid"),
                &authorized_keys,
                jump_two_port,
            );
            jump_two_sshd = spawn_openssh_test_server(sshd_path, &jump_two_config);
            wait_for_openssh_test_server(
                &mut jump_two_sshd,
                jump_two_port,
                "replacement jump two sshd",
            )
            .await;

            let trusted_before = state.store.lock().unwrap().host_keys.keys.clone();
            let mismatch = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap_err();
            assert!(mismatch.contains("alias=integration-jump-2"), "{mismatch}");
            assert!(mismatch.contains("observed="), "{mismatch}");
            assert!(mismatch.contains("expected=["), "{mismatch}");
            assert_eq!(state.store.lock().unwrap().host_keys.keys, trusted_before);
        });

        jump_one_sshd.stop();
        jump_two_sshd.stop();
        target_sshd.stop();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn openssh_identity_order_respects_max_auth_tries() {
        let Some(sshd_path) = openssh_test_server_path() else {
            eprintln!("skipping OpenSSH identity-order test: sshd is not installed");
            return;
        };
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping OpenSSH identity-order test: ssh-keygen is not installed");
            return;
        }

        let root =
            std::env::temp_dir().join(format!("portmate-auth-order-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let host_key = root.join("ssh_host_ed25519_key");
        let accepted_key = root.join("accepted_ed25519_key");
        let rejected_key_one = root.join("rejected_one_ed25519_key");
        let rejected_key_two = root.join("rejected_two_ed25519_key");
        for key_path in [
            &host_key,
            &accepted_key,
            &rejected_key_one,
            &rejected_key_two,
        ] {
            generate_ed25519_test_key(key_path);
        }
        let authorized_keys = root.join("authorized_keys");
        fs::copy(accepted_key.with_extension("pub"), &authorized_keys).unwrap();

        let port = {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let config_path = root.join("sshd_config");
        write_openssh_test_config_with_extra(
            &config_path,
            &host_key,
            &root.join("sshd.pid"),
            &authorized_keys,
            port,
            "MaxAuthTries 2\n",
        );
        let mut sshd = spawn_openssh_test_server(sshd_path, &config_path);

        tauri::async_runtime::block_on(async {
            wait_for_openssh_test_server(&mut sshd, port, "identity-order sshd").await;

            let mut profile = test_ssh_profile();
            if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
                ssh.endpoint.host = "127.0.0.1".to_string();
                ssh.endpoint.port = port;
                ssh.username = openssh_test_username();
                ssh.reconnect = false;
                ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
                ssh.host_key_policy.alias = Some("identity-order-target".to_string());
                ssh.identity_policy.identities_only = true;
                ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
                ssh.identity_refs = vec![
                    IdentityRef {
                        id: "rejected-key-one".to_string(),
                        label: "rejected key one".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(rejected_key_one.display().to_string()),
                        secret_ref: None,
                    },
                    IdentityRef {
                        id: "rejected-key-two".to_string(),
                        label: "rejected key two".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(rejected_key_two.display().to_string()),
                        secret_ref: None,
                    },
                    IdentityRef {
                        id: "accepted-key".to_string(),
                        label: "accepted key".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(accepted_key.display().to_string()),
                        secret_ref: None,
                    },
                ];
                ssh.agent_policy.enabled = false;
                ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
            }
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

            let exhausted = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap_err();
            assert!(
                exhausted.contains("认证失败") || exhausted.contains("authentication"),
                "{exhausted}"
            );
            assert!(exhausted.contains("rejected key one"), "{exhausted}");
            assert!(exhausted.contains("rejected key two"), "{exhausted}");
            assert!(exhausted.contains("accepted key"), "{exhausted}");
            assert!(state.store.lock().unwrap().host_keys.keys.is_empty());

            if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
                ssh.identity_refs.rotate_right(1);
            }
            state.store.lock().unwrap().upsert_profile(profile.clone());
            let connected = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap();
            assert_eq!(connected.runtime.status, SessionStatus::Connected);
            assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);
            close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
        });

        sshd.stop();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn serial_socat_loopback_round_trips_binary_bytes() {
        if Command::new("socat").arg("-V").output().is_err() {
            eprintln!("skipping serial integration test: socat is not installed");
            return;
        }
        let root = std::env::temp_dir().join(format!("portmate-serial-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let portmate_pty = root.join("portmate.pty");
        let peer_pty = root.join("peer.pty");
        let child = Command::new("socat")
            .args(["-d", "-d"])
            .arg(format!("pty,raw,echo=0,link={}", portmate_pty.display()))
            .arg(format!("pty,raw,echo=0,link={}", peer_pty.display()))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut socat = ChildGuard(Some(child));

        tauri::async_runtime::block_on(async {
            tokio::time::timeout(Duration::from_secs(3), async {
                while !portmate_pty.exists() || !peer_pty.exists() {
                    if let Some(status) = socat.0.as_mut().unwrap().try_wait().unwrap() {
                        panic!("socat exited before creating PTYs: {status}");
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("socat did not create PTYs");

            let profile = test_serial_profile(portmate_core::SerialConnection {
                port: portmate_pty.display().to_string(),
                baud_rate: 115_200,
                data_bits: 8,
                stop_bits: 1,
                parity: "none".to_string(),
                flow_control: "none".to_string(),
                dtr: false,
                rts: false,
                reconnect: false,
            });
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            let opened = open_serial_session(&state, profile.clone()).unwrap();
            assert_eq!(opened.runtime.status, SessionStatus::Connected);
            let mut inbound = state
                .serial
                .lock()
                .unwrap()
                .get(&profile.id)
                .unwrap()
                .tap
                .subscribe();
            let mut peer = serialport::new(peer_pty.display().to_string(), 115_200)
                .timeout(Duration::from_secs(2))
                .open()
                .unwrap();

            let outbound = vec![0xff, 0x00, 0x80];
            send_bytes_inner(state.session_io(), profile.id.clone(), outbound.clone())
                .await
                .unwrap();
            let mut peer_received = [0_u8; 3];
            peer.read_exact(&mut peer_received).unwrap();
            assert_eq!(peer_received, outbound.as_slice());

            let peer_reply = [0x41, 0x00, 0xff, 0x42];
            peer.write_all(&peer_reply).unwrap();
            peer.flush().unwrap();
            let received = tokio::time::timeout(Duration::from_secs(3), inbound.recv())
                .await
                .expect("serial runtime did not receive loopback bytes")
                .expect("serial runtime tap closed");
            assert_eq!(received, peer_reply);

            let closed = close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
            assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        socat.stop();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stopping_tunnel_marks_profile_tunnel_disabled() {
        let mut store = SessionStore::default();
        let mut profile = test_ssh_profile();
        let tunnel = TunnelSpec {
            id: "tunnel-1".to_string(),
            label: "127.0.0.1:10022 -> 127.0.0.1:22".to_string(),
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10022,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            enabled: true,
        };
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.tunnels.push(tunnel.clone());
        }
        store.upsert_profile(profile);

        let mut stopped = tunnel;
        stopped.enabled = false;
        mark_tunnel_stopped_in_store(&mut store, "ssh-session-1", &stopped);

        let saved = match store.profile("ssh-session-1").unwrap().connection {
            ConnectionConfig::Ssh(ssh) => ssh.tunnels,
            _ => panic!("expected SSH profile"),
        };
        assert_eq!(saved.len(), 1);
        assert!(!saved[0].enabled);
    }

    fn test_shell_profile() -> SessionProfile {
        SessionProfile {
            id: "session:1".to_string(),
            name: "Bench/Device".to_string(),
            kind: SessionKind::Shell,
            group: "Lab".to_string(),
            tags: Vec::new(),
            connection: ConnectionConfig::Shell(portmate_core::ShellConnection {
                program: "sh".to_string(),
                args: Vec::new(),
                cwd: None,
            }),
            terminal: portmate_core::TerminalSettings::default(),
            logging: portmate_core::LoggingSettings::default(),
            triggers: Vec::new(),
            transfer: portmate_core::TransferSettings::default(),
        }
    }

    fn test_app_state(profile: SessionProfile, store_path: PathBuf) -> AppState {
        let mut store = SessionStore::default();
        store.upsert_profile(profile);
        AppState {
            app_handle: None,
            store: Arc::new(Mutex::new(store)),
            ssh: Arc::new(Mutex::new(HashMap::new())),
            shell: Arc::new(Mutex::new(HashMap::new())),
            tcp: Arc::new(Mutex::new(HashMap::new())),
            serial: Arc::new(Mutex::new(HashMap::new())),
            tunnels: Arc::new(Mutex::new(HashMap::new())),
            transfer_cancellations: Arc::new(Mutex::new(HashMap::new())),
            transfer_lanes: Arc::new(Mutex::new(HashMap::new())),
            one_time_host_keys: Arc::new(Mutex::new(HashMap::new())),
            store_path,
        }
    }

    async fn wait_for_transfer_terminal_state(state: &AppState, task_id: &str) -> TransferTask {
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let task = state.store.lock().unwrap().transfer_by_id(task_id).unwrap();
                if matches!(
                    task.status,
                    TransferStatus::Completed | TransferStatus::Failed | TransferStatus::Cancelled
                ) {
                    break task;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("transfer did not reach a terminal state")
    }

    fn test_tcp_profile(connection: ConnectionConfig) -> SessionProfile {
        SessionProfile {
            id: "tcp-session-1".to_string(),
            name: "Bench TCP".to_string(),
            kind: connection.kind(),
            group: "Lab".to_string(),
            tags: Vec::new(),
            connection,
            terminal: portmate_core::TerminalSettings::default(),
            logging: portmate_core::LoggingSettings::default(),
            triggers: Vec::new(),
            transfer: portmate_core::TransferSettings::default(),
        }
    }

    fn test_serial_profile(serial: portmate_core::SerialConnection) -> SessionProfile {
        SessionProfile {
            id: "serial-session-1".to_string(),
            name: "Bench Serial".to_string(),
            kind: SessionKind::Serial,
            group: "Lab".to_string(),
            tags: Vec::new(),
            connection: ConnectionConfig::Serial(serial),
            terminal: portmate_core::TerminalSettings::default(),
            logging: portmate_core::LoggingSettings::default(),
            triggers: Vec::new(),
            transfer: portmate_core::TransferSettings::default(),
        }
    }

    fn test_ssh_profile() -> SessionProfile {
        SessionProfile {
            id: "ssh-session-1".to_string(),
            name: "Bench SSH".to_string(),
            kind: SessionKind::Ssh,
            group: "Lab".to_string(),
            tags: Vec::new(),
            connection: ConnectionConfig::Ssh(SshConnection {
                endpoint: portmate_core::HostEndpoint {
                    host: "192.0.2.10".to_string(),
                    port: 22,
                },
                username: "root".to_string(),
                reconnect: true,
                password_secret_ref: None,
                passphrase_secret_ref: None,
                host_key_policy: portmate_core::HostKeyPolicy::profile_alias("bench-device"),
                trusted_host_keys: Vec::new(),
                identity_policy: portmate_core::IdentityPolicy::default(),
                identity_refs: Vec::new(),
                agent_policy: portmate_core::AgentPolicy::default(),
                jumps: Vec::new(),
                tunnels: Vec::new(),
            }),
            terminal: portmate_core::TerminalSettings::default(),
            logging: portmate_core::LoggingSettings::default(),
            triggers: Vec::new(),
            transfer: portmate_core::TransferSettings::default(),
        }
    }
}

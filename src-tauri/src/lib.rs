use chrono::Utc;
use keyring_core::Entry;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use portmate_core::{
    compute_ssh_sha256_fingerprint, prompt_templates, resource_templates, tool_definitions,
    AuthMethod, ConnectionConfig, EventDirection, EventStream, HostKeyDecision, HostKeyEvaluation,
    HostKeyMode, HostKeyObservation, HostKeyStore, IdentityRef, IdentitySource, McpGrant,
    SessionEvent, SessionKind, SessionProfile, SessionStatus, SessionStore, SessionSummary,
    SshConnection, SysmonSnapshot, TimelineMark, TransferProtocol, TransferStatus, TransferTask,
    TriggerAction, TunnelMode, TunnelSpec,
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
use std::fs;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<SessionStore>>,
    ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
    tunnels: Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    store_path: PathBuf,
}

struct SshRuntime {
    runtime_id: String,
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    writer: Arc<tokio::sync::Mutex<ChannelWriteHalf<client::Msg>>>,
    tap: broadcast::Sender<Vec<u8>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelSpec>>>,
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
}

struct SerialRuntime {
    runtime_id: String,
    writer: Arc<Mutex<Box<dyn serialport::SerialPort>>>,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
}

struct TunnelRuntime {
    session_id: String,
    closed: Arc<AtomicBool>,
}

#[derive(Clone)]
struct RuntimeRegistry {
    ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
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
}

#[derive(Debug)]
struct PortMateSshHandler {
    profile_id: String,
    host: String,
    port: u16,
    alias: Option<String>,
    policy: portmate_core::HostKeyPolicy,
    host_keys: HostKeyStore,
    observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    host_key_error: Arc<Mutex<Option<String>>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelSpec>>>,
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

        let evaluation = self
            .host_keys
            .evaluate(&self.profile_id, &self.policy, &observation);
        let accepted = match evaluation {
            Ok(HostKeyEvaluation::Trusted {
                fingerprint_sha256, ..
            }) => {
                *self
                    .observed_key
                    .lock()
                    .expect("host key observation lock poisoned") = Some(observation);
                *self
                    .host_key_error
                    .lock()
                    .expect("host key error lock poisoned") = None;
                let _ = fingerprint_sha256;
                true
            }
            Ok(HostKeyEvaluation::Unknown {
                alias,
                fingerprint_sha256,
                ..
            }) if self.policy.mode == HostKeyMode::TrustOnFirstUse => {
                *self
                    .observed_key
                    .lock()
                    .expect("host key observation lock poisoned") = Some(observation);
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
            let spec = {
                let forwards = forwards
                    .lock()
                    .expect("remote forward target map lock poisoned");
                let key = remote_forward_key(&connected_address, connected_port as u16);
                forwards
                    .get(&key)
                    .or_else(|| forwards.get(&remote_forward_port_key(connected_port as u16)))
                    .cloned()
            };
            if let Some(spec) = spec {
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = handle_remote_tunnel_client(
                        channel,
                        spec,
                        originator_address,
                        originator_port as u16,
                    )
                    .await
                    {
                        eprintln!("PortMate: remote SSH tunnel client failed: {error}");
                    }
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
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<String>,
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
    limit: Option<usize>,
) -> Result<Vec<SessionEvent>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.tail_log(&session_id, limit.unwrap_or(100)))
}

#[tauri::command]
fn search_logs(
    state: State<'_, AppState>,
    query: String,
    session_id: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<SessionEvent>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.search_logs(&query, session_id.as_deref(), limit.unwrap_or(100)))
}

#[tauri::command]
async fn send_text(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<SessionEvent, String> {
    send_text_inner(
        state.inner().store.clone(),
        state.inner().ssh.clone(),
        state.inner().shell.clone(),
        state.inner().tcp.clone(),
        state.inner().serial.clone(),
        state.inner().store_path.clone(),
        session_id,
        text,
    )
    .await
}

#[tauri::command]
async fn send_bytes(
    state: State<'_, AppState>,
    session_id: String,
    bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    send_bytes_inner(
        state.inner().store.clone(),
        state.inner().ssh.clone(),
        state.inner().shell.clone(),
        state.inner().tcp.clone(),
        state.inner().serial.clone(),
        state.inner().store_path.clone(),
        session_id,
        bytes,
    )
    .await
}

#[tauri::command]
async fn send_key(
    state: State<'_, AppState>,
    session_id: String,
    key: String,
) -> Result<SessionEvent, String> {
    let text = terminal_key_sequence(&key)?;
    send_text_inner(
        state.inner().store.clone(),
        state.inner().ssh.clone(),
        state.inner().shell.clone(),
        state.inner().tcp.clone(),
        state.inner().serial.clone(),
        state.inner().store_path.clone(),
        session_id,
        text,
    )
    .await
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
    send_text_inner(
        state.inner().store.clone(),
        state.inner().ssh.clone(),
        state.inner().shell.clone(),
        state.inner().tcp.clone(),
        state.inner().serial.clone(),
        state.inner().store_path.clone(),
        session_id,
        text,
    )
    .await
}

async fn send_text_inner(
    store: Arc<Mutex<SessionStore>>,
    ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
    store_path: PathBuf,
    session_id: String,
    text: String,
) -> Result<SessionEvent, String> {
    let wire_text = outbound_text_for_session(&store, &session_id, &text)?;
    write_session_bytes(
        &store,
        &ssh,
        &shell,
        &tcp,
        &serial,
        &session_id,
        wire_text.as_bytes(),
    )
    .await?;

    let mut store = store.lock().map_err(|error| error.to_string())?;
    let event = store.send_text("desktop-user", &session_id, &text)?;
    save_store(&store_path, &store)?;
    Ok(event)
}

async fn send_bytes_inner(
    store: Arc<Mutex<SessionStore>>,
    ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
    store_path: PathBuf,
    session_id: String,
    bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    write_session_bytes(&store, &ssh, &shell, &tcp, &serial, &session_id, &bytes).await?;

    let mut store = store.lock().map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let event = store.send_text("desktop-user", &session_id, &text)?;
    save_store(&store_path, &store)?;
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
                let writer = {
                    let connections = serial.lock().map_err(|error| error.to_string())?;
                    connections
                        .get(session_id)
                        .map(|runtime| Arc::clone(&runtime.writer))
                };
                if let Some(writer) = writer {
                    let mut writer = writer.lock().map_err(|error| error.to_string())?;
                    writer
                        .write_all(bytes)
                        .map_err(|error| format!("串口写入失败: {error}"))?;
                    writer
                        .flush()
                        .map_err(|error| format!("串口刷新失败: {error}"))?;
                } else if profile_requires_runtime(store, session_id)? {
                    return Err("会话尚未连接，无法发送输入".to_string());
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

fn encode_telnet_outbound_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut previous = '\0';
    for ch in text.chars() {
        match ch {
            '\n' if previous != '\r' => output.push_str("\r\n"),
            '\u{00ff}' => output.push_str("\u{00ff}\u{00ff}"),
            _ => output.push(ch),
        }
        previous = ch;
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
    let summary = store.upsert_profile(profile);
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
        profile
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
        let handle = runtime.handle.lock().await;
        let _ = handle
            .disconnect(Disconnect::ByApplication, "PortMate close_session", "en")
            .await;
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
) -> Result<HostKeyScanResult, String> {
    scan_ssh_host_key_inner(state.inner(), profile).await
}

#[tauri::command]
fn trust_scanned_host_key(
    state: State<'_, AppState>,
    request: TrustScannedHostKeyRequest,
) -> Result<Option<portmate_core::TrustedHostKey>, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let profile_id = request.profile.id.clone();
    store.upsert_profile(request.profile);
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
    store.host_keys.keys.retain(|key| key.id != key_id);
    for profile in &mut store.profiles {
        if let ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) = &mut profile.connection {
            ssh.trusted_host_keys.retain(|key| key.id != key_id);
        }
    }
    let host_keys = store.host_keys.clone();
    save_store(&state.store_path, &store)?;
    Ok(host_keys)
}

#[tauri::command]
fn list_transfers(state: State<'_, AppState>) -> Result<Vec<TransferTask>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.transfers.clone())
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
    send_text_inner(
        Arc::clone(&state.store),
        Arc::clone(&state.ssh),
        Arc::clone(&state.shell),
        Arc::clone(&state.tcp),
        Arc::clone(&state.serial),
        state.store_path.clone(),
        session_id,
        command,
    )
    .await
}

#[tauri::command]
async fn list_files(
    state: State<'_, AppState>,
    request: ListFilesRequest,
) -> Result<Vec<FileEntry>, String> {
    list_files_inner(state.inner(), request).await
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
        connections
            .get(&request.session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    }
    .ok_or_else(|| "串口会话尚未连接".to_string())?;

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
        connections
            .get(&session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    }
    .ok_or_else(|| "串口会话尚未连接".to_string())?;

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

async fn handle_ipc_client(state: AppState, token: String, mut stream: TcpStream) {
    let mut raw = Vec::new();
    let response = match stream.read_to_end(&mut raw).await {
        Ok(_) => match serde_json::from_slice::<IpcRequest>(&raw) {
            Ok(request) if request.token == token => {
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
            let screen = store.screen(&session_id).unwrap_or_default();
            Ok(serde_json::json!(screen))
        }
        "tail_log" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let limit = request
                .args
                .get("limit")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(100) as usize;
            let store = state.store.lock().map_err(|error| error.to_string())?;
            serde_json::to_value(store.tail_log(&session_id, limit))
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
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(100) as usize;
            let store = state.store.lock().map_err(|error| error.to_string())?;
            serde_json::to_value(store.search_logs(&query, session_id, limit))
                .map_err(|error| error.to_string())
        }
        "send_text" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let text = ipc_string_arg(&request.args, "text")?.to_string();
            let event = send_text_inner(
                Arc::clone(&state.store),
                Arc::clone(&state.ssh),
                Arc::clone(&state.shell),
                Arc::clone(&state.tcp),
                Arc::clone(&state.serial),
                state.store_path.clone(),
                session_id,
                text,
            )
            .await?;
            serde_json::to_value(event).map_err(|error| error.to_string())
        }
        "send_key" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let key = ipc_string_arg(&request.args, "key")?.to_string();
            let text = terminal_key_sequence(&key)?;
            let event = send_text_inner(
                Arc::clone(&state.store),
                Arc::clone(&state.ssh),
                Arc::clone(&state.shell),
                Arc::clone(&state.tcp),
                Arc::clone(&state.serial),
                state.store_path.clone(),
                session_id,
                text,
            )
            .await?;
            serde_json::to_value(event).map_err(|error| error.to_string())
        }
        "run_command" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let mut text = ipc_string_arg(&request.args, "command")?.to_string();
            if !text.ends_with('\n') && !text.ends_with('\r') {
                text.push('\n');
            }
            let event = send_text_inner(
                Arc::clone(&state.store),
                Arc::clone(&state.ssh),
                Arc::clone(&state.shell),
                Arc::clone(&state.tcp),
                Arc::clone(&state.serial),
                state.store_path.clone(),
                session_id,
                text,
            )
            .await?;
            serde_json::to_value(event).map_err(|error| error.to_string())
        }
        "open_session" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
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
            let summary = close_session_inner(&state, session_id).await?;
            serde_json::to_value(summary).map_err(|error| error.to_string())
        }
        "start_transfer" => {
            let transfer = serde_json::from_value::<StartTransferRequest>(request.args)
                .map_err(|error| format!("invalid transfer request: {error}"))?;
            let task = start_transfer_inner(&state, transfer).await?;
            serde_json::to_value(task).map_err(|error| error.to_string())
        }
        "create_tunnel" => {
            let tunnel = serde_json::from_value::<CreateTunnelRequest>(request.args)
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
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
            let command = format!(
                "tmux switch-client -t {} || tmux attach -t {} || tmux new-session -A -s {}\r",
                shell_quote(&target),
                shell_quote(&target),
                shell_quote(&target)
            );
            let event = send_text_inner(
                Arc::clone(&state.store),
                Arc::clone(&state.ssh),
                Arc::clone(&state.shell),
                Arc::clone(&state.tcp),
                Arc::clone(&state.serial),
                state.store_path.clone(),
                session_id,
                command,
            )
            .await?;
            serde_json::to_value(event).map_err(|error| error.to_string())
        }
        other => Err(format!("unsupported IPC command: {other}")),
    }
}

async fn scan_ssh_host_key_inner(
    state: &AppState,
    profile: SessionProfile,
) -> Result<HostKeyScanResult, String> {
    let ssh = match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.clone(),
        _ => return Err("profile is not SSH-backed".to_string()),
    };
    let host = ssh.endpoint.host.trim().to_string();
    if host.is_empty() {
        return Err("SSH 主机不能为空".to_string());
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
    let evaluation = host_keys
        .evaluate(&profile.id, &ssh.host_key_policy, &observation)
        .map_err(|error| error.to_string())?;
    Ok(HostKeyScanResult {
        observation,
        evaluation,
    })
}

fn ipc_string_arg<'a>(value: &'a serde_json::Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("missing string argument `{key}`"))
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

    let mut task = TransferTask {
        id: Uuid::new_v4().to_string(),
        session_id: request.session_id.clone(),
        protocol: request.protocol.clone(),
        source: request.source.clone(),
        destination: request.destination.clone(),
        bytes_total: 0,
        bytes_done: 0,
        status: TransferStatus::Running,
        message: Some("started".to_string()),
    };

    {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        store.transfers.push(task.clone());
        store.record_system_event(
            &request.session_id,
            format!(
                "PortMate: transfer started ({:?}) {} -> {}",
                request.protocol, request.source, request.destination
            ),
        );
        save_store(&state.store_path, &store)?;
    }

    let result = match request.protocol {
        TransferProtocol::Sftp => transfer_file_via_sftp(state, &request).await,
        TransferProtocol::Scp => transfer_file_via_local_or_scp(state, &request).await,
        TransferProtocol::Xmodem => transfer_file_via_xmodem(state, &request).await,
        TransferProtocol::Ymodem => transfer_file_via_ymodem(state, &request).await,
        TransferProtocol::Zmodem => transfer_file_via_zmodem(state, &request).await,
    };

    match result {
        Ok(bytes) => {
            task.bytes_total = bytes;
            task.bytes_done = bytes;
            task.status = TransferStatus::Completed;
            task.message = Some("completed".to_string());
        }
        Err(error) => {
            task.status = TransferStatus::Failed;
            task.message = Some(error);
        }
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if let Some(existing) = store.transfers.iter_mut().find(|item| item.id == task.id) {
        *existing = task.clone();
    }
    store.record_system_event(
        &request.session_id,
        format!(
            "PortMate: transfer finished ({:?}, {:?})",
            task.protocol, task.status
        ),
    );
    save_store(&state.store_path, &store)?;
    Ok(task)
}

async fn transfer_file_via_sftp(
    state: &AppState,
    request: &StartTransferRequest,
) -> Result<u64, String> {
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);

    match (source_remote, destination_remote) {
        (None, None) => copy_local_file_for_transfer(&request.source, &request.destination),
        (None, Some(remote_destination)) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            let sftp = open_sftp_session(handle).await?;
            let result = sftp_upload(&sftp, &request.source, remote_destination).await;
            let _ = sftp.close().await;
            result
        }
        (Some(remote_source), None) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            let sftp = open_sftp_session(handle).await?;
            let result = sftp_download(&sftp, remote_source, &request.destination).await;
            let _ = sftp.close().await;
            result
        }
        (Some(remote_source), Some(remote_destination)) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            let sftp = open_sftp_session(handle).await?;
            let result = sftp_remote_copy(&sftp, remote_source, remote_destination).await;
            let _ = sftp.close().await;
            result
        }
    }
}

async fn transfer_file_via_local_or_scp(
    state: &AppState,
    request: &StartTransferRequest,
) -> Result<u64, String> {
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);

    match (source_remote, destination_remote) {
        (None, None) => copy_local_file_for_transfer(&request.source, &request.destination),
        (None, Some(remote_destination)) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            scp_upload(handle, &request.source, remote_destination).await
        }
        (Some(remote_source), None) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            scp_download(handle, remote_source, &request.destination).await
        }
        (Some(remote_source), Some(remote_destination)) => {
            let handle = ssh_handle_for_transfer(state, &request.session_id)?;
            remote_copy(handle, remote_source, remote_destination).await
        }
    }
}

async fn transfer_file_via_xmodem(
    state: &AppState,
    request: &StartTransferRequest,
) -> Result<u64, String> {
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Xmodem,
                true,
                &remote_destination,
            )
            .await?;
            xmodem_send_file(state, &request.session_id, receiver, &local_source).await
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Xmodem,
                false,
                &remote_source,
            )
            .await?;
            xmodem_receive_file(state, &request.session_id, receiver, &local_destination).await
        }
    }
}

async fn transfer_file_via_ymodem(
    state: &AppState,
    request: &StartTransferRequest,
) -> Result<u64, String> {
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Ymodem,
                true,
                &remote_destination,
            )
            .await?;
            ymodem_send_file(
                state,
                &request.session_id,
                receiver,
                &local_source,
                Some(&remote_destination),
            )
            .await
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Ymodem,
                false,
                &remote_source,
            )
            .await?;
            ymodem_receive_file(state, &request.session_id, receiver, &local_destination).await
        }
    }
}

async fn transfer_file_via_zmodem(
    state: &AppState,
    request: &StartTransferRequest,
) -> Result<u64, String> {
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Zmodem,
                true,
                &remote_destination,
            )
            .await?;
            zmodem_send_file(
                state,
                &request.session_id,
                receiver,
                &local_source,
                Some(&remote_destination),
            )
            .await
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Zmodem,
                false,
                &remote_source,
            )
            .await?;
            zmodem_receive_files(state, &request.session_id, receiver, &local_destination).await
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

async fn maybe_start_remote_modem(
    state: &AppState,
    session_id: &str,
    protocol: TransferProtocol,
    upload: bool,
    remote_path: &str,
) -> Result<(), String> {
    let profile = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.profile(session_id)
    };
    let Some(profile) = profile else {
        return Err(format!("unknown session: {session_id}"));
    };
    if !matches!(
        profile.kind,
        SessionKind::Ssh | SessionKind::Tmux | SessionKind::Shell | SessionKind::Telnet
    ) {
        return Ok(());
    }

    let command = modem_remote_command(protocol, upload, remote_path);
    let _ = send_text_inner(
        Arc::clone(&state.store),
        Arc::clone(&state.ssh),
        Arc::clone(&state.shell),
        Arc::clone(&state.tcp),
        Arc::clone(&state.serial),
        state.store_path.clone(),
        session_id.to_string(),
        command,
    )
    .await?;
    Ok(())
}

fn modem_remote_command(protocol: TransferProtocol, upload: bool, remote_path: &str) -> String {
    match (protocol, upload) {
        (TransferProtocol::Xmodem, true) => format!("rx {}\r", shell_quote(remote_path)),
        (TransferProtocol::Xmodem, false) => format!("sx {}\r", shell_quote(remote_path)),
        (TransferProtocol::Ymodem, true) => {
            let (parent, _) = remote_parent_and_file_name(remote_path);
            if parent.is_empty() {
                "rb -y\r".to_string()
            } else {
                format!(
                    "mkdir -p {} && cd {} && rb -y\r",
                    shell_quote(&parent),
                    shell_quote(&parent)
                )
            }
        }
        (TransferProtocol::Ymodem, false) => format!("sb {}\r", shell_quote(remote_path)),
        (TransferProtocol::Zmodem, true) => {
            let (parent, _) = remote_parent_and_file_name(remote_path);
            if parent.is_empty() {
                "rz -y\r".to_string()
            } else {
                format!(
                    "mkdir -p {} && cd {} && rz -y\r",
                    shell_quote(&parent),
                    shell_quote(&parent)
                )
            }
        }
        (TransferProtocol::Zmodem, false) => format!("sz {}\r", shell_quote(remote_path)),
        _ => String::new(),
    }
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
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = serial_writer {
        let mut writer = writer.lock().map_err(|error| error.to_string())?;
        writer
            .write_all(bytes)
            .map_err(|error| format!("串口 modem 写入失败: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("串口 modem 刷新失败: {error}"))?;
        return Ok(());
    }

    Err("会话尚未连接，无法执行 modem 写入".to_string())
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

async fn zmodem_send_file(
    state: &AppState,
    session_id: &str,
    receiver: broadcast::Receiver<Vec<u8>>,
    local_source: &str,
    remote_destination: Option<&str>,
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

    let mut reader = ModemByteReader::new(receiver);
    let mut input_buf = Vec::<u8>::new();
    let mut input_offset = 0_usize;
    let mut file_buf = vec![0_u8; 1024];
    let mut session_done = false;
    let mut last_progress = Instant::now();

    while !session_done || !sender.drain_outgoing().is_empty() {
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
    receiver: broadcast::Receiver<Vec<u8>>,
    local_destination: &str,
) -> Result<u64, String> {
    let mut modem_receiver =
        zmodem2::Receiver::new().map_err(|error| format!("ZModem receiver 初始化失败: {error}"))?;
    let mut reader = ModemByteReader::new(receiver);
    let mut input_buf = Vec::<u8>::new();
    let mut input_offset = 0_usize;
    let mut current_file: Option<(fs::File, PathBuf)> = None;
    let mut received_files = 0_usize;
    let mut bytes_done = 0_u64;
    let mut session_done = false;
    let mut last_progress = Instant::now();

    while !session_done || !modem_receiver.drain_outgoing().is_empty() {
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
    receiver: broadcast::Receiver<Vec<u8>>,
    local_source: &str,
) -> Result<u64, String> {
    let data =
        fs::read(local_source).map_err(|error| format!("读取 XModem 本地文件失败: {error}"))?;
    let mut reader = ModemByteReader::new(receiver);
    let crc = modem_wait_for_receiver(&mut reader).await?;
    let mut block_no = 1_u8;

    for chunk in data.chunks(XMODEM_BLOCK_SIZE) {
        modem_send_packet_with_retries(
            state,
            session_id,
            &mut reader,
            MODEM_SOH,
            block_no,
            chunk,
            crc,
        )
        .await?;
        block_no = block_no.wrapping_add(1);
    }
    modem_finish_eot(state, session_id, &mut reader).await?;
    Ok(data.len() as u64)
}

async fn xmodem_receive_file(
    state: &AppState,
    session_id: &str,
    receiver: broadcast::Receiver<Vec<u8>>,
    local_destination: &str,
) -> Result<u64, String> {
    let mut reader = ModemByteReader::new(receiver);
    let mut expected = 1_u8;
    let mut output = Vec::new();
    let mut first_packet = true;

    loop {
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
    receiver: broadcast::Receiver<Vec<u8>>,
    local_source: &str,
    remote_destination: Option<&str>,
) -> Result<u64, String> {
    let data =
        fs::read(local_source).map_err(|error| format!("读取 YModem 本地文件失败: {error}"))?;
    let mut reader = ModemByteReader::new(receiver);
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
    .await?;
    let _ = modem_wait_for_crc_request(&mut reader, Duration::from_secs(10)).await;

    let mut block_no = 1_u8;
    for chunk in data.chunks(YMODEM_BLOCK_SIZE) {
        modem_send_packet_with_retries(
            state,
            session_id,
            &mut reader,
            MODEM_STX,
            block_no,
            chunk,
            true,
        )
        .await?;
        block_no = block_no.wrapping_add(1);
    }
    modem_finish_eot(state, session_id, &mut reader).await?;
    let _ = modem_wait_for_crc_request(&mut reader, Duration::from_secs(10)).await;
    let empty = vec![0_u8; XMODEM_BLOCK_SIZE];
    modem_send_packet_with_retries(state, session_id, &mut reader, MODEM_SOH, 0, &empty, true)
        .await?;
    Ok(data.len() as u64)
}

async fn ymodem_receive_file(
    state: &AppState,
    session_id: &str,
    receiver: broadcast::Receiver<Vec<u8>>,
    local_destination: &str,
) -> Result<u64, String> {
    let mut reader = ModemByteReader::new(receiver);
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
    loop {
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
        Path::new(local_destination)
            .join(name)
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
    error.contains("timeout")
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

fn copy_local_file_for_transfer(source: &str, destination: &str) -> Result<u64, String> {
    let source = PathBuf::from(source);
    let destination = PathBuf::from(destination);
    if !source.is_file() {
        return Err(
            "only local file copy is available for this protocol path right now".to_string(),
        );
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create transfer destination: {error}"))?;
    }
    fs::copy(&source, &destination).map_err(|error| format!("local transfer failed: {error}"))
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

async fn sftp_upload(
    sftp: &SftpSession,
    local_source: &str,
    remote_destination: &str,
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
    let mut remote_file = sftp
        .open_with_flags(
            target.clone(),
            OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
        )
        .await
        .map_err(|error| format!("SFTP 创建远端文件失败 {target}: {error}"))?;

    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = local_file
            .read(&mut buffer)
            .map_err(|error| format!("读取本地文件失败 {local_source}: {error}"))?;
        if read == 0 {
            break;
        }
        remote_file
            .write_all(&buffer[..read])
            .await
            .map_err(|error| format!("SFTP 写入远端文件失败 {target}: {error}"))?;
        copied += read as u64;
    }
    remote_file
        .flush()
        .await
        .map_err(|error| format!("SFTP 刷新远端文件失败 {target}: {error}"))?;
    remote_file
        .shutdown()
        .await
        .map_err(|error| format!("SFTP 关闭远端文件失败 {target}: {error}"))?;
    Ok(copied)
}

async fn sftp_download(
    sftp: &SftpSession,
    remote_source: &str,
    local_destination: &str,
) -> Result<u64, String> {
    let mut remote_file = sftp
        .open(remote_source.to_string())
        .await
        .map_err(|error| format!("SFTP 打开远端文件失败 {remote_source}: {error}"))?;
    let target = local_destination_file_path(local_destination, remote_source)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建本地目录失败 {}: {error}", parent.display()))?;
    }
    let mut local_file = fs::File::create(&target)
        .map_err(|error| format!("创建本地目标文件失败 {}: {error}", target.display()))?;
    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = remote_file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("SFTP 读取远端文件失败 {remote_source}: {error}"))?;
        if read == 0 {
            break;
        }
        local_file
            .write_all(&buffer[..read])
            .map_err(|error| format!("写入本地目标文件失败 {}: {error}", target.display()))?;
        copied += read as u64;
    }
    local_file
        .flush()
        .map_err(|error| format!("刷新本地目标文件失败 {}: {error}", target.display()))?;
    let _ = remote_file.shutdown().await;
    Ok(copied)
}

async fn sftp_remote_copy(
    sftp: &SftpSession,
    remote_source: &str,
    remote_destination: &str,
) -> Result<u64, String> {
    let mut source_file = sftp
        .open(remote_source.to_string())
        .await
        .map_err(|error| format!("SFTP 打开远端源文件失败 {remote_source}: {error}"))?;
    let file_name = remote_file_name(remote_source);
    let target = sftp_destination_file_path(sftp, remote_destination, &file_name).await?;
    let mut target_file = sftp
        .open_with_flags(
            target.clone(),
            OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
        )
        .await
        .map_err(|error| format!("SFTP 创建远端目标文件失败 {target}: {error}"))?;

    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
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
            .map_err(|error| format!("SFTP 写入远端目标文件失败 {target}: {error}"))?;
        copied += read as u64;
    }
    target_file
        .flush()
        .await
        .map_err(|error| format!("SFTP 刷新远端目标文件失败 {target}: {error}"))?;
    let _ = source_file.shutdown().await;
    target_file
        .shutdown()
        .await
        .map_err(|error| format!("SFTP 关闭远端目标文件失败 {target}: {error}"))?;
    Ok(copied)
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
) -> Result<u64, String> {
    let command = format!(
        "cp -f -- {} {} && stat -c '%s' -- {}",
        shell_quote(remote_source),
        shell_quote(remote_destination),
        shell_quote(remote_destination)
    );
    let output = exec_ssh_command_capture(handle, &command, Duration::from_secs(60)).await?;
    output
        .lines()
        .last()
        .unwrap_or_default()
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("remote copy completed but size parse failed: {error}"))
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
        let path = PathBuf::from(&request.path);
        match operation {
            FileOperation::CreateDirectory => fs::create_dir_all(&path)
                .map_err(|error| format!("创建本地目录失败 {}: {error}", path.display())),
            FileOperation::Delete => {
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
        let old_path = PathBuf::from(&request.old_path);
        let new_path = PathBuf::from(&request.new_path);
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
) -> Result<u64, String> {
    let data = fs::read(local_source).map_err(|error| format!("读取本地文件失败: {error}"))?;
    let size = data.len() as u64;
    let file_name = Path::new(local_source)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-upload.bin");
    let mut channel = {
        let handle = handle.lock().await;
        handle
            .channel_open_session()
            .await
            .map_err(|error| format!("SCP 打开 SSH channel 失败: {error}"))?
    };
    channel
        .exec(true, format!("scp -t {}", shell_quote(remote_destination)))
        .await
        .map_err(|error| format!("SCP 启动远端接收失败: {error}"))?;

    let mut pending = VecDeque::new();
    scp_wait_ack(&mut channel, &mut pending).await?;
    channel
        .data(format!("C0644 {size} {file_name}\n").as_bytes())
        .await
        .map_err(|error| format!("SCP 写入文件头失败: {error}"))?;
    scp_wait_ack(&mut channel, &mut pending).await?;
    channel
        .data(&data[..])
        .await
        .map_err(|error| format!("SCP 写入文件内容失败: {error}"))?;
    channel
        .data(&[0_u8][..])
        .await
        .map_err(|error| format!("SCP 写入结束标记失败: {error}"))?;
    scp_wait_ack(&mut channel, &mut pending).await?;
    let _ = channel.eof().await;
    let _ = channel.close().await;
    Ok(size)
}

async fn scp_download(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    remote_source: &str,
    local_destination: &str,
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
    let mut file = fs::File::create(local_destination)
        .map_err(|error| format!("创建本地目标文件失败: {error}"))?;
    channel
        .data(&[0_u8][..])
        .await
        .map_err(|error| format!("SCP 写入文件头确认失败: {error}"))?;

    let mut remaining = size;
    while remaining > 0 {
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
        let take = pending.len().min(remaining as usize);
        let chunk = pending.drain(..take).collect::<Vec<_>>();
        file.write_all(&chunk)
            .map_err(|error| format!("写入本地目标文件失败: {error}"))?;
        remaining -= take as u64;
    }
    file.flush()
        .map_err(|error| format!("刷新本地目标文件失败: {error}"))?;
    scp_wait_ack(&mut channel, &mut pending).await?;
    channel
        .data(&[0_u8][..])
        .await
        .map_err(|error| format!("SCP 写入完成确认失败: {error}"))?;
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
        label: request.label.unwrap_or_else(|| match request.mode {
            TunnelMode::Local => format!(
                "{}:{} -> {}:{}",
                request.bind_host, request.bind_port, request.target_host, request.target_port
            ),
            TunnelMode::Dynamic => format!("SOCKS5 {}:{}", request.bind_host, request.bind_port),
            TunnelMode::Remote => format!(
                "remote {}:{} -> {}:{}",
                request.bind_host, request.bind_port, request.target_host, request.target_port
            ),
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
        {
            let mut forwards = remote_forwards.lock().map_err(|error| error.to_string())?;
            forwards.insert(
                remote_forward_key(&tunnel.bind_host, tunnel.bind_port),
                tunnel.clone(),
            );
            forwards.insert(remote_forward_port_key(tunnel.bind_port), tunnel.clone());
        }
        let closed = Arc::new(AtomicBool::new(false));
        {
            let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
            tunnels.insert(
                tunnel.id.clone(),
                TunnelRuntime {
                    session_id: request.session_id.clone(),
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
    let closed = Arc::new(AtomicBool::new(false));
    {
        let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels.insert(
            tunnel.id.clone(),
            TunnelRuntime {
                session_id: request.session_id.clone(),
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
                    let store = Arc::clone(&store);
                    let store_path = store_path.clone();
                    let session_id = session_id.clone();
                    tauri::async_runtime::spawn(async move {
                        let result = if spec.mode == TunnelMode::Dynamic {
                            handle_dynamic_tunnel_client(handle, stream, peer).await
                        } else {
                            handle_local_tunnel_client(handle, spec, stream, peer).await
                        };
                        if let Err(error) = result {
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
            remote_write
                .data(&buffer[..size])
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok::<(), String>(())
    };

    let remote_to_local = async move {
        while let Some(message) = remote_read.wait().await {
            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
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

    let _ = tokio::join!(local_to_remote, remote_to_local);
    Ok(())
}

async fn handle_remote_tunnel_client(
    channel: Channel<client::Msg>,
    tunnel: TunnelSpec,
    originator_address: String,
    originator_port: u16,
) -> Result<(), String> {
    let local_stream = TcpStream::connect((tunnel.target_host.clone(), tunnel.target_port))
        .await
        .map_err(|error| {
            format!(
                "remote tunnel target connect failed {}:{} for {}:{}: {error}",
                tunnel.target_host, tunnel.target_port, originator_address, originator_port
            )
        })?;
    pipe_ssh_channel_to_tcp(channel, local_stream, tunnel).await
}

async fn handle_dynamic_tunnel_client(
    handle: Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>,
    mut local_stream: TcpStream,
    peer: std::net::SocketAddr,
) -> Result<(), String> {
    let mut header = [0_u8; 2];
    local_stream
        .read_exact(&mut header)
        .await
        .map_err(|error| format!("SOCKS5 handshake read failed: {error}"))?;
    if header[0] != 5 {
        return Err("only SOCKS5 is supported for dynamic tunnel".to_string());
    }
    let mut methods = vec![0_u8; header[1] as usize];
    local_stream
        .read_exact(&mut methods)
        .await
        .map_err(|error| format!("SOCKS5 methods read failed: {error}"))?;
    local_stream
        .write_all(&[5, 0])
        .await
        .map_err(|error| format!("SOCKS5 method response failed: {error}"))?;

    let mut request = [0_u8; 4];
    local_stream
        .read_exact(&mut request)
        .await
        .map_err(|error| format!("SOCKS5 request read failed: {error}"))?;
    if request[0] != 5 || request[1] != 1 {
        local_stream
            .write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0])
            .await
            .ok();
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
            let mut name = vec![0_u8; len[0] as usize];
            local_stream
                .read_exact(&mut name)
                .await
                .map_err(|error| format!("SOCKS5 domain read failed: {error}"))?;
            String::from_utf8_lossy(&name).to_string()
        }
        4 => {
            let mut addr = [0_u8; 16];
            local_stream
                .read_exact(&mut addr)
                .await
                .map_err(|error| format!("SOCKS5 IPv6 read failed: {error}"))?;
            std::net::Ipv6Addr::from(addr).to_string()
        }
        other => return Err(format!("unsupported SOCKS5 address type: {other}")),
    };
    let mut port_bytes = [0_u8; 2];
    local_stream
        .read_exact(&mut port_bytes)
        .await
        .map_err(|error| format!("SOCKS5 port read failed: {error}"))?;
    let target_port = u16::from_be_bytes(port_bytes);

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
            .map_err(|error| format!("dynamic direct-tcpip open failed: {error}"))?
    };

    local_stream
        .write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
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
    pipe_ssh_channel_to_tcp(channel, local_stream, spec).await
}

async fn pipe_ssh_channel_to_tcp(
    channel: Channel<client::Msg>,
    local_stream: TcpStream,
    tunnel: TunnelSpec,
) -> Result<(), String> {
    let (mut remote_read, remote_write) = channel.split();
    let (mut local_read, mut local_write) = local_stream.into_split();

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
            remote_write
                .data(&buffer[..size])
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok::<(), String>(())
    };

    let remote_to_local = async move {
        while let Some(message) = remote_read.wait().await {
            match message {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
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

    let (a, b) = tokio::join!(local_to_remote, remote_to_local);
    a.and(b)
        .map_err(|error| format!("tunnel pipe failed ({}): {error}", tunnel.label))
}

async fn open_ssh_session(
    state: &AppState,
    profile: SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<SessionSummary, String> {
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

    if let Some(existing) = {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        let handle = existing.handle.lock().await;
        let _ = handle
            .disconnect(Disconnect::ByApplication, "PortMate reconnect", "en")
            .await;
    }

    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());

    let observed_key = Arc::new(Mutex::new(None));
    let host_key_error = Arc::new(Mutex::new(None));
    let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
    let alias = ssh
        .host_key_policy
        .alias
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| Some(profile.id.clone()));
    let handler = PortMateSshHandler {
        profile_id: profile.id.clone(),
        host: host.clone(),
        port: ssh.endpoint.port,
        alias,
        policy: ssh.host_key_policy.clone(),
        host_keys,
        observed_key: Arc::clone(&observed_key),
        host_key_error: Arc::clone(&host_key_error),
        remote_forwards: Arc::clone(&remote_forwards),
    };

    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    });

    let mut session = tokio::time::timeout(
        Duration::from_secs(20),
        client::connect(config, (host.clone(), ssh.endpoint.port), handler),
    )
    .await
    .map_err(|_| format!("SSH 连接超时: {host}:{}", ssh.endpoint.port))?
    .map_err(|error| {
        host_key_error
            .lock()
            .ok()
            .and_then(|reason| reason.clone())
            .unwrap_or_else(|| format!("SSH 握手失败: {error}"))
    })?;

    let auth_method = authenticate_ssh(
        &mut session,
        ssh.clone(),
        username.clone(),
        password,
        passphrase,
    )
    .await?;

    persist_observed_host_key(&state.store, &profile.id, &observed_key)?;
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
    {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            SshRuntime {
                runtime_id: runtime_id.clone(),
                handle: Arc::new(tokio::sync::Mutex::new(session)),
                writer: Arc::clone(&writer),
                tap: tap.clone(),
                remote_forwards,
            },
        );
    }

    if matches!(profile.connection, ConnectionConfig::Tmux(_)) {
        let writer = writer.lock().await;
        writer
            .data(&b"tmux new-session -A -s portmate\r"[..])
            .await
            .map_err(|error| format!("Tmux attach 命令发送失败: {error}"))?;
    }

    tauri::async_runtime::spawn(read_ssh_channel(
        Arc::clone(&state.store),
        state.runtimes(),
        Arc::clone(&state.ssh),
        state.store_path.clone(),
        profile.id.clone(),
        runtime_id,
        tap,
        read_half,
    ));

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let _ = store.record_auth_success(&profile.id, auth_method);
    store.record_system_event(
        &profile.id,
        format!("PortMate: SSH authentication succeeded via {auth_method:?}"),
    );
    let summary = store.open_session(&profile.id)?;
    save_store(&state.store_path, &store)?;
    Ok(summary)
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
    command.env("TERM", profile.terminal.term.as_str());
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
        .spawn(read_shell_pty(
            Arc::clone(&state.store),
            state.runtimes(),
            Arc::clone(&state.shell),
            state.store_path.clone(),
            profile.id.clone(),
            runtime_id,
            program.clone(),
            tap,
            closed,
            child,
            reader,
        ))
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

async fn open_tcp_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
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

    if let Some(existing) = {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        let mut writer = existing.writer.lock().await;
        let _ = writer.shutdown().await;
    }

    let stream = tokio::time::timeout(
        Duration::from_secs(15),
        TcpStream::connect((host.clone(), port)),
    )
    .await
    .map_err(|_| format!("{label} 连接超时: {host}:{port}"))?
    .map_err(|error| format!("{label} 连接失败: {host}:{port}: {error}"))?;
    stream
        .set_nodelay(true)
        .map_err(|error| format!("{label} 设置 TCP_NODELAY 失败: {error}"))?;

    let runtime_id = Uuid::new_v4().to_string();
    let (read_half, write_half) = stream.into_split();
    let writer = Arc::new(tokio::sync::Mutex::new(write_half));
    let (tap, _) = broadcast::channel(1024);
    {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: runtime_id.clone(),
                writer: Arc::clone(&writer),
                tap: tap.clone(),
            },
        );
    }

    tauri::async_runtime::spawn(read_tcp_stream(
        Arc::clone(&state.store),
        state.runtimes(),
        Arc::clone(&state.tcp),
        state.store_path.clone(),
        profile.id.clone(),
        runtime_id,
        label.to_string(),
        tap,
        Arc::clone(&writer),
        read_half,
    ));

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.record_system_event(&profile.id, format!("PortMate: {label} socket connected"));
    let summary = store.open_session(&profile.id)?;
    save_store(&state.store_path, &store)?;
    Ok(summary)
}

fn open_serial_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    let serial = match &profile.connection {
        ConnectionConfig::Serial(serial) => serial.clone(),
        _ => return Err("profile is not serial-backed".to_string()),
    };
    let port_name = serial.port.trim().to_string();
    if port_name.is_empty() {
        return Err("串口不能为空".to_string());
    }

    if let Some(existing) = {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        existing.closed.store(true, Ordering::SeqCst);
    }

    let mut port = serialport::new(&port_name, serial.baud_rate)
        .data_bits(serial_data_bits(serial.data_bits))
        .stop_bits(serial_stop_bits(serial.stop_bits))
        .parity(serial_parity(&serial.parity))
        .flow_control(serial_flow_control(&serial.flow_control))
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|error| format!("串口打开失败 {port_name}: {error}"))?;
    port.write_data_terminal_ready(serial.dtr)
        .map_err(|error| format!("设置 DTR 失败: {error}"))?;
    port.write_request_to_send(serial.rts)
        .map_err(|error| format!("设置 RTS 失败: {error}"))?;

    let reader = port
        .try_clone()
        .map_err(|error| format!("串口 reader 克隆失败: {error}"))?;
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
                writer,
                tap: tap.clone(),
                closed: Arc::clone(&closed),
            },
        );
    }

    if let Err(error) = std::thread::Builder::new()
        .name(format!("portmate-serial-{}", profile.id))
        .spawn(read_serial_port(
            Arc::clone(&state.store),
            state.runtimes(),
            Arc::clone(&state.serial),
            state.store_path.clone(),
            profile.id.clone(),
            runtime_id,
            port_name.clone(),
            tap,
            closed,
            reader,
        ))
    {
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
                        let rsa_hash = session
                            .best_supported_rsa_hash()
                            .await
                            .map_err(|error| format!("SSH 查询 RSA 签名算法失败: {error}"))?
                            .flatten();
                        let result = session
                            .authenticate_publickey(
                                username.clone(),
                                PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
                            )
                            .await
                            .map_err(|error| format!("SSH publickey 认证失败: {error}"))?;
                        if result.success() {
                            return Ok(AuthMethod::PublicKey);
                        }
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
        message.push_str(&format!("；密钥加载错误: {}", key_errors.join(" | ")));
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
) -> Result<(), String> {
    let observation = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "SSH 未收到服务器 host key".to_string())?;
    let mut store = store.lock().map_err(|error| error.to_string())?;

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
            let profile = store
                .profile(profile_id)
                .ok_or_else(|| format!("unknown session: {profile_id}"))?;
            let policy = match profile.connection {
                ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.host_key_policy,
                _ => return Err(format!("profile is not SSH-backed: {profile_id}")),
            };
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

async fn read_ssh_channel(
    store: Arc<Mutex<SessionStore>>,
    runtimes: RuntimeRegistry,
    ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    store_path: PathBuf,
    session_id: String,
    runtime_id: String,
    tap: broadcast::Sender<Vec<u8>>,
    mut read_half: ChannelReadHalf,
) {
    let mut last_persist = Instant::now();
    let mut has_unpersisted_stream = false;

    while let Some(message) = read_half.wait().await {
        match message {
            ChannelMsg::Data { data } => {
                let bytes = data.to_vec();
                let _ = tap.send(bytes.clone());
                record_channel_text(
                    &store,
                    &runtimes,
                    &store_path,
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
                    &store,
                    &runtimes,
                    &store_path,
                    &session_id,
                    stream,
                    String::from_utf8_lossy(&bytes).to_string(),
                );
                has_unpersisted_stream = true;
            }
            ChannelMsg::ExitStatus { exit_status } => {
                if let Ok(mut store) = store.lock() {
                    store.record_system_event(
                        &session_id,
                        format!("PortMate: SSH remote process exited with status {exit_status}"),
                    );
                    if let Err(error) = save_store(&store_path, &store) {
                        eprintln!("PortMate: failed to persist SSH exit status: {error}");
                    }
                }
            }
            ChannelMsg::ExitSignal {
                signal_name,
                error_message,
                ..
            } => {
                if let Ok(mut store) = store.lock() {
                    store.record_system_event(
                        &session_id,
                        format!(
                            "PortMate: SSH remote process exited by signal {signal_name:?} {error_message}"
                        ),
                    );
                    if let Err(error) = save_store(&store_path, &store) {
                        eprintln!("PortMate: failed to persist SSH exit signal: {error}");
                    }
                }
            }
            ChannelMsg::Eof | ChannelMsg::Close => break,
            _ => {}
        }

        if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
            if let Err(error) = persist_store_arc(&store_path, &store) {
                eprintln!("PortMate: failed to persist SSH stream data: {error}");
            }
            has_unpersisted_stream = false;
            last_persist = Instant::now();
        }
    }

    if has_unpersisted_stream {
        if let Err(error) = persist_store_arc(&store_path, &store) {
            eprintln!("PortMate: failed to persist final SSH stream data: {error}");
        }
    }

    let removed_current = {
        let mut connections = match ssh.lock() {
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
        if let Ok(mut store) = store.lock() {
            let _ = store.set_runtime_status(&session_id, SessionStatus::Disconnected);
            store.record_system_event(&session_id, "PortMate: SSH channel closed");
            if let Err(error) = save_store(&store_path, &store) {
                eprintln!("PortMate: failed to persist SSH close event: {error}");
            }
        }
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

async fn read_tcp_stream(
    store: Arc<Mutex<SessionStore>>,
    runtimes: RuntimeRegistry,
    tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    store_path: PathBuf,
    session_id: String,
    runtime_id: String,
    label: String,
    tap: broadcast::Sender<Vec<u8>>,
    writer: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
    mut read_half: OwnedReadHalf,
) {
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
                        if let Ok(mut store) = store.lock() {
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
                    &store,
                    &runtimes,
                    &store_path,
                    &session_id,
                    EventStream::Stdout,
                    String::from_utf8_lossy(&bytes).to_string(),
                );
                has_unpersisted_stream = true;
            }
            Err(error) => {
                if let Ok(mut store) = store.lock() {
                    store.record_system_event(
                        &session_id,
                        format!("PortMate: {label} read failed: {error}"),
                    );
                    if let Err(error) = save_store(&store_path, &store) {
                        eprintln!("PortMate: failed to persist {label} read error: {error}");
                    }
                }
                break;
            }
        }

        if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
            if let Err(error) = persist_store_arc(&store_path, &store) {
                eprintln!("PortMate: failed to persist {label} stream data: {error}");
            }
            has_unpersisted_stream = false;
            last_persist = Instant::now();
        }
    }

    if has_unpersisted_stream {
        if let Err(error) = persist_store_arc(&store_path, &store) {
            eprintln!("PortMate: failed to persist final {label} stream data: {error}");
        }
    }

    let removed_current = {
        let mut connections = match tcp.lock() {
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
        if let Ok(mut store) = store.lock() {
            let _ = store.set_runtime_status(&session_id, SessionStatus::Disconnected);
            store.record_system_event(&session_id, format!("PortMate: {label} socket closed"));
            if let Err(error) = save_store(&store_path, &store) {
                eprintln!("PortMate: failed to persist {label} close event: {error}");
            }
        }
    }
}

fn read_shell_pty(
    store: Arc<Mutex<SessionStore>>,
    runtimes: RuntimeRegistry,
    shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    store_path: PathBuf,
    session_id: String,
    runtime_id: String,
    program: String,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    mut reader: Box<dyn Read + Send>,
) -> impl FnOnce() + Send + 'static {
    move || {
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
                        &store,
                        &runtimes,
                        &store_path,
                        &session_id,
                        EventStream::Stdout,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    if let Ok(mut store) = store.lock() {
                        store.record_system_event(
                            &session_id,
                            format!("PortMate: shell read failed on {program}: {error}"),
                        );
                        if let Err(error) = save_store(&store_path, &store) {
                            eprintln!("PortMate: failed to persist shell read error: {error}");
                        }
                    }
                    break;
                }
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&store_path, &store) {
                    eprintln!("PortMate: failed to persist shell stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&store_path, &store) {
                eprintln!("PortMate: failed to persist final shell stream data: {error}");
            }
        }

        let removed_current = {
            let mut connections = match shell.lock() {
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
            if let Ok(mut store) = store.lock() {
                let _ = store.set_runtime_status(&session_id, SessionStatus::Disconnected);
                store.record_system_event(
                    &session_id,
                    format!("PortMate: shell closed ({program})"),
                );
                if let Err(error) = save_store(&store_path, &store) {
                    eprintln!("PortMate: failed to persist shell close event: {error}");
                }
            }
        }
    }
}

fn read_serial_port(
    store: Arc<Mutex<SessionStore>>,
    runtimes: RuntimeRegistry,
    serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
    store_path: PathBuf,
    session_id: String,
    runtime_id: String,
    port_name: String,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
    mut reader: Box<dyn serialport::SerialPort>,
) -> impl FnOnce() + Send + 'static {
    move || {
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
                        &store,
                        &runtimes,
                        &store_path,
                        &session_id,
                        EventStream::Stdout,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => {
                    if let Ok(mut store) = store.lock() {
                        store.record_system_event(
                            &session_id,
                            format!("PortMate: serial read failed on {port_name}: {error}"),
                        );
                        if let Err(error) = save_store(&store_path, &store) {
                            eprintln!("PortMate: failed to persist serial read error: {error}");
                        }
                    }
                    break;
                }
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&store_path, &store) {
                    eprintln!("PortMate: failed to persist serial stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&store_path, &store) {
                eprintln!("PortMate: failed to persist final serial stream data: {error}");
            }
        }

        let removed_current = {
            let mut connections = match serial.lock() {
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
            if let Ok(mut store) = store.lock() {
                let _ = store.set_runtime_status(&session_id, SessionStatus::Disconnected);
                store.record_system_event(
                    &session_id,
                    format!("PortMate: serial port closed ({port_name})"),
                );
                if let Err(error) = save_store(&store_path, &store) {
                    eprintln!("PortMate: failed to persist serial close event: {error}");
                }
            }
        }
    }
}

fn record_channel_text(
    store: &Arc<Mutex<SessionStore>>,
    runtimes: &RuntimeRegistry,
    store_path: &Path,
    session_id: &str,
    stream: EventStream,
    text: String,
) {
    if text.is_empty() {
        return;
    }
    let local_commands = if let Ok(mut store) = store.lock() {
        let _ =
            store.record_stream_event(session_id, EventDirection::Inbound, stream, text.clone());
        let (trigger_dispatch, trigger_changed_store) =
            apply_trigger_actions_locked(&mut store, session_id, &text);
        if trigger_changed_store {
            if let Err(error) = save_store(store_path, &store) {
                eprintln!("PortMate: failed to persist trigger actions: {error}");
            }
        }
        trigger_dispatch
    } else {
        TriggerDispatch::default()
    };
    for command in local_commands.local_commands {
        spawn_trigger_command(
            Arc::clone(store),
            store_path.to_path_buf(),
            session_id.to_string(),
            command,
        );
    }
    for text in local_commands.send_texts {
        spawn_trigger_send_text(
            Arc::clone(store),
            runtimes.clone(),
            store_path.to_path_buf(),
            session_id.to_string(),
            text,
        );
    }
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

fn spawn_trigger_send_text(
    store: Arc<Mutex<SessionStore>>,
    runtimes: RuntimeRegistry,
    store_path: PathBuf,
    session_id: String,
    text: String,
) {
    tauri::async_runtime::spawn(async move {
        let result = send_text_inner(
            Arc::clone(&store),
            Arc::clone(&runtimes.ssh),
            Arc::clone(&runtimes.shell),
            Arc::clone(&runtimes.tcp),
            Arc::clone(&runtimes.serial),
            store_path.clone(),
            session_id.clone(),
            text,
        )
        .await;

        if let Err(error) = result {
            if let Ok(mut store) = store.lock() {
                store.record_system_event(
                    &session_id,
                    format!("PortMate: trigger send_text failed: {error}"),
                );
                if let Err(error) = save_store(&store_path, &store) {
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
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
}

fn record_connection_failure(state: &AppState, session_id: &str, error: &str) {
    if let Ok(mut store) = state.store.lock() {
        let _ = store.set_runtime_status(session_id, SessionStatus::Error);
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

fn normalize_loaded_store(mut store: SessionStore) -> SessionStore {
    for profile in store.profiles.clone() {
        let _ = store.upsert_profile(profile);
    }
    for runtime in &mut store.runtimes {
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
    connection
        .execute(
            "insert into kv (key, value, updated_at) values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
             on conflict(key) do update set value = excluded.value, updated_at = excluded.updated_at",
            params![STORE_KEY, bytes],
        )
        .map_err(|error| format!("failed to save PortMate SQLite store: {error}"))?;
    save_store_sqlite_tables(&connection, store)?;
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
        .map_err(|error| format!("failed to initialize PortMate SQLite schema: {error}"))
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
                    active_transport, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    runtime.session_id,
                    runtime.pane_id,
                    enum_text(&runtime.status)?,
                    runtime.title,
                    runtime.cwd,
                    runtime.connected_since.map(|value| value.to_rfc3339()),
                    runtime.last_activity.to_rfc3339(),
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

#[cfg(test)]
mod tests {
    use super::*;

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
                store: Arc::new(Mutex::new(store)),
                ssh: Arc::new(Mutex::new(HashMap::new())),
                shell: Arc::new(Mutex::new(HashMap::new())),
                tcp: Arc::new(Mutex::new(HashMap::new())),
                serial: Arc::new(Mutex::new(HashMap::new())),
                tunnels: Arc::new(Mutex::new(HashMap::new())),
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
            list_transfers,
            list_mcp_audit,
            list_mcp_grants,
            save_mcp_grant,
            revoke_mcp_grant,
            list_host_keys,
            list_ssh_agent_identities,
            save_secret,
            delete_secret,
            has_secret,
            list_serial_ports,
            list_tmux_state,
            attach_tmux,
            list_files,
            create_directory,
            delete_path,
            rename_path,
            chmod_path,
            serial_set_lines,
            serial_send_break,
            refresh_sysmon,
            start_transfer,
            create_tunnel,
            mcp_manifest
        ])
        .run(tauri::generate_context!())
        .expect("error while running PortMate");
}

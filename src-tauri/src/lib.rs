use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer as _, SigningKey};
use flate2::{write::GzEncoder, Compression};
use keyring_core::Entry;
use portable_pty::PtySize;
#[cfg(test)]
use portmate_core::ProxyKind;
use portmate_core::{
    classify_mcp_start_transfer_source, compute_ssh_sha256_fingerprint,
    normalize_custom_script_content, normalize_loaded_custom_scripts, normalize_triggers,
    normalize_tunnel_route_rules, normalize_tunnels, parse_tftp_receiver_endpoint,
    prompt_templates, redact_custom_script_event_bodies, redact_secrets, redact_session_event,
    redact_session_events, redact_session_summary, redact_transfer_task, resource_templates,
    tool_definitions, tunnel_route_allowed, validate_custom_script, validate_tftp_file_name,
    validate_triggers, validate_tunnel_route_rules, validate_tunnels, AuditRecord, AuthMethod,
    ConnectionConfig, CustomScript, EventDirection, EventStream, HostKeyDecision,
    HostKeyEvaluation, HostKeyMode, HostKeyObservation, HostKeyScope, HostKeyStore, IdentityRef,
    IdentitySource, McpContentUploadMetadata, McpGrant, McpHttpSettings, McpScope,
    McpStartTransferSource, McpTransferDestination, OneKeyCredential, OneKeyIdentity, OneKeyKind,
    ProxyConfig, SessionEvent, SessionKind, SessionProfile, SessionStatus, SessionStore,
    SessionSummary, SshConnection, SysmonDisk, SysmonNetworkInterface, SysmonProcess,
    SysmonSnapshot, TcpConnection, TftpReceiverSpec, TimelineMark, TransferProtocol,
    TransferStatus, TransferTask, TriggerAction, TrustedHostKey, TunnelEgress, TunnelMode,
    TunnelSpec, CUSTOM_SCRIPT_EVENT_TEXT, DEFAULT_TFTP_PORT, MAX_COMMAND_HISTORY_ENTRIES,
    MAX_CUSTOM_SCRIPTS, MAX_MCP_CONTENT_UPLOAD_BYTES, MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH,
    MAX_MCP_TUNNEL_EXCHANGE_BYTES, MAX_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS,
    MAX_MCP_UDP_DATAGRAM_BASE64_LENGTH, MAX_MCP_UDP_DATAGRAM_BYTES, MAX_TUNNELS_PER_PROFILE,
    MAX_TUNNEL_HOST_CHARACTERS, MAX_TUNNEL_LABEL_CHARACTERS, MAX_TUNNEL_ROUTE_RULES,
    MCP_CONTENT_UPLOADS_DIRECTORY, MCP_CONTENT_UPLOAD_METADATA_FILE,
    MCP_CONTENT_UPLOAD_METADATA_VERSION, MCP_CONTENT_UPLOAD_PAYLOAD_FILE,
    MCP_CONTENT_UPLOAD_STAGING_DIRECTORY,
};
use rusqlite::{params, Connection as SqliteConnection};
use russh::client::{self, KeyboardInteractiveAuthResponse};
use russh::keys::{
    decode_secret_key, load_secret_key, ssh_key, PrivateKeyWithHashAlg, PublicKeyBase64,
};
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Disconnect};
use russh_sftp::{client::SftpSession, protocol::OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use socket2::SockRef;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
#[cfg(target_os = "linux")]
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, Write};
#[cfg(target_os = "linux")]
use std::net::{Ipv4Addr, Ipv6Addr};
use std::ops::Deref;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime};
use tar::{Builder as TarBuilder, Header as TarHeader};
#[cfg(feature = "desktop")]
use tauri::Manager;
use tauri::{Emitter, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{lookup_host, TcpListener, TcpStream, UdpSocket};
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

#[cfg(feature = "desktop")]
type AppRuntime = tauri::Wry;
#[cfg(not(feature = "desktop"))]
type AppRuntime = tauri::test::MockRuntime;
type AppHandle = tauri::AppHandle<AppRuntime>;
type WebviewWindow = tauri::WebviewWindow<AppRuntime>;

include!("backend_application.rs");
include!("backend_automation.rs");
include!("backend_security.rs");
include!("backend_storage.rs");
include!("backend_transport.rs");

#[cfg(feature = "desktop")]
pub use app_bootstrap::run;
#[cfg(not(feature = "desktop"))]
pub fn run() {
    panic!("PortMate desktop runtime was built without the desktop feature");
}
pub use command_types::*;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

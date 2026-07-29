use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signer as _, SigningKey};
use flate2::{write::GzEncoder, Compression};
use keyring_core::Entry;
use portable_pty::PtySize;
#[cfg(test)]
use portmate_core::ProxyKind;
use portmate_core::{
    compute_ssh_sha256_fingerprint, normalize_triggers, normalize_tunnels, prompt_templates,
    redact_secrets, redact_session_event, redact_session_events, redact_session_summary,
    redact_transfer_task, resource_templates, tool_definitions, validate_triggers,
    validate_tunnels, AuditRecord, AuthMethod, ConnectionConfig, EventDirection, EventStream,
    HostKeyDecision, HostKeyEvaluation, HostKeyMode, HostKeyObservation, HostKeyScope,
    HostKeyStore, IdentityRef, IdentitySource, McpGrant, McpScope, OneKeyCredential,
    OneKeyIdentity, OneKeyKind, ProxyConfig, SessionEvent, SessionKind, SessionProfile,
    SessionStatus, SessionStore, SessionSummary, SshConnection, SysmonDisk, SysmonNetworkInterface,
    SysmonProcess, SysmonSnapshot, TcpConnection, TimelineMark, TransferProtocol, TransferStatus,
    TransferTask, TriggerAction, TrustedHostKey, TunnelMode, TunnelSpec, MAX_TUNNELS_PER_PROFILE,
    MAX_TUNNEL_HOST_CHARACTERS, MAX_TUNNEL_LABEL_CHARACTERS,
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
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime};
use tar::{Builder as TarBuilder, Header as TarHeader};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

mod app_bootstrap;
mod app_data_migration;
mod archive_support;
mod bundle_export;
mod bundle_signing;
mod command_types;
mod file_batch;
mod file_commands;
mod file_operations;
mod file_transfer;
mod log_commands;
mod log_storage;
mod mcp_authorization;
mod mcp_commands;
mod mcp_control;
mod mcp_execution;
mod mcp_ipc;
mod migration_diagnostics;
mod migration_journal_store;
mod migration_planning;
mod migration_recovery;
mod migration_runtime;
mod migration_types;
mod modem_transfer;
mod one_key_commands;
mod one_key_prompt;
mod one_key_runtime;
mod outbound_events;
mod outbound_io;
mod portable_vault;
mod profile_commands;
mod profile_normalization;
mod profile_security;
mod proxy_protocol;
mod scp_protocol;
mod secret_commands;
mod secret_provider;
mod serial_capture;
mod serial_commands;
mod serial_transport;
mod session_commands;
mod session_events;
mod sftp_backend;
mod shell_transport;
mod sqlite_mirror;
mod sqlite_schema;
mod sqlite_store;
mod ssh_authentication;
mod ssh_backend;
mod ssh_connection;
mod ssh_exec;
mod ssh_health;
mod ssh_host_key_commands;
mod ssh_host_key_scan;
mod ssh_identity_commands;
mod ssh_runtime;
mod ssh_security;
mod ssh_transport;
mod ssh_tunnel;
mod state;
mod state_snapshot;
mod store_normalization;
mod store_persistence;
mod store_transactions;
mod sysmon_commands;
mod sysmon_remote_parsing;
mod sysmon_runtime;
mod system_event_sink;
mod tcp_transport;
mod telnet_protocol;
mod terminal_export_commands;
mod tmux_commands;
mod tmux_protocol;
mod tmux_runtime;
mod transfer_commands;
mod transfer_runtime;
mod trigger_runtime;
mod tunnel_commands;
mod vault_commands;

pub use app_bootstrap::run;
pub use command_types::*;
use sysmon_runtime::*;

use app_data_migration::*;
use archive_support::*;
use bundle_export::*;
use bundle_signing::*;
use file_batch::*;
use file_operations::*;
use file_transfer::*;
use log_commands::bounded_log_query_limit;
use log_storage::*;
use mcp_authorization::*;
#[cfg(test)]
use mcp_commands::export_mcp_audit_inner;
use mcp_control::*;
use mcp_execution::*;
use mcp_ipc::*;
use migration_diagnostics::*;
use migration_journal_store::*;
use migration_planning::*;
use migration_recovery::*;
use migration_runtime::*;
use migration_types::*;
#[cfg(test)]
use modem_transfer::runtime_tap_receiver;
use modem_transfer::*;
use one_key_prompt::*;
use one_key_runtime::*;
use outbound_events::*;
use outbound_io::*;
use portable_vault::*;
#[cfg(test)]
use profile_commands::validate_profile_tunnels;
use profile_normalization::*;
use profile_security::*;
use proxy_protocol::*;
use scp_protocol::*;
use secret_provider::*;
use serial_capture::*;
#[cfg(test)]
use serial_commands::{
    apply_serial_line_updates_with, pulse_serial_break_with, record_applied_serial_line_state,
    SerialControlLine,
};
use serial_transport::*;
use session_commands::{
    apply_proxy_password_update_with_io, merge_expected_json_value, merge_expected_profile_update,
    validate_expected_proxy_password, validate_profile_transport_change,
};
#[cfg(test)]
use session_commands::{
    apply_session_open_profile_credentials, cancel_pending_session_opens,
    delete_session_profile_inner, register_session_open_cancellation, resize_session_inner,
    resize_session_profile_in_store, session_has_registered_runtime, session_lifecycle_lane,
};
use session_commands::{
    close_session_inner, mark_session_connected_with_events, open_session_inner,
    profile_requires_runtime, SessionOpenCredentials, MAX_CONCURRENT_SESSION_OPENS,
};
use session_commands::{terminal_key_sequence_for_protocol, terminate_command_for_protocol};
use session_events::*;
use session_events::{append_logging_error, append_logging_errors, sync_stored_event};
use sftp_backend::*;
use shell_transport::*;
use sqlite_schema::*;
use sqlite_store::*;
use ssh_authentication::*;
use ssh_backend::*;
use ssh_connection::*;
use ssh_exec::*;
#[cfg(test)]
use ssh_host_key_commands::{
    delete_host_keys_from_store, merge_expected_host_key_update, update_host_key_in_store,
};
use ssh_host_key_scan::*;
use ssh_runtime::*;
use ssh_security::*;
use ssh_transport::*;
use ssh_tunnel::*;
use state::*;
use state_snapshot::*;
use store_normalization::*;
use store_persistence::*;
use store_transactions::*;
use sysmon_remote_parsing::*;
use system_event_sink::*;
use tcp_transport::*;
use telnet_protocol::*;
#[cfg(test)]
use terminal_export_commands::{export_terminal_text_inner, validate_terminal_text_export_request};
use tmux_protocol::*;
use tmux_runtime::*;
use transfer_runtime::*;
use trigger_runtime::*;

const STORE_KEY: &str = "session-store";
const STREAM_PERSIST_INTERVAL: Duration = Duration::from_secs(2);
const SSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const SSH_READER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const TEST_RUNTIME_TRANSITION_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SESSION_PROFILE_NAME_CHARACTERS: usize = 128;
const MAX_SESSION_PROFILE_ID_CHARACTERS: usize = 256;
const MAX_SESSION_PROFILE_GROUP_CHARACTERS: usize = 256;
const MAX_SESSION_PROFILE_TAGS: usize = 32;
const MAX_SESSION_PROFILE_TAG_CHARACTERS: usize = 64;
const PORTABLE_VAULT_CLIENT: &[u8] = b"portmate-secrets";
const DEFAULT_LOG_QUERY_LIMIT: u64 = 100;
const MAX_LOG_QUERY_LIMIT: u64 = 1000;
const DEFAULT_SYSMON_HISTORY_QUERY_LIMIT: usize = 120;
const MAX_MCP_AUDIT_EXPORT_RECORDS: usize = 5_000;
const MAX_MCP_AUDIT_EXPORT_RECORD_BYTES: usize = 64 * 1024;
const MAX_MCP_AUDIT_EXPORT_BYTES: usize = 16 * 1024 * 1024;
const MAX_LOG_RETENTION_DAYS: u32 = 3_650;
const MAX_TERMINAL_TEXT_EXPORT_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_TERMINAL_THEME: &str = "portmate-dark";
const DEFAULT_TERMINAL_NAME: &str = "xterm-256color";
const DEFAULT_TERMINAL_FONT_FAMILY: &str = "Roboto Mono, JetBrains Mono, monospace";
const MIN_TERMINAL_ROWS: u16 = 1;
const MAX_TERMINAL_ROWS: u16 = 512;
const MIN_TERMINAL_COLS: u16 = 1;
const MAX_TERMINAL_COLS: u16 = 1024;
const MAX_TERMINAL_SCROLLBACK: u32 = 10_000_000;
const MIN_TERMINAL_FONT_SIZE: u8 = 6;
const MAX_TERMINAL_FONT_SIZE: u8 = 72;
const MIN_TERMINAL_BACKGROUND_OPACITY: u8 = 20;
const MAX_TERMINAL_BACKGROUND_OPACITY: u8 = 100;
const MAX_TERMINAL_NAME_BYTES: usize = 64;
const MAX_TERMINAL_FONT_FAMILY_CHARACTERS: usize = 256;
const SUPPORTED_TERMINAL_THEMES: [&str; 4] = [
    DEFAULT_TERMINAL_THEME,
    "graphite",
    "solarized-dark",
    "portmate-light",
];
const RECONNECT_DELAY_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

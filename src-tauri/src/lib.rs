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
mod external_drop_execution;
mod external_drop_planning;
mod file_batch;
mod file_batch_planning;
mod file_commands;
mod file_create_delete;
mod file_delete;
mod file_metadata;
mod file_operation_paths;
mod file_operations;
mod file_transfer;
mod log_bytes_ref;
mod log_commands;
mod log_query;
mod log_retention;
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
mod modem_protocol;
mod modem_remote;
mod modem_runtime;
mod modem_transfer;
mod modem_xmodem;
mod modem_ymodem;
mod modem_zmodem;
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
mod remote_copy;
mod scp_channel;
mod scp_commands;
mod scp_download;
mod scp_download_wire;
mod scp_source;
mod scp_upload;
mod secret_commands;
mod secret_provider;
mod serial_capture;
mod serial_commands;
mod serial_reconnect;
mod serial_reconnect_runtime;
mod serial_transport;
mod session_close;
mod session_commands;
mod session_events;
mod session_open;
mod session_profile_delete;
mod session_terminal;
mod sftp_backend;
mod sftp_paths;
mod sftp_transfer;
mod shell_transport;
mod sqlite_mirror;
mod sqlite_schema;
mod sqlite_store;
mod ssh_agent;
mod ssh_authentication;
mod ssh_auxiliary;
mod ssh_backend;
mod ssh_channel;
mod ssh_connection;
mod ssh_connection_steps;
mod ssh_exec;
mod ssh_handler;
mod ssh_health;
mod ssh_host_key_commands;
mod ssh_host_key_scan;
mod ssh_host_key_temporary;
mod ssh_identity_commands;
mod ssh_libssh_authentication;
mod ssh_libssh_bridge;
mod ssh_libssh_channel;
mod ssh_libssh_transport;
mod ssh_reader;
mod ssh_reconnect;
mod ssh_reconnect_runtime;
mod ssh_runtime;
mod ssh_security;
mod ssh_session_lifecycle;
mod ssh_transport;
mod ssh_transport_setup;
mod ssh_tunnel;
mod ssh_tunnel_health;
mod ssh_tunnel_io;
mod ssh_tunnel_lifecycle;
mod ssh_tunnel_request;
mod ssh_tunnel_restore;
mod ssh_tunnel_runtime;
mod ssh_tunnel_store;
mod state;
mod state_snapshot;
mod store_compatibility;
mod store_normalization;
mod store_persistence;
mod store_transactions;
mod sysmon_commands;
mod sysmon_linux_network;
mod sysmon_linux_network_fallback;
mod sysmon_local_command;
mod sysmon_metrics;
mod sysmon_network;
mod sysmon_network_io;
mod sysmon_remote_parsing;
mod sysmon_runtime;
mod system_event_sink;
mod tcp_connection;
mod tcp_reconnect;
mod tcp_reconnect_runtime;
mod tcp_transport;
mod telnet_protocol;
mod terminal_export_commands;
mod tmux_commands;
mod tmux_protocol;
mod tmux_runtime;
mod transfer_commands;
mod transfer_progress;
mod transfer_request;
mod transfer_runtime;
mod transport_timing;
mod trigger_runtime;
mod tunnel_commands;
mod vault_commands;
mod webkit_runtime;

pub use app_bootstrap::run;
pub use command_types::*;
use sysmon_linux_network::*;
use sysmon_linux_network_fallback::*;
use sysmon_local_command::*;
use sysmon_metrics::*;
use sysmon_network::*;
use sysmon_network_io::*;
use sysmon_runtime::*;

use app_data_migration::*;
use archive_support::*;
use bundle_export::*;
use bundle_signing::*;
use external_drop_execution::*;
use external_drop_planning::*;
use file_batch::*;
use file_batch_planning::*;
use file_create_delete::*;
use file_delete::*;
use file_metadata::*;
use file_operation_paths::*;
use file_operations::*;
use file_transfer::*;
use log_bytes_ref::*;
use log_commands::bounded_log_query_limit;
use log_query::*;
use log_retention::*;
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
use modem_protocol::*;
use modem_remote::*;
use modem_runtime::*;
use modem_transfer::*;
use modem_xmodem::*;
use modem_ymodem::*;
use modem_zmodem::*;
use one_key_prompt::*;
use one_key_runtime::*;
use outbound_events::*;
use outbound_io::*;
use portable_vault::*;
use profile_commands::merge_expected_json_value;
#[cfg(test)]
use profile_commands::{
    apply_proxy_password_update_with_io, merge_expected_profile_update,
    validate_expected_proxy_password, validate_profile_transport_change, validate_profile_tunnels,
};
use profile_normalization::*;
use profile_security::*;
use proxy_protocol::*;
use remote_copy::*;
use scp_channel::*;
use scp_commands::*;
use scp_download::*;
use scp_download_wire::*;
use scp_source::*;
use scp_upload::*;
use secret_provider::*;
use serial_capture::*;
#[cfg(test)]
use serial_commands::{
    apply_serial_line_updates_with, pulse_serial_break_with, record_applied_serial_line_state,
    SerialControlLine,
};
use serial_reconnect::*;
use serial_reconnect_runtime::*;
use serial_transport::*;
use session_close::close_session_inner;
#[cfg(test)]
use session_close::session_has_registered_runtime;
use session_commands::{mark_session_connected_with_events, profile_requires_runtime};
use session_events::*;
use session_events::{append_logging_error, append_logging_errors, sync_stored_event};
#[cfg(test)]
use session_open::{
    apply_session_open_profile_credentials, cancel_pending_session_opens,
    register_session_open_cancellation, session_lifecycle_lane,
};
use session_open::{open_session_inner, SessionOpenCredentials, MAX_CONCURRENT_SESSION_OPENS};
#[cfg(test)]
use session_profile_delete::delete_session_profile_inner;
#[cfg(test)]
use session_terminal::{resize_session_inner, resize_session_profile_in_store};
use session_terminal::{terminal_key_sequence_for_protocol, terminate_command_for_protocol};
use sftp_backend::*;
use sftp_paths::*;
use sftp_transfer::*;
use shell_transport::*;
use sqlite_schema::*;
use sqlite_store::*;
use ssh_agent::*;
use ssh_authentication::*;
use ssh_auxiliary::*;
use ssh_backend::*;
use ssh_channel::*;
use ssh_connection::*;
use ssh_connection_steps::*;
use ssh_exec::*;
use ssh_handler::*;
#[cfg(test)]
use ssh_host_key_commands::{
    delete_host_keys_from_store, merge_expected_host_key_update, update_host_key_in_store,
};
use ssh_host_key_scan::*;
use ssh_host_key_temporary::*;
use ssh_libssh_authentication::*;
use ssh_libssh_bridge::*;
use ssh_libssh_channel::*;
use ssh_libssh_transport::*;
use ssh_reader::*;
use ssh_reconnect::*;
use ssh_reconnect_runtime::*;
use ssh_runtime::*;
use ssh_security::*;
use ssh_session_lifecycle::*;
use ssh_transport::*;
use ssh_transport_setup::*;
use ssh_tunnel::*;
use ssh_tunnel_health::*;
use ssh_tunnel_io::*;
use ssh_tunnel_lifecycle::*;
use ssh_tunnel_request::*;
use ssh_tunnel_restore::*;
use ssh_tunnel_runtime::*;
use ssh_tunnel_store::*;
use state::*;
use state_snapshot::*;
use store_compatibility::*;
use store_normalization::*;
use store_persistence::*;
use store_transactions::*;
use sysmon_remote_parsing::*;
use system_event_sink::*;
use tcp_connection::*;
use tcp_reconnect::*;
use tcp_reconnect_runtime::*;
use tcp_transport::*;
use telnet_protocol::*;
#[cfg(test)]
use terminal_export_commands::{export_terminal_text_inner, validate_terminal_text_export_request};
use tmux_protocol::*;
use tmux_runtime::*;
use transfer_progress::*;
use transfer_request::*;
use transfer_runtime::*;
use trigger_runtime::*;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;

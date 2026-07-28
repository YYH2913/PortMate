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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, AsyncWriteExt};
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
mod shell_transport;
mod sqlite_mirror;
mod sqlite_schema;
mod sqlite_store;
mod ssh_backend;
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
use shell_transport::*;
use sqlite_schema::*;
use sqlite_store::*;
use ssh_backend::*;
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
mod tests {
    use super::*;
    use std::process::Command;

    #[path = "app_migration_tests.rs"]
    mod app_migration_tests;
    #[path = "archive_tests.rs"]
    mod archive_tests;
    #[path = "connection_config_tests.rs"]
    mod connection_config_tests;
    #[path = "export_tests.rs"]
    mod export_tests;
    #[path = "external_sftp_compat.rs"]
    mod external_sftp_compat;
    #[path = "external_ssh_compat.rs"]
    mod external_ssh_compat;
    #[path = "external_ssh_gssapi_compat.rs"]
    mod external_ssh_gssapi_compat;
    #[path = "external_tcp_telnet_compat.rs"]
    mod external_tcp_telnet_compat;
    #[path = "host_key_tests.rs"]
    mod host_key_tests;
    #[path = "identity_tests.rs"]
    mod identity_tests;
    #[path = "mcp_tests.rs"]
    mod mcp_tests;
    #[path = "migration_tests.rs"]
    mod migration_tests;
    #[path = "modem_protocol_tests.rs"]
    mod modem_protocol_tests;
    #[path = "openssh_integration_tests.rs"]
    mod openssh_integration_tests;
    #[path = "portable_vault_tests.rs"]
    mod portable_vault_tests;
    #[path = "proxy_runtime_tests.rs"]
    mod proxy_runtime_tests;
    #[path = "runtime_capacity_tests.rs"]
    mod runtime_capacity_tests;
    #[path = "scp_protocol_tests.rs"]
    mod scp_protocol_tests;
    #[path = "serial_tests.rs"]
    mod serial_tests;
    #[path = "session_lifecycle_tests.rs"]
    mod session_lifecycle_tests;
    #[path = "session_logging_tests.rs"]
    mod session_logging_tests;
    #[path = "session_profile_tests.rs"]
    mod session_profile_tests;
    #[path = "ssh_policy_tests.rs"]
    mod ssh_policy_tests;
    #[path = "ssh_runtime_tests.rs"]
    mod ssh_runtime_tests;
    #[path = "ssh_test_support.rs"]
    mod ssh_test_support;
    #[path = "ssh_transport_tests.rs"]
    mod ssh_transport_tests;
    #[path = "storage_tests.rs"]
    mod storage_tests;
    #[path = "store_normalization_tests.rs"]
    mod store_normalization_tests;
    #[path = "sysmon_tests.rs"]
    mod sysmon_tests;
    #[path = "tcp_telnet_tests.rs"]
    mod tcp_telnet_tests;
    #[path = "tmux_protocol_tests.rs"]
    mod tmux_protocol_tests;
    #[path = "transfer_tests.rs"]
    mod transfer_tests;
    #[path = "transport_runtime_tests.rs"]
    mod transport_runtime_tests;
    #[path = "trigger_tests.rs"]
    mod trigger_tests;
    #[path = "tunnel_tests.rs"]
    mod tunnel_tests;

    use ssh_test_support::*;

    fn shared_runtime_test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn vault_identity(id: &str, secret_ref: &str) -> IdentityRef {
        IdentityRef {
            id: id.to_string(),
            label: id.to_string(),
            source: IdentitySource::ProfileVault,
            fingerprint_sha256: Some("SHA256:test".to_string()),
            path: None,
            secret_ref: Some(secret_ref.to_string()),
        }
    }

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

    async fn read_test_http_header(stream: &mut TcpStream) -> Vec<u8> {
        let mut header = Vec::new();
        let mut byte = [0_u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            assert!(header.len() < MAX_HTTP_CONNECT_RESPONSE_BYTES);
            stream.read_exact(&mut byte).await.unwrap();
            header.push(byte[0]);
        }
        header
    }

    fn test_connect_target(header: &[u8]) -> (String, u16) {
        let header = std::str::from_utf8(header).unwrap();
        let authority = header
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap();
        let (host, port) = authority.rsplit_once(':').unwrap();
        (
            host.trim_matches(['[', ']']).to_string(),
            port.parse().unwrap(),
        )
    }

    async fn spawn_test_http_connect_proxy(
        response_status: u16,
    ) -> (u16, Arc<AtomicU64>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicU64::new(0));
        let task_connections = Arc::clone(&connections);
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut client, _)) = listener.accept().await else {
                    return;
                };
                let connections = Arc::clone(&task_connections);
                tokio::spawn(async move {
                    let header = read_test_http_header(&mut client).await;
                    connections.fetch_add(1, Ordering::SeqCst);
                    if response_status != 200 {
                        client
                            .write_all(
                                format!("HTTP/1.1 {response_status} Rejected\r\n\r\n").as_bytes(),
                            )
                            .await
                            .unwrap();
                        return;
                    }
                    let (host, port) = test_connect_target(&header);
                    let Ok(mut target) = TcpStream::connect((host.as_str(), port)).await else {
                        let _ = client
                            .write_all(b"HTTP/1.1 502 Target Unavailable\r\n\r\n")
                            .await;
                        return;
                    };
                    client
                        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                        .await
                        .unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
                });
            }
        });
        (port, connections, task)
    }

    async fn spawn_test_socks5_proxy(
        reply: u8,
    ) -> (u16, Arc<AtomicU64>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let connections = Arc::new(AtomicU64::new(0));
        let task_connections = Arc::clone(&connections);
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut client, _)) = listener.accept().await else {
                    return;
                };
                let connections = Arc::clone(&task_connections);
                tokio::spawn(async move {
                    let mut greeting = [0_u8; 3];
                    client.read_exact(&mut greeting).await.unwrap();
                    assert_eq!(greeting, [0x05, 0x01, 0x00]);
                    client.write_all(&[0x05, 0x00]).await.unwrap();

                    let mut request_header = [0_u8; 5];
                    client.read_exact(&mut request_header).await.unwrap();
                    assert_eq!(&request_header[..4], &[0x05, 0x01, 0x00, 0x03]);
                    let mut host = vec![0_u8; usize::from(request_header[4])];
                    client.read_exact(&mut host).await.unwrap();
                    let mut port_bytes = [0_u8; 2];
                    client.read_exact(&mut port_bytes).await.unwrap();
                    connections.fetch_add(1, Ordering::SeqCst);
                    if reply != 0 {
                        client
                            .write_all(&[0x05, reply, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
                            .await
                            .unwrap();
                        return;
                    }
                    let host = String::from_utf8(host).unwrap();
                    let port = u16::from_be_bytes(port_bytes);
                    let mut target = TcpStream::connect((host.as_str(), port)).await.unwrap();
                    client
                        .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
                        .await
                        .unwrap();
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut target).await;
                });
            }
        });
        (port, connections, task)
    }

    async fn spawn_test_http_auth_endpoint(
        expected_authorization: String,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut client, _) = listener.accept().await.unwrap();
            let header = read_test_http_header(&mut client).await;
            let header = std::str::from_utf8(&header).unwrap();
            let authenticated = header
                .split("\r\n")
                .any(|line| line == expected_authorization);
            let response = if authenticated {
                b"HTTP/1.1 200 Connection Established\r\n\r\n".as_slice()
            } else {
                b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n".as_slice()
            };
            client.write_all(response).await.unwrap();
        });
        (port, task)
    }

    async fn spawn_test_socks5_auth_endpoint(
        expected_username: String,
        expected_password: String,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut client, _) = listener.accept().await.unwrap();
            let mut greeting = [0_u8; 3];
            client.read_exact(&mut greeting).await.unwrap();
            assert_eq!(greeting, [0x05, 0x01, 0x02]);
            client.write_all(&[0x05, 0x02]).await.unwrap();

            let mut auth_header = [0_u8; 2];
            client.read_exact(&mut auth_header).await.unwrap();
            assert_eq!(auth_header[0], 0x01);
            let mut username = vec![0_u8; usize::from(auth_header[1])];
            client.read_exact(&mut username).await.unwrap();
            let mut password_len = [0_u8; 1];
            client.read_exact(&mut password_len).await.unwrap();
            let mut password = vec![0_u8; usize::from(password_len[0])];
            client.read_exact(&mut password).await.unwrap();
            let authenticated = username == expected_username.as_bytes()
                && password == expected_password.as_bytes();
            client
                .write_all(&[0x01, if authenticated { 0x00 } else { 0x01 }])
                .await
                .unwrap();
            if !authenticated {
                return;
            }

            let mut request_header = [0_u8; 5];
            client.read_exact(&mut request_header).await.unwrap();
            assert_eq!(&request_header[..4], &[0x05, 0x01, 0x00, 0x03]);
            let mut address_and_port = vec![0_u8; usize::from(request_header[4]) + 2];
            client.read_exact(&mut address_and_port).await.unwrap();
            client
                .write_all(&[0x05, 0x00, 0x00, 0x01, 127, 0, 0, 1, 0, 0])
                .await
                .unwrap();
        });
        (port, task)
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

    #[cfg(unix)]
    async fn spawn_stalled_ssh_endpoint() -> (u16, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        (port, task)
    }

    fn assert_tunnel_client_closed(result: std::io::Result<usize>, label: &str) {
        match result {
            Ok(0) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::BrokenPipe
                ) => {}
            Ok(bytes) => panic!("{label} remained open and returned {bytes} bytes"),
            Err(error) => panic!("{label} closed with unexpected error: {error}"),
        }
    }

    #[test]
    fn one_key_completion_writes_value_with_prompt_audit_without_readable_text() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let expected = b"private-value\r".to_vec();
            let expected_len = expected.len();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut received = vec![0_u8; expected_len];
                socket.read_exact(&mut received).await.unwrap();
                let _ = release_rx.await;
                received
            });

            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: false,
                ..Default::default()
            }));
            let root =
                std::env::temp_dir().join(format!("portmate-one-key-send-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            open_tcp_session(&state, profile.clone()).await.unwrap();
            let (prompt_event_id, one_key_updated_at) = {
                let mut store = state.store.lock().unwrap();
                let now = Utc::now();
                store.one_keys.push(OneKeyCredential {
                    id: "onekey:completion".to_string(),
                    label: "Completion".to_string(),
                    kind: OneKeyKind::Account,
                    username: "operator".to_string(),
                    password_secret_ref: Some("keychain:completion".to_string()),
                    passphrase_secret_ref: None,
                    identity: None,
                    session_ids: vec![profile.id.clone()],
                    created_at: now,
                    updated_at: now,
                });
                let prompt_event_id = store
                    .record_stream_event(
                        &profile.id,
                        EventDirection::Inbound,
                        EventStream::Stdout,
                        "Password:",
                    )
                    .unwrap()
                    .id;
                (prompt_event_id, now)
            };
            let validation = OneKeyPromptValidation {
                one_key_id: "onekey:completion".to_string(),
                one_key_updated_at,
                field: OneKeyField::Password,
                prompt_event_id: prompt_event_id.clone(),
            };

            let event = send_one_key_value(
                state.session_io(),
                &profile.id,
                "private-value",
                "one-key-completion",
                Some(&prompt_event_id),
                Some(&validation),
            )
            .await
            .unwrap();
            assert!(event.text.is_none());
            assert_eq!(
                event.annotations.get("origin").map(String::as_str),
                Some("one-key-completion")
            );
            assert_eq!(
                event.annotations.get("relatedEventId").map(String::as_str),
                Some(prompt_event_id.as_str())
            );
            assert!(!serde_json::to_string(&event)
                .unwrap()
                .contains("private-value"));
            close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
            let _ = release_tx.send(());
            let received = tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("OneKey loopback server timed out")
                .expect("OneKey loopback server failed");
            assert_eq!(received, expected);
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn mcp_http_config_uses_bridge_token_ref_and_loopback_endpoint() {
        let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
        let store_path = Path::new("/home/operator/PortMate Data/portmate-store.sqlite3");
        let config = build_mcp_http_config(true, executable, store_path);
        assert_eq!(config.token_ref, MCP_HTTP_TOKEN_REF);
        assert_eq!(config.endpoint, "http://127.0.0.1:8787/mcp");
        assert!(config.token_available);
        assert!(config.start_command.contains("PORTMATE_MCP_HTTP=1"));
        assert_eq!(config.executable, executable.to_string_lossy());
        assert_eq!(config.store_path, store_path.to_string_lossy());
        assert!(config
            .start_command
            .contains("/opt/PortMate/bin/portmate-mcp"));
        assert!(config
            .start_command
            .contains("'/home/operator/PortMate Data/portmate-store.sqlite3'"));
        assert!(!config.start_command.contains("cargo run"));
        assert!(!config.start_command.contains(MCP_HTTP_TOKEN_REF));
        assert!(!config.start_command.contains("keychain:"));
        assert!(!config.start_command.contains("PORTMATE_MCP_HTTP_TOKEN"));
        assert!(!config.start_command.contains("example-token-body"));
        #[cfg(not(windows))]
        assert_eq!(
            config.start_command,
            "PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3' PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR=127.0.0.1:8787 PORTMATE_MCP_HTTP_ORIGINS=http://127.0.0.1:8787 '/opt/PortMate/bin/portmate-mcp' --http"
        );
        #[cfg(windows)]
        assert_eq!(
            config.start_command,
            "$env:PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3'; $env:PORTMATE_MCP_HTTP='1'; $env:PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787'; $env:PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787'; & '/opt/PortMate/bin/portmate-mcp' --http"
        );
    }

    #[test]
    fn shell_exit_status_disconnect_reason_preserves_code_and_signal() {
        assert_eq!(
            shell_exit_status_disconnect_reason("sh", &portable_pty::ExitStatus::with_exit_code(7)),
            "shell process exited with status 7 (sh)"
        );
        assert_eq!(
            shell_exit_status_disconnect_reason(
                "sh",
                &portable_pty::ExitStatus::with_signal("TERM")
            ),
            "shell process exited by signal TERM (sh)"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_fast_exit_cannot_leave_a_stale_connected_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let mut profile = test_shell_profile();
        let ConnectionConfig::Shell(shell) = &mut profile.connection else {
            panic!("expected Shell profile");
        };
        shell.args = vec!["-c".to_string(), "exit 7".to_string()];
        let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));

        let opened = open_shell_session(&state, profile.clone()).unwrap();
        assert_eq!(opened.runtime.status, SessionStatus::Connected);

        let started = Instant::now();
        let disconnected = loop {
            let summary = state
                .store
                .lock()
                .unwrap()
                .summaries()
                .into_iter()
                .find(|summary| summary.profile.id == profile.id)
                .unwrap();
            if summary.runtime.status == SessionStatus::Disconnected {
                break summary;
            }
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "fast Shell exit left the session connected"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(
            disconnected.runtime.last_disconnect_reason.as_deref(),
            Some("shell process exited with status 7 (sh)")
        );
        assert!(!state.shell.lock().unwrap().contains_key(&profile.id));
        let store = state.store.lock().unwrap();
        assert_eq!(
            store
                .events
                .iter()
                .filter(|event| {
                    event.text.as_deref()
                        == Some("PortMate: shell process exited with status 7 (sh)")
                })
                .count(),
            1
        );
        assert!(store.events.iter().all(|event| {
            event
                .text
                .as_deref()
                .is_none_or(|text| !text.contains("shell closed"))
        }));
    }

    #[test]
    fn reader_start_gate_waits_for_the_connected_commit() {
        let gate = Arc::new(ReaderStartGate::default());
        let (sender, receiver) = std::sync::mpsc::channel();
        let task_gate = Arc::clone(&gate);
        let worker = std::thread::spawn(move || {
            sender.send(task_gate.wait()).unwrap();
        });

        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(30)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        gate.start();
        assert!(receiver.recv_timeout(Duration::from_secs(1)).unwrap());
        worker.join().unwrap();

        let gate = ReaderStartGate::default();
        gate.cancel();
        assert!(!gate.wait());
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

    fn test_sysmon_snapshot(session_id: &str) -> SysmonSnapshot {
        SysmonSnapshot {
            session_id: session_id.to_string(),
            ts: Utc::now(),
            uptime_seconds: 60,
            cpu_percent: 12.5,
            memory_percent: 25.0,
            rx_kbps: 1.0,
            tx_kbps: 2.0,
            load_average: [0.1, 0.2, 0.3],
            memory_total_bytes: 1_024,
            memory_available_bytes: 768,
            processes: Vec::new(),
            disks: Vec::new(),
            network_interfaces: Vec::new(),
        }
    }

    fn test_transfer_task(session_id: &str, status: TransferStatus) -> TransferTask {
        TransferTask {
            id: "transfer-commit-test".to_string(),
            session_id: session_id.to_string(),
            protocol: TransferProtocol::Sftp,
            source: "input.bin".to_string(),
            destination: "output.bin".to_string(),
            bytes_total: 0,
            bytes_done: 0,
            started_at: (status == TransferStatus::Running).then(Utc::now),
            finished_at: None,
            average_bytes_per_second: None,
            message: Some(
                match status {
                    TransferStatus::Queued => "queued",
                    TransferStatus::Running => "running",
                    TransferStatus::Completed => "completed",
                    TransferStatus::Failed => "failed",
                    TransferStatus::Cancelled => "cancelled",
                }
                .to_string(),
            ),
            status,
        }
    }

    fn test_app_state(profile: SessionProfile, store_path: PathBuf) -> AppState {
        let mut store = SessionStore::default();
        store.upsert_profile(profile);
        AppState {
            app_handle: None,
            store: Arc::new(Mutex::new(store)),
            credential_ops: Arc::new(Mutex::new(())),
            credential_lock_path: store_path.with_file_name("test-credentials.lock"),
            system_event_sink: Arc::new(Mutex::new(None)),
            session_open_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SESSION_OPENS)),
            ssh: Arc::new(Mutex::new(HashMap::new())),
            ssh_auxiliary_slots: Arc::new(tokio::sync::Semaphore::new(
                MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS,
            )),
            tmux_controls: Arc::new(Mutex::new(HashMap::new())),
            tmux_control_slots: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIVE_TMUX_CONTROLS)),
            shell: Arc::new(Mutex::new(HashMap::new())),
            tcp: Arc::new(Mutex::new(HashMap::new())),
            serial: Arc::new(Mutex::new(HashMap::new())),
            serial_captures: Arc::new(Mutex::new(HashMap::new())),
            active_commands: Arc::new(Mutex::new(HashMap::new())),
            tunnels: Arc::new(Mutex::new(HashMap::new())),
            tunnel_connection_slots: Arc::new(tokio::sync::Semaphore::new(MAX_TUNNEL_CONNECTIONS)),
            transfer_cancellations: Arc::new(Mutex::new(HashMap::new())),
            transfer_task_slots: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIVE_TRANSFER_TASKS)),
            transfer_lanes: Arc::new(Mutex::new(HashMap::new())),
            sysmon_slots: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SYSMON_REFRESHES)),
            trigger_command_slots: Arc::new(tokio::sync::Semaphore::new(
                MAX_TRIGGER_COMMAND_CONCURRENCY,
            )),
            trigger_send_batch_slots: Arc::new(tokio::sync::Semaphore::new(
                MAX_TRIGGER_SEND_BATCH_CONCURRENCY,
            )),
            pending_mcp_approvals: Arc::new(Mutex::new(HashMap::new())),
            one_time_host_keys: Arc::new(Mutex::new(HashMap::new())),
            ipc_publication: Arc::new(Mutex::new(IpcPublicationState::default())),
            ssh_reconnect_install_error: Arc::new(Mutex::new(None)),
            store_path,
        }
    }

    fn test_transfer_progress_context(
        state: &AppState,
        task_id: &str,
        cancel: Arc<AtomicBool>,
    ) -> TransferProgressContext {
        TransferProgressContext {
            state: state.clone(),
            task_id: task_id.to_string(),
            cancel,
            last_emit: Arc::new(Mutex::new(Instant::now())),
            started: Instant::now(),
            rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
            rate_limit_bytes_per_second: None,
        }
    }

    async fn wait_for_transfer_progress(state: &AppState, task_id: &str, label: &str) {
        let result = tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let task = state.store.lock().unwrap().transfer_by_id(task_id).unwrap();
                if task.status == TransferStatus::Running && task.bytes_done > 0 {
                    break Ok(());
                }
                if matches!(
                    task.status,
                    TransferStatus::Completed | TransferStatus::Failed | TransferStatus::Cancelled
                ) {
                    break Err(task);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(task)) => panic!("{label} reached a terminal state before progress: {task:?}"),
            Err(_) => {
                let task = state.store.lock().unwrap().transfer_by_id(task_id).unwrap();
                panic!("{label} did not report progress: {task:?}");
            }
        }
    }

    async fn wait_for_transfer_terminal_state(state: &AppState, task_id: &str) -> TransferTask {
        let result = tokio::time::timeout(Duration::from_secs(60), async {
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
        .await;
        match result {
            Ok(task) => task,
            Err(_) => {
                let task = state.store.lock().unwrap().transfer_by_id(task_id).unwrap();
                panic!("transfer did not reach a terminal state: {task:?}");
            }
        }
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
                reconnect_delay_ms: portmate_core::DEFAULT_SSH_RECONNECT_DELAY_MS,
                keepalive_enabled: true,
                keepalive_interval_seconds: portmate_core::DEFAULT_SSH_KEEPALIVE_INTERVAL_SECONDS,
                keepalive_max_missed: portmate_core::DEFAULT_SSH_KEEPALIVE_MAX_MISSED,
                tcp_keepalive_enabled: None,
                proxy: portmate_core::ProxyConfig::default(),
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

    struct TestMigrationJournalFixture {
        before: SessionStore,
        after: SessionStore,
        journal: LoadedProfileSecretMigrationJournal,
        values: HashMap<String, String>,
    }

    fn test_migration_journal_fixture() -> TestMigrationJournalFixture {
        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.password_secret_ref = Some("keychain:source-a".to_string());
            ssh.passphrase_secret_ref = Some("keychain:source-b".to_string());
        }
        let mut before = SessionStore::default();
        before.upsert_profile(profile);
        let request = ProfileSecretMigrationRequest {
            target_storage: SecretStorage::Portable,
            profile_ids: vec!["ssh-session-1".to_string()],
            cleanup_source: true,
        };
        let plan = build_profile_secret_migration_plan(&before, &request).unwrap();
        let prepared = vec![
            PreparedProfileSecretMigration {
                source_ref: "keychain:source-a".to_string(),
                target_ref: "stronghold:11111111-1111-4111-8111-111111111111".to_string(),
                secret: Zeroizing::new("private-a".to_string()),
            },
            PreparedProfileSecretMigration {
                source_ref: "keychain:source-b".to_string(),
                target_ref: "stronghold:22222222-2222-4222-8222-222222222222".to_string(),
                secret: Zeroizing::new("private-b".to_string()),
            },
        ];
        let replacements = prepared
            .iter()
            .map(|item| (item.source_ref.clone(), item.target_ref.clone()))
            .collect::<HashMap<_, _>>();
        let mut after = before.clone();
        replace_profile_secret_refs(&mut after.profiles[0], &replacements);
        let payload =
            build_profile_secret_migration_journal(&before, &after, &plan, &request, &prepared)
                .unwrap();
        validate_profile_secret_migration_journal(&payload).unwrap();
        let now = Utc::now();
        let values = prepared
            .iter()
            .flat_map(|item| {
                [
                    (item.source_ref.clone(), item.secret.to_string()),
                    (item.target_ref.clone(), item.secret.to_string()),
                ]
            })
            .collect();
        TestMigrationJournalFixture {
            before,
            after,
            journal: LoadedProfileSecretMigrationJournal {
                state: ProfileSecretMigrationJournalState::TargetWritePending,
                payload,
                created_at: now,
                updated_at: now,
            },
            values,
        }
    }
}

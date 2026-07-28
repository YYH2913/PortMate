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

    #[path = "archive_tests.rs"]
    mod archive_tests;
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
    #[path = "mcp_tests.rs"]
    mod mcp_tests;
    #[path = "migration_tests.rs"]
    mod migration_tests;
    #[path = "modem_protocol_tests.rs"]
    mod modem_protocol_tests;
    #[path = "portable_vault_tests.rs"]
    mod portable_vault_tests;
    #[path = "scp_protocol_tests.rs"]
    mod scp_protocol_tests;
    #[path = "serial_tests.rs"]
    mod serial_tests;
    #[path = "session_lifecycle_tests.rs"]
    mod session_lifecycle_tests;
    #[path = "session_logging_tests.rs"]
    mod session_logging_tests;
    #[path = "ssh_policy_tests.rs"]
    mod ssh_policy_tests;
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

    #[test]
    fn legacy_app_identifier_data_directory_migrates_atomically() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(LEGACY_APP_IDENTIFIER);
        let current = root.path().join("dev.portmate.desktop");
        fs::create_dir_all(legacy.join("logs")).unwrap();
        fs::write(legacy.join(STORE_FILE_NAME), b"store").unwrap();
        fs::write(legacy.join("logs/session.txt"), b"log").unwrap();
        fs::create_dir_all(current.join("mediakeys/v1")).unwrap();
        fs::write(current.join("mediakeys/v1/salt"), b"bootstrap").unwrap();

        migrate_legacy_app_data_dir(root.path(), &current).unwrap();

        assert!(!legacy.exists());
        assert_eq!(fs::read(current.join(STORE_FILE_NAME)).unwrap(), b"store");
        assert_eq!(fs::read(current.join("logs/session.txt")).unwrap(), b"log");
    }

    #[test]
    fn legacy_app_identifier_migration_refuses_to_merge_two_live_stores() {
        let root = tempfile::tempdir().unwrap();
        let legacy = root.path().join(LEGACY_APP_IDENTIFIER);
        let current = root.path().join("dev.portmate.desktop");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(legacy.join(STORE_FILE_NAME), b"legacy").unwrap();
        fs::write(current.join(STORE_FILE_NAME), b"current").unwrap();

        let error = migrate_legacy_app_data_dir(root.path(), &current).unwrap_err();

        assert!(error.contains("refusing to merge"), "{error}");
        assert_eq!(fs::read(legacy.join(STORE_FILE_NAME)).unwrap(), b"legacy");
        assert_eq!(fs::read(current.join(STORE_FILE_NAME)).unwrap(), b"current");
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
    fn terminal_resize_metadata_changes_memory_only_after_persistence_succeeds() {
        let mut store = SessionStore::default();
        let profile = test_shell_profile();
        let session_id = profile.id.clone();
        let original_size = (profile.terminal.cols, profile.terminal.rows);
        store.upsert_profile(profile);

        let error = commit_store_mutation_with(
            &mut store,
            |next_store| resize_session_profile_in_store(next_store, &session_id, 132, 43),
            |next_store| {
                let profile = next_store.profile(&session_id).unwrap();
                assert_eq!((profile.terminal.cols, profile.terminal.rows), (132, 43));
                Err("store conflict".to_string())
            },
            |_| Ok(false),
        )
        .unwrap_err();
        assert_eq!(error, "store conflict");
        let profile = store.profile(&session_id).unwrap();
        assert_eq!(
            (profile.terminal.cols, profile.terminal.rows),
            original_size
        );

        let summary = commit_store_mutation_with(
            &mut store,
            |next_store| resize_session_profile_in_store(next_store, &session_id, 132, 43),
            |_| Err("post-commit version read failed".to_string()),
            |_| Ok(true),
        )
        .unwrap();
        assert_eq!(
            (summary.profile.terminal.cols, summary.profile.terminal.rows),
            (132, 43)
        );
        let profile = store.profile(&session_id).unwrap();
        assert_eq!((profile.terminal.cols, profile.terminal.rows), (132, 43));
    }

    #[test]
    fn telnet_loopback_applies_binary_naws_resize_and_profile_terminal_type() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let (negotiated_tx, negotiated_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                accepted_tx.send(()).unwrap();
                release_rx.await.unwrap();
                socket
                    .write_all(&[
                        TELNET_IAC,
                        TELNET_DO,
                        TELNET_OPT_BINARY,
                        TELNET_IAC,
                        TELNET_WILL,
                        TELNET_OPT_BINARY,
                        TELNET_IAC,
                        TELNET_DO,
                        TELNET_OPT_NAWS,
                        TELNET_IAC,
                        TELNET_DO,
                        TELNET_OPT_TERMINAL_TYPE,
                    ])
                    .await
                    .unwrap();

                let initial_naws = telnet_naws_message(255, 511);
                let expected_negotiation = [
                    [TELNET_IAC, TELNET_WILL, TELNET_OPT_BINARY].as_slice(),
                    [TELNET_IAC, TELNET_DO, TELNET_OPT_BINARY].as_slice(),
                    [TELNET_IAC, TELNET_WILL, TELNET_OPT_NAWS].as_slice(),
                    initial_naws.as_slice(),
                    [TELNET_IAC, TELNET_WILL, TELNET_OPT_TERMINAL_TYPE].as_slice(),
                ]
                .concat();
                let mut negotiation = vec![0_u8; expected_negotiation.len()];
                socket.read_exact(&mut negotiation).await.unwrap();
                assert_eq!(negotiation, expected_negotiation);

                socket
                    .write_all(&[
                        TELNET_IAC,
                        TELNET_SB,
                        TELNET_OPT_TERMINAL_TYPE,
                        TELNET_TTYPE_SEND,
                        TELNET_IAC,
                        TELNET_SE,
                    ])
                    .await
                    .unwrap();
                let expected_terminal = [
                    [
                        TELNET_IAC,
                        TELNET_SB,
                        TELNET_OPT_TERMINAL_TYPE,
                        TELNET_TTYPE_IS,
                    ]
                    .as_slice(),
                    b"vt100".as_slice(),
                    [TELNET_IAC, TELNET_SE].as_slice(),
                ]
                .concat();
                let mut terminal = vec![0_u8; expected_terminal.len()];
                socket.read_exact(&mut terminal).await.unwrap();
                assert_eq!(terminal, expected_terminal);
                socket.write_all(&[b'\r', 0]).await.unwrap();
                negotiated_tx.send(()).unwrap();

                let mut text = [0_u8; 5];
                socket.read_exact(&mut text).await.unwrap();
                assert_eq!(&text, b"show\n");
                for expected in [telnet_naws_message(80, 24), telnet_naws_message(100, 40)] {
                    let mut resize = vec![0_u8; expected.len()];
                    socket.read_exact(&mut resize).await.unwrap();
                    assert_eq!(resize, expected);
                }
            });

            let mut profile = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: false,
                ..Default::default()
            }));
            profile.terminal.term = "vt100".to_string();
            profile.logging.enabled = true;
            profile.logging.raw = true;
            profile.logging.text = false;
            profile.logging.jsonl = false;
            let root = std::env::temp_dir()
                .join(format!("portmate-telnet-binary-naws-{}", Uuid::new_v4()));
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            open_tcp_session(&state, profile.clone()).await.unwrap();
            accepted_rx.await.unwrap();

            resize_session_inner(&state, profile.id.clone(), 255, 511)
                .await
                .unwrap();
            release_tx.send(()).unwrap();
            negotiated_rx.await.unwrap();

            let text_event =
                send_text_inner(state.session_io(), profile.id.clone(), "show\n".to_string())
                    .await
                    .unwrap();
            assert_eq!(
                read_log_bytes_ref(&state.store_path, text_event.bytes_ref.as_deref().unwrap())
                    .unwrap()
                    .2,
                b"show\n"
            );
            resize_session_inner(&state, profile.id.clone(), 80, 24)
                .await
                .unwrap();
            let summary = resize_session_inner(&state, profile.id.clone(), 100, 40)
                .await
                .unwrap();
            assert_eq!(
                (summary.profile.terminal.cols, summary.profile.terminal.rows),
                (100, 40)
            );

            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("Telnet BINARY/NAWS server timed out")
                .expect("Telnet BINARY/NAWS server failed");
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let disconnected = state
                        .store
                        .lock()
                        .unwrap()
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == profile.id)
                        .is_some_and(|summary| {
                            summary.runtime.status == SessionStatus::Disconnected
                        });
                    if disconnected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("Telnet BINARY/NAWS runtime did not close after EOF");

            let store = state.store.lock().unwrap();
            let stdout = store
                .events
                .iter()
                .filter(|event| {
                    event.session_id == profile.id && event.stream == EventStream::Stdout
                })
                .filter_map(|event| event.text.as_deref())
                .collect::<String>();
            assert!(stdout.contains("\r\0"));
            let control_bytes = store
                .events
                .iter()
                .filter(|event| {
                    event.session_id == profile.id
                        && event.direction == EventDirection::Outbound
                        && event.stream == EventStream::Control
                })
                .map(|event| {
                    read_log_bytes_ref(&state.store_path, event.bytes_ref.as_deref().unwrap())
                        .unwrap()
                        .2
                })
                .collect::<Vec<_>>();
            assert!(control_bytes.contains(&telnet_naws_message(255, 511)));
            assert!(control_bytes.contains(&telnet_naws_message(80, 24)));
            assert!(control_bytes.contains(&telnet_naws_message(100, 40)));
            drop(store);
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn telnet_user_sends_bind_exact_wire_bytes_to_outbound_events() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut text = [0_u8; 6];
                socket.read_exact(&mut text).await.unwrap();
                assert_eq!(&text, b"show\r\n");
                let mut raw = [0_u8; 3];
                socket.read_exact(&mut raw).await.unwrap();
                assert_eq!(raw, [0x01, TELNET_IAC, TELNET_IAC]);
                let mut modem = [0_u8; 3];
                socket.read_exact(&mut modem).await.unwrap();
                assert_eq!(modem, [MODEM_CAN, TELNET_IAC, TELNET_IAC]);
            });

            let mut profile =
                test_tcp_profile(ConnectionConfig::Telnet(portmate_core::TcpConnection {
                    host: "127.0.0.1".to_string(),
                    port: address.port(),
                    reconnect: false,
                    ..Default::default()
                }));
            profile.logging.enabled = true;
            profile.logging.raw = true;
            profile.logging.text = false;
            profile.logging.jsonl = false;
            let root =
                std::env::temp_dir().join(format!("portmate-telnet-send-{}", Uuid::new_v4()));
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            open_tcp_session(&state, profile.clone()).await.unwrap();

            let text_event =
                send_text_inner(state.session_io(), profile.id.clone(), "show\n".to_string())
                    .await
                    .unwrap();
            let bytes_event = send_bytes_inner(
                state.session_io(),
                profile.id.clone(),
                vec![0x01, TELNET_IAC],
            )
            .await
            .unwrap();
            write_runtime_bytes(&state, &profile.id, &[MODEM_CAN, TELNET_IAC])
                .await
                .unwrap();

            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("Telnet send server timed out")
                .expect("Telnet send server failed");
            assert_eq!(text_event.direction, EventDirection::Outbound);
            assert_eq!(bytes_event.direction, EventDirection::Outbound);
            assert_eq!(bytes_event.text.as_deref(), Some("Binary payload: 2 bytes"));
            assert_eq!(
                read_log_bytes_ref(&state.store_path, text_event.bytes_ref.as_deref().unwrap())
                    .unwrap()
                    .2,
                b"show\r\n"
            );
            assert_eq!(
                read_log_bytes_ref(&state.store_path, bytes_event.bytes_ref.as_deref().unwrap())
                    .unwrap()
                    .2,
                [0x01, TELNET_IAC, TELNET_IAC]
            );
            let audit_actions = state
                .store
                .lock()
                .unwrap()
                .audit
                .iter()
                .map(|record| record.action.clone())
                .collect::<Vec<_>>();
            assert_eq!(audit_actions, ["send_text", "send_bytes"]);
            let modem_event = state
                .store
                .lock()
                .unwrap()
                .events
                .iter()
                .find(|event| {
                    event.direction == EventDirection::Outbound
                        && event.stream == EventStream::Control
                        && event.annotations.get("origin").map(String::as_str) == Some("modem")
                })
                .cloned()
                .unwrap();
            assert!(modem_event.text.is_none());
            assert_eq!(
                read_log_bytes_ref(&state.store_path, modem_event.bytes_ref.as_deref().unwrap())
                    .unwrap()
                    .2,
                [MODEM_CAN, TELNET_IAC, TELNET_IAC]
            );

            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn concurrent_outbound_lane_matches_wire_raw_and_event_order() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut received = [0_u8; 6];
                socket.read_exact(&mut received).await.unwrap();
                received.to_vec()
            });

            let mut profile =
                test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                    host: "127.0.0.1".to_string(),
                    port: address.port(),
                    reconnect: false,
                    ..Default::default()
                }));
            profile.logging.enabled = true;
            profile.logging.raw = true;
            profile.logging.text = false;
            profile.logging.jsonl = false;
            let root =
                std::env::temp_dir().join(format!("portmate-outbound-lane-{}", Uuid::new_v4()));
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            let stream = TcpStream::connect(address).await.unwrap();
            let (_reader, writer) = stream.into_split();
            let (tap, _) = broadcast::channel(8);
            state.tcp.lock().unwrap().insert(
                profile.id.clone(),
                TcpRuntime {
                    runtime_id: Uuid::new_v4().to_string(),
                    writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
                    tap,
                    closed: Arc::new(AtomicBool::new(false)),
                    telnet: None,
                },
            );

            let lane = outbound_lane(&state.store_path, &profile.id).unwrap();
            let lane_guard = lane.lock().await;
            let barrier = Arc::new(tokio::sync::Barrier::new(4));
            let first = {
                let io = state.session_io();
                let session_id = profile.id.clone();
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    send_bytes_inner(io, session_id, vec![0x11, 0xa1]).await
                })
            };
            let second = {
                let io = state.session_io();
                let session_id = profile.id.clone();
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    send_bytes_inner(io, session_id, vec![0x22, 0xa2]).await
                })
            };
            let modem = {
                let state = state.clone();
                let session_id = profile.id.clone();
                let barrier = Arc::clone(&barrier);
                tokio::spawn(async move {
                    barrier.wait().await;
                    write_runtime_bytes(&state, &session_id, &[0x33, 0xa3]).await
                })
            };
            barrier.wait().await;
            tokio::task::yield_now().await;
            assert!(!first.is_finished());
            assert!(!second.is_finished());
            assert!(!modem.is_finished());
            drop(lane_guard);
            first.await.unwrap().unwrap();
            second.await.unwrap().unwrap();
            modem.await.unwrap().unwrap();
            let received = tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("TCP server timed out")
                .expect("TCP server failed");

            let event_bytes = state
                .store
                .lock()
                .unwrap()
                .events
                .iter()
                .filter(|event| {
                    event.session_id == profile.id
                        && event.direction == EventDirection::Outbound
                        && event.bytes_ref.is_some()
                })
                .flat_map(|event| {
                    read_log_bytes_ref(&state.store_path, event.bytes_ref.as_deref().unwrap())
                        .unwrap()
                        .2
                })
                .collect::<Vec<_>>();
            assert_eq!(event_bytes, received);
            let raw_path = log_shard_path(&state.store_path, &profile, "raw").unwrap();
            assert_eq!(fs::read(raw_path).unwrap(), received);

            let _ = fs::remove_dir_all(root);
        });
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

            let tcp = TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                ..Default::default()
            };
            let mut client = connect_tcp_socket(&tcp, "Telnet").await.unwrap();
            let mut incoming = [0_u8; 10];
            client.read_exact(&mut incoming).await.unwrap();
            let profile = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection::default()));
            let mut negotiator =
                TelnetNegotiator::new(TelnetRuntimeState::from_profile(&profile).unwrap());
            let (text, replies) = negotiator.filter(&incoming);
            assert_eq!(text, b"login: ");
            assert_eq!(replies.len(), 1);
            client.write_all(&replies[0]).await.unwrap();
            client
                .write_all(encode_telnet_outbound_text("show\n", false).as_bytes())
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

    #[cfg(unix)]
    #[test]
    fn telnet_tls_rejects_untrusted_certificate_and_connects_when_explicitly_allowed() {
        let _runtime_guard = shared_runtime_test_guard();
        tauri::async_runtime::block_on(async {
            use native_tls::{Identity, TlsAcceptor};
            use rcgen::generate_simple_self_signed;
            use tokio_native_tls::TlsAcceptor as TokioTlsAcceptor;

            let certificate = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
            let identity = Identity::from_pkcs8(
                certificate.cert.pem().as_bytes(),
                certificate.signing_key.serialize_pem().as_bytes(),
            )
            .unwrap();
            let acceptor = TokioTlsAcceptor::from(TlsAcceptor::builder(identity).build().unwrap());
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tauri::async_runtime::spawn(async move {
                for attempt in 0..2 {
                    let (stream, _) = listener.accept().await.unwrap();
                    match acceptor.accept(stream).await {
                        Ok(mut stream) => {
                            stream.write_all(b"__PORTMATE_TLS_OK__\\n").await.unwrap();
                            return;
                        }
                        Err(error) if attempt == 0 => {
                            eprintln!("expected first TLS certificate failure: {error}");
                        }
                        Err(error) => panic!("TLS server handshake failed: {error}"),
                    }
                }
                panic!("TLS client did not complete the allowed handshake");
            });

            let root =
                std::env::temp_dir().join(format!("portmate-telnet-tls-test-{}", Uuid::new_v4()));
            let mut rejected = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: false,
                tls_enabled: true,
                tls_server_name: Some("localhost".to_string()),
                ..Default::default()
            }));
            let rejected_state = test_app_state(rejected.clone(), root.join("rejected.sqlite3"));
            let error = open_tcp_session(&rejected_state, rejected.clone())
                .await
                .expect_err("untrusted TLS certificate should fail closed");
            assert!(
                error.contains("TLS 握手失败"),
                "unexpected TLS error: {error}"
            );

            rejected.connection = ConnectionConfig::Telnet(TcpConnection {
                tls_accept_invalid_cert: true,
                ..match rejected.connection {
                    ConnectionConfig::Telnet(tcp) => tcp,
                    _ => unreachable!(),
                }
            });
            let accepted_state = test_app_state(rejected.clone(), root.join("accepted.sqlite3"));
            open_tcp_session(&accepted_state, rejected.clone())
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if accepted_state
                        .store
                        .lock()
                        .unwrap()
                        .screen(&rejected.id)
                        .is_some_and(|screen| screen.contains("__PORTMATE_TLS_OK__"))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("TLS session did not receive server output");
            close_session_inner(&accepted_state, rejected.id.clone())
                .await
                .unwrap();
            server.await.unwrap();
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn tcp_loopback_reconnects_after_remote_disconnect() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (drop_first_tx, drop_first_rx) = tokio::sync::oneshot::channel();
            let (second_connected_tx, second_connected_rx) = tokio::sync::oneshot::channel();
            let (release_server_tx, release_server_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (first, _) = listener.accept().await.unwrap();
                let _ = drop_first_rx.await;
                drop(first);
                let (mut second, _) = listener.accept().await.unwrap();
                second.write_all(b"new generation\n").await.unwrap();
                let _ = second_connected_tx.send(());
                let _ = release_server_rx.await;
                drop(second);
            });

            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: true,
                ..Default::default()
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
            let io = state.session_io();
            set_active_command(&io, &profile.id, "stale-command-id");
            let _ = drop_first_tx.send(());

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
            assert!(active_command_id(&io, &profile.id).is_none());

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
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let reconnected_output = state
                        .store
                        .lock()
                        .unwrap()
                        .events
                        .iter()
                        .find(|event| event.text.as_deref() == Some("new generation\n"))
                        .cloned();
                    if let Some(event) = reconnected_output {
                        assert!(!event.annotations.contains_key("commandId"));
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("reconnected TCP output was not recorded");

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

    #[cfg(target_os = "linux")]
    #[test]
    fn tcp_read_failure_preserves_the_socket_error_as_the_disconnect_reason() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (reset_tx, reset_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let _ = reset_rx.await;
                SockRef::from(&socket)
                    .set_linger(Some(Duration::ZERO))
                    .unwrap();
                drop(socket);
            });
            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: false,
                ..Default::default()
            }));
            let root = std::env::temp_dir()
                .join(format!("portmate-tcp-read-error-test-{}", Uuid::new_v4()));
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

            open_tcp_session(&state, profile.clone()).await.unwrap();
            let _ = reset_tx.send(());
            server.await.unwrap();
            let reason = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let runtime = state
                        .store
                        .lock()
                        .unwrap()
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == profile.id)
                        .map(|summary| summary.runtime);
                    if let Some(runtime) =
                        runtime.filter(|runtime| runtime.status == SessionStatus::Disconnected)
                    {
                        break runtime.last_disconnect_reason;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("TCP read failure did not transition to disconnected");

            assert!(
                reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("TCP read failed")),
                "unexpected TCP disconnect reason: {reason:?}"
            );
            let read_error_events = state
                .store
                .lock()
                .unwrap()
                .events
                .iter()
                .filter(|event| {
                    event
                        .text
                        .as_deref()
                        .is_some_and(|text| text.contains("TCP read failed"))
                })
                .count();
            assert_eq!(read_error_events, 1);
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn tcp_reconnect_uses_latest_endpoint_and_stops_when_disabled() {
        tauri::async_runtime::block_on(async {
            let first_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let first_address = first_listener.local_addr().unwrap();
            let first_server = tokio::spawn(async move {
                let (socket, _) = first_listener.accept().await.unwrap();
                drop(first_listener);
                drop(socket);
            });

            let replacement_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let replacement_address = replacement_listener.local_addr().unwrap();
            let (replacement_connected_tx, replacement_connected_rx) =
                tokio::sync::oneshot::channel();
            let (drop_replacement_tx, drop_replacement_rx) = tokio::sync::oneshot::channel();
            let replacement_server = tokio::spawn(async move {
                let (socket, _) = replacement_listener.accept().await.unwrap();
                drop(replacement_listener);
                let _ = replacement_connected_tx.send(());
                let _ = drop_replacement_rx.await;
                drop(socket);
            });
            let (proxy_port, proxy_connections, proxy_task) =
                spawn_test_http_connect_proxy(200).await;

            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: first_address.port(),
                reconnect: true,
                reconnect_delay_ms: 5_000,
                ..Default::default()
            }));
            let root = std::env::temp_dir().join(format!(
                "portmate-tcp-latest-reconnect-test-{}",
                Uuid::new_v4()
            ));
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            open_tcp_session(&state, profile.clone()).await.unwrap();
            first_server.await.unwrap();

            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let reconnecting = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                        runtime.session_id == profile.id
                            && runtime.status == SessionStatus::Reconnecting
                    });
                    if reconnecting {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("TCP runtime never entered reconnecting state before profile update");

            {
                let mut store = state.store.lock().unwrap();
                let mut updated = store.profile(&profile.id).unwrap();
                updated.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
                    host: "127.0.0.1".to_string(),
                    port: replacement_address.port(),
                    reconnect: true,
                    reconnect_delay_ms: 100,
                    proxy: ProxyConfig {
                        enabled: true,
                        kind: ProxyKind::HttpConnect,
                        host: "127.0.0.1".to_string(),
                        port: proxy_port,
                        ..ProxyConfig::default()
                    },
                    ..Default::default()
                });
                store.upsert_profile(updated);
                save_store(&state.store_path, &store).unwrap();
            }

            tokio::time::timeout(Duration::from_millis(800), replacement_connected_rx)
                .await
                .expect("TCP reconnect did not use the updated endpoint and shorter delay")
                .expect("replacement TCP server dropped its connection signal");
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let connected = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                        runtime.session_id == profile.id
                            && runtime.status == SessionStatus::Connected
                    });
                    if connected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("TCP runtime did not commit the updated endpoint connection");

            let _ = drop_replacement_tx.send(());
            replacement_server.await.unwrap();
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let reconnecting = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                        runtime.session_id == profile.id
                            && runtime.status == SessionStatus::Reconnecting
                    });
                    if reconnecting {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("updated TCP runtime never re-entered reconnecting state");
            let second_disconnect_at = state
                .store
                .lock()
                .unwrap()
                .runtimes
                .iter()
                .find(|runtime| runtime.session_id == profile.id)
                .and_then(|runtime| runtime.last_disconnect)
                .expect("second TCP outage did not record its disconnect time");

            {
                let mut store = state.store.lock().unwrap();
                let mut disabled = store.profile(&profile.id).unwrap();
                if let ConnectionConfig::Tcp(tcp) = &mut disabled.connection {
                    tcp.reconnect = false;
                }
                store.upsert_profile(disabled);
                save_store(&state.store_path, &store).unwrap();
            }

            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let runtime_removed = !state.tcp.lock().unwrap().contains_key(&profile.id);
                    let disconnected = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                        runtime.session_id == profile.id
                            && runtime.status == SessionStatus::Disconnected
                            && runtime
                                .last_disconnect_reason
                                .as_deref()
                                .is_some_and(|reason| reason.contains("disabled"))
                    });
                    if runtime_removed && disconnected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("disabling TCP reconnect did not remove the pending runtime");
            let stopped_runtime = state
                .store
                .lock()
                .unwrap()
                .runtimes
                .iter()
                .find(|runtime| runtime.session_id == profile.id)
                .cloned()
                .expect("stopped TCP runtime summary is missing");
            assert_eq!(stopped_runtime.last_disconnect, Some(second_disconnect_at));

            let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
            assert!(screen.contains("reconnecting in 5000ms"));
            assert!(screen.contains("socket reconnected"));
            assert!(screen.contains("reconnect stopped"));
            assert!(proxy_connections.load(Ordering::SeqCst) >= 1);
            proxy_task.abort();
            let _ = proxy_task.await;
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn tcp_reconnect_store_commit_failure_does_not_install_runtime() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (drop_first_tx, drop_first_rx) = tokio::sync::oneshot::channel();
            let (second_connected_tx, second_connected_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (first, _) = listener.accept().await.unwrap();
                let _ = drop_first_rx.await;
                drop(first);

                let (mut second, _) = listener.accept().await.unwrap();
                let _ = second_connected_tx.send(());
                let mut byte = [0_u8; 1];
                tokio::time::timeout(Duration::from_secs(5), second.read(&mut byte))
                    .await
                    .expect("failed reconnect socket was not closed")
                    .unwrap()
            });

            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: true,
                reconnect_delay_ms: 100,
                ..Default::default()
            }));
            let root = std::env::temp_dir().join(format!(
                "portmate-tcp-reconnect-commit-failure-{}",
                Uuid::new_v4()
            ));
            fs::create_dir_all(&root).unwrap();
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            open_tcp_session(&state, profile.clone()).await.unwrap();

            fs::remove_dir_all(&root).unwrap();
            fs::write(&root, b"blocked").unwrap();
            let _ = drop_first_tx.send(());
            tokio::time::timeout(Duration::from_secs(5), second_connected_rx)
                .await
                .expect("TCP reconnect did not reach the replacement socket")
                .unwrap();

            let failed = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let summary = state
                        .store
                        .lock()
                        .unwrap()
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == profile.id)
                        .unwrap();
                    if summary.runtime.status == SessionStatus::Error {
                        break summary;
                    }
                    assert_ne!(
                        summary.runtime.status,
                        SessionStatus::Connected,
                        "failed TCP reconnect exposed a connected runtime"
                    );
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("TCP reconnect Store failure did not settle to Error");

            assert!(
                failed
                    .runtime
                    .last_disconnect_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("TCP reconnect install failed")),
                "unexpected reconnect failure: {:?}",
                failed.runtime.last_disconnect_reason
            );
            assert!(!state.tcp.lock().unwrap().contains_key(&profile.id));
            assert_eq!(server.await.unwrap(), 0);
            assert!(state.store.lock().unwrap().events.iter().any(|event| {
                event
                    .text
                    .as_deref()
                    .is_some_and(|text| text.contains("TCP reconnect install failed"))
            }));

            fs::remove_file(root).unwrap();
        });
    }

    #[test]
    fn tcp_disconnect_observes_reconnect_disabled_while_connected() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let _ = release_rx.await;
                drop(socket);
            });
            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: true,
                ..Default::default()
            }));
            let root = std::env::temp_dir().join(format!(
                "portmate-tcp-disable-connected-test-{}",
                Uuid::new_v4()
            ));
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            open_tcp_session(&state, profile.clone()).await.unwrap();

            {
                let mut store = state.store.lock().unwrap();
                let mut disabled = store.profile(&profile.id).unwrap();
                if let ConnectionConfig::Tcp(tcp) = &mut disabled.connection {
                    tcp.reconnect = false;
                }
                store.upsert_profile(disabled);
                save_store(&state.store_path, &store).unwrap();
            }
            let _ = release_tx.send(());
            server.await.unwrap();

            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let runtime_removed = !state.tcp.lock().unwrap().contains_key(&profile.id);
                    let disconnected = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                        runtime.session_id == profile.id
                            && runtime.status == SessionStatus::Disconnected
                    });
                    if runtime_removed && disconnected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("TCP disconnect ignored the latest reconnect=false setting");
            let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
            assert!(!screen.contains("reconnecting in 1000ms"));
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn cancelling_silent_xmodem_sends_can_and_stops_worker() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut cancel = [0_u8; 3];
                socket.read_exact(&mut cancel).await.unwrap();
                assert_eq!(cancel, [MODEM_CAN; 3]);
                let _ = release_rx.await;
            });

            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: false,
                ..Default::default()
            }));
            let root =
                std::env::temp_dir().join(format!("portmate-modem-cancel-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let source = root.join("source.bin");
            fs::write(&source, b"cancel this XModem transfer").unwrap();
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            open_tcp_session(&state, profile.clone()).await.unwrap();

            let task = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Xmodem,
                    source: source.display().to_string(),
                    destination: "remote:/tmp/cancelled.bin".to_string(),
                },
            )
            .await
            .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if state
                        .store
                        .lock()
                        .unwrap()
                        .transfer_by_id(&task.id)
                        .is_some_and(|task| task.status == TransferStatus::Running)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("XModem task did not start");

            let cancelling = cancel_transfer_inner(&state, &task.id).unwrap();
            assert_eq!(cancelling.status, TransferStatus::Cancelled);
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if state
                        .store
                        .lock()
                        .unwrap()
                        .transfer_by_id(&task.id)
                        .is_some_and(|task| {
                            task.status == TransferStatus::Cancelled
                                && task.message.as_deref() == Some("cancelled")
                        })
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("cancelled XModem worker did not stop promptly");
            let cancelled = state
                .store
                .lock()
                .unwrap()
                .transfer_by_id(&task.id)
                .unwrap();
            assert_eq!(cancelled.status, TransferStatus::Cancelled);
            assert_eq!(cancelled.message.as_deref(), Some("cancelled"));

            let _ = release_tx.send(());
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("XModem cancellation bytes were not received")
                .expect("XModem cancellation server failed");
            close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn silent_xmodem_fails_promptly_when_transport_reconnects() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let _ = disconnect_rx.await;
                drop(socket);
                let _ = release_rx.await;
            });

            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: true,
                ..Default::default()
            }));
            let root =
                std::env::temp_dir().join(format!("portmate-modem-disconnect-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let source = root.join("source.bin");
            fs::write(&source, b"disconnect this XModem transfer").unwrap();
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            open_tcp_session(&state, profile.clone()).await.unwrap();

            let task = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Xmodem,
                    source: source.display().to_string(),
                    destination: "remote:/tmp/disconnected.bin".to_string(),
                },
            )
            .await
            .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if state
                        .store
                        .lock()
                        .unwrap()
                        .transfer_by_id(&task.id)
                        .is_some_and(|task| task.status == TransferStatus::Running)
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("XModem disconnect task did not start");
            let _ = disconnect_tx.send(());

            let failed = tokio::time::timeout(TEST_RUNTIME_TRANSITION_TIMEOUT, async {
                loop {
                    let task = state
                        .store
                        .lock()
                        .unwrap()
                        .transfer_by_id(&task.id)
                        .unwrap();
                    if task.status == TransferStatus::Failed {
                        break task;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("XModem worker did not fail promptly after transport loss");
            assert!(failed
                .message
                .as_deref()
                .is_some_and(|message| message.contains("modem session disconnected")));
            assert!(!state
                .transfer_cancellations
                .lock()
                .unwrap()
                .contains_key(&task.id));

            close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
            let _ = release_tx.send(());
            server.await.unwrap();
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

            let tcp = TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                ..Default::default()
            };
            let mut client = connect_tcp_socket(&tcp, "TCP").await.unwrap();
            client.write_all(&[0x01, TELNET_IAC]).await.unwrap();

            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .expect("TCP loopback server timed out")
                .expect("TCP loopback server task failed");
        });
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
    fn ssh_exec_capture_buffer_accepts_exact_limit_and_rejects_whole_overflow_chunk() {
        let mut buffer = vec![1_u8, 2];
        append_bounded_ssh_exec_data(&mut buffer, &[3, 4], 4, "stdout").unwrap();
        assert_eq!(buffer, [1, 2, 3, 4]);

        let before_overflow = buffer.clone();
        let error = append_bounded_ssh_exec_data(&mut buffer, &[5], 4, "stdout").unwrap_err();
        assert!(error.contains("stdout"));
        assert!(error.contains("4"));
        assert_eq!(buffer, before_overflow);
    }

    #[test]
    fn sysmon_history_query_limit_defaults_and_rejects_out_of_range_values() {
        assert_eq!(
            validate_sysmon_history_query_limit(None).unwrap(),
            DEFAULT_SYSMON_HISTORY_QUERY_LIMIT
        );
        assert_eq!(validate_sysmon_history_query_limit(Some(1)).unwrap(), 1);
        assert_eq!(
            validate_sysmon_history_query_limit(Some(MAX_SYSMON_HISTORY_QUERY_LIMIT)).unwrap(),
            MAX_SYSMON_HISTORY_QUERY_LIMIT
        );
        assert!(validate_sysmon_history_query_limit(Some(0)).is_err());
        assert!(
            validate_sysmon_history_query_limit(Some(MAX_SYSMON_HISTORY_QUERY_LIMIT + 1)).is_err()
        );
    }

    #[test]
    fn sysmon_refresh_saturation_rejects_before_collection_side_effects() {
        tauri::async_runtime::block_on(async {
            let root = std::env::temp_dir()
                .join(format!("portmate-sysmon-refresh-limit-{}", Uuid::new_v4()));
            let profile = test_shell_profile();
            let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
            let _permits = Arc::clone(&state.sysmon_slots)
                .try_acquire_many_owned(MAX_CONCURRENT_SYSMON_REFRESHES as u32)
                .unwrap();

            let error = refresh_sysmon_inner(&state, &profile.id).await.unwrap_err();

            assert!(error.contains("refresh limit"), "{error}");
            let store = state.store.lock().unwrap();
            assert!(store.sysmon.is_empty());
            assert!(store.events.is_empty());
            let _ = fs::remove_dir_all(root);
        });
    }

    #[test]
    fn ssh_auxiliary_capacity_rejects_before_lookup_and_recovers_permits() {
        let temp = tempfile::tempdir().unwrap();
        let state = test_app_state(test_ssh_profile(), temp.path().join("store.sqlite3"));
        let permits = Arc::clone(&state.ssh_auxiliary_slots)
            .try_acquire_many_owned(MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS as u32)
            .unwrap();

        let saturated = ssh_auxiliary_lease(&state, "missing-session")
            .err()
            .expect("saturated auxiliary capacity unexpectedly allowed a lease");
        assert_eq!(
            saturated,
            format!(
                "SSH auxiliary operation limit reached ({MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS})"
            )
        );
        assert_eq!(state.ssh_auxiliary_slots.available_permits(), 0);

        drop(permits);
        assert_eq!(
            state.ssh_auxiliary_slots.available_permits(),
            MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS
        );
        let missing_runtime = ssh_auxiliary_lease(&state, "missing-session")
            .err()
            .expect("missing SSH runtime unexpectedly produced a lease");
        assert_eq!(missing_runtime, "需要先连接 SSH/Tmux 会话才能执行远端操作");
        assert_eq!(
            state.ssh_auxiliary_slots.available_permits(),
            MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS
        );
    }

    #[test]
    fn ssh_auxiliary_saturation_blocks_remote_entry_points_without_side_effects() {
        tauri::async_runtime::block_on(async {
            let temp = tempfile::tempdir().unwrap();
            let profile = test_ssh_profile();
            let state = test_app_state(profile.clone(), temp.path().join("store.sqlite3"));
            let _permits = Arc::clone(&state.ssh_auxiliary_slots)
                .try_acquire_many_owned(MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS as u32)
                .unwrap();

            let file_error = list_files_inner(
                &state,
                ListFilesRequest {
                    session_id: Some(profile.id.clone()),
                    path: "/tmp".to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap_err();
            let tmux_list_error = list_tmux_state_inner(&state, &profile.id)
                .await
                .unwrap_err();
            let tmux_mutation_error = mutate_tmux_inner(
                &state,
                TmuxMutationRequest {
                    session_id: profile.id.clone(),
                    action: TmuxMutationAction::KillPane,
                    target: "%1".to_string(),
                    name: None,
                    destination: None,
                    layout: None,
                    amount: None,
                },
            )
            .await
            .unwrap_err();
            let tmux_control_error = start_tmux_control_inner(&state, &profile.id, "lab")
                .await
                .unwrap_err();
            let sysmon_error = refresh_sysmon_inner(&state, &profile.id).await.unwrap_err();
            let tunnel_error = probe_remote_tunnel_health(
                &state,
                &TunnelRuntime {
                    session_id: profile.id.clone(),
                    ssh_runtime_id: "missing-runtime".to_string(),
                    spec: TunnelSpec {
                        id: "saturated-health-check".to_string(),
                        label: "saturated health check".to_string(),
                        mode: TunnelMode::Remote,
                        bind_host: "127.0.0.1".to_string(),
                        bind_port: 10_022,
                        target_host: "127.0.0.1".to_string(),
                        target_port: 22,
                        enabled: true,
                    },
                    metrics: Arc::new(TunnelMetrics::default()),
                    closed: Arc::new(AtomicBool::new(false)),
                },
            )
            .await
            .unwrap_err();

            for error in [
                file_error,
                tmux_list_error,
                tmux_mutation_error,
                tmux_control_error,
                sysmon_error,
                tunnel_error,
            ] {
                assert!(error.contains("auxiliary operation limit"), "{error}");
            }
            assert!(state.ssh.lock().unwrap().is_empty());
            assert!(state.tmux_controls.lock().unwrap().is_empty());
            assert_eq!(
                state.tmux_control_slots.available_permits(),
                MAX_ACTIVE_TMUX_CONTROLS
            );
            assert_eq!(
                state.sysmon_slots.available_permits(),
                MAX_CONCURRENT_SYSMON_REFRESHES
            );
            let store = state.store.lock().unwrap();
            assert!(store.sysmon.is_empty());
            assert!(store.events.is_empty());
        });
    }

    #[cfg(unix)]
    #[test]
    fn tmux_mutation_reuses_one_ssh_auxiliary_lease_for_state_refresh() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping tmux auxiliary lease test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let profile = test_ssh_profile();
            let mut state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
            state.ssh_auxiliary_slots = Arc::new(tokio::sync::Semaphore::new(1));
            let username = "portmate-tmux-lease-user";
            let secret = "PortMate tmux lease secret";
            let (port, counters, server_task) =
                spawn_mixed_auth_test_server(&host_key, username, secret).await;
            let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
            let handler = PortMateSshHandler {
                profile_id: profile.id.clone(),
                host: "127.0.0.1".to_string(),
                port,
                alias: None,
                policy: portmate_core::HostKeyPolicy {
                    mode: HostKeyMode::TrustOnFirstUse,
                    alias: None,
                    trust_scope: HostKeyScope::Profile,
                    allow_rotation: false,
                    check_ip: false,
                },
                host_keys: state.store.lock().unwrap().host_keys.clone(),
                one_time_host_key_ids: Vec::new(),
                observed_key: Arc::new(Mutex::new(None)),
                host_key_error: Arc::new(Mutex::new(None)),
                remote_forwards: Arc::clone(&remote_forwards),
            };
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                handler,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let terminal =
                SshBackendChannel::from_russh(handle.channel_open_session().await.unwrap());
            let (_reader, writer) = terminal.split();
            let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_russh(
                handle,
            )));
            let (tap, _) = broadcast::channel(1);
            let (_reader_finished_sender, reader_finished) = tokio::sync::oneshot::channel();
            state.ssh.lock().unwrap().insert(
                profile.id.clone(),
                SshRuntime {
                    runtime_id: "tmux-lease-runtime".to_string(),
                    handle: Arc::clone(&handle),
                    sftp: Arc::new(tokio::sync::Mutex::new(None)),
                    jump_handles: Vec::new(),
                    writer: Arc::new(tokio::sync::Mutex::new(writer)),
                    tap,
                    remote_forwards,
                    closed: Arc::new(AtomicBool::new(false)),
                    reader_finished,
                },
            );

            let state_after_mutation = mutate_tmux_inner(
                &state,
                TmuxMutationRequest {
                    session_id: profile.id.clone(),
                    action: TmuxMutationAction::KillPane,
                    target: "%1".to_string(),
                    name: None,
                    destination: None,
                    layout: None,
                    amount: None,
                },
            )
            .await
            .unwrap();

            assert!(state_after_mutation.sessions.is_empty());
            assert!(state_after_mutation.windows.is_empty());
            assert!(state_after_mutation.panes.is_empty());
            assert_eq!(state.ssh_auxiliary_slots.available_permits(), 1);
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 5);

            state.ssh.lock().unwrap().remove(&profile.id);
            let handle = handle.lock().await;
            let _ = handle.disconnect("PortMate tmux lease test complete").await;
            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[test]
    fn session_profile_normalization_bounds_metadata_and_terminal_settings() {
        let mut profile = test_shell_profile();
        profile.id = format!(" \0edge\n{} ", "界".repeat(300));
        profile.name = format!(" \0Router\n{} ", "界".repeat(200));
        profile.group = format!(" Lab\u{0085}{} ", "g".repeat(300));
        profile.tags = std::iter::once(" edge ".to_string())
            .chain(std::iter::once("edge".to_string()))
            .chain((0..40).map(|index| format!("tag-{index}-{}", "x".repeat(80))))
            .collect();
        profile.terminal.term = "xterm\nmalformed".to_string();
        profile.terminal.rows = 0;
        profile.terminal.cols = u16::MAX;
        profile.terminal.scrollback = u32::MAX;
        profile.terminal.font_family = "bad\0font".to_string();
        profile.terminal.font_size = 0;
        profile.terminal.theme = " graphite ".to_string();
        profile.terminal.background_opacity = 0;
        profile.triggers = vec![
            portmate_core::TriggerSpec {
                id: "valid-trigger".to_string(),
                label: "Valid".to_string(),
                matcher: portmate_core::TriggerMatcher::Contains {
                    text: "match".to_string(),
                    case_sensitive: true,
                },
                actions: vec![TriggerAction::TimelineMark {
                    label: "mark".to_string(),
                }],
                enabled: true,
            },
            portmate_core::TriggerSpec {
                id: "invalid\ntrigger".to_string(),
                label: "Invalid".to_string(),
                matcher: portmate_core::TriggerMatcher::Contains {
                    text: "match".to_string(),
                    case_sensitive: true,
                },
                actions: Vec::new(),
                enabled: true,
            },
        ];
        let normalized = normalize_session_profile(profile.clone());
        assert_eq!(
            normalized.id.chars().count(),
            MAX_SESSION_PROFILE_ID_CHARACTERS
        );
        assert!(normalized.id.starts_with("edge"));
        assert!(!normalized.id.chars().any(char::is_control));
        assert_eq!(
            normalized.name.chars().count(),
            MAX_SESSION_PROFILE_NAME_CHARACTERS
        );
        assert!(normalized.name.starts_with("Router"));
        assert!(!normalized.name.chars().any(char::is_control));
        assert_eq!(
            normalized.group.chars().count(),
            MAX_SESSION_PROFILE_GROUP_CHARACTERS
        );
        assert_eq!(normalized.tags.len(), MAX_SESSION_PROFILE_TAGS);
        assert_eq!(normalized.tags[0], "edge");
        assert!(normalized
            .tags
            .iter()
            .all(|tag| tag.chars().count() <= MAX_SESSION_PROFILE_TAG_CHARACTERS));
        assert_eq!(
            normalized.tags.iter().collect::<HashSet<_>>().len(),
            normalized.tags.len()
        );
        assert_eq!(normalized.terminal.term, DEFAULT_TERMINAL_NAME);
        assert_eq!(normalized.terminal.rows, MIN_TERMINAL_ROWS);
        assert_eq!(normalized.terminal.cols, MAX_TERMINAL_COLS);
        assert_eq!(normalized.terminal.scrollback, MAX_TERMINAL_SCROLLBACK);
        assert_eq!(
            normalized.terminal.font_family,
            DEFAULT_TERMINAL_FONT_FAMILY
        );
        assert_eq!(normalized.terminal.font_size, MIN_TERMINAL_FONT_SIZE);
        assert_eq!(normalized.terminal.theme, "graphite");
        assert_eq!(
            normalized.terminal.background_opacity,
            MIN_TERMINAL_BACKGROUND_OPACITY
        );
        assert_eq!(normalized.triggers.len(), 1);
        assert_eq!(normalized.triggers[0].id, "valid-trigger");

        let mut fallback = profile.clone();
        fallback.name = "\0\n".to_string();
        assert_eq!(
            normalize_session_profile(fallback).name,
            normalized_profile_metadata_text(
                &normalized_session_profile_id(&profile.id),
                MAX_SESSION_PROFILE_NAME_CHARACTERS,
            )
        );

        profile.terminal.theme = "future-or-corrupt-theme".to_string();
        assert_eq!(
            normalize_session_profile(profile).terminal.theme,
            DEFAULT_TERMINAL_THEME
        );
    }

    #[test]
    fn ssh_auth_success_hint_respects_the_current_policy() {
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("test profile must be SSH");
        };
        ssh.identity_policy.auth_order = vec![AuthMethod::Password, AuthMethod::PublicKey];
        ssh.identity_policy.last_successful = Some(AuthMethod::PublicKey);
        assert_eq!(
            ordered_auth_methods(ssh),
            vec![AuthMethod::PublicKey, AuthMethod::Password]
        );

        ssh.identity_policy.last_successful = Some(AuthMethod::KeyboardInteractive);
        assert_eq!(
            ordered_auth_methods(ssh),
            vec![AuthMethod::Password, AuthMethod::PublicKey]
        );

        ssh.identity_policy.record_success = false;
        ssh.identity_policy.last_successful = Some(AuthMethod::PublicKey);
        assert_eq!(
            ordered_auth_methods(ssh),
            vec![AuthMethod::Password, AuthMethod::PublicKey]
        );
        let normalized = normalize_session_profile(profile);
        let ConnectionConfig::Ssh(ssh) = normalized.connection else {
            panic!("normalized test profile must remain SSH");
        };
        assert_eq!(ssh.identity_policy.last_successful, None);
    }

    #[test]
    fn active_session_rejects_cross_transport_profile_changes() {
        let current = test_shell_profile();
        let mut next = current.clone();
        next.kind = SessionKind::Serial;
        next.connection = ConnectionConfig::Serial(portmate_core::SerialConnection {
            port: "/dev/ttyUSB0".to_string(),
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".to_string(),
            flow_control: "none".to_string(),
            dtr: false,
            rts: false,
            reconnect: true,
            reconnect_delay_ms: portmate_core::DEFAULT_SERIAL_RECONNECT_DELAY_MS,
            receive_idle_timeout_enabled: false,
            receive_idle_timeout_seconds:
                portmate_core::DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS,
        });

        for status in [
            SessionStatus::Connecting,
            SessionStatus::Connected,
            SessionStatus::Reconnecting,
        ] {
            let error =
                validate_profile_transport_change(Some(&current), &next, Some(status)).unwrap_err();
            assert!(error.contains("切换到 Serial 前请先关闭会话"));
        }
        for status in [
            SessionStatus::Disconnected,
            SessionStatus::Blocked,
            SessionStatus::Error,
        ] {
            validate_profile_transport_change(Some(&current), &next, Some(status)).unwrap();
        }

        let mut same_transport = current.clone();
        if let ConnectionConfig::Shell(shell) = &mut same_transport.connection {
            shell.program = "/bin/bash".to_string();
        }
        validate_profile_transport_change(
            Some(&current),
            &same_transport,
            Some(SessionStatus::Connected),
        )
        .unwrap();
        validate_profile_transport_change(None, &next, None).unwrap();
    }

    #[test]
    fn tcp_connection_details_validate_endpoint_and_reconnect_flag() {
        let mut profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: " 127.0.0.1 ".to_string(),
            port: 2323,
            reconnect: true,
            ..Default::default()
        }));
        let (tcp, label) = tcp_connection_details(&profile).unwrap();
        assert_eq!(
            (tcp.host, tcp.port, label),
            ("127.0.0.1".to_string(), 2323, "TCP")
        );
        assert!(tcp_reconnect_enabled(&profile));

        profile.connection = ConnectionConfig::Telnet(portmate_core::TcpConnection {
            host: "console.lab".to_string(),
            port: 23,
            reconnect: false,
            ..Default::default()
        });
        let (tcp, label) = tcp_connection_details(&profile).unwrap();
        assert_eq!(
            (tcp.host, tcp.port, label),
            ("console.lab".to_string(), 23, "Telnet")
        );
        assert!(!tcp_reconnect_enabled(&profile));

        profile.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: " ".to_string(),
            port: 23,
            reconnect: true,
            ..Default::default()
        });
        assert!(tcp_connection_details(&profile)
            .unwrap_err()
            .contains("主机不能为空"));

        profile.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: 0,
            reconnect: true,
            ..Default::default()
        });
        assert!(tcp_connection_details(&profile)
            .unwrap_err()
            .contains("端口不能为空"));
    }

    #[test]
    fn tcp_socket_enables_bounded_kernel_keepalive() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let _ = release_rx.await;
                drop(socket);
            });

            let mut tcp = TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                keepalive_idle_seconds: 45,
                keepalive_interval_seconds: 7,
                keepalive_retries: 5,
                ..Default::default()
            };
            let stream = connect_tcp_socket(&tcp, "TCP").await.unwrap();
            let socket = SockRef::from(&stream);
            assert!(socket.keepalive().unwrap());
            #[cfg(target_os = "linux")]
            {
                assert_eq!(
                    socket.tcp_keepalive_time().unwrap(),
                    Duration::from_secs(tcp.keepalive_idle_seconds)
                );
                assert_eq!(
                    socket.tcp_keepalive_interval().unwrap(),
                    Duration::from_secs(tcp.keepalive_interval_seconds)
                );
                assert_eq!(
                    socket.tcp_keepalive_retries().unwrap(),
                    tcp.keepalive_retries
                );
            }
            tcp.keepalive_enabled = false;
            configure_tcp_socket(&stream, "TCP", &tcp).unwrap();
            assert!(!socket.keepalive().unwrap());

            drop(stream);
            let _ = release_tx.send(());
            server.await.unwrap();
        });
    }

    #[test]
    fn ssh_socket_applies_explicit_tcp_keepalive_only() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let _ = release_rx.await;
                drop(socket);
            });

            let stream = TcpStream::connect(address).await.unwrap();
            configure_ssh_tcp_keepalive(&stream, "SSH", None).unwrap();
            configure_ssh_tcp_keepalive(&stream, "SSH", Some(true)).unwrap();
            let socket = SockRef::from(&stream);
            assert!(socket.keepalive().unwrap());
            configure_ssh_tcp_keepalive(&stream, "SSH", Some(false)).unwrap();
            assert!(!socket.keepalive().unwrap());

            drop(stream);
            let _ = release_tx.send(());
            server.await.unwrap();
        });
    }

    #[test]
    fn authenticated_proxy_handshakes_accept_valid_credentials_and_reject_invalid_ones() {
        tauri::async_runtime::block_on(async {
            let expected_http = format!(
                "Proxy-Authorization: Basic {}",
                BASE64_STANDARD.encode("proxy-user:proxy-password")
            );
            let (http_port, http_task) = spawn_test_http_auth_endpoint(expected_http.clone()).await;
            let mut stream = TcpStream::connect(("127.0.0.1", http_port)).await.unwrap();
            let credentials = ProxyCredentials {
                username: "proxy-user".to_string(),
                password: Zeroizing::new("proxy-password".to_string()),
            };
            perform_http_connect(&mut stream, "target.example:443", Some(&credentials), "TCP")
                .await
                .unwrap();
            http_task.await.unwrap();

            let (http_port, http_task) = spawn_test_http_auth_endpoint(expected_http).await;
            let mut stream = TcpStream::connect(("127.0.0.1", http_port)).await.unwrap();
            let wrong_credentials = ProxyCredentials {
                username: "proxy-user".to_string(),
                password: Zeroizing::new("wrong-password".to_string()),
            };
            let error = perform_http_connect(
                &mut stream,
                "target.example:443",
                Some(&wrong_credentials),
                "TCP",
            )
            .await
            .unwrap_err();
            assert!(
                error.contains("407 Proxy Authentication Required"),
                "{error}"
            );
            assert!(!error.contains("wrong-password"));
            http_task.await.unwrap();

            let (socks_port, socks_task) = spawn_test_socks5_auth_endpoint(
                "proxy-user".to_string(),
                "proxy-password".to_string(),
            )
            .await;
            let mut stream = TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap();
            perform_socks5_connect(
                &mut stream,
                "target.example",
                443,
                Some(&credentials),
                "TCP",
            )
            .await
            .unwrap();
            socks_task.await.unwrap();

            let (socks_port, socks_task) = spawn_test_socks5_auth_endpoint(
                "proxy-user".to_string(),
                "proxy-password".to_string(),
            )
            .await;
            let mut stream = TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap();
            let error = perform_socks5_connect(
                &mut stream,
                "target.example",
                443,
                Some(&wrong_credentials),
                "TCP",
            )
            .await
            .unwrap_err();
            assert!(error.contains("用户名/密码认证失败"), "{error}");
            assert!(!error.contains("wrong-password"));
            socks_task.await.unwrap();
        });
    }

    #[test]
    fn tcp_proxy_transports_forward_and_report_rejections() {
        tauri::async_runtime::block_on(async {
            let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let target_port = target.local_addr().unwrap().port();
            let target_task = tokio::spawn(async move {
                for payload in [
                    b"http-ok".as_slice(),
                    b"socks-ok".as_slice(),
                    b"direct-ok".as_slice(),
                ] {
                    let (mut socket, _) = target.accept().await.unwrap();
                    socket.write_all(payload).await.unwrap();
                }
            });

            let (http_port, http_connections, http_task) = spawn_test_http_connect_proxy(200).await;
            let http = TcpConnection {
                host: "127.0.0.1".to_string(),
                port: target_port,
                proxy: ProxyConfig {
                    enabled: true,
                    kind: ProxyKind::HttpConnect,
                    host: "127.0.0.1".to_string(),
                    port: http_port,
                    ..ProxyConfig::default()
                },
                ..Default::default()
            };
            let mut stream = connect_tcp_socket(&http, "TCP").await.unwrap();
            let mut payload = [0_u8; 7];
            stream.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"http-ok");
            drop(stream);

            let (socks_port, socks_connections, socks_task) = spawn_test_socks5_proxy(0).await;
            let socks = TcpConnection {
                proxy: ProxyConfig {
                    enabled: true,
                    kind: ProxyKind::Socks5,
                    host: "127.0.0.1".to_string(),
                    port: socks_port,
                    ..ProxyConfig::default()
                },
                ..http.clone()
            };
            let mut stream = connect_tcp_socket(&socks, "Telnet").await.unwrap();
            let mut payload = [0_u8; 8];
            stream.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"socks-ok");
            drop(stream);

            let disabled = TcpConnection {
                proxy: ProxyConfig {
                    enabled: false,
                    host: "invalid\r\nproxy".to_string(),
                    port: 0,
                    ..socks.proxy.clone()
                },
                ..socks.clone()
            };
            let mut stream = connect_tcp_socket(&disabled, "TCP").await.unwrap();
            let mut payload = [0_u8; 9];
            stream.read_exact(&mut payload).await.unwrap();
            assert_eq!(&payload, b"direct-ok");
            drop(stream);
            target_task.await.unwrap();
            assert_eq!(http_connections.load(Ordering::SeqCst), 1);
            assert_eq!(socks_connections.load(Ordering::SeqCst), 1);

            let (rejected_http_port, _, rejected_http_task) =
                spawn_test_http_connect_proxy(407).await;
            let rejected_http = TcpConnection {
                proxy: ProxyConfig {
                    port: rejected_http_port,
                    ..http.proxy.clone()
                },
                ..http.clone()
            };
            let error = connect_tcp_socket(&rejected_http, "TCP").await.unwrap_err();
            assert!(error.contains("407 Rejected"), "{error}");

            let (rejected_socks_port, _, rejected_socks_task) = spawn_test_socks5_proxy(0x05).await;
            let rejected_socks = TcpConnection {
                proxy: ProxyConfig {
                    enabled: true,
                    kind: ProxyKind::Socks5,
                    host: "127.0.0.1".to_string(),
                    port: rejected_socks_port,
                    ..ProxyConfig::default()
                },
                ..http
            };
            let error = connect_tcp_socket(&rejected_socks, "TCP")
                .await
                .unwrap_err();
            assert!(error.contains("connection refused (0x05)"), "{error}");

            let invalid_proxy = TcpConnection {
                proxy: ProxyConfig {
                    enabled: true,
                    host: "   ".to_string(),
                    port: 0,
                    ..ProxyConfig::default()
                },
                ..rejected_socks.clone()
            };
            let error = connect_tcp_socket(&invalid_proxy, "TCP").await.unwrap_err();
            assert!(error.contains("代理主机不能为空"), "{error}");

            let injected_target = TcpConnection {
                host: "target.example\r\nX-Injected: true".to_string(),
                ..rejected_socks
            };
            let error = connect_tcp_socket(&injected_target, "TCP")
                .await
                .unwrap_err();
            assert!(error.contains("代理目标主机不能包含换行符"), "{error}");

            for task in [
                http_task,
                socks_task,
                rejected_http_task,
                rejected_socks_task,
            ] {
                task.abort();
                let _ = task.await;
            }
        });
    }

    #[test]
    fn tcp_reconnect_profile_reloads_latest_endpoint_and_disable_state() {
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "old.example".to_string(),
            port: 2323,
            reconnect: true,
            ..Default::default()
        }));
        let state = test_app_state(profile.clone(), PathBuf::from("tcp-reconnect-test.sqlite3"));
        assert!(tcp_reconnect_attempt_matches_profile(&profile, &profile));
        assert_eq!(
            tcp_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
            TcpReconnectProfileState::Current
        );

        let mut renamed = profile.clone();
        renamed.name = "Renamed TCP".to_string();
        assert!(tcp_reconnect_attempt_matches_profile(&profile, &renamed));

        let mut terminal_updated = profile.clone();
        terminal_updated.terminal.term = "vt100".to_string();
        assert!(!tcp_reconnect_attempt_matches_profile(
            &profile,
            &terminal_updated
        ));

        let mut updated = profile.clone();
        updated.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "new.example".to_string(),
            port: 4242,
            reconnect: true,
            proxy: ProxyConfig {
                enabled: true,
                kind: ProxyKind::HttpConnect,
                host: "proxy.example".to_string(),
                port: 3128,
                ..ProxyConfig::default()
            },
            reconnect_delay_ms: 2_500,
            keepalive_enabled: false,
            keepalive_idle_seconds: 90,
            keepalive_interval_seconds: 15,
            keepalive_retries: 6,
            telnet_binary: false,
            telnet_naws: false,
            tls_enabled: false,
            tls_server_name: None,
            tls_accept_invalid_cert: false,
        });
        state.store.lock().unwrap().upsert_profile(updated);
        assert_eq!(
            tcp_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
            TcpReconnectProfileState::Changed
        );
        let latest = latest_tcp_reconnect_profile(&state, &profile.id)
            .unwrap()
            .unwrap();
        let (tcp, label) = tcp_connection_details(&latest).unwrap();
        assert_eq!(
            (tcp.host, tcp.port, label),
            ("new.example".to_string(), 4242, "TCP")
        );
        assert_eq!(tcp.reconnect_delay_ms, 2_500);
        assert!(tcp.proxy.enabled);
        assert_eq!(tcp.proxy.kind, ProxyKind::HttpConnect);
        assert_eq!(tcp.proxy.host, "proxy.example");
        assert_eq!(tcp.proxy.port, 3128);
        assert!(!tcp.keepalive_enabled);
        assert_eq!(tcp.keepalive_idle_seconds, 90);
        assert_eq!(tcp.keepalive_interval_seconds, 15);
        assert_eq!(tcp.keepalive_retries, 6);
        assert!(!tcp.telnet_binary);
        assert!(!tcp.telnet_naws);

        let mut disabled = latest;
        if let ConnectionConfig::Tcp(tcp) = &mut disabled.connection {
            tcp.reconnect = false;
        }
        state.store.lock().unwrap().upsert_profile(disabled);
        assert_eq!(
            tcp_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
            TcpReconnectProfileState::Disabled
        );
        assert!(latest_tcp_reconnect_profile(&state, &profile.id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn mcp_audit_export_is_atomic_exact_and_checksummed() {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-export-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let audit = vec![
            AuditRecord {
                id: "audit-a".to_string(),
                ts: Utc::now(),
                actor: "mcp:alpha".to_string(),
                action: "send_text".to_string(),
                session_id: Some("edge".to_string()),
                decision: "succeeded".to_string(),
                details: BTreeMap::from([("scope".to_string(), "write-input".to_string())]),
            },
            AuditRecord {
                id: "audit-b".to_string(),
                ts: Utc::now(),
                actor: "mcp:beta".to_string(),
                action: "create_tunnel".to_string(),
                session_id: Some("lab".to_string()),
                decision: "denied".to_string(),
                details: BTreeMap::from([("scope".to_string(), "tunnel".to_string())]),
            },
            AuditRecord {
                id: "audit-c".to_string(),
                ts: Utc::now(),
                actor: "mcp:gamma".to_string(),
                action: "list_sessions".to_string(),
                session_id: None,
                decision: "authorized".to_string(),
                details: BTreeMap::from([("scope".to_string(), "read-sessions".to_string())]),
            },
        ];
        let result = export_mcp_audit_inner(
            &root.join("portmate-store.sqlite3"),
            &audit,
            ExportMcpAuditRequest {
                record_ids: vec!["audit-c".to_string(), "audit-a".to_string()],
            },
        )
        .unwrap();
        assert_eq!(result.records, 2);
        assert_eq!(result.sha256, sha256_file(Path::new(&result.path)).unwrap());
        assert!(fs::read_to_string(&result.checksum_path)
            .unwrap()
            .starts_with(&result.sha256));

        let lines = fs::read_to_string(&result.path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["format"], "portmate-mcp-audit");
        assert_eq!(lines[0]["recordCount"], 2);
        assert_eq!(lines[0]["containsSecretBodies"], false);
        assert_eq!(lines[1]["record"]["id"], "audit-c");
        assert_eq!(lines[2]["record"]["id"], "audit-a");
        assert!(!fs::read_to_string(&result.path)
            .unwrap()
            .contains("audit-b"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&result.checksum_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        let duplicate = export_mcp_audit_inner(
            &root.join("portmate-store.sqlite3"),
            &audit,
            ExportMcpAuditRequest {
                record_ids: vec!["audit-a".to_string(), "audit-a".to_string()],
            },
        )
        .unwrap_err();
        assert!(duplicate.contains("duplicate"));
        let stale = export_mcp_audit_inner(
            &root.join("portmate-store.sqlite3"),
            &audit,
            ExportMcpAuditRequest {
                record_ids: vec!["missing".to_string()],
            },
        )
        .unwrap_err();
        assert!(stale.contains("refresh"));

        assert!(fs::read_dir(root.join("exports"))
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn terminal_text_export_is_atomic_bounded_and_checksummed() {
        let root =
            std::env::temp_dir().join(format!("portmate-terminal-export-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let request = ExportTerminalTextRequest {
            session_id: "../shell export".to_string(),
            view_id: "view-mirror".to_string(),
            source: TerminalTextExportSource::Buffer,
            text: "prompt$ echo 终端\n终端\n".to_string(),
        };
        let result =
            export_terminal_text_inner(&root.join("portmate-store.sqlite3"), request.clone())
                .unwrap();
        assert_eq!(result.session_id, request.session_id);
        assert_eq!(result.view_id, request.view_id);
        assert_eq!(result.source, TerminalTextExportSource::Buffer);
        assert_eq!(fs::read_to_string(&result.path).unwrap(), request.text);
        assert_eq!(result.size as usize, request.text.len());
        assert_eq!(result.sha256, sha256_file(Path::new(&result.path)).unwrap());
        assert!(fs::read_to_string(&result.checksum_path)
            .unwrap()
            .starts_with(&result.sha256));
        let export_dir = root.join("exports").canonicalize().unwrap();
        assert_eq!(Path::new(&result.path).parent().unwrap(), export_dir);
        assert!(Path::new(&result.path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("shell_export"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&result.checksum_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        assert!(fs::read_dir(&export_dir).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")));
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn terminal_text_export_rejects_a_symlinked_exports_directory() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let exports = root.path().join("exports");
        std::os::unix::fs::symlink(outside.path(), &exports).unwrap();

        let error = export_terminal_text_inner(
            &root.path().join("portmate-store.sqlite3"),
            ExportTerminalTextRequest {
                session_id: "shell-a".to_string(),
                view_id: "view-a".to_string(),
                source: TerminalTextExportSource::Buffer,
                text: "sensitive terminal output".to_string(),
            },
        )
        .unwrap_err();

        assert!(error.contains("symbolic link"), "{error}");
        assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
    }

    #[test]
    fn terminal_text_export_rejects_empty_invalid_and_oversized_requests() {
        let mut request = ExportTerminalTextRequest {
            session_id: "shell-a".to_string(),
            view_id: "view-a".to_string(),
            source: TerminalTextExportSource::Selection,
            text: "selected".to_string(),
        };
        assert!(validate_terminal_text_export_request(&request, 8).is_ok());
        request.text.push('!');
        assert!(validate_terminal_text_export_request(&request, 8)
            .unwrap_err()
            .contains("8 byte limit"));
        request.text.clear();
        assert!(validate_terminal_text_export_request(&request, 8)
            .unwrap_err()
            .contains("empty"));
        request.text = "text".to_string();
        request.view_id = "bad\nview".to_string();
        assert!(validate_terminal_text_export_request(&request, 8)
            .unwrap_err()
            .contains("view id"));
        request.view_id = "view-a".to_string();
        request.session_id.clear();
        assert!(validate_terminal_text_export_request(&request, 8)
            .unwrap_err()
            .contains("session id"));
        request.session_id = "bad\nsession".to_string();
        assert!(validate_terminal_text_export_request(&request, 8)
            .unwrap_err()
            .contains("session id"));
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
    fn bounded_connection_step_preserves_results_and_stops_pending_operations() {
        tauri::async_runtime::block_on(async {
            let success = bounded_connection_step(
                async { Ok::<_, &'static str>("connected") },
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert_eq!(success, "connected");

            let failed = bounded_connection_step(
                async { Err::<(), _>("connection refused") },
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert_eq!(
                failed,
                BoundedConnectionStepError::Failed("connection refused".to_string())
            );

            let timed_out = bounded_connection_step(
                std::future::pending::<Result<(), &'static str>>(),
                Duration::from_millis(20),
            )
            .await
            .unwrap_err();
            assert_eq!(timed_out, BoundedConnectionStepError::TimedOut);
        });
    }

    #[cfg(unix)]
    #[test]
    fn direct_tcpip_open_timeout_disconnects_a_stalled_russh_session() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping direct-tcpip timeout test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-direct-tcpip-user";
            let secret = "PortMate direct-tcpip secret";
            let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
                &host_key,
                username,
                secret,
                None,
                None,
                Some(Duration::from_millis(200)),
            )
            .await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());

            let error = open_direct_tcpip_with_timeout(
                &handle,
                "127.0.0.1".to_string(),
                9,
                "127.0.0.1".to_string(),
                0,
                Duration::from_millis(30),
                "PortMate direct-tcpip timeout test",
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                DirectTcpipOpenError::TimedOut {
                    timeout_ms: 30,
                    cleanup_warning: None,
                }
            );
            assert_eq!(counters.direct_tcpip_attempts.load(Ordering::SeqCst), 1);
            tokio::time::timeout(Duration::from_secs(1), async {
                while counters.direct_tcpip_completions.load(Ordering::SeqCst) != 1 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("delayed direct-tcpip callback did not finish");

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn ssh_authentication_timeout_disconnects_a_stalled_russh_session() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping SSH authentication timeout test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-auth-timeout-user";
            let secret = "PortMate authentication timeout secret";
            let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
                &host_key,
                username,
                secret,
                Some(Duration::from_millis(200)),
                None,
                None,
            )
            .await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            let mut ssh = match test_ssh_profile().connection {
                ConnectionConfig::Ssh(ssh) => ssh,
                _ => unreachable!("test SSH profile changed transport"),
            };
            ssh.identity_policy.auth_order = vec![AuthMethod::Password];
            ssh.identity_policy.last_successful = None;
            ssh.identity_refs.clear();
            ssh.agent_policy.enabled = false;

            let error = authenticate_ssh_with_timeout(
                &mut handle,
                SshAuthenticationRequest {
                    ssh,
                    username: username.to_string(),
                    password: Some(secret.to_string()),
                    passphrase: None,
                    agent_socket_path: None,
                    timeout: Duration::from_millis(30),
                    disconnect_description: "PortMate authentication timeout test",
                },
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                SshAuthenticationError::TimedOut {
                    timeout_ms: 30,
                    cleanup_warning: None,
                }
            );
            assert_eq!(counters.password_attempts.load(Ordering::SeqCst), 1);
            tokio::time::timeout(Duration::from_secs(1), async {
                while counters.password_completions.load(Ordering::SeqCst) != 1 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("delayed password authentication callback did not finish");
            assert_eq!(counters.password_successes.load(Ordering::SeqCst), 1);

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn ssh_terminal_setup_timeout_disconnects_a_stalled_russh_session() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping SSH terminal setup timeout test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-terminal-setup-user";
            let secret = "PortMate terminal setup secret";
            let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
                &host_key,
                username,
                secret,
                None,
                Some(Duration::from_millis(200)),
                None,
            )
            .await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let profile = test_ssh_profile();
            let ssh = match &profile.connection {
                ConnectionConfig::Ssh(ssh) => ssh,
                _ => unreachable!("test SSH profile changed transport"),
            };

            let error = open_ssh_terminal_channel_with_timeout(
                &handle,
                &profile,
                ssh,
                Duration::from_millis(30),
                "PortMate terminal setup timeout test",
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                SshTerminalSetupError::TimedOut {
                    timeout_ms: 30,
                    cleanup_warning: None,
                }
            );
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 1);
            tokio::time::timeout(Duration::from_secs(1), async {
                while counters.session_channel_completions.load(Ordering::SeqCst) != 1 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("delayed SSH session-channel callback did not finish");

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn ssh_auxiliary_setups_timeout_and_disconnect_stalled_russh_sessions() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping SSH auxiliary setup timeout test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-auxiliary-setup-user";
            let secret = "PortMate auxiliary setup secret";
            let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
                &host_key,
                username,
                secret,
                None,
                Some(Duration::from_millis(200)),
                None,
            )
            .await;

            let mut exec_handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(exec_handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let exec_handle = Arc::new(tokio::sync::Mutex::new(exec_handle));
            let error = open_shared_russh_exec_channel(
                &exec_handle,
                "true",
                Duration::from_millis(30),
                "SSH auxiliary exec test",
            )
            .await
            .unwrap_err();
            assert_eq!(error, "SSH auxiliary exec test setup 超时（30 ms）");
            tokio::time::timeout(Duration::from_secs(1), async {
                while counters.session_channel_completions.load(Ordering::SeqCst) != 1 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("delayed auxiliary exec channel callback did not finish");
            drop(exec_handle);

            let mut sftp_handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(sftp_handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let sftp_handle = Arc::new(tokio::sync::Mutex::new(sftp_handle));
            let error = open_sftp_session_with_timeout(sftp_handle, Duration::from_millis(30))
                .await
                .err()
                .expect("stalled SFTP setup unexpectedly succeeded");
            assert_eq!(error, "SFTP setup 超时（30 ms）");
            tokio::time::timeout(Duration::from_secs(1), async {
                while counters.session_channel_completions.load(Ordering::SeqCst) != 2 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("delayed SFTP channel callback did not finish");
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);

            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn ssh_exec_capture_handles_eof_status_order_and_closes_every_exit_path() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping SSH exec cleanup test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-exec-cleanup-user";
            let secret = "PortMate exec cleanup secret";
            let (port, counters, server_task) =
                spawn_mixed_auth_test_server(&host_key, username, secret).await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_russh(
                handle,
            )));

            let output = exec_ssh_command_capture(
                Arc::clone(&handle),
                "__PORTMATE_TEST_EXEC_SUCCESS__",
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert_eq!(output, "captured");

            let nonzero_error = exec_ssh_command_capture(
                Arc::clone(&handle),
                "__PORTMATE_TEST_EXEC_NONZERO__",
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert_eq!(nonzero_error, "SSH exec 返回非零状态 7: remote failure");

            let late_status_error = exec_ssh_command_capture(
                Arc::clone(&handle),
                "__PORTMATE_TEST_EXEC_EOF_BEFORE_NONZERO__",
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert_eq!(
                late_status_error,
                "SSH exec 返回非零状态 9: late status failure"
            );

            let timeout_error = exec_ssh_command_capture(
                Arc::clone(&handle),
                "__PORTMATE_TEST_EXEC_TIMEOUT__",
                Duration::from_millis(500),
            )
            .await
            .unwrap_err();
            assert_eq!(timeout_error, "SSH exec 超时");

            let overflow_error = exec_ssh_command_capture(
                Arc::clone(&handle),
                "__PORTMATE_TEST_EXEC_OVERFLOW__",
                Duration::from_secs(2),
            )
            .await
            .unwrap_err();
            assert!(overflow_error.contains("stderr"), "{overflow_error}");
            assert!(
                overflow_error.contains(&MAX_SSH_EXEC_STDERR_BYTES.to_string()),
                "{overflow_error}"
            );

            tokio::time::timeout(Duration::from_secs(1), async {
                while counters.channel_closes.load(Ordering::SeqCst) < 5 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("SSH exec channels were not closed on every exit path");
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 5);

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn sftp_in_flight_request_observes_transfer_cancellation() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping SFTP cancellation test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-silent-sftp-user";
            let secret = "PortMate silent SFTP secret";
            let (port, counters, server_task) =
                spawn_silent_sftp_test_server(&host_key, username, secret, Duration::from_secs(1))
                    .await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let handle = Arc::new(tokio::sync::Mutex::new(handle));
            let sftp = open_sftp_session_with_timeout(Arc::clone(&handle), Duration::from_secs(1))
                .await
                .unwrap();
            let state = test_app_state(
                test_shell_profile(),
                root.path().join("portmate-store.sqlite3"),
            );
            let cancel = Arc::new(AtomicBool::new(false));
            let progress = test_transfer_progress_context(
                &state,
                "unused-silent-sftp-cancel",
                Arc::clone(&cancel),
            );
            let request_counters = Arc::clone(&counters);
            let cancel_task = tokio::spawn(async move {
                tokio::time::timeout(Duration::from_secs(1), async {
                    while request_counters.lstat_attempts.load(Ordering::SeqCst) == 0 {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await
                .expect("silent SFTP server did not receive LSTAT");
                cancel.store(true, Ordering::SeqCst);
            });

            let started = Instant::now();
            let local_target = root.path().join("silent-target.bin");
            let transfer = sftp_download(
                &sftp,
                "/silent-source.bin",
                local_target.to_str().unwrap(),
                &progress,
            );
            let error = await_sftp_transfer_with_cancellation(transfer, &progress)
                .await
                .unwrap_err();
            assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
            assert!(started.elapsed() < Duration::from_millis(500));
            cancel_task.await.unwrap();
            sftp.close().await.unwrap();

            let channel = open_shared_russh_exec_channel(
                &handle,
                "true",
                Duration::from_secs(1),
                "SFTP cancellation follow-up exec",
            )
            .await
            .unwrap();
            close_russh_channel_bounded(&channel).await;
            assert_eq!(counters.subsystem_requests.load(Ordering::SeqCst), 1);
            assert_eq!(counters.lstat_attempts.load(Ordering::SeqCst), 1);
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);
            assert_eq!(
                counters.session_channel_completions.load(Ordering::SeqCst),
                2
            );

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn scp_upload_closes_success_and_rejects_status_after_eof() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping SCP upload completion test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);
        let source = root.path().join("empty.bin");
        fs::write(&source, []).unwrap();

        tauri::async_runtime::block_on(async {
            let username = "portmate-scp-upload-completion-user";
            let secret = "PortMate SCP upload completion secret";
            let (port, counters, server_task) =
                spawn_mixed_auth_test_server(&host_key, username, secret).await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let handle = Arc::new(tokio::sync::Mutex::new(handle));
            let profile = test_shell_profile();
            let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
            state
                .store
                .lock()
                .unwrap()
                .transfers
                .push(test_transfer_task(&profile.id, TransferStatus::Running));
            let progress = test_transfer_progress_context(
                &state,
                "transfer-commit-test",
                Arc::new(AtomicBool::new(false)),
            );

            let uploaded = scp_upload(
                Arc::clone(&handle),
                source.to_str().unwrap(),
                "/__PORTMATE_TEST_SCP_UPLOAD_SUCCESS__",
                &progress,
            )
            .await
            .unwrap();
            assert_eq!(uploaded, 0);

            let error = scp_upload(
                Arc::clone(&handle),
                source.to_str().unwrap(),
                "/__PORTMATE_TEST_SCP_UPLOAD_EOF_BEFORE_NONZERO__",
                &progress,
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                "SCP upload remote returned non-zero 12: late SCP upload failure"
            );

            tokio::time::timeout(Duration::from_secs(1), async {
                while counters.channel_closes.load(Ordering::SeqCst) < 2 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("SCP upload channels were not closed on success and failure");
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn scp_download_validates_completion_and_protocol_streams() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping SCP download completion test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-scp-download-completion-user";
            let secret = "PortMate SCP download completion secret";
            let (port, counters, server_task) =
                spawn_mixed_auth_test_server(&host_key, username, secret).await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let handle = Arc::new(tokio::sync::Mutex::new(handle));
            let profile = test_shell_profile();
            let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
            state
                .store
                .lock()
                .unwrap()
                .transfers
                .push(test_transfer_task(&profile.id, TransferStatus::Running));
            let progress = test_transfer_progress_context(
                &state,
                "transfer-commit-test",
                Arc::new(AtomicBool::new(false)),
            );

            let target = root.path().join("download-success.bin");
            let part = local_resume_part_path(&target);
            fs::write(&part, b"zz").unwrap();
            let downloaded = scp_download_with_idle_timeout(
                Arc::clone(&handle),
                "/__PORTMATE_TEST_SCP_DOWNLOAD_SUCCESS__",
                target.to_str().unwrap(),
                &progress,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert_eq!(downloaded, 4);
            assert_eq!(fs::read(&target).unwrap(), b"data");
            assert!(!part.exists());

            let failed_target = root.path().join("download-failed.bin");
            let failed_part = local_resume_part_path(&failed_target);
            let error = scp_download_with_idle_timeout(
                Arc::clone(&handle),
                "/__PORTMATE_TEST_SCP_DOWNLOAD_EOF_BEFORE_NONZERO__",
                failed_target.to_str().unwrap(),
                &progress,
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                "SCP download remote returned non-zero 13: late SCP download failure"
            );
            assert!(!failed_target.exists());
            assert_eq!(fs::read(&failed_part).unwrap(), b"data");

            let stderr_target = root.path().join("download-with-stderr.bin");
            let downloaded = scp_download_with_idle_timeout(
                Arc::clone(&handle),
                "/__PORTMATE_TEST_SCP_DOWNLOAD_STDERR_BEFORE_DATA__",
                stderr_target.to_str().unwrap(),
                &progress,
                Duration::from_secs(1),
            )
            .await
            .unwrap();
            assert_eq!(downloaded, 4);
            assert_eq!(fs::read(&stderr_target).unwrap(), b"data");

            let oversized_target = root.path().join("download-oversized-header.bin");
            let error = scp_download_with_idle_timeout(
                Arc::clone(&handle),
                "/__PORTMATE_TEST_SCP_DOWNLOAD_OVERSIZED_HEADER__",
                oversized_target.to_str().unwrap(),
                &progress,
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                format!("SCP 读取文件头 超过协议行上限（{MAX_SCP_PROTOCOL_LINE_BYTES} bytes）")
            );
            assert!(!oversized_target.exists());

            tokio::time::timeout(Duration::from_secs(1), async {
                while counters.channel_closes.load(Ordering::SeqCst) < 4 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("SCP download channels were not closed across protocol outcomes");
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 4);

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn scp_download_silent_peer_observes_cancellation_and_idle_timeout() {
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping silent SCP test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-silent-scp-user";
            let secret = "PortMate silent SCP secret";
            let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
                &host_key, username, secret, None, None, None,
            )
            .await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let handle = Arc::new(tokio::sync::Mutex::new(handle));
            let state = test_app_state(
                test_shell_profile(),
                root.path().join("portmate-store.sqlite3"),
            );

            let cancel = Arc::new(AtomicBool::new(false));
            let cancellation_progress = TransferProgressContext {
                state: state.clone(),
                task_id: "unused-silent-scp-cancel".to_string(),
                cancel: Arc::clone(&cancel),
                last_emit: Arc::new(Mutex::new(Instant::now())),
                started: Instant::now(),
                rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
                rate_limit_bytes_per_second: None,
            };
            let cancel_task = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                cancel.store(true, Ordering::SeqCst);
            });
            let started = Instant::now();
            let error = scp_download_with_idle_timeout(
                Arc::clone(&handle),
                "/silent-cancel.bin",
                root.path().join("cancel.bin").to_str().unwrap(),
                &cancellation_progress,
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
            assert!(started.elapsed() < Duration::from_millis(500));
            cancel_task.await.unwrap();

            let idle_progress = TransferProgressContext {
                state,
                task_id: "unused-silent-scp-idle".to_string(),
                cancel: Arc::new(AtomicBool::new(false)),
                last_emit: Arc::new(Mutex::new(Instant::now())),
                started: Instant::now(),
                rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
                rate_limit_bytes_per_second: None,
            };
            let started = Instant::now();
            let error = scp_download_with_idle_timeout(
                Arc::clone(&handle),
                "/silent-idle.bin",
                root.path().join("idle.bin").to_str().unwrap(),
                &idle_progress,
                Duration::from_millis(30),
            )
            .await
            .unwrap_err();
            assert_eq!(error, "SCP 等待文件头 空闲超时（30 ms）");
            assert!(started.elapsed() < Duration::from_millis(500));
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);
            assert_eq!(
                counters.session_channel_completions.load(Ordering::SeqCst),
                2
            );

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
        });
    }

    #[cfg(unix)]
    #[test]
    fn remote_copy_silent_peer_observes_cancellation_and_idle_timeout() {
        let _runtime_guard = shared_runtime_test_guard();
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping silent remote-copy test: ssh-keygen is not installed");
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let host_key = root.path().join("ssh_host_ed25519_key");
        generate_ed25519_test_key(&host_key);

        tauri::async_runtime::block_on(async {
            let username = "portmate-silent-remote-copy-user";
            let secret = "PortMate silent remote-copy secret";
            let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
                &host_key, username, secret, None, None, None,
            )
            .await;
            let mut handle = client::connect(
                Arc::new(client::Config::default()),
                ("127.0.0.1", port),
                AcceptAnyTestSshClient,
            )
            .await
            .unwrap();
            assert!(handle
                .authenticate_password(username, secret)
                .await
                .unwrap()
                .success());
            let handle = Arc::new(tokio::sync::Mutex::new(handle));
            let state = test_app_state(
                test_shell_profile(),
                root.path().join("portmate-store.sqlite3"),
            );

            let cancel = Arc::new(AtomicBool::new(false));
            let cancellation_progress = test_transfer_progress_context(
                &state,
                "unused-silent-remote-copy-cancel",
                Arc::clone(&cancel),
            );
            let cancel_task = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                cancel.store(true, Ordering::SeqCst);
            });
            let started = Instant::now();
            let error = remote_copy_with_timeouts(
                Arc::clone(&handle),
                "/silent-source.bin",
                "/silent-destination.bin",
                &cancellation_progress,
                Duration::from_secs(1),
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
            assert!(started.elapsed() < Duration::from_millis(500));
            cancel_task.await.unwrap();

            let idle_progress = test_transfer_progress_context(
                &state,
                "unused-silent-remote-copy-idle",
                Arc::new(AtomicBool::new(false)),
            );
            let started = Instant::now();
            let error = remote_copy_with_timeouts(
                Arc::clone(&handle),
                "/silent-source.bin",
                "/silent-destination.bin",
                &idle_progress,
                Duration::from_millis(30),
                Duration::from_secs(1),
            )
            .await
            .unwrap_err();
            assert_eq!(error, "SSH remote copy 空闲超时（30 ms）");
            assert!(started.elapsed() < Duration::from_millis(500));
            state
                .store
                .lock()
                .unwrap()
                .transfers
                .push(test_transfer_task(
                    &test_shell_profile().id,
                    TransferStatus::Running,
                ));
            let late_status_progress = test_transfer_progress_context(
                &state,
                "transfer-commit-test",
                Arc::new(AtomicBool::new(false)),
            );
            let error = remote_copy_with_timeouts(
                Arc::clone(&handle),
                "/__PORTMATE_TEST_REMOTE_COPY_EOF_BEFORE_NONZERO__",
                "/destination.bin",
                &late_status_progress,
                Duration::from_secs(1),
                Duration::from_secs(15),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error,
                "SSH remote copy 返回非零状态 11: late remote-copy failure"
            );
            assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 3);
            assert_eq!(
                counters.session_channel_completions.load(Ordering::SeqCst),
                3
            );

            drop(handle);
            server_task.abort();
            let _ = server_task.await;
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
    fn openssh_reconnect_store_commit_failure_does_not_install_runtime() {
        let _runtime_guard = shared_runtime_test_guard();
        let Some(sshd_path) = openssh_test_server_path() else {
            eprintln!("skipping OpenSSH reconnect Store test: sshd is not installed");
            return;
        };
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping OpenSSH reconnect Store test: ssh-keygen is not installed");
            return;
        }

        let root = std::env::temp_dir().join(format!(
            "portmate-ssh-reconnect-commit-failure-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let host_key = root.join("ssh_host_ed25519_key");
        let client_key = root.join("id_ed25519");
        generate_ed25519_test_key(&host_key);
        generate_ed25519_test_key(&client_key);
        let authorized_keys = root.join("authorized_keys");
        fs::copy(client_key.with_extension("pub"), &authorized_keys).unwrap();
        let port = {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
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
            wait_for_openssh_test_server(&mut sshd, port, "reconnect Store sshd").await;
            let mut profile = test_ssh_profile();
            let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
                panic!("expected SSH profile");
            };
            ssh.endpoint.host = "127.0.0.1".to_string();
            ssh.endpoint.port = port;
            ssh.username = openssh_test_username();
            ssh.reconnect = true;
            ssh.reconnect_delay_ms = 100;
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
            ssh.identity_refs = vec![IdentityRef {
                id: "reconnect-store-client-key".to_string(),
                label: "reconnect Store client key".to_string(),
                source: IdentitySource::SystemFile,
                fingerprint_sha256: None,
                path: Some(client_key.display().to_string()),
                secret_ref: None,
            }];
            ssh.agent_policy.enabled = false;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;

            let store_dir = root.join("store");
            fs::create_dir_all(&store_dir).unwrap();
            let state = test_app_state(profile.clone(), store_dir.join("portmate-store.sqlite3"));
            open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap();
            let reconnect_handle = {
                let connections = state.ssh.lock().unwrap();
                Arc::clone(&connections.get(&profile.id).unwrap().handle)
            };

            *state.ssh_reconnect_install_error.lock().unwrap() =
                Some("injected SSH reconnect install commit failure".to_string());
            {
                let handle = reconnect_handle.lock().await;
                handle
                    .disconnect("PortMate reconnect Store failure test")
                    .await
                    .unwrap();
            }

            tokio::time::timeout(Duration::from_secs(30), async {
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
                    if status != SessionStatus::Connected {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("SSH disconnect did not leave the connected state");

            let failed = tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let summary = state
                        .store
                        .lock()
                        .unwrap()
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == profile.id)
                        .unwrap();
                    if summary.runtime.status == SessionStatus::Error {
                        break summary;
                    }
                    assert_ne!(
                        summary.runtime.status,
                        SessionStatus::Connected,
                        "failed SSH reconnect exposed a connected runtime"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("SSH reconnect Store failure did not settle to Error");

            assert!(
                failed
                    .runtime
                    .last_disconnect_reason
                    .as_deref()
                    .is_some_and(|reason| reason.contains("SSH reconnect install failed")),
                "unexpected SSH reconnect failure: {:?}",
                failed.runtime.last_disconnect_reason
            );
            assert!(!state.ssh.lock().unwrap().contains_key(&profile.id));
            assert!(state.store.lock().unwrap().events.iter().any(|event| {
                event
                    .text
                    .as_deref()
                    .is_some_and(|text| text.contains("SSH reconnect install failed"))
            }));
        });

        sshd.stop();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn openssh_sftp_scp_and_tunnels_end_to_end() {
        let _runtime_guard = shared_runtime_test_guard();
        use std::os::unix::fs::PermissionsExt;

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
                ssh.reconnect = true;
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

            let entries = list_files_inner(
                &state,
                ListFilesRequest {
                    session_id: Some(profile.id.clone()),
                    path: ".".to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap();
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

            let sftp_new_file = root.join("sftp-created-file.txt");
            file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: sftp_new_file.display().to_string(),
                    remote: true,
                },
                FileOperation::CreateFile,
            )
            .await
            .unwrap();
            assert_eq!(fs::read(&sftp_new_file).unwrap(), b"");
            fs::write(&sftp_new_file, b"existing remote contents").unwrap();
            let error = file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: sftp_new_file.display().to_string(),
                    remote: true,
                },
                FileOperation::CreateFile,
            )
            .await
            .unwrap_err();
            assert!(error.contains("新建远端文件"), "{error}");
            assert_eq!(
                fs::read(&sftp_new_file).unwrap(),
                b"existing remote contents"
            );

            let sftp_move_source = root.join("sftp-move-source");
            let sftp_move_destination = root.join("sftp-move-destination");
            let sftp_move_file = sftp_move_source.join("report.txt");
            let sftp_move_directory = sftp_move_source.join("nested");
            fs::create_dir_all(&sftp_move_directory).unwrap();
            fs::create_dir(&sftp_move_destination).unwrap();
            fs::write(&sftp_move_file, b"remote report").unwrap();
            fs::write(sftp_move_directory.join("detail.txt"), b"remote detail").unwrap();
            move_paths_inner(
                &state,
                MovePathsRequest {
                    session_id: Some(profile.id.clone()),
                    paths: vec![
                        sftp_move_file.display().to_string(),
                        sftp_move_directory.display().to_string(),
                    ],
                    destination: sftp_move_destination.display().to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap();
            assert!(!sftp_move_file.exists());
            assert!(!sftp_move_directory.exists());
            assert_eq!(
                fs::read(sftp_move_destination.join("report.txt")).unwrap(),
                b"remote report"
            );
            assert_eq!(
                fs::read(sftp_move_destination.join("nested/detail.txt")).unwrap(),
                b"remote detail"
            );

            let sftp_move_first = sftp_move_source.join("first.txt");
            let sftp_move_collision = sftp_move_source.join("collision.txt");
            fs::write(&sftp_move_first, b"first source").unwrap();
            fs::write(&sftp_move_collision, b"collision source").unwrap();
            fs::write(
                sftp_move_destination.join("collision.txt"),
                b"existing remote target",
            )
            .unwrap();
            let error = move_paths_inner(
                &state,
                MovePathsRequest {
                    session_id: Some(profile.id.clone()),
                    paths: vec![
                        sftp_move_first.display().to_string(),
                        sftp_move_collision.display().to_string(),
                    ],
                    destination: sftp_move_destination.display().to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap_err();
            assert!(error.contains("已存在"), "{error}");
            assert_eq!(fs::read(&sftp_move_first).unwrap(), b"first source");
            assert_eq!(fs::read(&sftp_move_collision).unwrap(), b"collision source");
            assert!(!sftp_move_destination.join("first.txt").exists());
            assert_eq!(
                fs::read(sftp_move_destination.join("collision.txt")).unwrap(),
                b"existing remote target"
            );

            let sftp_delete_root = root.join("sftp-delete-root");
            let sftp_delete_file = sftp_delete_root.join("single.txt");
            let sftp_delete_directory = sftp_delete_root.join("nested");
            fs::create_dir_all(&sftp_delete_directory).unwrap();
            fs::write(&sftp_delete_file, b"delete remote file").unwrap();
            fs::write(
                sftp_delete_directory.join("value.txt"),
                b"delete remote nested",
            )
            .unwrap();
            delete_paths_inner(
                &state,
                DeletePathsRequest {
                    session_id: Some(profile.id.clone()),
                    paths: vec![
                        sftp_delete_file.display().to_string(),
                        sftp_delete_directory.display().to_string(),
                    ],
                    remote: true,
                },
            )
            .await
            .unwrap();
            assert!(!sftp_delete_file.exists());
            assert!(!sftp_delete_directory.exists());
            fs::remove_dir(&sftp_delete_root).unwrap();

            let sftp_link_target = root.join("sftp-link-target");
            let sftp_directory_link = root.join("sftp-directory-link");
            fs::create_dir(&sftp_link_target).unwrap();
            std::os::unix::fs::symlink(&sftp_link_target, &sftp_directory_link).unwrap();
            let error = file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: sftp_directory_link
                        .join("new-file.txt")
                        .display()
                        .to_string(),
                    remote: true,
                },
                FileOperation::CreateFile,
            )
            .await
            .unwrap_err();
            assert!(error.contains("符号链接"), "{error}");
            assert!(!sftp_link_target.join("new-file.txt").exists());
            let error = file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: sftp_directory_link.join("nested").display().to_string(),
                    remote: true,
                },
                FileOperation::CreateDirectory,
            )
            .await
            .unwrap_err();
            assert!(error.contains("符号链接"), "{error}");
            assert!(!sftp_link_target.join("nested").exists());
            let linked_file = sftp_link_target.join("protected.bin");
            fs::write(&linked_file, b"protected").unwrap();
            let linked_path = sftp_directory_link.join("protected.bin");
            let original_mode = fs::metadata(&linked_file).unwrap().permissions().mode() & 0o777;
            let error = chmod_path_inner(
                &state,
                ChmodPathRequest {
                    session_id: Some(profile.id.clone()),
                    path: linked_path.display().to_string(),
                    mode: 0o600,
                    remote: true,
                },
            )
            .await
            .unwrap_err();
            assert!(error.contains("符号链接"), "{error}");
            assert_eq!(
                fs::metadata(&linked_file).unwrap().permissions().mode() & 0o777,
                original_mode
            );
            let error = rename_path_inner(
                &state,
                RenamePathRequest {
                    session_id: Some(profile.id.clone()),
                    old_path: linked_path.display().to_string(),
                    new_path: sftp_directory_link
                        .join("renamed.bin")
                        .display()
                        .to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap_err();
            assert!(error.contains("符号链接"), "{error}");
            let error = file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: linked_path.display().to_string(),
                    remote: true,
                },
                FileOperation::Delete,
            )
            .await
            .unwrap_err();
            assert!(error.contains("符号链接"), "{error}");
            assert_eq!(fs::read(&linked_file).unwrap(), b"protected");

            let renamed_directory_link = root.join("sftp-directory-link-renamed");
            rename_path_inner(
                &state,
                RenamePathRequest {
                    session_id: Some(profile.id.clone()),
                    old_path: sftp_directory_link.display().to_string(),
                    new_path: renamed_directory_link.display().to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap();
            assert!(fs::symlink_metadata(&renamed_directory_link)
                .unwrap()
                .file_type()
                .is_symlink());
            assert_eq!(fs::read(&linked_file).unwrap(), b"protected");
            file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: renamed_directory_link.display().to_string(),
                    remote: true,
                },
                FileOperation::Delete,
            )
            .await
            .unwrap();
            assert!(fs::symlink_metadata(&renamed_directory_link).is_err());
            assert_eq!(fs::read(&linked_file).unwrap(), b"protected");
            fs::remove_dir_all(&sftp_link_target).unwrap();

            let drop_source = root.join("external-drop-source");
            let drop_source_nested = drop_source.join("nested");
            fs::create_dir_all(drop_source.join("empty")).unwrap();
            fs::create_dir_all(&drop_source_nested).unwrap();
            fs::write(drop_source.join("alpha.txt"), b"external-alpha").unwrap();
            fs::write(drop_source_nested.join("beta.bin"), b"external-beta").unwrap();
            let drop_remote_target = root.join("external-drop-remote");
            let drop_result = start_external_drop_inner(
                &state,
                StartExternalDropRequest {
                    session_id: profile.id.clone(),
                    paths: vec![drop_source.display().to_string()],
                    destination: drop_remote_target.display().to_string(),
                    remote: true,
                    conflict_policy: TransferConflictPolicy::Fail,
                },
            )
            .await
            .unwrap();
            assert_eq!(drop_result.tasks.len(), 2);
            assert_eq!(drop_result.directories_prepared, 3);
            assert_eq!(drop_result.total_bytes, 27);
            assert!(drop_result.skipped.is_empty());
            for task in drop_result.tasks {
                let task = wait_for_transfer_terminal_state(&state, &task.id).await;
                assert_eq!(
                    task.status,
                    TransferStatus::Completed,
                    "recursive external SFTP drop failed: {:?}",
                    task.message
                );
            }
            let dropped_remote_tree = drop_remote_target.join("external-drop-source");
            assert_eq!(
                fs::read(dropped_remote_tree.join("alpha.txt")).unwrap(),
                b"external-alpha"
            );
            assert_eq!(
                fs::read(dropped_remote_tree.join("nested/beta.bin")).unwrap(),
                b"external-beta"
            );
            assert!(dropped_remote_tree.join("empty").is_dir());

            let sftp_source = root.join("sftp-upload-source.bin");
            let sftp_payload = b"PortMate OpenSSH SFTP integration payload\n";
            fs::write(&sftp_source, sftp_payload).unwrap();
            let uploaded_sftp_file = sftp_nested.join("sftp-upload-source.bin");
            let uploaded_sftp_part = PathBuf::from(remote_resume_part_path(
                uploaded_sftp_file.to_str().unwrap(),
            ));
            fs::write(&uploaded_sftp_part, b"wrong-prefix").unwrap();
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

            let existing_rename_target = sftp_nested.join("existing-rename-target.bin");
            fs::write(&existing_rename_target, b"existing rename target").unwrap();
            let error = rename_path_inner(
                &state,
                RenamePathRequest {
                    session_id: Some(profile.id.clone()),
                    old_path: uploaded_sftp_file.display().to_string(),
                    new_path: existing_rename_target.display().to_string(),
                    remote: true,
                },
            )
            .await
            .unwrap_err();
            assert!(error.contains("已存在"), "{error}");
            assert_eq!(fs::read(&uploaded_sftp_file).unwrap(), sftp_payload);
            assert_eq!(
                fs::read(&existing_rename_target).unwrap(),
                b"existing rename target"
            );
            fs::remove_file(&existing_rename_target).unwrap();

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

            let chmod_link = sftp_nested.join("chmod-link.bin");
            std::os::unix::fs::symlink(&renamed_sftp_file, &chmod_link).unwrap();
            let original_mode = fs::metadata(&renamed_sftp_file)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            let error = chmod_path_inner(
                &state,
                ChmodPathRequest {
                    session_id: Some(profile.id.clone()),
                    path: chmod_link.display().to_string(),
                    mode: 0o600,
                    remote: true,
                },
            )
            .await
            .unwrap_err();
            assert!(error.contains("符号链接"), "{error}");
            assert_eq!(
                fs::metadata(&renamed_sftp_file)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                original_mode
            );
            fs::remove_file(&chmod_link).unwrap();

            let copied_sftp_file = sftp_root.join("copied.bin");
            let copied_sftp_part =
                PathBuf::from(remote_resume_part_path(copied_sftp_file.to_str().unwrap()));
            fs::write(&copied_sftp_part, b"wrong-prefix").unwrap();
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
            fs::write(&sftp_download_part, b"wrong-prefix").unwrap();
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

            let sftp_empty = sftp_root.join("empty");
            file_operation_inner(
                &state,
                FileOperationRequest {
                    session_id: Some(profile.id.clone()),
                    path: sftp_empty.display().to_string(),
                    remote: true,
                },
                FileOperation::CreateDirectory,
            )
            .await
            .unwrap();
            let recursive_download_root = root.join("recursive-download");
            fs::create_dir_all(&recursive_download_root).unwrap();
            let recursive_download = start_file_batch_inner(
                &state,
                StartFileBatchRequest {
                    session_id: profile.id.clone(),
                    paths: vec![sftp_root.display().to_string()],
                    source_remote: true,
                    destination: recursive_download_root.display().to_string(),
                    destination_remote: false,
                    conflict_policy: TransferConflictPolicy::Fail,
                },
            )
            .await
            .unwrap();
            assert_eq!(recursive_download.tasks.len(), 2);
            assert_eq!(recursive_download.directories_prepared, 3);
            assert_eq!(
                recursive_download.total_bytes,
                (sftp_payload.len() * 2) as u64
            );
            assert!(recursive_download.skipped.is_empty());
            for task in recursive_download.tasks {
                let task = wait_for_transfer_terminal_state(&state, &task.id).await;
                assert_eq!(
                    task.status,
                    TransferStatus::Completed,
                    "recursive SFTP download failed: {:?}",
                    task.message
                );
            }
            let downloaded_tree = recursive_download_root.join("sftp-workspace");
            assert_eq!(
                fs::read(downloaded_tree.join("copied.bin")).unwrap(),
                sftp_payload
            );
            assert_eq!(
                fs::read(downloaded_tree.join("nested/renamed.bin")).unwrap(),
                sftp_payload
            );
            assert!(downloaded_tree.join("empty").is_dir());

            fs::write(recursive_download_root.join("copied.bin"), b"existing").unwrap();
            let renamed_download = start_file_batch_inner(
                &state,
                StartFileBatchRequest {
                    session_id: profile.id.clone(),
                    paths: vec![copied_sftp_file.display().to_string()],
                    source_remote: true,
                    destination: recursive_download_root.display().to_string(),
                    destination_remote: false,
                    conflict_policy: TransferConflictPolicy::Rename,
                },
            )
            .await
            .unwrap();
            assert_eq!(renamed_download.tasks.len(), 1);
            let renamed_task =
                wait_for_transfer_terminal_state(&state, &renamed_download.tasks[0].id).await;
            assert_eq!(renamed_task.status, TransferStatus::Completed);
            assert_eq!(
                fs::read(recursive_download_root.join("copied (1).bin")).unwrap(),
                sftp_payload
            );

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

            let empty_sftp_source = root.join("empty-sftp-source.bin");
            let empty_sftp_target = root.join("empty-sftp-target.bin");
            fs::write(&empty_sftp_source, []).unwrap();
            let empty_sftp_upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Sftp,
                    source: empty_sftp_source.display().to_string(),
                    destination: format!("remote:{}", empty_sftp_target.display()),
                },
            )
            .await
            .unwrap();
            let empty_sftp_upload =
                wait_for_transfer_terminal_state(&state, &empty_sftp_upload.id).await;
            assert_eq!(
                empty_sftp_upload.status,
                TransferStatus::Completed,
                "empty SFTP upload failed: {:?}",
                empty_sftp_upload.message
            );
            assert_eq!(fs::metadata(&empty_sftp_target).unwrap().len(), 0);

            let upload_source = root.join("scp-upload-source.bin");
            let remote_file = root.join("scp-remote.bin");
            let download_target = root.join("scp-download-target.bin");
            let payload = b"PortMate OpenSSH SCP integration payload\n";
            fs::write(&upload_source, payload).unwrap();
            let remote_part = PathBuf::from(remote_resume_part_path(remote_file.to_str().unwrap()));
            fs::write(&remote_part, b"wrong-prefix").unwrap();
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

            let denied_target = format!("/proc/portmate-transfer-denied-{}.bin", Uuid::new_v4());
            for protocol in [TransferProtocol::Sftp, TransferProtocol::Scp] {
                let failed_upload = start_transfer_inner(
                    &state,
                    StartTransferRequest {
                        session_id: profile.id.clone(),
                        protocol: protocol.clone(),
                        source: upload_source.display().to_string(),
                        destination: format!("remote:{denied_target}"),
                    },
                )
                .await
                .unwrap();
                let failed_upload =
                    wait_for_transfer_terminal_state(&state, &failed_upload.id).await;
                assert_eq!(
                    failed_upload.status,
                    TransferStatus::Failed,
                    "{protocol:?} server-side write failure was not reported: {:?}",
                    failed_upload.message
                );
                let message = failed_upload.message.unwrap_or_default();
                assert!(
                    message.contains("SFTP") || message.contains("SCP"),
                    "{protocol:?} failure lacked protocol context: {message}"
                );
                assert!(
                    !state
                        .transfer_cancellations
                        .lock()
                        .unwrap()
                        .contains_key(&failed_upload.id),
                    "{protocol:?} failed transfer retained its cancellation handle"
                );
            }

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
            // Keep enough limited payload remaining that a heavily loaded parallel test
            // runner cannot finish the transfer before the cancellation poll is scheduled.
            let cancel_payload = (0..2 * 1024 * 1024)
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
            wait_for_transfer_progress(&state, &cancelled_upload.id, "limited SFTP upload").await;
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
            wait_for_transfer_progress(&state, &cancelled_scp_upload.id, "limited SCP upload")
                .await;
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

            for (label, protocol) in [
                ("sftp", TransferProtocol::Sftp),
                ("scp", TransferProtocol::Scp),
            ] {
                {
                    let mut store = state.store.lock().unwrap();
                    let mut limited = store.profile(&profile.id).unwrap();
                    limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
                    store.upsert_profile(limited);
                }
                let disconnect_remote = root.join(format!("{label}-disconnect-remote.bin"));
                let disconnect_remote_part =
                    PathBuf::from(remote_resume_part_path(disconnect_remote.to_str().unwrap()));
                let interrupted_upload = start_transfer_inner(
                    &state,
                    StartTransferRequest {
                        session_id: profile.id.clone(),
                        protocol: protocol.clone(),
                        source: cancel_source.display().to_string(),
                        destination: format!("remote:{}", disconnect_remote.display()),
                    },
                )
                .await
                .unwrap();
                wait_for_transfer_progress(
                    &state,
                    &interrupted_upload.id,
                    &format!("limited {label} upload"),
                )
                .await;

                let disconnected = close_session_inner(&state, profile.id.clone())
                    .await
                    .unwrap();
                assert_eq!(disconnected.runtime.status, SessionStatus::Disconnected);
                let interrupted =
                    wait_for_transfer_terminal_state(&state, &interrupted_upload.id).await;
                assert_eq!(
                    interrupted.status,
                    TransferStatus::Failed,
                    "{protocol:?} SSH disconnect was not reported as a failure: {:?}",
                    interrupted.message
                );
                assert!(
                    !state
                        .transfer_cancellations
                        .lock()
                        .unwrap()
                        .contains_key(&interrupted.id),
                    "{protocol:?} disconnected transfer retained its cancellation handle"
                );
                assert!(!disconnect_remote.exists());
                let partial_size = fs::metadata(&disconnect_remote_part).unwrap().len();
                assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

                let reopened = open_ssh_session(&state, profile.clone(), None, None)
                    .await
                    .unwrap();
                assert_eq!(reopened.runtime.status, SessionStatus::Connected);
                {
                    let mut store = state.store.lock().unwrap();
                    let mut unlimited = store.profile(&profile.id).unwrap();
                    unlimited.transfer.rate_limit_bytes_per_second = None;
                    store.upsert_profile(unlimited);
                }
                let retried = retry_transfer_inner(&state, &interrupted_upload.id)
                    .await
                    .unwrap();
                let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
                assert_eq!(
                    retried.status,
                    TransferStatus::Completed,
                    "{protocol:?} retry after reconnect failed: {:?}",
                    retried.message
                );
                assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
                assert_eq!(fs::read(&disconnect_remote).unwrap(), cancel_payload);
                assert!(!disconnect_remote_part.exists());
            }

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
                assert!(
                    !PathBuf::from(remote_resume_part_path(zmodem_remote.to_str().unwrap()))
                        .exists()
                );

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
                let xmodem_payload = b"PortMate XModem integration payload\n".repeat(8);
                assert!(xmodem_payload.len() > XMODEM_BLOCK_SIZE);
                fs::write(&xmodem_source, &xmodem_payload).unwrap();
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
                assert!(
                    !PathBuf::from(remote_resume_part_path(xmodem_remote.to_str().unwrap()))
                        .exists()
                );

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
                let ymodem_payload = b"PortMate YModem\x00binary\xffpayload\n".repeat(40);
                assert!(ymodem_payload.len() > YMODEM_BLOCK_SIZE);
                fs::write(&ymodem_source, &ymodem_payload).unwrap();
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
                assert!(
                    !PathBuf::from(remote_resume_part_path(ymodem_remote.to_str().unwrap()))
                        .exists()
                );

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
            drop(echo_listener);
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

            let mut failed_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
                .await
                .unwrap();
            failed_client.write_all(b"ping").await.unwrap();
            let mut closed_byte = [0_u8; 1];
            let read =
                tokio::time::timeout(Duration::from_secs(2), failed_client.read(&mut closed_byte))
                    .await
                    .expect("failed local tunnel client did not close");
            assert_tunnel_client_closed(read, "failed local tunnel client");
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let status = list_tunnels_inner(&state, Some(&profile.id))
                        .unwrap()
                        .into_iter()
                        .find(|status| status.spec.id == tunnel.id)
                        .unwrap();
                    if status.active_connections == 0 && status.total_connections == 1 {
                        assert!(status
                            .last_error
                            .as_deref()
                            .is_some_and(|error| error.contains("direct-tcpip open failed")));
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("local tunnel failure metrics did not settle");

            let echo_listener = TcpListener::bind(echo_address).await.unwrap();
            let echo = tokio::spawn(async move {
                let (mut socket, _) = echo_listener.accept().await.unwrap();
                let mut request = [0_u8; 4];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                socket.write_all(b"pong").await.unwrap();
            });
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
                    if status.active_connections == 0 && status.total_connections == 2 {
                        assert_eq!(status.tcp_to_ssh_bytes, 4);
                        assert_eq!(status.ssh_to_tcp_bytes, 4);
                        assert!(status.last_error.is_none());
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
            drop(dynamic_echo_listener);
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

            let [port_high, port_low] = dynamic_echo_address.port().to_be_bytes();
            let mut failed_socks_client =
                TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
                    .await
                    .unwrap();
            failed_socks_client.write_all(&[5, 1, 0]).await.unwrap();
            let mut failed_method = [0_u8; 2];
            failed_socks_client
                .read_exact(&mut failed_method)
                .await
                .unwrap();
            assert_eq!(failed_method, [5, 0]);
            failed_socks_client
                .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
                .await
                .unwrap();
            let mut failed_socks_reply = [0_u8; 10];
            failed_socks_client
                .read_exact(&mut failed_socks_reply)
                .await
                .unwrap();
            assert_eq!(failed_socks_reply, super::socks5_reply(5));
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let status = list_tunnels_inner(&state, Some(&profile.id))
                        .unwrap()
                        .into_iter()
                        .find(|status| status.spec.id == dynamic_tunnel.id)
                        .unwrap();
                    if status.active_connections == 0 && status.total_connections == 1 {
                        assert!(status.last_error.as_deref().is_some_and(
                            |error| error.contains("dynamic direct-tcpip open failed")
                        ));
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("dynamic tunnel failure metrics did not settle");

            let dynamic_echo_listener = TcpListener::bind(dynamic_echo_address).await.unwrap();
            let dynamic_echo = tokio::spawn(async move {
                let (mut socket, _) = dynamic_echo_listener.accept().await.unwrap();
                let mut request = [0_u8; 4];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                socket.write_all(b"pong").await.unwrap();
            });
            let mut socks_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
                .await
                .unwrap();
            socks_client.write_all(&[5, 1, 0]).await.unwrap();
            let mut method = [0_u8; 2];
            socks_client.read_exact(&mut method).await.unwrap();
            assert_eq!(method, [5, 0]);
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
                    if status.active_connections == 0 && status.total_connections == 2 {
                        assert_eq!(status.tcp_to_ssh_bytes, 4);
                        assert_eq!(status.ssh_to_tcp_bytes, 4);
                        assert!(status.last_error.is_none());
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
            drop(remote_echo_listener);
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

            let mut failed_remote_client =
                TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
                    .await
                    .unwrap();
            failed_remote_client.write_all(b"ping").await.unwrap();
            let mut closed_byte = [0_u8; 1];
            let read = tokio::time::timeout(
                Duration::from_secs(2),
                failed_remote_client.read(&mut closed_byte),
            )
            .await
            .expect("failed remote tunnel client did not close");
            assert_tunnel_client_closed(read, "failed remote tunnel client");
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let status = list_tunnels_inner(&state, Some(&profile.id))
                        .unwrap()
                        .into_iter()
                        .find(|status| status.spec.id == remote_tunnel.id)
                        .unwrap();
                    if status.active_connections == 0 && status.total_connections == 1 {
                        assert!(status
                            .last_error
                            .as_deref()
                            .is_some_and(|error| error.contains("target connect failed")));
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("remote tunnel failure metrics did not settle");

            let remote_echo_listener = TcpListener::bind(remote_echo_address).await.unwrap();
            let remote_echo = tokio::spawn(async move {
                let (mut socket, _) = remote_echo_listener.accept().await.unwrap();
                let mut request = [0_u8; 4];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                socket.write_all(b"pong").await.unwrap();
            });
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
                    if status.active_connections == 0 && status.total_connections == 2 {
                        assert_eq!(status.tcp_to_ssh_bytes, 4);
                        assert_eq!(status.ssh_to_tcp_bytes, 4);
                        assert!(status.last_error.is_none());
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("remote tunnel metrics did not settle");

            let (remote_health_handle, remote_forward_routes) = {
                let connections = state.ssh.lock().unwrap();
                let runtime = connections.get(&profile.id).unwrap();
                (
                    Arc::clone(&runtime.handle),
                    Arc::clone(&runtime.remote_forwards),
                )
            };
            {
                let handle = remote_health_handle.lock().await;
                handle
                    .russh_compat()
                    .unwrap()
                    .cancel_tcpip_forward(
                        remote_tunnel.bind_host.clone(),
                        u32::from(remote_tunnel.bind_port),
                    )
                    .await
                    .unwrap();
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    match TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port)).await {
                        Err(_) => break,
                        Ok(stream) => drop(stream),
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("server-side remote forward cancellation did not close the listener");

            assert_eq!(
                check_remote_tunnel_health(&state, &remote_tunnel.id)
                    .await
                    .unwrap(),
                RemoteTunnelHealth::Restored
            );
            assert!(state
                .store
                .lock()
                .unwrap()
                .tail_log(&profile.id, 100)
                .iter()
                .any(|event| event.text.as_deref().is_some_and(|text| {
                    text.contains(&remote_tunnel.id)
                        && text.contains("listener was missing and has been restored")
                })));

            let restored_remote_echo_listener =
                TcpListener::bind(remote_echo_address).await.unwrap();
            let restored_remote_echo = tokio::spawn(async move {
                let (mut socket, _) = restored_remote_echo_listener.accept().await.unwrap();
                let mut request = [0_u8; 4];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                socket.write_all(b"pong").await.unwrap();
            });
            let mut restored_remote_client =
                TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
                    .await
                    .unwrap();
            restored_remote_client.write_all(b"ping").await.unwrap();
            let mut restored_remote_response = [0_u8; 4];
            restored_remote_client
                .read_exact(&mut restored_remote_response)
                .await
                .unwrap();
            assert_eq!(&restored_remote_response, b"pong");
            drop(restored_remote_client);
            restored_remote_echo.await.unwrap();

            {
                let handle = remote_health_handle.lock().await;
                handle
                    .russh_compat()
                    .unwrap()
                    .cancel_tcpip_forward(
                        remote_tunnel.bind_host.clone(),
                        u32::from(remote_tunnel.bind_port),
                    )
                    .await
                    .unwrap();
            }
            let stopped = stop_tunnel_inner(&state, &remote_tunnel.id).await.unwrap();
            assert!(!stopped.spec.enabled);
            assert!(stopped
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("remote SSH tunnel cancel failed")));
            assert!(list_tunnels_inner(&state, Some(&profile.id))
                .unwrap()
                .iter()
                .all(|status| status.spec.id != remote_tunnel.id));
            {
                let routes = remote_forward_routes.lock().unwrap();
                assert!(!routes.contains_key(&remote_forward_key(
                    &remote_tunnel.bind_host,
                    remote_tunnel.bind_port,
                )));
                assert!(!routes.contains_key(&remote_forward_port_key(remote_tunnel.bind_port)));
            }
            let saved_profile = state.store.lock().unwrap().profile(&profile.id).unwrap();
            let saved_remote_tunnel = match saved_profile.connection {
                ConnectionConfig::Ssh(ssh) => ssh
                    .tunnels
                    .into_iter()
                    .find(|tunnel| tunnel.id == remote_tunnel.id)
                    .unwrap(),
                _ => panic!("expected SSH profile"),
            };
            assert!(!saved_remote_tunnel.enabled);

            let reconnect_tunnel = create_tunnel_inner(
                &state,
                CreateTunnelRequest {
                    session_id: profile.id.clone(),
                    mode: TunnelMode::Local,
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 0,
                    target_host: "127.0.0.1".to_string(),
                    target_port: port,
                    label: Some("reconnect tunnel".to_string()),
                },
            )
            .await
            .unwrap();
            let reconnect_remote_tunnel = create_tunnel_inner(
                &state,
                CreateTunnelRequest {
                    session_id: profile.id.clone(),
                    mode: TunnelMode::Remote,
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 0,
                    target_host: "127.0.0.1".to_string(),
                    target_port: port,
                    label: Some("reconnect remote tunnel".to_string()),
                },
            )
            .await
            .unwrap();
            let conflict_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let conflict_port = conflict_listener.local_addr().unwrap().port();
            let conflict_tunnel = TunnelSpec {
                id: "reconnect-conflict".to_string(),
                label: "occupied reconnect tunnel".to_string(),
                mode: TunnelMode::Local,
                bind_host: "127.0.0.1".to_string(),
                bind_port: conflict_port,
                target_host: "127.0.0.1".to_string(),
                target_port: port,
                enabled: true,
            };
            {
                let mut store = state.store.lock().unwrap();
                let mut saved_profile = store.profile(&profile.id).unwrap();
                match &mut saved_profile.connection {
                    ConnectionConfig::Ssh(ssh) => {
                        ssh.tunnels.push(conflict_tunnel.clone());
                        ssh.reconnect_delay_ms = 5_000;
                    }
                    _ => panic!("expected SSH profile"),
                }
                store.upsert_profile(saved_profile);
                save_store(&state.store_path, &store).unwrap();
            }
            let (previous_runtime_id, reconnect_handle) = {
                let connections = state.ssh.lock().unwrap();
                let runtime = connections.get(&profile.id).unwrap();
                (runtime.runtime_id.clone(), Arc::clone(&runtime.handle))
            };
            {
                let handle = reconnect_handle.lock().await;
                handle
                    .disconnect("PortMate tunnel reconnect integration test")
                    .await
                    .unwrap();
            }

            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let reconnecting = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                        runtime.session_id == profile.id
                            && runtime.status == SessionStatus::Reconnecting
                    });
                    if reconnecting {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("SSH runtime did not enter reconnecting state");
            {
                let mut store = state.store.lock().unwrap();
                let mut updated = store.profile(&profile.id).unwrap();
                match &mut updated.connection {
                    ConnectionConfig::Ssh(ssh) => ssh.reconnect_delay_ms = 100,
                    _ => panic!("expected SSH profile"),
                }
                store.upsert_profile(updated);
                save_store(&state.store_path, &store).unwrap();
            }
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    let runtime_replaced = state
                        .ssh
                        .lock()
                        .unwrap()
                        .get(&profile.id)
                        .is_some_and(|runtime| runtime.runtime_id != previous_runtime_id);
                    if runtime_replaced {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("SSH reconnect did not adopt the shortened profile delay");

            let restored = tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    let runtime_replaced = state
                        .ssh
                        .lock()
                        .unwrap()
                        .get(&profile.id)
                        .is_some_and(|runtime| runtime.runtime_id != previous_runtime_id);
                    let restored = list_tunnels_inner(&state, Some(&profile.id))
                        .unwrap()
                        .into_iter()
                        .find(|status| status.spec.id == reconnect_tunnel.id);
                    let remote_restored = list_tunnels_inner(&state, Some(&profile.id))
                        .unwrap()
                        .iter()
                        .any(|status| status.spec.id == reconnect_remote_tunnel.id);
                    let conflict_reported = state
                        .store
                        .lock()
                        .unwrap()
                        .tail_log(&profile.id, 200)
                        .iter()
                        .any(|event| {
                            event.text.as_deref().is_some_and(|text| {
                                text.contains("failed to restore SSH tunnel reconnect-conflict")
                                    && text.contains("SSH tunnel bind failed")
                            })
                        });
                    if runtime_replaced && remote_restored && conflict_reported {
                        if let Some(restored) = restored {
                            break restored;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap_or_else(|_| {
                let statuses = list_tunnels_inner(&state, Some(&profile.id)).unwrap();
                let events = state.store.lock().unwrap().tail_log(&profile.id, 20);
                panic!(
                    "SSH reconnect did not restore the tunnel runtime; statuses={statuses:?}; recent events={events:?}"
                )
            });
            assert_eq!(restored.spec.id, reconnect_tunnel.id);
            assert_eq!(restored.spec.label, reconnect_tunnel.label);
            assert_eq!(restored.spec.bind_port, reconnect_tunnel.bind_port);
            let restored_tunnels = list_tunnels_inner(&state, Some(&profile.id)).unwrap();
            let restored_remote = restored_tunnels
                .iter()
                .find(|status| status.spec.id == reconnect_remote_tunnel.id)
                .unwrap();
            assert_eq!(restored_remote.spec.label, reconnect_remote_tunnel.label);
            assert_eq!(
                restored_remote.spec.bind_port,
                reconnect_remote_tunnel.bind_port
            );
            assert!(restored_tunnels
                .iter()
                .all(|status| status.spec.id != conflict_tunnel.id));

            let saved_profile = state.store.lock().unwrap().profile(&profile.id).unwrap();
            let saved_tunnels = match saved_profile.connection {
                ConnectionConfig::Ssh(ssh) => ssh.tunnels,
                _ => panic!("expected SSH profile"),
            };
            assert!(saved_tunnels
                .iter()
                .any(|tunnel| tunnel.id == conflict_tunnel.id && tunnel.enabled));
            assert!(state
                .store
                .lock()
                .unwrap()
                .tail_log(&profile.id, 200)
                .iter()
                .any(|event| event.text.as_deref().is_some_and(|text| {
                    text.contains("failed to restore SSH tunnel reconnect-conflict")
                        && text.contains("SSH tunnel bind failed")
                })));
            let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
            assert!(screen.contains("reconnecting in 5000ms"), "{screen}");

            let mut restored_client = TcpStream::connect(("127.0.0.1", reconnect_tunnel.bind_port))
                .await
                .unwrap();
            let mut ssh_banner = [0_u8; 4];
            tokio::time::timeout(
                Duration::from_secs(2),
                restored_client.read_exact(&mut ssh_banner),
            )
            .await
            .expect("restored tunnel did not receive an SSH banner")
            .unwrap();
            assert_eq!(&ssh_banner, b"SSH-");
            drop(restored_client);

            let mut restored_remote_client =
                TcpStream::connect(("127.0.0.1", reconnect_remote_tunnel.bind_port))
                    .await
                    .unwrap();
            let mut remote_ssh_banner = [0_u8; 4];
            tokio::time::timeout(
                Duration::from_secs(2),
                restored_remote_client.read_exact(&mut remote_ssh_banner),
            )
            .await
            .expect("restored remote tunnel did not receive an SSH banner")
            .unwrap();
            assert_eq!(&remote_ssh_banner, b"SSH-");
            drop(restored_remote_client);
            drop(conflict_listener);
            let stopped = stop_tunnel_inner(&state, &reconnect_tunnel.id)
                .await
                .unwrap();
            assert!(!stopped.spec.enabled);
            let stopped = stop_tunnel_inner(&state, &reconnect_remote_tunnel.id)
                .await
                .unwrap();
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
        let _runtime_guard = shared_runtime_test_guard();
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
        let jump_one_client_key = root.join("jump_one_id_ed25519");
        let jump_two_client_key = root.join("jump_two_id_ed25519");
        let target_client_key = root.join("target_id_ed25519");
        for key_path in [
            &jump_one_host_key,
            &jump_two_host_key,
            &replacement_jump_two_host_key,
            &target_host_key,
            &jump_one_client_key,
            &jump_two_client_key,
            &target_client_key,
        ] {
            generate_ed25519_test_key(key_path);
        }
        let jump_one_authorized_keys = root.join("jump_one_authorized_keys");
        let jump_two_authorized_keys = root.join("jump_two_authorized_keys");
        let target_authorized_keys = root.join("target_authorized_keys");
        fs::copy(
            jump_one_client_key.with_extension("pub"),
            &jump_one_authorized_keys,
        )
        .unwrap();
        fs::copy(
            jump_two_client_key.with_extension("pub"),
            &jump_two_authorized_keys,
        )
        .unwrap();
        fs::copy(
            target_client_key.with_extension("pub"),
            &target_authorized_keys,
        )
        .unwrap();

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
            &jump_one_authorized_keys,
            jump_one_port,
        );
        write_openssh_test_config(
            &jump_two_config,
            &jump_two_host_key,
            &root.join("jump_two_sshd.pid"),
            &jump_two_authorized_keys,
            jump_two_port,
        );
        write_openssh_test_config(
            &target_config,
            &target_host_key,
            &root.join("target_sshd.pid"),
            &target_authorized_keys,
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
                        path: Some(target_client_key.display().to_string()),
                        secret_ref: None,
                    },
                    IdentityRef {
                        id: "jump-one-client-key".to_string(),
                        label: "jump one client key".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(jump_one_client_key.display().to_string()),
                        secret_ref: None,
                    },
                    IdentityRef {
                        id: "jump-two-client-key".to_string(),
                        label: "jump two client key".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(jump_two_client_key.display().to_string()),
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

            let (stalled_first_port, stalled_first) = spawn_stalled_ssh_endpoint().await;
            let mut timed_out_first = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut timed_out_first.connection {
                ssh.jumps[0].port = stalled_first_port;
            }
            let error = establish_ssh_runtime_with_timeout(
                &state,
                &timed_out_first,
                None,
                None,
                Duration::from_millis(200),
                None,
            )
            .await
            .err()
            .expect("stalled first Jump Host unexpectedly connected");
            stalled_first.abort();
            let _ = stalled_first.await;
            assert!(error.contains("Jump Host 第 1 跳连接超时"), "{error}");
            assert!(
                error.contains(&format!("127.0.0.1:{stalled_first_port}")),
                "{error}"
            );
            assert!(state.store.lock().unwrap().host_keys.keys.is_empty());

            let refused_first_port = {
                let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
                reservation.local_addr().unwrap().port()
            };
            let mut refused_first = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut refused_first.connection {
                ssh.jumps[0].port = refused_first_port;
            }
            let error = open_ssh_session(&state, refused_first, None, None)
                .await
                .unwrap_err();
            assert!(error.contains("Jump Host 第 1 跳"), "{error}");
            assert!(
                error.contains(&format!("127.0.0.1:{refused_first_port}")),
                "{error}"
            );
            assert!(state.store.lock().unwrap().host_keys.keys.is_empty());

            let (stalled_second_port, stalled_second) = spawn_stalled_ssh_endpoint().await;
            let mut timed_out_second = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut timed_out_second.connection {
                ssh.jumps[1].port = stalled_second_port;
            }
            let error = establish_ssh_runtime_with_timeout(
                &state,
                &timed_out_second,
                None,
                None,
                Duration::from_millis(200),
                None,
            )
            .await
            .err()
            .expect("stalled second Jump Host unexpectedly connected");
            stalled_second.abort();
            let _ = stalled_second.await;
            assert!(error.contains("Jump Host 第 2 跳连接超时"), "{error}");
            assert!(
                error.contains(&format!("127.0.0.1:{stalled_second_port}")),
                "{error}"
            );
            assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

            let refused_second_port = {
                let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
                reservation.local_addr().unwrap().port()
            };
            let mut refused_second = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut refused_second.connection {
                ssh.jumps[1].port = refused_second_port;
            }
            let error = open_ssh_session(&state, refused_second, None, None)
                .await
                .unwrap_err();
            assert!(
                error.contains("Jump Host 第 2 跳打开 direct-tcpip"),
                "{error}"
            );
            assert!(
                error.contains(&format!("127.0.0.1:{refused_second_port}")),
                "{error}"
            );
            assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

            let mut rejected_second_identity = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut rejected_second_identity.connection {
                ssh.jumps[1].identity_ref = Some("target-client-key".to_string());
            }
            let error = open_ssh_session(&state, rejected_second_identity, None, None)
                .await
                .unwrap_err();
            assert!(error.contains("Jump Host 第 2 跳认证失败"), "{error}");
            assert!(
                error.contains(&format!("127.0.0.1:{jump_two_port}")),
                "{error}"
            );
            assert!(error.contains("target client key"), "{error}");
            assert!(error.contains("被服务器拒绝"), "{error}");
            assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

            let (stalled_target_port, stalled_target) = spawn_stalled_ssh_endpoint().await;
            let mut timed_out_target = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut timed_out_target.connection {
                ssh.endpoint.port = stalled_target_port;
            }
            let error = establish_ssh_runtime_with_timeout(
                &state,
                &timed_out_target,
                None,
                None,
                Duration::from_millis(200),
                None,
            )
            .await
            .err()
            .expect("stalled Jump Host target unexpectedly connected");
            stalled_target.abort();
            let _ = stalled_target.await;
            assert!(error.contains("SSH 经 Jump Host 连接超时"), "{error}");
            assert!(
                error.contains(&format!("127.0.0.1:{stalled_target_port}")),
                "{error}"
            );
            assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 2);

            let mut rejected_target_identities = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut rejected_target_identities.connection {
                ssh.identity_refs
                    .retain(|identity| identity.id != "target-client-key");
            }
            let error = open_ssh_session(&state, rejected_target_identities, None, None)
                .await
                .unwrap_err();
            assert!(error.contains("SSH 目标认证失败"), "{error}");
            assert!(
                error.contains(&format!("127.0.0.1:{target_port}")),
                "{error}"
            );
            assert!(error.contains("jump one client key"), "{error}");
            assert!(error.contains("jump two client key"), "{error}");
            assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 2);

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
                &jump_two_authorized_keys,
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
            let store = state.store.lock().unwrap();
            let trusted_after = &store.host_keys.keys;
            assert_eq!(trusted_after.len(), trusted_before.len());
            for before in &trusted_before {
                let after = trusted_after
                    .iter()
                    .find(|key| key.id == before.id)
                    .expect("host key mismatch must not replace trusted keys");
                if before.alias == "integration-jump-1" {
                    assert!(after.last_seen > before.last_seen);
                    let mut expected = before.clone();
                    expected.last_seen = after.last_seen;
                    assert_eq!(after, &expected);
                } else {
                    assert_eq!(after, before);
                }
            }
            let profile_keys = store
                .profiles
                .iter()
                .find(|stored| stored.id == profile.id)
                .and_then(|stored| match &stored.connection {
                    ConnectionConfig::Ssh(ssh) => Some(&ssh.trusted_host_keys),
                    _ => None,
                })
                .unwrap();
            assert_eq!(profile_keys, trusted_after);
        });

        jump_one_sshd.stop();
        jump_two_sshd.stop();
        target_sshd.stop();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn jump_host_password_and_keyboard_interactive_mix_with_public_keys() {
        let Some(sshd_path) = openssh_test_server_path() else {
            eprintln!("skipping mixed-auth Jump Host test: sshd is not installed");
            return;
        };
        if Command::new("ssh-keygen").arg("-V").output().is_err() {
            eprintln!("skipping mixed-auth Jump Host test: ssh-keygen is not installed");
            return;
        }

        let root =
            std::env::temp_dir().join(format!("portmate-mixed-auth-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let password_jump_host_key = root.join("password_jump_host_ed25519_key");
        let public_key_jump_host_key = root.join("public_key_jump_host_ed25519_key");
        let target_host_key = root.join("target_host_ed25519_key");
        let public_key_jump_client_key = root.join("public_key_jump_id_ed25519");
        let target_client_key = root.join("target_id_ed25519");
        for key_path in [
            &password_jump_host_key,
            &public_key_jump_host_key,
            &target_host_key,
            &public_key_jump_client_key,
            &target_client_key,
        ] {
            generate_ed25519_test_key(key_path);
        }
        let public_key_jump_authorized_keys = root.join("public_key_jump_authorized_keys");
        let target_authorized_keys = root.join("target_authorized_keys");
        fs::copy(
            public_key_jump_client_key.with_extension("pub"),
            &public_key_jump_authorized_keys,
        )
        .unwrap();
        fs::copy(
            target_client_key.with_extension("pub"),
            &target_authorized_keys,
        )
        .unwrap();

        let public_key_jump_port = {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let target_port = {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let public_key_jump_config = root.join("public_key_jump_sshd_config");
        let target_config = root.join("target_sshd_config");
        write_openssh_test_config(
            &public_key_jump_config,
            &public_key_jump_host_key,
            &root.join("public_key_jump_sshd.pid"),
            &public_key_jump_authorized_keys,
            public_key_jump_port,
        );
        write_openssh_test_config(
            &target_config,
            &target_host_key,
            &root.join("target_sshd.pid"),
            &target_authorized_keys,
            target_port,
        );
        let mut public_key_jump_sshd =
            spawn_openssh_test_server(sshd_path, &public_key_jump_config);
        let mut target_sshd = spawn_openssh_test_server(sshd_path, &target_config);

        tauri::async_runtime::block_on(async {
            wait_for_openssh_test_server(
                &mut public_key_jump_sshd,
                public_key_jump_port,
                "mixed-auth public-key jump sshd",
            )
            .await;
            wait_for_openssh_test_server(&mut target_sshd, target_port, "mixed-auth target sshd")
                .await;
            let mixed_username = "portmate-mixed-user";
            let mixed_secret = "PortMate mixed auth secret";
            let (password_jump_port, counters, password_jump_task) =
                spawn_mixed_auth_test_server(&password_jump_host_key, mixed_username, mixed_secret)
                    .await;
            let (proxy_port, proxy_connections, proxy_task) =
                spawn_test_http_connect_proxy(200).await;

            let openssh_username = openssh_test_username();
            let mut profile = test_ssh_profile();
            if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
                ssh.endpoint.host = "127.0.0.1".to_string();
                ssh.endpoint.port = target_port;
                ssh.username = openssh_username.clone();
                ssh.reconnect = false;
                ssh.proxy = ProxyConfig {
                    enabled: true,
                    kind: ProxyKind::HttpConnect,
                    host: "127.0.0.1".to_string(),
                    port: proxy_port,
                    ..ProxyConfig::default()
                };
                ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
                ssh.host_key_policy.alias = Some("mixed-auth-target".to_string());
                ssh.identity_refs = vec![
                    IdentityRef {
                        id: "mixed-target-key".to_string(),
                        label: "mixed target key".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(target_client_key.display().to_string()),
                        secret_ref: None,
                    },
                    IdentityRef {
                        id: "mixed-public-jump-key".to_string(),
                        label: "mixed public jump key".to_string(),
                        source: IdentitySource::SystemFile,
                        fingerprint_sha256: None,
                        path: Some(public_key_jump_client_key.display().to_string()),
                        secret_ref: None,
                    },
                ];
                ssh.agent_policy.enabled = false;
                ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
                let mut password_jump_policy =
                    portmate_core::HostKeyPolicy::profile_alias("mixed-auth-password-jump");
                password_jump_policy.mode = HostKeyMode::TrustOnFirstUse;
                let mut public_key_jump_policy =
                    portmate_core::HostKeyPolicy::profile_alias("mixed-auth-public-key-jump");
                public_key_jump_policy.mode = HostKeyMode::TrustOnFirstUse;
                ssh.jumps = vec![
                    portmate_core::JumpHop {
                        host: "127.0.0.1".to_string(),
                        port: password_jump_port,
                        username: mixed_username.to_string(),
                        password_secret_ref: None,
                        passphrase_secret_ref: None,
                        identity_ref: Some("no-profile-key-for-password-jump".to_string()),
                        host_key_policy: Some(password_jump_policy),
                    },
                    portmate_core::JumpHop {
                        host: "127.0.0.1".to_string(),
                        port: public_key_jump_port,
                        username: openssh_username,
                        password_secret_ref: None,
                        passphrase_secret_ref: None,
                        identity_ref: Some("mixed-public-jump-key".to_string()),
                        host_key_policy: Some(public_key_jump_policy),
                    },
                ];
            }
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

            let scan = scan_ssh_host_key_inner(&state, profile.clone(), Some(mixed_secret), None)
                .await
                .unwrap();
            assert_eq!(scan.label.as_deref(), Some("目标 SSH"));
            counters.password_successes.store(0, Ordering::SeqCst);
            counters
                .keyboard_interactive_successes
                .store(0, Ordering::SeqCst);

            let mut password_profile = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut password_profile.connection {
                ssh.identity_policy.auth_order = vec![AuthMethod::Password, AuthMethod::PublicKey];
            }
            let error = open_ssh_session(
                &state,
                password_profile.clone(),
                Some("wrong mixed auth secret".to_string()),
                None,
            )
            .await
            .unwrap_err();
            assert!(error.contains("Jump Host 第 1 跳认证失败"), "{error}");
            assert!(error.contains("password"), "{error}");
            assert!(state.store.lock().unwrap().host_keys.keys.is_empty());

            let connected = open_ssh_session(
                &state,
                password_profile,
                Some(mixed_secret.to_string()),
                None,
            )
            .await
            .unwrap();
            assert_eq!(connected.runtime.status, SessionStatus::Connected);
            assert_eq!(counters.password_successes.load(Ordering::SeqCst), 1);
            assert_eq!(
                counters
                    .keyboard_interactive_successes
                    .load(Ordering::SeqCst),
                0
            );
            assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 3);
            close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();

            let mut keyboard_profile = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut keyboard_profile.connection {
                ssh.identity_policy.auth_order =
                    vec![AuthMethod::KeyboardInteractive, AuthMethod::PublicKey];
            }
            let connected = open_ssh_session(
                &state,
                keyboard_profile,
                Some(mixed_secret.to_string()),
                None,
            )
            .await
            .unwrap();
            assert_eq!(connected.runtime.status, SessionStatus::Connected);
            assert_eq!(counters.password_successes.load(Ordering::SeqCst), 1);
            assert_eq!(
                counters
                    .keyboard_interactive_successes
                    .load(Ordering::SeqCst),
                1
            );
            close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();

            assert_eq!(proxy_connections.load(Ordering::SeqCst), 4);
            proxy_task.abort();
            let _ = proxy_task.await;
            password_jump_task.abort();
            let _ = password_jump_task.await;
        });

        public_key_jump_sshd.stop();
        target_sshd.stop();
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn openssh_identity_order_respects_max_auth_tries() {
        let _runtime_guard = shared_runtime_test_guard();
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
    fn openssh_agent_policy_and_identity_filtering_end_to_end() {
        let _runtime_guard = shared_runtime_test_guard();
        let Some(sshd_path) = openssh_test_server_path() else {
            eprintln!("skipping OpenSSH agent test: sshd is not installed");
            return;
        };
        let client_tools_available = Command::new("sh")
            .args([
                "-c",
                "command -v ssh-agent >/dev/null 2>&1 && command -v ssh-add >/dev/null 2>&1",
            ])
            .status()
            .is_ok_and(|status| status.success());
        if Command::new("ssh-keygen").arg("-V").output().is_err() || !client_tools_available {
            eprintln!("skipping OpenSSH agent test: OpenSSH client tools are not installed");
            return;
        }

        let root = std::env::temp_dir().join(format!("portmate-agent-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let host_key = root.join("ssh_host_ed25519_key");
        let accepted_key = root.join("accepted_agent_ed25519_key");
        let rejected_key = root.join("rejected_agent_ed25519_key");
        for key_path in [&host_key, &accepted_key, &rejected_key] {
            generate_ed25519_test_key(key_path);
        }
        let authorized_keys = root.join("authorized_keys");
        fs::copy(accepted_key.with_extension("pub"), &authorized_keys).unwrap();

        let port = {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            listener.local_addr().unwrap().port()
        };
        let config_path = root.join("sshd_config");
        write_openssh_test_config(
            &config_path,
            &host_key,
            &root.join("sshd.pid"),
            &authorized_keys,
            port,
        );
        let mut sshd = spawn_openssh_test_server(sshd_path, &config_path);
        let agent_socket = root.join("agent.sock");
        let mut agent = spawn_openssh_test_agent(&agent_socket);

        tauri::async_runtime::block_on(async {
            wait_for_openssh_test_server(&mut sshd, port, "agent-policy sshd").await;
            wait_for_openssh_test_agent(&mut agent, &agent_socket, "agent-policy ssh-agent").await;
            for key_path in [&rejected_key, &accepted_key] {
                let status = Command::new("ssh-add")
                    .arg(key_path)
                    .env("SSH_AUTH_SOCK", &agent_socket)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap();
                assert!(
                    status.success(),
                    "ssh-add failed for {}",
                    key_path.display()
                );
            }

            let identities = list_ssh_agent_identities_on_thread(Some(agent_socket.clone()))
                .await
                .unwrap();
            assert_eq!(identities.len(), 2);
            let accepted_public_key = fs::read_to_string(accepted_key.with_extension("pub"))
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            let accepted_fingerprint =
                compute_ssh_sha256_fingerprint(&accepted_public_key).unwrap();
            let accepted_comment = identities
                .iter()
                .find(|identity| {
                    compute_ssh_sha256_fingerprint(&identity.public_key().public_key_base64())
                        .ok()
                        .as_deref()
                        == Some(accepted_fingerprint.as_str())
                })
                .unwrap()
                .comment()
                .to_string();

            let mut profile = test_ssh_profile();
            if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
                ssh.endpoint.host = "127.0.0.1".to_string();
                ssh.endpoint.port = port;
                ssh.username = openssh_test_username();
                ssh.reconnect = false;
                ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
                ssh.host_key_policy.alias = Some("agent-policy-target".to_string());
                ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
                ssh.identity_policy.identities_only = false;
                ssh.identity_refs.clear();
                ssh.agent_policy.enabled = true;
                ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::AfterProfileKeys;
            }
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            let missing_socket = root.join("missing-agent.sock");

            let mut disabled = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut disabled.connection {
                ssh.agent_policy.enabled = false;
                ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
            }
            let error = establish_ssh_runtime_with_timeout(
                &state,
                &disabled,
                None,
                None,
                SSH_CONNECT_TIMEOUT,
                Some(missing_socket.clone()),
            )
            .await
            .err()
            .expect("disabled ssh-agent policy unexpectedly authenticated");
            assert!(error.contains("没有可尝试的认证方式"), "{error}");
            assert!(!error.contains("无法连接 SSH agent socket"), "{error}");

            let mut identities_only_without_refs = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut identities_only_without_refs.connection {
                ssh.identity_policy.identities_only = true;
            }
            let error = establish_ssh_runtime_with_timeout(
                &state,
                &identities_only_without_refs,
                None,
                None,
                SSH_CONNECT_TIMEOUT,
                Some(missing_socket),
            )
            .await
            .err()
            .expect("IdentitiesOnly without agent refs unexpectedly authenticated");
            assert!(error.contains("IdentitiesOnly"), "{error}");
            assert!(!error.contains("无法连接 SSH agent socket"), "{error}");

            let unfiltered = establish_ssh_runtime_with_timeout(
                &state,
                &profile,
                None,
                None,
                SSH_CONNECT_TIMEOUT,
                Some(agent_socket.clone()),
            )
            .await
            .unwrap();
            assert_eq!(unfiltered.auth_method, AuthMethod::PublicKey);
            disconnect_ssh_runtime(unfiltered.runtime, "PortMate agent test").await;

            let matching_ref = IdentityRef {
                id: "accepted-agent-key".to_string(),
                label: accepted_comment.clone(),
                source: IdentitySource::Agent,
                fingerprint_sha256: Some(accepted_fingerprint),
                path: Some(accepted_comment.clone()),
                secret_ref: None,
            };
            let mut filtered = profile.clone();
            if let ConnectionConfig::Ssh(ssh) = &mut filtered.connection {
                ssh.identity_policy.identities_only = true;
                ssh.identity_refs = vec![matching_ref.clone()];
            }
            let filtered_runtime = establish_ssh_runtime_with_timeout(
                &state,
                &filtered,
                None,
                None,
                SSH_CONNECT_TIMEOUT,
                Some(agent_socket.clone()),
            )
            .await
            .unwrap();
            assert_eq!(filtered_runtime.auth_method, AuthMethod::PublicKey);
            disconnect_ssh_runtime(filtered_runtime.runtime, "PortMate agent filter test").await;

            let mut mismatched = filtered;
            if let ConnectionConfig::Ssh(ssh) = &mut mismatched.connection {
                ssh.identity_refs[0].fingerprint_sha256 =
                    Some("SHA256:deliberately-wrong-fingerprint".to_string());
            }
            let error = establish_ssh_runtime_with_timeout(
                &state,
                &mismatched,
                None,
                None,
                SSH_CONNECT_TIMEOUT,
                Some(agent_socket.clone()),
            )
            .await
            .err()
            .expect("mismatched agent fingerprint was bypassed by its comment");
            assert!(error.contains("IdentitiesOnly"), "{error}");
            assert!(error.contains("agent(after-profile-keys)"), "{error}");
        });

        agent.stop();
        sshd.stop();
        let _ = fs::remove_dir_all(root);
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

    #[test]
    fn keyring_initialization_is_persistent_only_and_retries_transient_failures() {
        let initialized = Mutex::new(false);
        let attempts = std::cell::Cell::new(0_u32);
        let first = ensure_keyring_store_with(&initialized, || {
            attempts.set(attempts.get() + 1);
            Err("secret service offline".to_string())
        });
        assert_eq!(first.unwrap_err(), "secret service offline");
        assert!(!*initialized.lock().unwrap());

        ensure_keyring_store_with(&initialized, || {
            attempts.set(attempts.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(attempts.get(), 2);
        ensure_keyring_store_with(&initialized, || {
            panic!("successful initialization must be cached")
        })
        .unwrap();

        let selectors = std::cell::RefCell::new(Vec::new());
        let error = initialize_persistent_native_keyring_with(|not_keyutils| {
            selectors.borrow_mut().push(not_keyutils);
            Err("persistent store unavailable".to_string())
        })
        .unwrap_err();
        assert_eq!(error, "persistent store unavailable");
        assert_eq!(selectors.into_inner(), vec![true]);
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

    #[test]
    fn client_identity_validation_enforces_immutable_id_and_source_fields() {
        let immutable_error = normalize_client_identity(
            "identity-a",
            IdentityRef {
                id: "identity-b".to_string(),
                label: "Key".to_string(),
                source: IdentitySource::Agent,
                fingerprint_sha256: None,
                path: None,
                secret_ref: None,
            },
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(immutable_error.contains("不可修改"));

        let path_error = normalize_client_identity(
            "identity-a",
            IdentityRef {
                id: "identity-a".to_string(),
                label: "Key".to_string(),
                source: IdentitySource::SystemFile,
                fingerprint_sha256: None,
                path: Some("  ".to_string()),
                secret_ref: Some("keychain:ignored".to_string()),
            },
            |_| Ok(()),
        )
        .unwrap_err();
        assert!(path_error.contains("私钥路径"));

        let vault_error = normalize_client_identity(
            "identity-a",
            vault_identity("identity-a", "keychain:missing"),
            |_| Err("secret unavailable".to_string()),
        )
        .unwrap_err();
        assert_eq!(vault_error, "secret unavailable");

        let agent = normalize_client_identity(
            "identity-a",
            IdentityRef {
                id: "identity-a".to_string(),
                label: "  Agent Key  ".to_string(),
                source: IdentitySource::Agent,
                fingerprint_sha256: Some("  SHA256:agent  ".to_string()),
                path: Some(" socket comment ".to_string()),
                secret_ref: Some("keychain:must-clear".to_string()),
            },
            |_| Ok(()),
        )
        .unwrap();
        assert_eq!(agent.label, "Agent Key");
        assert_eq!(agent.fingerprint_sha256.as_deref(), Some("SHA256:agent"));
        assert!(agent.secret_ref.is_none());
    }

    #[test]
    fn concurrent_client_identity_updates_merge_fields_and_preserve_new_secrets() {
        let mut expected = vault_identity("identity-a", "keychain:old");
        expected.label = "Original label".to_string();
        expected.fingerprint_sha256 = Some("SHA256:original".to_string());

        let mut current = expected.clone();
        current.label = "Current label".to_string();
        current.secret_ref = Some("keychain:rotated".to_string());

        let mut incoming = expected.clone();
        incoming.fingerprint_sha256 = Some("SHA256:incoming".to_string());

        let merged =
            merge_expected_client_identity_update(&current, &expected, incoming.clone()).unwrap();
        assert_eq!(merged.label, "Current label");
        assert_eq!(
            merged.fingerprint_sha256.as_deref(),
            Some("SHA256:incoming")
        );
        assert_eq!(merged.secret_ref.as_deref(), Some("keychain:rotated"));

        let mut conflicting = incoming;
        conflicting.label = "Incoming label".to_string();
        let error =
            merge_expected_client_identity_update(&current, &expected, conflicting).unwrap_err();
        assert!(error.contains("Client Identity 字段"), "{error}");
        assert!(!error.contains("Profile 字段"), "{error}");
        assert!(error.contains("identity.label"), "{error}");
        assert!(!error.contains("Current label"), "{error}");
        assert!(!error.contains("Incoming label"), "{error}");

        let mut wrong_expected = expected.clone();
        wrong_expected.id = "identity-b".to_string();
        assert!(
            merge_expected_client_identity_update(&current, &wrong_expected, expected)
                .unwrap_err()
                .contains("不是同一个 identity")
        );
    }

    #[test]
    fn rotating_shared_identity_keeps_the_old_secret_for_other_profiles() {
        let mut first = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut first.connection {
            ssh.identity_refs = vec![vault_identity("shared-key", "keychain:shared")];
        }
        let mut second = first.clone();
        second.id = "ssh-session-2".to_string();
        second.name = "Bench SSH 2".to_string();
        let mut store = SessionStore::default();
        store.upsert_profile(first);
        store.upsert_profile(second);

        let (summary, old_secret_ref) = replace_client_identity(
            &mut store,
            "ssh-session-1",
            "shared-key",
            vault_identity("shared-key", "keychain:rotated"),
        )
        .unwrap();
        let delete_called = std::cell::Cell::new(false);
        let response = client_identity_mutation_response(
            &store,
            summary,
            old_secret_ref.as_deref(),
            true,
            |_| {
                delete_called.set(true);
                Ok(())
            },
        );
        assert!(response.old_secret_shared);
        assert!(!response.old_secret_deleted);
        assert!(!delete_called.get());
        assert_eq!(secret_ref_usage_count(&store, "keychain:shared"), 1);
        assert_eq!(secret_ref_usage_count(&store, "keychain:rotated"), 1);
    }

    #[test]
    fn failed_orphan_cleanup_keeps_the_persisted_identity_valid() {
        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.identity_refs = vec![vault_identity("vault-key", "keychain:old")];
        }
        let mut store = SessionStore::default();
        store.upsert_profile(profile);
        let (summary, old_secret_ref) = replace_client_identity(
            &mut store,
            "ssh-session-1",
            "vault-key",
            vault_identity("vault-key", "keychain:new"),
        )
        .unwrap();
        let response = client_identity_mutation_response(
            &store,
            summary,
            old_secret_ref.as_deref(),
            true,
            |_| Err("keyring locked".to_string()),
        );
        assert!(!response.old_secret_deleted);
        assert!(!response.old_secret_shared);
        assert!(response
            .cleanup_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("keyring locked")));
        let saved = find_client_identity(&store, "ssh-session-1", "vault-key").unwrap();
        assert_eq!(saved.secret_ref.as_deref(), Some("keychain:new"));
    }

    #[test]
    fn deleting_jump_identity_is_blocked_and_duplicate_ids_are_rejected() {
        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.identity_refs = vec![vault_identity("jump-key", "keychain:jump")];
            ssh.jumps.push(portmate_core::JumpHop {
                host: "bastion.example".to_string(),
                port: 22,
                username: "root".to_string(),
                password_secret_ref: None,
                passphrase_secret_ref: None,
                identity_ref: Some("jump-key".to_string()),
                host_key_policy: None,
            });
        }
        let mut store = SessionStore::default();
        store.upsert_profile(profile);
        assert!(
            remove_client_identity(&mut store, "ssh-session-1", "jump-key")
                .unwrap_err()
                .contains("Jump Host")
        );

        if let ConnectionConfig::Ssh(ssh) = &mut store.profiles[0].connection {
            ssh.jumps.clear();
            ssh.identity_refs
                .push(vault_identity("jump-key", "keychain:duplicate"));
        }
        assert!(find_client_identity(&store, "ssh-session-1", "jump-key")
            .unwrap_err()
            .contains("重复"));
    }

    #[test]
    fn secret_usage_counts_target_jump_and_identity_credentials() {
        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.password_secret_ref = Some(" keychain:shared ".to_string());
            ssh.passphrase_secret_ref = Some("keychain:shared".to_string());
            ssh.identity_refs = vec![vault_identity("vault-key", "keychain:shared")];
            ssh.jumps.push(portmate_core::JumpHop {
                host: "bastion.example".to_string(),
                port: 22,
                username: "root".to_string(),
                password_secret_ref: Some("keychain:shared".to_string()),
                passphrase_secret_ref: Some("keychain:shared".to_string()),
                identity_ref: None,
                host_key_policy: None,
            });
        }
        let mut store = SessionStore::default();
        store.upsert_profile(profile);
        assert_eq!(secret_ref_usage_count(&store, "keychain:shared"), 5);
    }

    #[test]
    fn one_key_identity_updates_clone_only_bound_authenticating_identities() {
        let mut profile = test_ssh_profile();
        let selected_identity = vault_identity("vault-key", "keychain:vault-key");
        let public_key_only = IdentityRef {
            id: "public-key".to_string(),
            label: "Public key".to_string(),
            source: IdentitySource::PublicKeyOnly,
            fingerprint_sha256: Some("SHA256:public".to_string()),
            path: Some("/tmp/public-key.pub".to_string()),
            secret_ref: None,
        };
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.identity_refs = vec![selected_identity.clone(), public_key_only];
        }
        let mut store = SessionStore::default();
        store.upsert_profile(profile);
        let sessions = vec!["ssh-session-1".to_string()];

        let selected = apply_one_key_identity_update(
            &store,
            OneKeyKind::Ssh,
            &sessions,
            None,
            OneKeyIdentityUpdate::Set {
                source_profile_id: "ssh-session-1".to_string(),
                identity_id: "vault-key".to_string(),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(selected.source_profile_id, "ssh-session-1");
        assert_eq!(selected.identity, selected_identity);

        assert!(apply_one_key_identity_update(
            &store,
            OneKeyKind::Ssh,
            &["other-session".to_string()],
            None,
            OneKeyIdentityUpdate::Set {
                source_profile_id: "ssh-session-1".to_string(),
                identity_id: "vault-key".to_string(),
            },
        )
        .unwrap_err()
        .contains("已绑定"));
        assert!(apply_one_key_identity_update(
            &store,
            OneKeyKind::Ssh,
            &sessions,
            None,
            OneKeyIdentityUpdate::Set {
                source_profile_id: "ssh-session-1".to_string(),
                identity_id: "public-key".to_string(),
            },
        )
        .unwrap_err()
        .contains("私钥"));

        assert!(apply_one_key_identity_update(
            &store,
            OneKeyKind::Ssh,
            &["other-session".to_string()],
            Some(selected),
            OneKeyIdentityUpdate::Preserve,
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn one_key_prompt_completion_revalidates_field_username_and_event_freshness() {
        let legacy_request: SendOneKeyRequest = serde_json::from_value(serde_json::json!({
            "id": "onekey:legacy",
            "sessionId": "ssh-session-1",
            "field": "username"
        }))
        .unwrap();
        assert_eq!(legacy_request.source, OneKeySendSource::Manual);
        assert!(legacy_request.prompt_event_id.is_none());

        let mut store = SessionStore::default();
        store.upsert_profile(test_ssh_profile());
        let now = Utc::now();
        let one_key = OneKeyCredential {
            id: "onekey:prompt".to_string(),
            label: "Prompt login".to_string(),
            kind: OneKeyKind::Account,
            username: "operator".to_string(),
            password_secret_ref: Some("keychain:prompt-password".to_string()),
            passphrase_secret_ref: None,
            identity: None,
            session_ids: vec!["ssh-session-1".to_string()],
            created_at: now,
            updated_at: now,
        };
        store
            .record_stream_event(
                "ssh-session-1",
                EventDirection::Inbound,
                EventStream::Stdout,
                "\x1b[33mPass",
            )
            .unwrap();
        let prompt = store
            .record_stream_event(
                "ssh-session-1",
                EventDirection::Inbound,
                EventStream::Stdout,
                "word for operator:\x1b[0m",
            )
            .unwrap();
        store.record_system_event("ssh-session-1", "PortMate: diagnostic");

        validate_one_key_prompt_completion(
            &store,
            &one_key,
            "ssh-session-1",
            OneKeyField::Password,
            &prompt.id,
        )
        .unwrap();
        assert!(validate_one_key_prompt_completion(
            &store,
            &one_key,
            "ssh-session-1",
            OneKeyField::Username,
            &prompt.id,
        )
        .unwrap_err()
        .contains("字段"));

        let mut wrong_username = one_key.clone();
        wrong_username.username = "root".to_string();
        assert!(validate_one_key_prompt_completion(
            &store,
            &wrong_username,
            "ssh-session-1",
            OneKeyField::Password,
            &prompt.id,
        )
        .unwrap_err()
        .contains("用户名"));

        store
            .record_event(
                "ssh-session-1",
                EventDirection::Outbound,
                EventStream::Control,
                None,
                None,
                BTreeMap::new(),
            )
            .unwrap();
        assert!(validate_one_key_prompt_completion(
            &store,
            &one_key,
            "ssh-session-1",
            OneKeyField::Password,
            &prompt.id,
        )
        .unwrap_err()
        .contains("已变化"));

        assert_eq!(
            detect_one_key_terminal_prompt("root@router's password:"),
            Some(DetectedOneKeyPrompt::Password {
                username_hint: Some("root".to_string()),
            })
        );
        assert_eq!(
            detect_one_key_terminal_prompt("device login:"),
            Some(DetectedOneKeyPrompt::Username)
        );
        assert!(detect_one_key_terminal_prompt("Password:\r\n").is_none());
        assert!(detect_one_key_terminal_prompt("New password:").is_none());
        assert!(detect_one_key_terminal_prompt("Retype new password:").is_none());
    }

    #[test]
    fn one_key_summaries_hide_refs_and_count_secret_usage() {
        let mut store = SessionStore::default();
        let now = Utc::now();
        store.one_keys.push(OneKeyCredential {
            id: "onekey:test".to_string(),
            label: "Lab account".to_string(),
            kind: OneKeyKind::Ssh,
            username: "operator".to_string(),
            password_secret_ref: Some("keychain:onekey-password".to_string()),
            passphrase_secret_ref: Some("keychain:onekey-passphrase".to_string()),
            identity: Some(OneKeyIdentity {
                source_profile_id: "ssh-session-1".to_string(),
                identity: vault_identity("onekey-key", "keychain:onekey-identity"),
            }),
            session_ids: vec!["ssh-session-1".to_string()],
            created_at: now,
            updated_at: now,
        });

        assert_eq!(
            secret_ref_usage_count(&store, "keychain:onekey-password"),
            1
        );
        assert_eq!(
            secret_ref_usage_count(&store, "keychain:onekey-passphrase"),
            1
        );
        assert_eq!(
            secret_ref_usage_count(&store, "keychain:onekey-identity"),
            1
        );
        let summaries = one_key_summaries(&store);
        assert!(summaries[0].has_password);
        assert!(summaries[0].has_passphrase);
        assert_eq!(
            summaries[0]
                .identity
                .as_ref()
                .map(|identity| identity.id.as_str()),
            Some("onekey-key")
        );
        let json = serde_json::to_string(&summaries).unwrap();
        assert!(!json.contains("onekey-password"));
        assert!(!json.contains("onekey-passphrase"));
        assert!(!json.contains("onekey-identity"));
    }

    #[test]
    fn one_key_login_resolves_only_bound_ssh_credentials() {
        let mut store = SessionStore::default();
        let mut profile = test_ssh_profile();
        let selected_identity = vault_identity("login-key", "keychain:login-identity");
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.identity_refs.push(selected_identity.clone());
        }
        store.upsert_profile(profile);
        let now = Utc::now();
        store.one_keys.push(OneKeyCredential {
            id: "onekey:login".to_string(),
            label: "Operations".to_string(),
            kind: OneKeyKind::Ssh,
            username: "operator".to_string(),
            password_secret_ref: Some(" keychain:login-password ".to_string()),
            passphrase_secret_ref: Some("stronghold:login-passphrase".to_string()),
            identity: Some(OneKeyIdentity {
                source_profile_id: "ssh-session-1".to_string(),
                identity: selected_identity.clone(),
            }),
            session_ids: vec!["ssh-session-1".to_string()],
            created_at: now,
            updated_at: now,
        });

        let mut reads = Vec::new();
        let resolved = resolve_one_key_login_credentials_with(
            &store,
            "ssh-session-1",
            "onekey:login",
            |secret_ref| {
                reads.push(secret_ref.to_string());
                Ok(match secret_ref {
                    "keychain:login-password" => "login-secret",
                    "stronghold:login-passphrase" => "key-secret",
                    _ => panic!("unexpected OneKey Secret reference"),
                }
                .to_string())
            },
        )
        .unwrap();
        assert_eq!(
            resolved,
            OneKeyLoginCredentials {
                username: "operator".to_string(),
                password: Some("login-secret".to_string()),
                passphrase: Some("key-secret".to_string()),
                identity: Some(selected_identity.clone()),
            }
        );
        assert_eq!(
            reads,
            [
                "keychain:login-password".to_string(),
                "stronghold:login-passphrase".to_string(),
            ]
        );

        let mut runtime_profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut runtime_profile.connection {
            ssh.password_secret_ref = Some("keychain:profile-password".to_string());
            ssh.passphrase_secret_ref = Some("keychain:profile-passphrase".to_string());
        }
        apply_session_open_profile_credentials(
            &mut runtime_profile,
            Some("operator"),
            Some(&selected_identity),
            true,
        )
        .unwrap();
        let runtime_ssh = ssh_connection(&runtime_profile).unwrap();
        assert_eq!(runtime_ssh.username, "operator");
        assert!(runtime_ssh.password_secret_ref.is_none());
        assert!(runtime_ssh.passphrase_secret_ref.is_none());
        assert_eq!(
            runtime_ssh.identity_refs.as_slice(),
            std::slice::from_ref(&selected_identity)
        );
        assert!(runtime_ssh.identity_policy.identities_only);
        assert!(runtime_ssh
            .identity_policy
            .auth_order
            .contains(&AuthMethod::PublicKey));

        store.one_keys[0].password_secret_ref = None;
        store.one_keys[0].passphrase_secret_ref = None;
        assert_eq!(
            resolve_one_key_login_credentials_with(
                &store,
                "ssh-session-1",
                "onekey:login",
                |_| panic!("identity-only OneKey must not read Secret data"),
            )
            .unwrap(),
            OneKeyLoginCredentials {
                username: "operator".to_string(),
                password: None,
                passphrase: None,
                identity: Some(selected_identity),
            }
        );

        store.one_keys[0].session_ids = vec!["another-session".to_string()];
        assert!(resolve_one_key_login_credentials_with(
            &store,
            "ssh-session-1",
            "onekey:login",
            |_| panic!("unbound OneKey must not read Secret data"),
        )
        .unwrap_err()
        .contains("未绑定"));

        store.one_keys[0].session_ids = vec!["ssh-session-1".to_string()];
        store.one_keys[0].kind = OneKeyKind::Account;
        assert!(resolve_one_key_login_credentials_with(
            &store,
            "ssh-session-1",
            "onekey:login",
            |_| panic!("Account OneKey must not read SSH Secret data"),
        )
        .unwrap_err()
        .contains("SSH OneKey"));
    }

    #[test]
    fn proxy_secret_usage_counts_all_supported_profile_kinds() {
        fn shared_proxy() -> ProxyConfig {
            ProxyConfig {
                enabled: true,
                username: "proxy-user".to_string(),
                password_secret_ref: Some(" keychain:shared-proxy ".to_string()),
                ..ProxyConfig::default()
            }
        }

        let mut ssh = test_ssh_profile();
        if let ConnectionConfig::Ssh(connection) = &mut ssh.connection {
            connection.proxy = shared_proxy();
        }
        let mut tmux = test_ssh_profile();
        tmux.id = "tmux-session-1".to_string();
        tmux.kind = SessionKind::Tmux;
        let ConnectionConfig::Ssh(mut tmux_connection) = tmux.connection else {
            unreachable!();
        };
        tmux_connection.proxy = shared_proxy();
        tmux.connection = ConnectionConfig::Tmux(tmux_connection);
        let tcp = test_tcp_profile(ConnectionConfig::Tcp(TcpConnection {
            proxy: shared_proxy(),
            ..TcpConnection::default()
        }));
        let mut telnet = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection {
            proxy: shared_proxy(),
            ..TcpConnection::default()
        }));
        telnet.id = "telnet-session-1".to_string();

        let mut store = SessionStore::default();
        for profile in [ssh, tmux, tcp, telnet] {
            store.upsert_profile(profile);
        }
        assert_eq!(secret_ref_usage_count(&store, "keychain:shared-proxy"), 4);
        assert!(store
            .profiles
            .iter()
            .all(|profile| { profile_secret_refs(profile).contains("keychain:shared-proxy") }));
    }

    #[test]
    fn proxy_password_updates_store_only_a_secret_reference() {
        let mut profile = test_tcp_profile(ConnectionConfig::Tcp(TcpConnection {
            proxy: ProxyConfig {
                enabled: true,
                kind: ProxyKind::Socks5,
                username: "proxy-user".to_string(),
                ..ProxyConfig::default()
            },
            ..TcpConnection::default()
        }));
        let written = std::cell::RefCell::new(None::<String>);
        let generated = apply_proxy_password_update_with_io(
            &mut profile,
            Some(ProxyPasswordUpdate::Set {
                password: "private-proxy-password".to_string(),
                storage: None,
            }),
            |storage, password| {
                assert!(storage.is_none());
                written.replace(Some(password.to_string()));
                Ok("keychain:proxy-password".to_string())
            },
        )
        .unwrap();
        assert_eq!(generated.as_deref(), Some("keychain:proxy-password"));
        assert_eq!(written.borrow().as_deref(), Some("private-proxy-password"));
        let proxy = profile_proxy(&profile).unwrap();
        assert_eq!(
            proxy.password_secret_ref.as_deref(),
            Some("keychain:proxy-password")
        );
        let serialized = serde_json::to_string(&profile).unwrap();
        assert!(!serialized.contains("private-proxy-password"));

        let credentials = resolve_proxy_credentials_with(proxy, |secret_ref| {
            assert_eq!(secret_ref, "keychain:proxy-password");
            Ok("private-proxy-password".to_string())
        })
        .unwrap()
        .unwrap();
        assert_eq!(credentials.username, "proxy-user");
        assert_eq!(credentials.password.as_str(), "private-proxy-password");

        apply_proxy_password_update_with_io(
            &mut profile,
            Some(ProxyPasswordUpdate::Clear),
            |_, _| panic!("clearing a proxy password must not write a secret"),
        )
        .unwrap();
        assert!(profile_proxy(&profile)
            .unwrap()
            .password_secret_ref
            .is_none());

        assert!(
            validate_proxy_credentials(ProxyKind::HttpConnect, "bad:user", "password")
                .unwrap_err()
                .contains("冒号")
        );
        assert!(
            validate_proxy_credentials(ProxyKind::Socks5, &"u".repeat(256), "password")
                .unwrap_err()
                .contains("1-255")
        );
        assert!(
            validate_proxy_credentials(ProxyKind::Socks5, "proxy-user", &"p".repeat(256))
                .unwrap_err()
                .contains("1-255")
        );
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

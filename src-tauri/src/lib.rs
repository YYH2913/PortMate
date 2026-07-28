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

use super::*;
use std::process::Command;

const TEST_RUNTIME_TRANSITION_TIMEOUT: Duration = Duration::from_secs(15);

#[path = "app_migration_tests.rs"]
mod app_migration_tests;
#[path = "archive_tests.rs"]
mod archive_tests;
#[path = "command_type_tests.rs"]
mod command_type_tests;
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
#[path = "file_batch_tests.rs"]
mod file_batch_tests;
#[path = "file_operation_tests.rs"]
mod file_operation_tests;
#[path = "host_key_tests.rs"]
mod host_key_tests;
#[path = "identity_tests.rs"]
mod identity_tests;
#[path = "mcp_approval_tests.rs"]
mod mcp_approval_tests;
#[path = "mcp_grant_tests.rs"]
mod mcp_grant_tests;
#[path = "mcp_ipc_tests.rs"]
mod mcp_ipc_tests;
#[path = "mcp_read_tests.rs"]
mod mcp_read_tests;
#[path = "mcp_tests.rs"]
mod mcp_tests;
#[path = "migration_diagnostic_tests.rs"]
mod migration_diagnostic_tests;
#[path = "migration_tests.rs"]
mod migration_tests;
#[path = "modem_protocol_tests.rs"]
mod modem_protocol_tests;
#[path = "modem_runtime_tests.rs"]
mod modem_runtime_tests;
#[path = "openssh_authentication_tests.rs"]
mod openssh_authentication_tests;
#[path = "openssh_jump_host_tests.rs"]
mod openssh_jump_host_tests;
#[path = "openssh_reconnect_tests.rs"]
mod openssh_reconnect_tests;
#[path = "openssh_transfer_tunnel_tests.rs"]
mod openssh_transfer_tunnel_tests;
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
#[path = "shell_runtime_tests.rs"]
mod shell_runtime_tests;
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
#[path = "transfer_io_tests.rs"]
mod transfer_io_tests;
#[path = "transfer_queue_tests.rs"]
mod transfer_queue_tests;
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

async fn spawn_test_socks5_proxy(reply: u8) -> (u16, Arc<AtomicU64>, tokio::task::JoinHandle<()>) {
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
        let authenticated =
            username == expected_username.as_bytes() && password == expected_password.as_bytes();
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

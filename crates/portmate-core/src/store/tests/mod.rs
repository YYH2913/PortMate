use super::*;

mod event_tests;
mod export_tests;
mod history_tests;
mod security_tests;
mod session_tests;

fn test_store() -> SessionStore {
    let now = chrono::Utc::now();
    SessionStore {
        profiles: vec![SessionProfile {
            id: "test-session".to_string(),
            name: "test session".to_string(),
            kind: SessionKind::Shell,
            group: "tests".to_string(),
            tags: Vec::new(),
            connection: ConnectionConfig::Shell(ShellConnection {
                program: "/bin/sh".to_string(),
                args: Vec::new(),
                cwd: None,
            }),
            terminal: TerminalSettings::default(),
            logging: LoggingSettings::default(),
            triggers: Vec::new(),
            transfer: TransferSettings::default(),
        }],
        runtimes: vec![SessionRuntime {
            session_id: "test-session".to_string(),
            pane_id: "test-session:main".to_string(),
            status: SessionStatus::Connected,
            title: "test session".to_string(),
            cwd: None,
            connected_since: Some(now),
            last_activity: now,
            last_disconnect: None,
            last_disconnect_reason: None,
            active_transport: SessionKind::Shell,
        }],
        grants: vec![
            McpGrant {
                client_id: "test-client".to_string(),
                name: "test client".to_string(),
                scopes: vec![McpScope::ReadLogs, McpScope::WriteInput],
                allowed_sessions: vec!["test-session".to_string()],
                confirm_writes: false,
                expires_at: None,
                revoked_at: None,
            },
            McpGrant {
                client_id: "readonly".to_string(),
                name: "readonly".to_string(),
                scopes: vec![McpScope::ReadLogs],
                allowed_sessions: Vec::new(),
                confirm_writes: false,
                expires_at: None,
                revoked_at: None,
            },
        ],
        ..SessionStore::default()
    }
}

fn sensitive_ssh_connection() -> SshConnection {
    SshConnection {
        endpoint: HostEndpoint {
            host: "diagnostic.example".to_string(),
            port: 22,
        },
        username: "operator".to_string(),
        reconnect: true,
        reconnect_delay_ms: DEFAULT_SSH_RECONNECT_DELAY_MS,
        keepalive_enabled: true,
        keepalive_interval_seconds: DEFAULT_SSH_KEEPALIVE_INTERVAL_SECONDS,
        keepalive_max_missed: DEFAULT_SSH_KEEPALIVE_MAX_MISSED,
        tcp_keepalive_enabled: None,
        proxy: ProxyConfig {
            enabled: true,
            password_secret_ref: Some("keyring:proxy-password-ref".to_string()),
            ..ProxyConfig::default()
        },
        password_secret_ref: Some("keyring:target-password-ref".to_string()),
        passphrase_secret_ref: Some("stronghold:target-passphrase-ref".to_string()),
        host_key_policy: HostKeyPolicy::profile_alias("test-session"),
        trusted_host_keys: Vec::new(),
        identity_policy: IdentityPolicy::default(),
        identity_refs: vec![IdentityRef {
            id: "identity-diagnostic-id".to_string(),
            label: "diagnostic identity".to_string(),
            source: IdentitySource::ProfileVault,
            fingerprint_sha256: Some("SHA256:diagnostic-fingerprint".to_string()),
            path: Some("/home/operator/.ssh/private-key".to_string()),
            secret_ref: Some("stronghold:identity-secret-ref".to_string()),
        }],
        agent_policy: AgentPolicy::default(),
        jumps: vec![JumpHop {
            host: "jump.example".to_string(),
            port: 22,
            username: "jump-operator".to_string(),
            password_secret_ref: Some("keyring:jump-password-ref".to_string()),
            passphrase_secret_ref: Some("stronghold:jump-passphrase-ref".to_string()),
            identity_ref: Some("identity-diagnostic-id".to_string()),
            host_key_policy: None,
        }],
        tunnels: Vec::new(),
    }
}

fn test_transfer(id: String, status: TransferStatus) -> TransferTask {
    TransferTask {
        id,
        session_id: "test-session".to_string(),
        protocol: TransferProtocol::Sftp,
        source: "source.bin".to_string(),
        destination: "destination.bin".to_string(),
        bytes_total: 1,
        bytes_done: usize::from(matches!(status, TransferStatus::Completed)) as u64,
        status,
        message: None,
        started_at: None,
        finished_at: None,
        average_bytes_per_second: None,
    }
}

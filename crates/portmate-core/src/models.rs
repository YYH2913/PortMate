use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

mod connection;
mod security;
mod transfer;

pub use connection::*;
pub use security::*;
pub use transfer::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionKind {
    Ssh,
    Serial,
    Shell,
    Telnet,
    Tcp,
    Tmux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionStatus {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Blocked,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventDirection {
    Inbound,
    Outbound,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventStream {
    Stdout,
    Stderr,
    Control,
    Audit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettings {
    pub term: String,
    pub rows: u16,
    pub cols: u16,
    pub scrollback: u32,
    pub font_family: String,
    pub font_size: u8,
    pub theme: String,
    #[serde(default = "default_terminal_background_opacity")]
    pub background_opacity: u8,
}

fn default_terminal_background_opacity() -> u8 {
    100
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            term: "xterm-256color".to_string(),
            rows: 32,
            cols: 120,
            scrollback: 200_000,
            font_family: "\"JetBrains Mono\", \"Noto Sans Mono CJK SC\", \"Sarasa Mono SC\", \"Microsoft YaHei UI\", monospace".to_string(),
            font_size: 13,
            theme: "portmate-dark".to_string(),
            background_opacity: default_terminal_background_opacity(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoggingSettings {
    pub enabled: bool,
    pub raw: bool,
    pub text: bool,
    pub jsonl: bool,
    pub redact_secrets: bool,
    pub path_template: String,
    #[serde(default)]
    pub retention_days: u32,
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            raw: false,
            text: true,
            jsonl: true,
            redact_secrets: true,
            path_template: "{profile}/{date}/{session}.jsonl".to_string(),
            retention_days: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSpec {
    pub id: String,
    pub label: String,
    pub matcher: TriggerMatcher,
    pub actions: Vec<TriggerAction>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum TriggerMatcher {
    Contains { text: String, case_sensitive: bool },
    Regex { pattern: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum TriggerAction {
    Highlight { color: String },
    SendText { text: String },
    LocalCommand { command: String },
    Notification { message: String },
    TimelineMark { label: String },
    CustomLink { url_template: String },
    Sound { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProfile {
    pub id: String,
    pub name: String,
    pub kind: SessionKind,
    pub group: String,
    pub tags: Vec<String>,
    pub connection: ConnectionConfig,
    pub terminal: TerminalSettings,
    pub logging: LoggingSettings,
    pub triggers: Vec<TriggerSpec>,
    pub transfer: TransferSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntime {
    pub session_id: String,
    pub pane_id: String,
    pub status: SessionStatus,
    pub title: String,
    pub cwd: Option<String>,
    pub connected_since: Option<DateTime<Utc>>,
    pub last_activity: DateTime<Utc>,
    #[serde(default)]
    pub last_disconnect: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_disconnect_reason: Option<String>,
    pub active_transport: SessionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub id: String,
    pub session_id: String,
    pub pane_id: String,
    pub ts: DateTime<Utc>,
    pub direction: EventDirection,
    pub stream: EventStream,
    pub bytes_ref: Option<String>,
    pub text: Option<String>,
    pub annotations: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub profile: SessionProfile,
    pub runtime: SessionRuntime,
    pub log_lines: usize,
    pub last_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineMark {
    pub id: String,
    pub session_id: String,
    pub ts: DateTime<Utc>,
    pub label: String,
    pub details: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonProcess {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonDisk {
    pub filesystem: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonNetworkInterface {
    pub name: String,
    #[serde(default)]
    pub addresses: Vec<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_kbps: f32,
    pub tx_kbps: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonSnapshot {
    pub session_id: String,
    pub ts: DateTime<Utc>,
    pub uptime_seconds: u64,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub rx_kbps: f32,
    pub tx_kbps: f32,
    #[serde(default)]
    pub load_average: [f32; 3],
    #[serde(default)]
    pub memory_total_bytes: u64,
    #[serde(default)]
    pub memory_available_bytes: u64,
    #[serde(default)]
    pub processes: Vec<SysmonProcess>,
    #[serde(default)]
    pub disks: Vec<SysmonDisk>,
    #[serde(default)]
    pub network_interfaces: Vec<SysmonNetworkInterface>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandHistoryEntry {
    pub command: String,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpScope {
    ReadSessions,
    ReadLogs,
    WriteInput,
    Transfer,
    Tunnel,
    ManageSessions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpGrant {
    pub client_id: String,
    pub name: String,
    pub scopes: Vec<McpScope>,
    pub allowed_sessions: Vec<String>,
    #[serde(default)]
    pub confirm_writes: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl McpGrant {
    pub fn allows(&self, scope: McpScope, session_id: Option<&str>, now: DateTime<Utc>) -> bool {
        if self.revoked_at.is_some() || self.expires_at.is_some_and(|expires| expires <= now) {
            return false;
        }
        if !self.scopes.contains(&scope) {
            return false;
        }
        match session_id {
            Some(id) => {
                self.allowed_sessions.is_empty() || self.allowed_sessions.iter().any(|s| s == id)
            }
            None => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub session_id: Option<String>,
    pub decision: String,
    pub details: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_key_deserializes_legacy_shape_without_identity() {
        let one_key: OneKeyCredential = serde_json::from_str(
            r#"{
                "id": "onekey:legacy",
                "label": "Legacy SSH",
                "kind": "ssh",
                "username": "operator",
                "passwordSecretRef": "keychain:legacy-password",
                "passphraseSecretRef": null,
                "sessionIds": ["ssh-session-1"],
                "createdAt": "2026-07-15T00:00:00Z",
                "updatedAt": "2026-07-15T00:00:00Z"
            }"#,
        )
        .expect("legacy OneKey should deserialize");

        assert!(one_key.identity.is_none());
        assert_eq!(one_key.session_ids, ["ssh-session-1"]);
    }

    #[test]
    fn jump_hop_deserializes_legacy_shape_with_optional_defaults() {
        let jump: JumpHop = serde_json::from_str(
            r#"{
                "host": "bastion.example",
                "port": 22,
                "username": "deploy",
                "identityRef": "jump-key"
            }"#,
        )
        .expect("legacy jump hop should deserialize");

        assert_eq!(jump.host, "bastion.example");
        assert_eq!(jump.port, 22);
        assert_eq!(jump.username, "deploy");
        assert_eq!(jump.identity_ref.as_deref(), Some("jump-key"));
        assert!(jump.password_secret_ref.is_none());
        assert!(jump.passphrase_secret_ref.is_none());
        assert!(jump.host_key_policy.is_none());
    }

    #[test]
    fn logging_settings_deserialize_without_legacy_retention_field() {
        let logging: LoggingSettings = serde_json::from_str(
            r#"{
                "enabled": true,
                "raw": false,
                "text": true,
                "jsonl": true,
                "redactSecrets": true,
                "pathTemplate": "{profile}/{date}/{session}.jsonl"
            }"#,
        )
        .expect("legacy logging settings should deserialize");

        assert_eq!(logging.retention_days, 0);
    }

    #[test]
    fn terminal_settings_deserialize_legacy_background_as_opaque() {
        let terminal: TerminalSettings = serde_json::from_str(
            r#"{
                "term": "xterm-256color",
                "rows": 32,
                "cols": 120,
                "scrollback": 200000,
                "fontFamily": "monospace",
                "fontSize": 13,
                "theme": "portmate-dark"
            }"#,
        )
        .expect("legacy terminal settings should deserialize");

        assert_eq!(terminal.background_opacity, 100);
    }

    #[test]
    fn logging_defaults_do_not_capture_unredacted_raw_bytes() {
        let logging = LoggingSettings::default();
        assert!(!logging.enabled);
        assert!(!logging.raw);
        assert!(logging.redact_secrets);
    }

    #[test]
    fn sysmon_snapshot_deserializes_legacy_summary_without_details() {
        let snapshot: SysmonSnapshot = serde_json::from_str(
            r#"{
                "sessionId": "legacy-session",
                "ts": "2026-07-14T10:00:00Z",
                "uptimeSeconds": 42,
                "cpuPercent": 12.5,
                "memoryPercent": 33.0,
                "rxKbps": 4.0,
                "txKbps": 5.0
            }"#,
        )
        .expect("legacy Sysmon summary should deserialize");

        assert_eq!(snapshot.load_average, [0.0; 3]);
        assert_eq!(snapshot.memory_total_bytes, 0);
        assert_eq!(snapshot.memory_available_bytes, 0);
        assert!(snapshot.processes.is_empty());
        assert!(snapshot.disks.is_empty());
        assert!(snapshot.network_interfaces.is_empty());
    }

    #[test]
    fn ssh_connection_deserializes_legacy_health_defaults_and_clamps_values() {
        let mut legacy: SshConnection = serde_json::from_str(
            r#"{
                "endpoint": {"host": "device.example", "port": 22},
                "username": "root",
                "reconnect": true,
                "hostKeyPolicy": {
                    "mode": "strict",
                    "alias": "legacy-device",
                    "trustScope": "profile",
                    "allowRotation": false,
                    "checkIp": false
                },
                "trustedHostKeys": [],
                "identityPolicy": {
                    "identitiesOnly": true,
                    "authOrder": ["public-key", "keyboard-interactive", "password"],
                    "recordSuccess": true,
                    "lastSuccessful": null
                },
                "identityRefs": [],
                "agentPolicy": {
                    "enabled": false,
                    "forwarding": false,
                    "offerMode": "after-profile-keys"
                },
                "jumps": [],
                "tunnels": []
            }"#,
        )
        .expect("legacy SSH connection should deserialize");

        assert_eq!(legacy.reconnect_delay_ms, DEFAULT_SSH_RECONNECT_DELAY_MS);
        assert!(legacy.keepalive_enabled);
        assert_eq!(
            legacy.keepalive_interval_seconds,
            DEFAULT_SSH_KEEPALIVE_INTERVAL_SECONDS
        );
        assert_eq!(
            legacy.keepalive_max_missed,
            DEFAULT_SSH_KEEPALIVE_MAX_MISSED
        );
        assert_eq!(legacy.tcp_keepalive_enabled, None);
        legacy.reconnect_delay_ms = 0;
        legacy.keepalive_interval_seconds = 0;
        legacy.keepalive_max_missed = u32::MAX;
        legacy.normalize_health_settings();
        assert_eq!(legacy.reconnect_delay_ms, MIN_SSH_RECONNECT_DELAY_MS);
        assert_eq!(
            legacy.keepalive_interval_seconds,
            MIN_SSH_KEEPALIVE_INTERVAL_SECONDS
        );
        assert_eq!(legacy.keepalive_max_missed, MAX_SSH_KEEPALIVE_MAX_MISSED);

        legacy.keepalive_max_missed = 0;
        legacy.normalize_health_settings();
        assert_eq!(legacy.keepalive_max_missed, 0);
    }

    #[test]
    fn serial_connection_deserializes_legacy_health_defaults_and_clamps_values() {
        let mut legacy: SerialConnection = serde_json::from_str(
            r#"{
                "port": "/dev/ttyUSB0",
                "baudRate": 115200,
                "dataBits": 8,
                "stopBits": 1,
                "parity": "none",
                "flowControl": "none",
                "dtr": false,
                "rts": false
            }"#,
        )
        .expect("legacy serial connection should deserialize");

        assert!(legacy.reconnect);
        assert_eq!(legacy.reconnect_delay_ms, DEFAULT_SERIAL_RECONNECT_DELAY_MS);
        assert!(!legacy.receive_idle_timeout_enabled);
        assert_eq!(
            legacy.receive_idle_timeout_seconds,
            DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS
        );
        legacy.reconnect_delay_ms = 0;
        legacy.receive_idle_timeout_seconds = u64::MAX;
        legacy.normalize_health_settings();
        assert_eq!(legacy.reconnect_delay_ms, MIN_SERIAL_RECONNECT_DELAY_MS);
        assert_eq!(
            legacy.receive_idle_timeout_seconds,
            MAX_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn tcp_connection_deserializes_legacy_health_defaults_and_clamps_values() {
        let legacy: TcpConnection = serde_json::from_str(
            r#"{
                "host": "console.example",
                "port": 23,
                "reconnect": true
            }"#,
        )
        .expect("legacy TCP connection should deserialize");
        assert_eq!(legacy.reconnect_delay_ms, DEFAULT_TCP_RECONNECT_DELAY_MS);
        assert!(!legacy.proxy.enabled);
        assert_eq!(legacy.proxy.kind, ProxyKind::Socks5);
        assert_eq!(legacy.proxy.host, "127.0.0.1");
        assert_eq!(legacy.proxy.port, 1080);
        assert!(legacy.proxy.username.is_empty());
        assert!(legacy.proxy.password_secret_ref.is_none());
        assert!(legacy.keepalive_enabled);
        assert_eq!(
            legacy.keepalive_idle_seconds,
            DEFAULT_TCP_KEEPALIVE_IDLE_SECONDS
        );
        assert_eq!(
            legacy.keepalive_interval_seconds,
            DEFAULT_TCP_KEEPALIVE_INTERVAL_SECONDS
        );
        assert_eq!(legacy.keepalive_retries, DEFAULT_TCP_KEEPALIVE_RETRIES);
        assert!(legacy.telnet_binary);
        assert!(legacy.telnet_naws);
        assert!(!legacy.tls_enabled);
        assert!(legacy.tls_server_name.is_none());
        assert!(!legacy.tls_accept_invalid_cert);

        let disabled: TcpConnection = serde_json::from_str(
            r#"{
                "host": "console.example",
                "port": 23,
                "reconnect": true,
                "telnetBinary": false,
                "telnetNaws": false
            }"#,
        )
        .expect("explicit Telnet feature switches should deserialize");
        assert!(!disabled.telnet_binary);
        assert!(!disabled.telnet_naws);

        let mut tls = TcpConnection {
            tls_enabled: true,
            tls_server_name: Some("  console.example  ".to_string()),
            tls_accept_invalid_cert: true,
            ..TcpConnection::default()
        };
        tls.normalize_health_settings();
        assert_eq!(tls.tls_server_name.as_deref(), Some("console.example"));
        assert!(tls.tls_accept_invalid_cert);

        tls.tls_server_name = Some("bad name".to_string());
        tls.normalize_health_settings();
        assert!(tls.tls_server_name.is_none());

        let mut invalid = TcpConnection {
            reconnect_delay_ms: 0,
            keepalive_idle_seconds: u64::MAX,
            keepalive_interval_seconds: 0,
            keepalive_retries: u32::MAX,
            ..TcpConnection::default()
        };
        invalid.normalize_health_settings();
        assert_eq!(invalid.reconnect_delay_ms, MIN_TCP_RECONNECT_DELAY_MS);
        assert_eq!(
            invalid.keepalive_idle_seconds,
            MAX_TCP_KEEPALIVE_IDLE_SECONDS
        );
        assert_eq!(
            invalid.keepalive_interval_seconds,
            MIN_TCP_KEEPALIVE_INTERVAL_SECONDS
        );
        assert_eq!(invalid.keepalive_retries, MAX_TCP_KEEPALIVE_RETRIES);
    }

    #[test]
    fn proxy_config_normalizes_authentication_metadata_without_a_password_body() {
        let mut proxy = ProxyConfig {
            username: "  proxy-user  ".to_string(),
            password_secret_ref: Some("  keychain:proxy-password  ".to_string()),
            ..ProxyConfig::default()
        };
        proxy.normalize();
        assert_eq!(proxy.username, "proxy-user");
        assert_eq!(
            proxy.password_secret_ref.as_deref(),
            Some("keychain:proxy-password")
        );

        let serialized = serde_json::to_string(&proxy).unwrap();
        assert!(serialized.contains("keychain:proxy-password"));
        assert!(!serialized.contains("plaintext-proxy-password"));

        proxy.password_secret_ref = Some("   ".to_string());
        proxy.normalize();
        assert!(proxy.password_secret_ref.is_none());
    }
}

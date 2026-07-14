use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKeyMode {
    Strict,
    TrustOnFirstUse,
    AskEveryTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKeyScope {
    Profile,
    Project,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPolicy {
    pub mode: HostKeyMode,
    pub alias: Option<String>,
    pub trust_scope: HostKeyScope,
    pub allow_rotation: bool,
    pub check_ip: bool,
}

impl HostKeyPolicy {
    pub fn profile_alias(profile_id: &str) -> Self {
        Self {
            mode: HostKeyMode::Strict,
            alias: Some(profile_id.to_string()),
            trust_scope: HostKeyScope::Profile,
            allow_rotation: false,
            check_ip: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKeyDecision {
    TrustOnce,
    AppendToProfile,
    AppendToProject,
    ReplaceForProfile,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedHostKey {
    pub id: String,
    pub profile_id: Option<String>,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint_sha256: String,
    pub public_key_base64: String,
    pub scope: HostKeyScope,
    pub label: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

impl TrustedHostKey {
    pub fn target_key(&self) -> String {
        format!("{}:{}", self.alias, self.port)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    PublicKey,
    KeyboardInteractive,
    Password,
    GssapiWithMic,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentitySource {
    ProfileVault,
    SystemFile,
    Agent,
    PublicKeyOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRef {
    pub id: String,
    pub label: String,
    pub source: IdentitySource,
    pub fingerprint_sha256: Option<String>,
    pub path: Option<String>,
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityPolicy {
    pub identities_only: bool,
    pub auth_order: Vec<AuthMethod>,
    pub record_success: bool,
    pub last_successful: Option<AuthMethod>,
}

impl Default for IdentityPolicy {
    fn default() -> Self {
        Self {
            identities_only: true,
            auth_order: vec![
                AuthMethod::PublicKey,
                AuthMethod::KeyboardInteractive,
                AuthMethod::Password,
            ],
            record_success: true,
            last_successful: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentOfferMode {
    Disabled,
    AfterProfileKeys,
    BeforeProfileKeys,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicy {
    pub enabled: bool,
    pub forwarding: bool,
    pub offer_mode: AgentOfferMode,
}

impl Default for AgentPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            forwarding: false,
            offer_mode: AgentOfferMode::AfterProfileKeys,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JumpHop {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub password_secret_ref: Option<String>,
    #[serde(default)]
    pub passphrase_secret_ref: Option<String>,
    pub identity_ref: Option<String>,
    #[serde(default)]
    pub host_key_policy: Option<HostKeyPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelSpec {
    pub id: String,
    pub label: String,
    pub mode: TunnelMode,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TunnelMode {
    Local,
    Remote,
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProxyKind {
    HttpConnect,
    #[default]
    Socks5,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub kind: ProxyKind,
    #[serde(default = "default_proxy_host")]
    pub host: String,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
}

fn default_proxy_host() -> String {
    "127.0.0.1".to_string()
}

const fn default_proxy_port() -> u16 {
    1080
}

impl ProxyConfig {
    pub fn normalize(&mut self) {
        self.host = self.host.trim().to_string();
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: ProxyKind::Socks5,
            host: default_proxy_host(),
            port: default_proxy_port(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnection {
    pub endpoint: HostEndpoint,
    pub username: String,
    #[serde(default = "default_true")]
    pub reconnect: bool,
    #[serde(default = "default_true")]
    pub keepalive_enabled: bool,
    #[serde(default = "default_ssh_keepalive_interval_seconds")]
    pub keepalive_interval_seconds: u64,
    #[serde(default = "default_ssh_keepalive_max_missed")]
    pub keepalive_max_missed: u32,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub password_secret_ref: Option<String>,
    #[serde(default)]
    pub passphrase_secret_ref: Option<String>,
    pub host_key_policy: HostKeyPolicy,
    pub trusted_host_keys: Vec<TrustedHostKey>,
    pub identity_policy: IdentityPolicy,
    pub identity_refs: Vec<IdentityRef>,
    pub agent_policy: AgentPolicy,
    pub jumps: Vec<JumpHop>,
    pub tunnels: Vec<TunnelSpec>,
}

pub const MIN_SSH_KEEPALIVE_INTERVAL_SECONDS: u64 = 1;
pub const MAX_SSH_KEEPALIVE_INTERVAL_SECONDS: u64 = 3_600;
pub const DEFAULT_SSH_KEEPALIVE_INTERVAL_SECONDS: u64 = 30;
pub const MIN_SSH_KEEPALIVE_MAX_MISSED: u32 = 1;
pub const MAX_SSH_KEEPALIVE_MAX_MISSED: u32 = 20;
pub const DEFAULT_SSH_KEEPALIVE_MAX_MISSED: u32 = 3;

const fn default_ssh_keepalive_interval_seconds() -> u64 {
    DEFAULT_SSH_KEEPALIVE_INTERVAL_SECONDS
}

const fn default_ssh_keepalive_max_missed() -> u32 {
    DEFAULT_SSH_KEEPALIVE_MAX_MISSED
}

impl SshConnection {
    pub fn normalize_health_settings(&mut self) {
        self.keepalive_interval_seconds = self.keepalive_interval_seconds.clamp(
            MIN_SSH_KEEPALIVE_INTERVAL_SECONDS,
            MAX_SSH_KEEPALIVE_INTERVAL_SECONDS,
        );
        self.keepalive_max_missed = self
            .keepalive_max_missed
            .clamp(MIN_SSH_KEEPALIVE_MAX_MISSED, MAX_SSH_KEEPALIVE_MAX_MISSED);
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialConnection {
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: String,
    pub flow_control: String,
    pub dtr: bool,
    pub rts: bool,
    #[serde(default = "default_true")]
    pub reconnect: bool,
    #[serde(default = "default_serial_reconnect_delay_ms")]
    pub reconnect_delay_ms: u64,
    #[serde(default)]
    pub receive_idle_timeout_enabled: bool,
    #[serde(default = "default_serial_receive_idle_timeout_seconds")]
    pub receive_idle_timeout_seconds: u64,
}

pub const MIN_SERIAL_RECONNECT_DELAY_MS: u64 = 100;
pub const MAX_SERIAL_RECONNECT_DELAY_MS: u64 = 60_000;
pub const DEFAULT_SERIAL_RECONNECT_DELAY_MS: u64 = 1_000;
pub const MIN_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS: u64 = 1;
pub const MAX_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS: u64 = 86_400;
pub const DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS: u64 = 60;

const fn default_serial_reconnect_delay_ms() -> u64 {
    DEFAULT_SERIAL_RECONNECT_DELAY_MS
}

const fn default_serial_receive_idle_timeout_seconds() -> u64 {
    DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS
}

impl SerialConnection {
    pub fn normalize_health_settings(&mut self) {
        self.reconnect_delay_ms = self
            .reconnect_delay_ms
            .clamp(MIN_SERIAL_RECONNECT_DELAY_MS, MAX_SERIAL_RECONNECT_DELAY_MS);
        self.receive_idle_timeout_seconds = self.receive_idle_timeout_seconds.clamp(
            MIN_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS,
            MAX_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS,
        );
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellConnection {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TcpConnection {
    pub host: String,
    pub port: u16,
    #[serde(default = "default_true")]
    pub reconnect: bool,
    #[serde(default)]
    pub proxy: ProxyConfig,
    #[serde(default = "default_tcp_reconnect_delay_ms")]
    pub reconnect_delay_ms: u64,
    #[serde(default = "default_true")]
    pub keepalive_enabled: bool,
    #[serde(default = "default_tcp_keepalive_idle_seconds")]
    pub keepalive_idle_seconds: u64,
    #[serde(default = "default_tcp_keepalive_interval_seconds")]
    pub keepalive_interval_seconds: u64,
    #[serde(default = "default_tcp_keepalive_retries")]
    pub keepalive_retries: u32,
}

pub const MIN_TCP_RECONNECT_DELAY_MS: u64 = 100;
pub const MAX_TCP_RECONNECT_DELAY_MS: u64 = 60_000;
pub const DEFAULT_TCP_RECONNECT_DELAY_MS: u64 = 1_000;
pub const MIN_TCP_KEEPALIVE_IDLE_SECONDS: u64 = 1;
pub const MAX_TCP_KEEPALIVE_IDLE_SECONDS: u64 = 86_400;
pub const DEFAULT_TCP_KEEPALIVE_IDLE_SECONDS: u64 = 30;
pub const MIN_TCP_KEEPALIVE_INTERVAL_SECONDS: u64 = 1;
pub const MAX_TCP_KEEPALIVE_INTERVAL_SECONDS: u64 = 3_600;
pub const DEFAULT_TCP_KEEPALIVE_INTERVAL_SECONDS: u64 = 10;
pub const MIN_TCP_KEEPALIVE_RETRIES: u32 = 1;
pub const MAX_TCP_KEEPALIVE_RETRIES: u32 = 20;
pub const DEFAULT_TCP_KEEPALIVE_RETRIES: u32 = 3;

const fn default_tcp_reconnect_delay_ms() -> u64 {
    DEFAULT_TCP_RECONNECT_DELAY_MS
}

const fn default_tcp_keepalive_idle_seconds() -> u64 {
    DEFAULT_TCP_KEEPALIVE_IDLE_SECONDS
}

const fn default_tcp_keepalive_interval_seconds() -> u64 {
    DEFAULT_TCP_KEEPALIVE_INTERVAL_SECONDS
}

const fn default_tcp_keepalive_retries() -> u32 {
    DEFAULT_TCP_KEEPALIVE_RETRIES
}

impl TcpConnection {
    pub fn normalize_health_settings(&mut self) {
        self.reconnect_delay_ms = self
            .reconnect_delay_ms
            .clamp(MIN_TCP_RECONNECT_DELAY_MS, MAX_TCP_RECONNECT_DELAY_MS);
        self.keepalive_idle_seconds = self.keepalive_idle_seconds.clamp(
            MIN_TCP_KEEPALIVE_IDLE_SECONDS,
            MAX_TCP_KEEPALIVE_IDLE_SECONDS,
        );
        self.keepalive_interval_seconds = self.keepalive_interval_seconds.clamp(
            MIN_TCP_KEEPALIVE_INTERVAL_SECONDS,
            MAX_TCP_KEEPALIVE_INTERVAL_SECONDS,
        );
        self.keepalive_retries = self
            .keepalive_retries
            .clamp(MIN_TCP_KEEPALIVE_RETRIES, MAX_TCP_KEEPALIVE_RETRIES);
    }
}

impl Default for TcpConnection {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 0,
            reconnect: true,
            proxy: ProxyConfig::default(),
            reconnect_delay_ms: DEFAULT_TCP_RECONNECT_DELAY_MS,
            keepalive_enabled: true,
            keepalive_idle_seconds: DEFAULT_TCP_KEEPALIVE_IDLE_SECONDS,
            keepalive_interval_seconds: DEFAULT_TCP_KEEPALIVE_INTERVAL_SECONDS,
            keepalive_retries: DEFAULT_TCP_KEEPALIVE_RETRIES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ConnectionConfig {
    Ssh(SshConnection),
    Serial(SerialConnection),
    Shell(ShellConnection),
    Telnet(TcpConnection),
    Tcp(TcpConnection),
    Tmux(SshConnection),
}

impl ConnectionConfig {
    pub fn kind(&self) -> SessionKind {
        match self {
            Self::Ssh(_) => SessionKind::Ssh,
            Self::Serial(_) => SessionKind::Serial,
            Self::Shell(_) => SessionKind::Shell,
            Self::Telnet(_) => SessionKind::Telnet,
            Self::Tcp(_) => SessionKind::Tcp,
            Self::Tmux(_) => SessionKind::Tmux,
        }
    }
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
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            term: "xterm-256color".to_string(),
            rows: 32,
            cols: 120,
            scrollback: 200_000,
            font_family: "Roboto Mono, JetBrains Mono, monospace".to_string(),
            font_size: 13,
            theme: "portmate-dark".to_string(),
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
pub struct TransferSettings {
    pub sftp: bool,
    pub scp: bool,
    pub xmodem: bool,
    pub ymodem: bool,
    pub zmodem: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_bytes_per_second: Option<u64>,
    pub default_local_dir: Option<String>,
}

impl Default for TransferSettings {
    fn default() -> Self {
        Self {
            sftp: true,
            scp: true,
            xmodem: true,
            ymodem: true,
            zmodem: true,
            rate_limit_bytes_per_second: None,
            default_local_dir: None,
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
pub struct SysmonSnapshot {
    pub session_id: String,
    pub ts: DateTime<Utc>,
    pub uptime_seconds: u64,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub rx_kbps: f32,
    pub tx_kbps: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferProtocol {
    Sftp,
    Scp,
    Xmodem,
    Ymodem,
    Zmodem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferTask {
    pub id: String,
    pub session_id: String,
    pub protocol: TransferProtocol,
    pub source: String,
    pub destination: String,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub status: TransferStatus,
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average_bytes_per_second: Option<f64>,
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
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl McpGrant {
    pub fn allows(&self, scope: McpScope, session_id: Option<&str>, now: DateTime<Utc>) -> bool {
        if self.revoked_at.is_some() || self.expires_at.is_some_and(|expires| expires < now) {
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
    fn logging_defaults_do_not_capture_unredacted_raw_bytes() {
        let logging = LoggingSettings::default();
        assert!(!logging.enabled);
        assert!(!logging.raw);
        assert!(logging.redact_secrets);
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

        assert!(legacy.keepalive_enabled);
        assert_eq!(
            legacy.keepalive_interval_seconds,
            DEFAULT_SSH_KEEPALIVE_INTERVAL_SECONDS
        );
        assert_eq!(
            legacy.keepalive_max_missed,
            DEFAULT_SSH_KEEPALIVE_MAX_MISSED
        );
        legacy.keepalive_interval_seconds = 0;
        legacy.keepalive_max_missed = u32::MAX;
        legacy.normalize_health_settings();
        assert_eq!(
            legacy.keepalive_interval_seconds,
            MIN_SSH_KEEPALIVE_INTERVAL_SECONDS
        );
        assert_eq!(legacy.keepalive_max_missed, MAX_SSH_KEEPALIVE_MAX_MISSED);
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
}

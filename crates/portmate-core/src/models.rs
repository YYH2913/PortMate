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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConnection {
    pub endpoint: HostEndpoint,
    pub username: String,
    #[serde(default = "default_true")]
    pub reconnect: bool,
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
    pub reconnect: bool,
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
    pub reconnect: bool,
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
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            raw: true,
            text: true,
            jsonl: true,
            redact_secrets: true,
            path_template: "{profile}/{date}/{session}.jsonl".to_string(),
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
}

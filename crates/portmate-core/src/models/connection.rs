use super::security::{AgentPolicy, HostKeyPolicy, IdentityPolicy, IdentityRef, TrustedHostKey};
use super::SessionKind;
use serde::{Deserialize, Serialize};

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
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password_secret_ref: Option<String>,
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
        self.username = self.username.trim().to_string();
        self.password_secret_ref = self
            .password_secret_ref
            .as_deref()
            .map(str::trim)
            .filter(|secret_ref| !secret_ref.is_empty())
            .map(ToOwned::to_owned);
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: ProxyKind::Socks5,
            host: default_proxy_host(),
            port: default_proxy_port(),
            username: String::new(),
            password_secret_ref: None,
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
    #[serde(default = "default_ssh_reconnect_delay_ms")]
    pub reconnect_delay_ms: u64,
    #[serde(default = "default_true")]
    pub keepalive_enabled: bool,
    #[serde(default = "default_ssh_keepalive_interval_seconds")]
    pub keepalive_interval_seconds: u64,
    #[serde(default = "default_ssh_keepalive_max_missed")]
    pub keepalive_max_missed: u32,
    #[serde(default)]
    pub tcp_keepalive_enabled: Option<bool>,
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

pub const MIN_SSH_RECONNECT_DELAY_MS: u64 = 100;
pub const MAX_SSH_RECONNECT_DELAY_MS: u64 = 60_000;
pub const DEFAULT_SSH_RECONNECT_DELAY_MS: u64 = 1_000;
pub const MIN_SSH_KEEPALIVE_INTERVAL_SECONDS: u64 = 1;
pub const MAX_SSH_KEEPALIVE_INTERVAL_SECONDS: u64 = 3_600;
pub const DEFAULT_SSH_KEEPALIVE_INTERVAL_SECONDS: u64 = 30;
pub const MIN_SSH_KEEPALIVE_MAX_MISSED: u32 = 0;
pub const MAX_SSH_KEEPALIVE_MAX_MISSED: u32 = 20;
pub const DEFAULT_SSH_KEEPALIVE_MAX_MISSED: u32 = 3;

const fn default_ssh_reconnect_delay_ms() -> u64 {
    DEFAULT_SSH_RECONNECT_DELAY_MS
}

const fn default_ssh_keepalive_interval_seconds() -> u64 {
    DEFAULT_SSH_KEEPALIVE_INTERVAL_SECONDS
}

const fn default_ssh_keepalive_max_missed() -> u32 {
    DEFAULT_SSH_KEEPALIVE_MAX_MISSED
}

impl SshConnection {
    pub fn normalize_health_settings(&mut self) {
        self.reconnect_delay_ms = self
            .reconnect_delay_ms
            .clamp(MIN_SSH_RECONNECT_DELAY_MS, MAX_SSH_RECONNECT_DELAY_MS);
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

pub const MAX_SHELL_ARGUMENTS: usize = 128;
pub const MAX_SHELL_ARGUMENT_CHARACTERS: usize = 4_096;

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
    #[serde(default = "default_true")]
    pub telnet_binary: bool,
    #[serde(default = "default_true")]
    pub telnet_naws: bool,
    #[serde(default)]
    pub tls_enabled: bool,
    #[serde(default)]
    pub tls_server_name: Option<String>,
    #[serde(default)]
    pub tls_accept_invalid_cert: bool,
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
        self.tls_server_name = self.tls_server_name.take().and_then(|name| {
            let name = name.trim();
            (!name.is_empty()
                && name.chars().take(254).count() <= 253
                && !name
                    .chars()
                    .any(|character| character.is_control() || character.is_whitespace()))
            .then(|| name.to_string())
        });
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
            telnet_binary: true,
            telnet_naws: true,
            tls_enabled: false,
            tls_server_name: None,
            tls_accept_invalid_cert: false,
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

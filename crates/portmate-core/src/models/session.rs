use super::connection::ConnectionConfig;
use super::transfer::TransferSettings;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandHistoryEntry {
    pub command: String,
    pub recorded_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

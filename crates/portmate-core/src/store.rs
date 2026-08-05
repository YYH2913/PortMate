use crate::host_keys::HostKeyStore;
use crate::models::*;
use crate::store_system_events::SystemEventSinkRuntime;
#[cfg(test)]
use crate::store_system_events::MAX_SYSTEM_EVENT_OUTBOX;
#[cfg(test)]
use chrono::Utc;
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::HashMap;

mod events;
mod exports;
mod histories;
mod security;
mod sessions;
#[cfg(test)]
use events::{EVENT_TRIM_BATCH, MAX_EVENTS_PER_SESSION};
#[cfg(test)]
use histories::{
    AUX_HISTORY_TRIM_BATCH, MAX_AUDIT_RECORDS_PER_SCOPE, MAX_SYSMON_SNAPSHOTS_PER_SESSION,
    MAX_TERMINAL_TRANSFERS_PER_SESSION, MAX_TIMELINE_MARKS_PER_SESSION,
};
pub use histories::{
    MAX_COMMAND_HISTORY_COMMAND_CHARACTERS, MAX_COMMAND_HISTORY_ENTRIES,
    MAX_COMMAND_HISTORY_RETENTION_DAYS, MAX_COMMAND_HISTORY_STORAGE_BYTES,
};
pub use sessions::{
    normalize_session_disconnect_reason, MAX_SESSION_DISCONNECT_REASON_CHARACTERS,
    MAX_SESSION_PROFILES,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStore {
    pub profiles: Vec<SessionProfile>,
    pub runtimes: Vec<SessionRuntime>,
    pub events: Vec<SessionEvent>,
    pub transfers: Vec<TransferTask>,
    #[serde(default)]
    pub command_history: Vec<CommandHistoryEntry>,
    #[serde(default)]
    pub command_history_migrated: bool,
    #[serde(default)]
    pub command_history_revision: u64,
    #[serde(default)]
    pub one_keys: Vec<OneKeyCredential>,
    pub host_keys: HostKeyStore,
    pub grants: Vec<McpGrant>,
    #[serde(default)]
    pub mcp_http_settings: McpHttpSettings,
    pub audit: Vec<AuditRecord>,
    pub timeline: Vec<TimelineMark>,
    pub sysmon: Vec<SysmonSnapshot>,
    /// Perf cache for `trim_events_if_needed`, not semantic state: lazily seeded per
    /// session (one full scan of `events`) and kept in sync incrementally afterward,
    /// so bounding a chatty session's log no longer rescans every session's events.
    #[serde(skip)]
    event_counts: HashMap<String, usize>,
    /// Runtime-only bounded outbox for the desktop system-event sink. Cloned
    /// stores share it because several Tauri mutations use a
    /// clone-then-persist-then-swap transaction pattern.
    #[serde(skip)]
    system_event_sink: SystemEventSinkRuntime,
}

#[cfg(test)]
mod tests;

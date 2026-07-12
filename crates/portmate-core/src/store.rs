use crate::host_keys::{HostKeyEvaluation, HostKeyObservation, HostKeyStore};
use crate::models::*;
use crate::redaction::redact_secrets;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

const MAX_EVENTS_PER_SESSION: usize = 5000;
const EVENT_TRIM_BATCH: usize = 512;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStore {
    pub profiles: Vec<SessionProfile>,
    pub runtimes: Vec<SessionRuntime>,
    pub events: Vec<SessionEvent>,
    pub transfers: Vec<TransferTask>,
    pub host_keys: HostKeyStore,
    pub grants: Vec<McpGrant>,
    pub audit: Vec<AuditRecord>,
    pub timeline: Vec<TimelineMark>,
    pub sysmon: Vec<SysmonSnapshot>,
    /// Perf cache for `trim_events_if_needed`, not semantic state: lazily seeded per
    /// session (one full scan of `events`) and kept in sync incrementally afterward,
    /// so bounding a chatty session's log no longer rescans every session's events.
    #[serde(skip)]
    event_counts: HashMap<String, usize>,
}

impl SessionStore {
    pub fn profile(&self, session_id: &str) -> Option<SessionProfile> {
        self.profiles
            .iter()
            .find(|profile| profile.id == session_id)
            .cloned()
    }

    pub fn upsert_profile(&mut self, profile: SessionProfile) -> SessionSummary {
        let now = Utc::now();
        let session_id = profile.id.clone();
        let kind = profile.kind;
        let title = profile.name.clone();

        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|existing| existing.id == session_id)
        {
            *existing = profile;
        } else {
            self.profiles.push(profile);
        }

        if let Some(runtime) = self
            .runtimes
            .iter_mut()
            .find(|runtime| runtime.session_id == session_id)
        {
            runtime.title = title;
            runtime.active_transport = kind;
            runtime.last_activity = now;
        } else {
            self.runtimes.push(SessionRuntime {
                session_id: session_id.clone(),
                pane_id: format!("{session_id}:main"),
                status: SessionStatus::Disconnected,
                title,
                cwd: None,
                connected_since: None,
                last_activity: now,
                last_disconnect: None,
                last_disconnect_reason: None,
                active_transport: kind,
            });
        }

        self.summaries()
            .into_iter()
            .find(|summary| summary.profile.id == session_id)
            .expect("upserted profile must have a runtime summary")
    }

    pub fn open_session(&mut self, session_id: &str) -> Result<SessionSummary, String> {
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == session_id)
            .cloned()
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        self.set_runtime_status(session_id, SessionStatus::Connected)?;

        self.push_system_event(
            session_id,
            format!(
                "PortMate: connected to {} ({:?})",
                Self::describe_endpoint(&profile),
                profile.kind
            ),
        );
        self.summary_for(session_id)
    }

    pub fn set_runtime_status(
        &mut self,
        session_id: &str,
        status: SessionStatus,
    ) -> Result<SessionSummary, String> {
        self.set_runtime_status_with_reason(session_id, status, None)
    }

    pub fn set_runtime_status_with_reason(
        &mut self,
        session_id: &str,
        status: SessionStatus,
        reason: Option<String>,
    ) -> Result<SessionSummary, String> {
        let profile = self
            .profiles
            .iter()
            .find(|profile| profile.id == session_id)
            .cloned()
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        let now = Utc::now();

        if let Some(runtime) = self
            .runtimes
            .iter_mut()
            .find(|runtime| runtime.session_id == session_id)
        {
            runtime.status = status;
            runtime.title = profile.name.clone();
            runtime.connected_since = if status == SessionStatus::Connected {
                Some(runtime.connected_since.unwrap_or(now))
            } else {
                None
            };
            runtime.last_activity = now;
            runtime.active_transport = profile.kind;
            apply_runtime_health(runtime, status, reason);
        } else {
            let mut runtime = SessionRuntime {
                session_id: session_id.to_string(),
                pane_id: format!("{session_id}:main"),
                status,
                title: profile.name.clone(),
                cwd: None,
                connected_since: (status == SessionStatus::Connected).then_some(now),
                last_activity: now,
                last_disconnect: None,
                last_disconnect_reason: None,
                active_transport: profile.kind,
            };
            apply_runtime_health(&mut runtime, status, reason);
            self.runtimes.push(runtime);
        }

        self.summary_for(session_id)
    }

    pub fn close_session(&mut self, session_id: &str) -> Result<SessionSummary, String> {
        if !self.profiles.iter().any(|profile| profile.id == session_id) {
            return Err(format!("unknown session: {session_id}"));
        }
        let now = Utc::now();

        if let Some(runtime) = self
            .runtimes
            .iter_mut()
            .find(|runtime| runtime.session_id == session_id)
        {
            runtime.status = SessionStatus::Disconnected;
            runtime.connected_since = None;
            runtime.last_activity = now;
            runtime.last_disconnect = Some(now);
            runtime.last_disconnect_reason = Some("user closed session".to_string());
        }

        self.push_system_event(session_id, "PortMate: session disconnected".to_string());
        self.summary_for(session_id)
    }

    pub fn record_system_event(&mut self, session_id: &str, text: impl Into<String>) {
        self.push_system_event(session_id, text.into());
    }

    pub fn record_stream_event(
        &mut self,
        session_id: &str,
        direction: EventDirection,
        stream: EventStream,
        text: impl Into<String>,
    ) -> Result<SessionEvent, String> {
        self.record_stream_event_with_bytes_ref(session_id, direction, stream, text, None)
    }

    pub fn record_stream_event_with_bytes_ref(
        &mut self,
        session_id: &str,
        direction: EventDirection,
        stream: EventStream,
        text: impl Into<String>,
        bytes_ref: Option<String>,
    ) -> Result<SessionEvent, String> {
        if !self.profiles.iter().any(|profile| profile.id == session_id) {
            return Err(format!("unknown session: {session_id}"));
        }
        let now = Utc::now();
        if let Some(runtime) = self
            .runtimes
            .iter_mut()
            .find(|runtime| runtime.session_id == session_id)
        {
            runtime.last_activity = now;
        }
        let event = SessionEvent {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            pane_id: format!("{session_id}:main"),
            ts: now,
            direction,
            stream,
            bytes_ref,
            text: Some(text.into()),
            annotations: BTreeMap::new(),
        };
        self.events.push(event.clone());
        self.trim_events_if_needed(session_id);
        Ok(event)
    }

    pub fn record_auth_success(
        &mut self,
        session_id: &str,
        method: AuthMethod,
    ) -> Result<(), String> {
        let profile = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;

        match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
                if ssh.identity_policy.record_success {
                    ssh.identity_policy.last_successful = Some(method);
                }
                Ok(())
            }
            _ => Err(format!("profile is not SSH-backed: {session_id}")),
        }
    }

    pub fn summaries(&self) -> Vec<SessionSummary> {
        // Relies on `self.events` staying in per-session insertion order (only ever
        // appended/retained-in-place, never sorted) so the last-seen text while
        // iterating forward is genuinely the chronologically last event.
        let mut log_stats: HashMap<&str, (usize, Option<&str>)> = HashMap::new();
        for event in &self.events {
            let Some(text) = event.text.as_deref() else {
                continue;
            };
            let entry = log_stats
                .entry(event.session_id.as_str())
                .or_insert((0, None));
            entry.0 += 1;
            entry.1 = Some(text);
        }

        self.profiles
            .iter()
            .filter_map(|profile| {
                let runtime = self
                    .runtimes
                    .iter()
                    .find(|runtime| runtime.session_id == profile.id)?
                    .clone();
                let (log_lines, last_line) = log_stats
                    .get(profile.id.as_str())
                    .map(|(count, last)| (*count, last.map(ToOwned::to_owned)))
                    .unwrap_or((0, None));
                Some(SessionSummary {
                    profile: profile.clone(),
                    runtime,
                    log_lines,
                    last_line,
                })
            })
            .collect()
    }

    fn summary_for(&self, session_id: &str) -> Result<SessionSummary, String> {
        self.summaries()
            .into_iter()
            .find(|summary| summary.profile.id == session_id)
            .ok_or_else(|| format!("session summary is missing: {session_id}"))
    }

    fn push_system_event(&mut self, session_id: &str, text: String) {
        let event = SessionEvent {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            pane_id: format!("{session_id}:main"),
            ts: Utc::now(),
            direction: EventDirection::System,
            stream: EventStream::Control,
            bytes_ref: None,
            text: Some(text),
            annotations: BTreeMap::new(),
        };
        self.events.push(event);
        self.trim_events_if_needed(session_id);
    }

    fn trim_events_if_needed(&mut self, session_id: &str) {
        // Callers always push exactly one event for `session_id` before calling this.
        // A cold cache entry is seeded with a fresh scan (already reflecting that push);
        // a warm entry just needs +1 for the push that happened since the last check.
        let already_cached = self.event_counts.contains_key(session_id);
        let events = &self.events;
        let count_ref = self
            .event_counts
            .entry(session_id.to_string())
            .or_insert_with(|| {
                events
                    .iter()
                    .filter(|event| event.session_id == session_id)
                    .count()
            });
        if already_cached {
            *count_ref += 1;
        }
        let session_count = *count_ref;

        if session_count <= MAX_EVENTS_PER_SESSION + EVENT_TRIM_BATCH {
            return;
        }

        let mut to_drop = session_count - MAX_EVENTS_PER_SESSION;
        self.events.retain(|event| {
            if to_drop > 0 && event.session_id == session_id {
                to_drop -= 1;
                false
            } else {
                true
            }
        });
        self.event_counts
            .insert(session_id.to_string(), MAX_EVENTS_PER_SESSION);
    }

    fn describe_endpoint(profile: &SessionProfile) -> String {
        match &profile.connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
                if ssh.username.is_empty() {
                    format!("{}:{}", ssh.endpoint.host, ssh.endpoint.port)
                } else {
                    format!(
                        "{}@{}:{}",
                        ssh.username, ssh.endpoint.host, ssh.endpoint.port
                    )
                }
            }
            ConnectionConfig::Serial(serial) => serial.port.clone(),
            ConnectionConfig::Shell(shell) => shell.program.clone(),
            ConnectionConfig::Telnet(tcp) | ConnectionConfig::Tcp(tcp) => {
                format!("{}:{}", tcp.host, tcp.port)
            }
        }
    }

    pub fn screen(&self, session_id: &str) -> Option<String> {
        let lines = self
            .events
            .iter()
            .filter(|event| event.session_id == session_id)
            .filter_map(|event| event.text.as_deref())
            .rev()
            .take(80)
            .collect::<Vec<_>>();
        if lines.is_empty() {
            None
        } else {
            Some(lines.into_iter().rev().collect::<Vec<_>>().join("\n"))
        }
    }

    pub fn tail_log(&self, session_id: &str, limit: usize) -> Vec<SessionEvent> {
        let mut events = self
            .events
            .iter()
            .filter(|event| event.session_id == session_id)
            .cloned()
            .collect::<Vec<_>>();
        let start = events.len().saturating_sub(limit);
        events.drain(..start);
        events
    }

    pub fn search_logs(
        &self,
        query: &str,
        session_id: Option<&str>,
        limit: usize,
    ) -> Vec<SessionEvent> {
        let needle = query.to_lowercase();
        let mut events = self
            .events
            .iter()
            .rev()
            .filter(|event| session_id.is_none_or(|id| event.session_id == id))
            .filter(|event| {
                event
                    .text
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&needle)
            })
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        events.reverse();
        events
    }

    pub fn send_text(
        &mut self,
        actor: &str,
        session_id: &str,
        text: &str,
    ) -> Result<SessionEvent, String> {
        if !self.profiles.iter().any(|profile| profile.id == session_id) {
            return Err(format!("unknown session: {session_id}"));
        }
        let redacted = redact_secrets(text);
        let event = SessionEvent {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            pane_id: format!("{session_id}:main"),
            ts: Utc::now(),
            direction: EventDirection::Outbound,
            stream: EventStream::Stdout,
            bytes_ref: None,
            text: Some(redacted),
            annotations: BTreeMap::from([("actor".to_string(), actor.to_string())]),
        };
        self.events.push(event.clone());
        self.trim_events_if_needed(session_id);
        self.audit.push(AuditRecord {
            id: Uuid::new_v4().to_string(),
            ts: Utc::now(),
            actor: actor.to_string(),
            action: "send_text".to_string(),
            session_id: Some(session_id.to_string()),
            decision: "recorded".to_string(),
            details: BTreeMap::from([("bytes".to_string(), text.len().to_string())]),
        });
        Ok(event)
    }

    pub fn evaluate_host_key(
        &self,
        profile_id: &str,
        observation: &HostKeyObservation,
    ) -> Result<HostKeyEvaluation, String> {
        let policy = self
            .ssh_profile(profile_id)
            .map(|ssh| &ssh.host_key_policy)
            .ok_or_else(|| format!("profile is not SSH-backed: {profile_id}"))?;
        self.host_keys
            .evaluate(profile_id, policy, observation)
            .map_err(|error| error.to_string())
    }

    pub fn apply_host_key_decision(
        &mut self,
        profile_id: &str,
        observation: &HostKeyObservation,
        decision: HostKeyDecision,
    ) -> Result<Option<TrustedHostKey>, String> {
        let policy = self
            .ssh_profile(profile_id)
            .map(|ssh| ssh.host_key_policy.clone())
            .ok_or_else(|| format!("profile is not SSH-backed: {profile_id}"))?;
        self.host_keys
            .apply_decision(profile_id, &policy, observation, decision)
            .map_err(|error| error.to_string())
    }

    pub fn transfer_by_id(&self, id: &str) -> Option<TransferTask> {
        self.transfers
            .iter()
            .find(|transfer| transfer.id == id)
            .cloned()
    }

    pub fn sysmon_for(&self, session_id: &str) -> Option<SysmonSnapshot> {
        self.sysmon
            .iter()
            .rev()
            .find(|snapshot| snapshot.session_id == session_id)
            .cloned()
    }

    pub fn timeline_for(&self, session_id: &str) -> Vec<TimelineMark> {
        self.timeline
            .iter()
            .filter(|mark| mark.session_id == session_id)
            .cloned()
            .collect()
    }

    pub fn mcp_can(&self, client_id: &str, scope: McpScope, session_id: Option<&str>) -> bool {
        let now = Utc::now();
        self.grants
            .iter()
            .filter(|grant| grant.client_id == client_id)
            .any(|grant| grant.allows(scope, session_id, now))
    }

    pub fn export_session_bundle(&self, session_id: &str) -> serde_json::Value {
        self.export_session_bundle_with_redaction(session_id, false)
    }

    pub fn export_session_bundle_redacted(&self, session_id: &str) -> serde_json::Value {
        self.export_session_bundle_with_redaction(session_id, true)
    }

    fn export_session_bundle_with_redaction(
        &self,
        session_id: &str,
        redact_text: bool,
    ) -> serde_json::Value {
        let mut summary = self
            .summaries()
            .into_iter()
            .find(|summary| summary.profile.id == session_id);
        let mut events = self.tail_log(session_id, 500);
        if redact_text {
            if let Some(summary) = &mut summary {
                summary.last_line = summary.last_line.take().map(|text| redact_secrets(&text));
            }
            for event in &mut events {
                event.text = event.text.take().map(|text| redact_secrets(&text));
            }
        }
        let log_shards = events
            .iter()
            .filter_map(|event| event.bytes_ref.as_ref())
            .cloned()
            .collect::<Vec<_>>();
        let transfers = self
            .transfers
            .iter()
            .filter(|transfer| transfer.session_id == session_id)
            .cloned()
            .collect::<Vec<_>>();
        let audit = self
            .audit
            .iter()
            .filter(|record| record.session_id.as_deref() == Some(session_id))
            .cloned()
            .collect::<Vec<_>>();

        serde_json::json!({
            "summary": summary,
            "events": events,
            "logShards": log_shards,
            "timeline": self.timeline_for(session_id),
            "sysmon": self.sysmon_for(session_id),
            "transfers": transfers,
            "audit": audit,
        })
    }

    fn ssh_profile(&self, profile_id: &str) -> Option<&SshConnection> {
        self.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .and_then(|profile| match &profile.connection {
                ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => Some(ssh),
                _ => None,
            })
    }
}

fn apply_runtime_health(
    runtime: &mut SessionRuntime,
    status: SessionStatus,
    reason: Option<String>,
) {
    if matches!(
        status,
        SessionStatus::Disconnected | SessionStatus::Reconnecting | SessionStatus::Error
    ) {
        runtime.last_disconnect = Some(runtime.last_activity);
        runtime.last_disconnect_reason = Some(reason.unwrap_or_else(|| match status {
            SessionStatus::Disconnected => "session disconnected".to_string(),
            SessionStatus::Reconnecting => "session reconnecting".to_string(),
            SessionStatus::Error => "connection error".to_string(),
            _ => "runtime status changed".to_string(),
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_scope_requires_grant() {
        let store = test_store();
        assert!(store.mcp_can("test-client", McpScope::ReadLogs, Some("test-session")));
        assert!(store.mcp_can("test-client", McpScope::WriteInput, Some("test-session")));
        assert!(!store.mcp_can("readonly", McpScope::WriteInput, Some("test-session")));
    }

    #[test]
    fn send_text_redacts_and_audits() {
        let mut store = test_store();
        let event = store
            .send_text("test", "test-session", "password=hunter2\n")
            .unwrap();
        assert!(!event.text.unwrap().contains("hunter2"));
        assert_eq!(store.audit.last().unwrap().action, "send_text");
    }

    #[test]
    fn upsert_profile_creates_runtime_for_new_session() {
        let mut store = SessionStore::default();
        let summary = store.upsert_profile(SessionProfile {
            id: "new-session".to_string(),
            name: "new session".to_string(),
            kind: SessionKind::Serial,
            group: "serial".to_string(),
            tags: Vec::new(),
            connection: ConnectionConfig::Serial(SerialConnection {
                port: "COM7".to_string(),
                baud_rate: 115_200,
                data_bits: 8,
                stop_bits: 1,
                parity: "none".to_string(),
                flow_control: "none".to_string(),
                dtr: false,
                rts: false,
                reconnect: true,
            }),
            terminal: TerminalSettings::default(),
            logging: LoggingSettings::default(),
            triggers: Vec::new(),
            transfer: TransferSettings::default(),
        });

        assert_eq!(summary.profile.id, "new-session");
        assert_eq!(summary.runtime.status, SessionStatus::Disconnected);
        assert_eq!(store.summaries().len(), 1);
    }

    #[test]
    fn open_and_close_session_updates_runtime_and_log() {
        let mut store = test_store();
        let opened = store.open_session("test-session").unwrap();
        assert_eq!(opened.runtime.status, SessionStatus::Connected);
        assert!(store.screen("test-session").unwrap().contains("connected"));

        let closed = store.close_session("test-session").unwrap();
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
        assert!(closed.runtime.last_disconnect.is_some());
        assert_eq!(
            closed.runtime.last_disconnect_reason.as_deref(),
            Some("user closed session")
        );
        assert!(store
            .screen("test-session")
            .unwrap()
            .contains("disconnected"));
    }

    #[test]
    fn runtime_status_reason_records_disconnect_health() {
        let mut store = test_store();
        let summary = store
            .set_runtime_status_with_reason(
                "test-session",
                SessionStatus::Reconnecting,
                Some("network timeout".to_string()),
            )
            .unwrap();

        assert_eq!(summary.runtime.status, SessionStatus::Reconnecting);
        assert!(summary.runtime.last_disconnect.is_some());
        assert_eq!(
            summary.runtime.last_disconnect_reason.as_deref(),
            Some("network timeout")
        );
    }

    #[test]
    fn stream_events_are_bounded_per_session() {
        let mut store = test_store();
        for index in 0..(MAX_EVENTS_PER_SESSION + EVENT_TRIM_BATCH + 64) {
            store
                .record_stream_event(
                    "test-session",
                    EventDirection::Inbound,
                    EventStream::Stdout,
                    format!("line {index}"),
                )
                .unwrap();
        }

        let events = store.tail_log("test-session", usize::MAX);
        assert!(events.len() <= MAX_EVENTS_PER_SESSION + EVENT_TRIM_BATCH);
        assert_ne!(
            events.first().and_then(|event| event.text.as_deref()),
            Some("line 0")
        );
    }

    #[test]
    fn search_logs_limits_to_recent_matches_in_chronological_order() {
        let mut store = test_store();
        for text in ["match old", "unrelated", "match middle", "match newest"] {
            store
                .record_stream_event(
                    "test-session",
                    EventDirection::Inbound,
                    EventStream::Stdout,
                    text,
                )
                .unwrap();
        }

        let latest = store.search_logs("MATCH", Some("test-session"), 1);
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].text.as_deref(), Some("match newest"));

        let latest_two = store.search_logs("match", Some("test-session"), 2);
        assert_eq!(
            latest_two
                .iter()
                .filter_map(|event| event.text.as_deref())
                .collect::<Vec<_>>(),
            vec!["match middle", "match newest"]
        );
        assert!(store
            .search_logs("match", Some("other-session"), 10)
            .is_empty());
    }

    #[test]
    fn export_bundle_includes_diagnostics_and_redacts_text() {
        let mut store = test_store();
        store
            .record_stream_event_with_bytes_ref(
                "test-session",
                EventDirection::Inbound,
                EventStream::Stdout,
                "password=hunter2",
                Some("test.raw:0:16".to_string()),
            )
            .unwrap();
        store.transfers.push(TransferTask {
            id: "transfer-1".to_string(),
            session_id: "test-session".to_string(),
            protocol: TransferProtocol::Sftp,
            source: "a".to_string(),
            destination: "b".to_string(),
            bytes_total: 16,
            bytes_done: 16,
            status: TransferStatus::Completed,
            message: Some("completed".to_string()),
            started_at: None,
            finished_at: None,
            average_bytes_per_second: None,
        });
        store
            .send_text("client", "test-session", "token=abc123")
            .unwrap();

        let bundle = store.export_session_bundle_redacted("test-session");
        let rendered = serde_json::to_string(&bundle).unwrap();

        assert!(rendered.contains("test.raw:0:16"));
        assert!(rendered.contains("transfer-1"));
        assert!(rendered.contains("send_text"));
        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains("abc123"));
    }

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
                    expires_at: None,
                    revoked_at: None,
                },
                McpGrant {
                    client_id: "readonly".to_string(),
                    name: "readonly".to_string(),
                    scopes: vec![McpScope::ReadLogs],
                    allowed_sessions: Vec::new(),
                    expires_at: None,
                    revoked_at: None,
                },
            ],
            ..SessionStore::default()
        }
    }
}

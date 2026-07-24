use crate::host_keys::{HostKeyEvaluation, HostKeyObservation, HostKeyStore};
use crate::models::*;
use crate::redaction::{
    redact_audit_records, redact_secrets, redact_session_events, redact_session_summary,
    redact_sysmon_snapshot, redact_timeline_marks, redact_transfer_task,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const MAX_EVENTS_PER_SESSION: usize = 5000;
const EVENT_TRIM_BATCH: usize = 512;
const MAX_SYSTEM_EVENT_OUTBOX: usize = 4096;
const MAX_AUDIT_RECORDS_PER_SCOPE: usize = 5000;
const MAX_TIMELINE_MARKS_PER_SESSION: usize = 2000;
const MAX_SYSMON_SNAPSHOTS_PER_SESSION: usize = 1024;
const MAX_TERMINAL_TRANSFERS_PER_SESSION: usize = 1000;
const AUX_HISTORY_TRIM_BATCH: usize = 128;
pub const MAX_SESSION_PROFILES: usize = 10_000;
pub const MAX_SESSION_DISCONNECT_REASON_CHARACTERS: usize = 256;

type SystemEventEnvelope = (SessionEvent, Option<SessionProfile>);

#[derive(Debug, Default)]
enum SystemEventSinkStatus {
    #[default]
    Inactive,
    Active(SyncSender<()>),
    Failed(String),
}

#[derive(Debug, Default)]
struct SystemEventSinkState {
    status: SystemEventSinkStatus,
    outbox: VecDeque<SystemEventEnvelope>,
}

#[derive(Debug, Clone, Default)]
struct SystemEventSinkRuntime {
    state: Arc<Mutex<SystemEventSinkState>>,
}

impl SystemEventSinkRuntime {
    fn set_notifier(&self, sender: SyncSender<()>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "system event sink state poisoned".to_string())?;
        state.status = SystemEventSinkStatus::Active(sender.clone());
        if !state.outbox.is_empty() {
            if let Err(std::sync::mpsc::TrySendError::Disconnected(())) = sender.try_send(()) {
                let error = "system event sink worker disconnected".to_string();
                state.status = SystemEventSinkStatus::Failed(error.clone());
                return Err(error);
            }
        }
        Ok(())
    }

    fn clear_notifier(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.status = SystemEventSinkStatus::Inactive;
        }
    }

    fn enqueue(&self, event: SessionEvent, profile: Option<SessionProfile>) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "system event sink state poisoned".to_string())?;
        let notifier = match &state.status {
            SystemEventSinkStatus::Inactive => return Ok(()),
            SystemEventSinkStatus::Active(notifier) => notifier.clone(),
            SystemEventSinkStatus::Failed(error) => return Err(error.clone()),
        };
        if state.outbox.len() >= MAX_SYSTEM_EVENT_OUTBOX {
            return Err(format!(
                "system event sink backlog exceeded {MAX_SYSTEM_EVENT_OUTBOX} events"
            ));
        }
        state.outbox.push_back((event, profile));
        match notifier.try_send(()) {
            Ok(()) | Err(std::sync::mpsc::TrySendError::Full(())) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Disconnected(())) => {
                state.outbox.pop_back();
                let error = "system event sink worker disconnected".to_string();
                state.status = SystemEventSinkStatus::Failed(error.clone());
                Err(error)
            }
        }
    }

    fn drain(&self) -> Vec<SystemEventEnvelope> {
        self.state
            .lock()
            .map(|mut state| state.outbox.drain(..).collect())
            .unwrap_or_default()
    }

    fn discard_session(&self, session_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .outbox
            .retain(|(event, _)| event.session_id != session_id);
    }

    fn discard_event(&self, event_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.outbox.retain(|(event, _)| event.id != event_id);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStore {
    pub profiles: Vec<SessionProfile>,
    pub runtimes: Vec<SessionRuntime>,
    pub events: Vec<SessionEvent>,
    pub transfers: Vec<TransferTask>,
    #[serde(default)]
    pub one_keys: Vec<OneKeyCredential>,
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
    /// Runtime-only bounded outbox for the desktop system-event sink. Cloned
    /// stores share it because several Tauri mutations use a
    /// clone-then-persist-then-swap transaction pattern.
    #[serde(skip)]
    system_event_sink: SystemEventSinkRuntime,
}

impl SessionStore {
    pub fn set_system_event_notifier(&mut self, sender: SyncSender<()>) -> Result<(), String> {
        self.system_event_sink.set_notifier(sender)
    }

    pub fn clear_system_event_notifier(&mut self) {
        self.system_event_sink.clear_notifier();
    }

    pub fn drain_system_event_outbox(&mut self) -> Vec<(SessionEvent, Option<SessionProfile>)> {
        self.system_event_sink.drain()
    }

    pub fn discard_system_events_for_session(&mut self, session_id: &str) {
        self.system_event_sink.discard_session(session_id);
    }

    pub fn discard_queued_system_event(&mut self, event_id: &str) {
        self.system_event_sink.discard_event(event_id);
    }

    pub fn profile(&self, session_id: &str) -> Option<SessionProfile> {
        self.profiles
            .iter()
            .find(|profile| profile.id == session_id)
            .cloned()
    }

    pub fn validate_profile_count(&self) -> Result<(), String> {
        if self.profiles.len() > MAX_SESSION_PROFILES {
            return Err(format!(
                "session profile count exceeds {MAX_SESSION_PROFILES}"
            ));
        }
        Ok(())
    }

    pub fn validate_profile_capacity(&self, session_id: &str) -> Result<(), String> {
        self.validate_profile_count()?;
        if self.profiles.iter().any(|profile| profile.id == session_id) {
            return Ok(());
        }
        if self.profiles.len() >= MAX_SESSION_PROFILES {
            return Err(format!(
                "session profile count has reached {MAX_SESSION_PROFILES}"
            ));
        }
        Ok(())
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
            if matches!(
                runtime.status,
                SessionStatus::Disconnected | SessionStatus::Blocked | SessionStatus::Error
            ) {
                runtime.active_transport = kind;
            }
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

        let _ = self.push_system_event(
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
            let previous_status = runtime.status;
            runtime.status = status;
            runtime.title = profile.name.clone();
            runtime.connected_since = if status == SessionStatus::Connected {
                Some(runtime.connected_since.unwrap_or(now))
            } else {
                None
            };
            runtime.last_activity = now;
            runtime.active_transport = profile.kind;
            apply_runtime_health(runtime, Some(previous_status), status, reason);
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
            apply_runtime_health(&mut runtime, None, status, reason);
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
            let previous_status = runtime.status;
            runtime.status = SessionStatus::Disconnected;
            runtime.connected_since = None;
            runtime.last_activity = now;
            apply_runtime_health(
                runtime,
                Some(previous_status),
                SessionStatus::Disconnected,
                Some("user closed session".to_string()),
            );
        }

        let _ = self.push_system_event(session_id, "PortMate: session disconnected".to_string());
        self.summary_for(session_id)
    }

    pub fn delete_profile(&mut self, session_id: &str) -> Result<SessionProfile, String> {
        let profile = self.delete_profile_deferred_system_event_cleanup(session_id)?;
        self.discard_system_events_for_session(session_id);
        Ok(profile)
    }

    /// Deletes persisted profile state without touching the runtime system-event outbox.
    /// Clone-then-persist callers must discard that session's queued events only after
    /// the new snapshot commits, because cloned stores share the same outbox.
    pub fn delete_profile_deferred_system_event_cleanup(
        &mut self,
        session_id: &str,
    ) -> Result<SessionProfile, String> {
        let profile_index = self
            .profiles
            .iter()
            .position(|profile| profile.id == session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        if self.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id
                && !matches!(
                    runtime.status,
                    SessionStatus::Disconnected | SessionStatus::Blocked | SessionStatus::Error
                )
        }) {
            return Err("session must be disconnected before deleting its profile".to_string());
        }
        if self.transfers.iter().any(|transfer| {
            transfer.session_id == session_id
                && matches!(
                    transfer.status,
                    TransferStatus::Queued | TransferStatus::Running
                )
        }) {
            return Err("session has an active transfer and cannot be deleted".to_string());
        }

        let now = Utc::now();
        let profile = self.profiles.remove(profile_index);
        self.runtimes
            .retain(|runtime| runtime.session_id != session_id);
        self.events.retain(|event| event.session_id != session_id);
        self.event_counts.remove(session_id);
        self.transfers
            .retain(|transfer| transfer.session_id != session_id);
        self.timeline.retain(|mark| mark.session_id != session_id);
        self.sysmon
            .retain(|snapshot| snapshot.session_id != session_id);

        self.host_keys.keys.retain(|key| {
            !(key.scope == HostKeyScope::Profile && key.profile_id.as_deref() == Some(session_id))
        });
        for key in &mut self.host_keys.keys {
            if key.profile_id.as_deref() == Some(session_id) {
                key.profile_id = None;
            }
        }

        for one_key in &mut self.one_keys {
            let previous_session_count = one_key.session_ids.len();
            one_key
                .session_ids
                .retain(|bound_session_id| bound_session_id != session_id);
            let removed_identity = one_key
                .identity
                .as_ref()
                .is_some_and(|identity| identity.source_profile_id == session_id);
            if removed_identity {
                one_key.identity = None;
            }
            if previous_session_count != one_key.session_ids.len() || removed_identity {
                one_key.updated_at = now;
            }
        }

        for grant in &mut self.grants {
            if grant.allowed_sessions.is_empty() {
                continue;
            }
            let previous_session_count = grant.allowed_sessions.len();
            grant
                .allowed_sessions
                .retain(|allowed_session_id| allowed_session_id != session_id);
            if previous_session_count != grant.allowed_sessions.len()
                && grant.allowed_sessions.is_empty()
                && grant.revoked_at.is_none()
            {
                grant.revoked_at = Some(now);
            }
        }

        Ok(profile)
    }

    pub fn record_system_event(&mut self, session_id: &str, text: impl Into<String>) {
        let _ = self.record_system_event_tracked(session_id, text);
    }

    pub fn record_system_event_tracked(
        &mut self,
        session_id: &str,
        text: impl Into<String>,
    ) -> Option<String> {
        self.push_system_event(session_id, text.into())
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
        self.record_event(
            session_id,
            direction,
            stream,
            Some(text.into()),
            bytes_ref,
            BTreeMap::new(),
        )
    }

    pub fn record_event(
        &mut self,
        session_id: &str,
        direction: EventDirection,
        stream: EventStream,
        text: Option<String>,
        bytes_ref: Option<String>,
        annotations: BTreeMap<String, String>,
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
            text,
            annotations,
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
                ssh.identity_policy.last_successful = ssh
                    .identity_policy
                    .record_success
                    .then_some(method)
                    .filter(|method| ssh.identity_policy.auth_order.contains(method));
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

    fn push_system_event(&mut self, session_id: &str, text: String) -> Option<String> {
        let profile = self.profile(session_id)?;
        let mut event = SessionEvent {
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
        let event_id = event.id.clone();
        if let Err(error) = self.system_event_sink.enqueue(event.clone(), Some(profile)) {
            event.annotations.insert("loggingError".to_string(), error);
        }
        self.events.push(event);
        self.trim_events_if_needed(session_id);
        Some(event_id)
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
        self.send_text_with_bytes_ref(actor, session_id, text, None)
    }

    pub fn send_text_with_bytes_ref(
        &mut self,
        actor: &str,
        session_id: &str,
        text: &str,
        bytes_ref: Option<String>,
    ) -> Result<SessionEvent, String> {
        self.send_text_with_bytes_ref_and_audit_action(
            actor,
            session_id,
            text,
            bytes_ref,
            Some("send_text"),
        )
    }

    pub fn send_text_with_bytes_ref_and_audit_action(
        &mut self,
        actor: &str,
        session_id: &str,
        text: &str,
        bytes_ref: Option<String>,
        audit_action: Option<&str>,
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
        let redacted = redact_secrets(text);
        let event = SessionEvent {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            pane_id: format!("{session_id}:main"),
            ts: now,
            direction: EventDirection::Outbound,
            stream: EventStream::Stdout,
            bytes_ref,
            text: Some(redacted),
            annotations: BTreeMap::from([("actor".to_string(), actor.to_string())]),
        };
        self.events.push(event.clone());
        self.trim_events_if_needed(session_id);
        if let Some(action) = audit_action {
            self.record_audit(AuditRecord {
                id: Uuid::new_v4().to_string(),
                ts: now,
                actor: actor.to_string(),
                action: action.to_string(),
                session_id: Some(session_id.to_string()),
                decision: "recorded".to_string(),
                details: BTreeMap::from([("bytes".to_string(), text.len().to_string())]),
            });
        }
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

    pub fn record_transfer(&mut self, transfer: TransferTask) {
        let session_id = transfer.session_id.clone();
        self.transfers.push(transfer);
        self.trim_transfer_history(&session_id);
    }

    pub fn trim_transfer_history(&mut self, session_id: &str) {
        let mut terminal = self
            .transfers
            .iter()
            .enumerate()
            .filter(|(_, transfer)| {
                transfer.session_id == session_id
                    && matches!(
                        transfer.status,
                        TransferStatus::Completed
                            | TransferStatus::Failed
                            | TransferStatus::Cancelled
                    )
            })
            .map(|(index, transfer)| (index, transfer.finished_at))
            .collect::<Vec<_>>();
        if terminal.len() <= MAX_TERMINAL_TRANSFERS_PER_SESSION {
            return;
        }
        let to_drop = terminal.len() - MAX_TERMINAL_TRANSFERS_PER_SESSION;
        terminal.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        let remove = terminal
            .into_iter()
            .take(to_drop)
            .map(|(index, _)| index)
            .collect::<HashSet<_>>();
        let mut index = 0_usize;
        self.transfers.retain(|_| {
            let keep = !remove.contains(&index);
            index += 1;
            keep
        });
    }

    pub fn record_audit(&mut self, record: AuditRecord) {
        let scope = record.session_id.clone();
        self.audit.push(record);
        trim_oldest_matching(
            &mut self.audit,
            MAX_AUDIT_RECORDS_PER_SCOPE,
            AUX_HISTORY_TRIM_BATCH,
            |record| record.session_id == scope,
        );
    }

    pub fn record_timeline_mark(&mut self, mark: TimelineMark) {
        let session_id = mark.session_id.clone();
        self.timeline.push(mark);
        trim_oldest_matching(
            &mut self.timeline,
            MAX_TIMELINE_MARKS_PER_SESSION,
            AUX_HISTORY_TRIM_BATCH,
            |mark| mark.session_id == session_id,
        );
    }

    pub fn record_sysmon_snapshot(&mut self, snapshot: SysmonSnapshot) {
        let session_id = snapshot.session_id.clone();
        self.sysmon.push(snapshot);
        trim_oldest_matching(
            &mut self.sysmon,
            MAX_SYSMON_SNAPSHOTS_PER_SESSION,
            AUX_HISTORY_TRIM_BATCH,
            |snapshot| snapshot.session_id == session_id,
        );
    }

    pub fn normalize_bounded_histories(&mut self) {
        let mut remaining_event_counts = HashMap::<String, usize>::new();
        for event in &self.events {
            *remaining_event_counts
                .entry(event.session_id.clone())
                .or_default() += 1;
        }
        self.events.retain(|event| {
            let remaining = remaining_event_counts
                .get_mut(&event.session_id)
                .expect("event session count was seeded");
            let keep = *remaining <= MAX_EVENTS_PER_SESSION;
            *remaining -= 1;
            keep
        });
        self.event_counts.clear();
        for event in &self.events {
            *self
                .event_counts
                .entry(event.session_id.clone())
                .or_default() += 1;
        }

        let audit_scopes = self
            .audit
            .iter()
            .map(|record| record.session_id.clone())
            .collect::<HashSet<_>>();
        for scope in audit_scopes {
            trim_oldest_matching(&mut self.audit, MAX_AUDIT_RECORDS_PER_SCOPE, 0, |record| {
                record.session_id == scope
            });
        }

        let timeline_sessions = self
            .timeline
            .iter()
            .map(|mark| mark.session_id.clone())
            .collect::<HashSet<_>>();
        for session_id in timeline_sessions {
            trim_oldest_matching(
                &mut self.timeline,
                MAX_TIMELINE_MARKS_PER_SESSION,
                0,
                |mark| mark.session_id == session_id,
            );
        }

        let sysmon_sessions = self
            .sysmon
            .iter()
            .map(|snapshot| snapshot.session_id.clone())
            .collect::<HashSet<_>>();
        for session_id in sysmon_sessions {
            trim_oldest_matching(
                &mut self.sysmon,
                MAX_SYSMON_SNAPSHOTS_PER_SESSION,
                0,
                |snapshot| snapshot.session_id == session_id,
            );
        }

        let transfer_sessions = self
            .transfers
            .iter()
            .map(|transfer| transfer.session_id.clone())
            .collect::<HashSet<_>>();
        for session_id in transfer_sessions {
            self.trim_transfer_history(&session_id);
        }
    }

    pub fn sysmon_for(&self, session_id: &str) -> Option<SysmonSnapshot> {
        self.sysmon
            .iter()
            .rev()
            .find(|snapshot| snapshot.session_id == session_id)
            .cloned()
    }

    pub fn sysmon_history_for(&self, session_id: &str, limit: usize) -> Vec<SysmonSnapshot> {
        if limit == 0 {
            return Vec::new();
        }
        let mut snapshots = self
            .sysmon
            .iter()
            .rev()
            .filter(|snapshot| snapshot.session_id == session_id)
            .take(limit)
            .cloned()
            .collect::<Vec<_>>();
        snapshots.reverse();
        snapshots
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

    pub fn mcp_can_read(&self, client_id: &str, scope: McpScope, session_id: Option<&str>) -> bool {
        let client_id = client_id.trim();
        !client_id.is_empty()
            && client_id.len() <= 128
            && !client_id.chars().any(char::is_control)
            && (self.grants.is_empty() || self.mcp_can(client_id, scope, session_id))
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
        redact_bundle: bool,
    ) -> serde_json::Value {
        let mut summary = self
            .summaries()
            .into_iter()
            .find(|summary| summary.profile.id == session_id);
        let mut events = self.tail_log(session_id, 500);
        let mut timeline = self.timeline_for(session_id);
        let mut transfers = self
            .transfers
            .iter()
            .filter(|transfer| transfer.session_id == session_id)
            .cloned()
            .collect::<Vec<_>>();
        let mut audit = self
            .audit
            .iter()
            .filter(|record| record.session_id.as_deref() == Some(session_id))
            .cloned()
            .collect::<Vec<_>>();
        let mut sysmon = self.sysmon_for(session_id);
        if redact_bundle {
            summary = summary.map(redact_session_summary);
            events = redact_session_events(events);
            timeline = redact_timeline_marks(timeline);
            transfers = transfers.into_iter().map(redact_transfer_task).collect();
            audit = redact_audit_records(audit);
            sysmon = sysmon.map(redact_sysmon_snapshot);
        }
        let log_shards = events
            .iter()
            .filter_map(|event| event.bytes_ref.as_ref())
            .cloned()
            .collect::<Vec<_>>();

        serde_json::json!({
            "summary": summary,
            "events": events,
            "logShards": log_shards,
            "timeline": timeline,
            "sysmon": sysmon,
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

fn trim_oldest_matching<T>(
    items: &mut Vec<T>,
    max: usize,
    slack: usize,
    mut matches: impl FnMut(&T) -> bool,
) {
    let count = items.iter().filter(|item| matches(item)).count();
    if count <= max.saturating_add(slack) {
        return;
    }
    let mut to_drop = count - max;
    items.retain(|item| {
        if to_drop > 0 && matches(item) {
            to_drop -= 1;
            false
        } else {
            true
        }
    });
}

fn apply_runtime_health(
    runtime: &mut SessionRuntime,
    previous_status: Option<SessionStatus>,
    status: SessionStatus,
    reason: Option<String>,
) {
    if runtime_outage_status(status) {
        let continuing_outage = previous_status.is_some_and(runtime_outage_status);
        if !continuing_outage || runtime.last_disconnect.is_none() {
            runtime.last_disconnect = Some(runtime.last_activity);
        }
        let default_reason = match status {
            SessionStatus::Disconnected => "session disconnected".to_string(),
            SessionStatus::Reconnecting => "session reconnecting".to_string(),
            SessionStatus::Error => "connection error".to_string(),
            _ => "runtime status changed".to_string(),
        };
        runtime.last_disconnect_reason = Some(
            reason
                .as_deref()
                .and_then(normalize_session_disconnect_reason)
                .unwrap_or(default_reason),
        );
    }
}

pub fn normalize_session_disconnect_reason(value: &str) -> Option<String> {
    const ELLIPSIS_CHARACTERS: usize = 3;
    let mut normalized = String::with_capacity(MAX_SESSION_DISCONNECT_REASON_CHARACTERS * 4);
    let mut characters = 0;
    let mut pending_space = false;
    let mut truncated = false;

    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = characters > 0;
            continue;
        }
        if pending_space {
            if characters == MAX_SESSION_DISCONNECT_REASON_CHARACTERS {
                truncated = true;
                break;
            }
            normalized.push(' ');
            characters += 1;
            pending_space = false;
        }
        if characters == MAX_SESSION_DISCONNECT_REASON_CHARACTERS {
            truncated = true;
            break;
        }
        normalized.push(character);
        characters += 1;
    }

    if normalized.is_empty() {
        return None;
    }
    if truncated {
        while characters
            > MAX_SESSION_DISCONNECT_REASON_CHARACTERS.saturating_sub(ELLIPSIS_CHARACTERS)
        {
            normalized.pop();
            characters -= 1;
        }
        normalized.push_str("...");
    }
    Some(normalized)
}

fn runtime_outage_status(status: SessionStatus) -> bool {
    matches!(
        status,
        SessionStatus::Disconnected | SessionStatus::Reconnecting | SessionStatus::Error
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_store_without_one_keys_deserializes_empty() {
        let mut value = serde_json::to_value(SessionStore::default()).unwrap();
        value.as_object_mut().unwrap().remove("oneKeys");
        let store: SessionStore = serde_json::from_value(value).unwrap();
        assert!(store.one_keys.is_empty());
    }

    #[test]
    fn write_scope_requires_grant() {
        let store = test_store();
        assert!(store.mcp_can("test-client", McpScope::ReadLogs, Some("test-session")));
        assert!(store.mcp_can("test-client", McpScope::WriteInput, Some("test-session")));
        assert!(!store.mcp_can("readonly", McpScope::WriteInput, Some("test-session")));
    }

    #[test]
    fn read_scopes_default_open_then_follow_explicit_grants() {
        let mut store = test_store();
        store.grants.clear();
        assert!(store.mcp_can_read("reader", McpScope::ReadSessions, None));
        assert!(store.mcp_can_read("reader", McpScope::ReadLogs, Some("test-session")));
        assert!(!store.mcp_can_read("  ", McpScope::ReadSessions, None));
        assert!(!store.mcp_can_read("bad\nreader", McpScope::ReadSessions, None));
        assert!(!store.mcp_can_read(&"x".repeat(129), McpScope::ReadSessions, None));

        store.grants.push(McpGrant {
            client_id: "scoped-reader".to_string(),
            name: "Scoped reader".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: vec!["test-session".to_string()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });
        assert!(!store.mcp_can_read("unknown", McpScope::ReadLogs, Some("test-session")));
        assert!(!store.mcp_can_read("scoped-reader", McpScope::ReadSessions, None));
        assert!(store.mcp_can_read(" scoped-reader ", McpScope::ReadLogs, Some("test-session")));
        assert!(!store.mcp_can_read("scoped-reader", McpScope::ReadLogs, Some("other-session")));

        store.grants[0].revoked_at = Some(Utc::now());
        assert!(!store.mcp_can_read("scoped-reader", McpScope::ReadLogs, Some("test-session")));
    }

    #[test]
    fn grant_is_expired_at_its_exact_deadline() {
        let now = Utc::now();
        let grant = McpGrant {
            client_id: "reader".to_string(),
            name: "Reader".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: Vec::new(),
            confirm_writes: false,
            expires_at: Some(now),
            revoked_at: None,
        };

        assert!(!grant.allows(McpScope::ReadLogs, Some("test-session"), now));
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
    fn send_text_preserves_raw_bytes_reference_while_redacting_text() {
        let mut store = test_store();
        let event = store
            .send_text_with_bytes_ref(
                "test",
                "test-session",
                "password=hunter2\n",
                Some("v2:session.raw:0:3:digest".to_string()),
            )
            .unwrap();
        assert_eq!(
            event.bytes_ref.as_deref(),
            Some("v2:session.raw:0:3:digest")
        );
        assert!(!event.text.unwrap().contains("hunter2"));
    }

    #[test]
    fn send_text_updates_runtime_activity_with_event_timestamp() {
        let mut store = test_store();
        let previous = Utc::now() - chrono::Duration::minutes(5);
        store.runtimes[0].last_activity = previous;

        let event = store
            .send_text("test", "test-session", "show version\n")
            .unwrap();
        let runtime = store
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == "test-session")
            .unwrap();

        assert!(runtime.last_activity > previous);
        assert_eq!(runtime.last_activity, event.ts);
        assert_eq!(store.audit.last().unwrap().ts, event.ts);
    }

    #[test]
    fn binary_control_events_allow_bytes_without_fake_text() {
        let mut store = test_store();
        let event = store
            .record_event(
                "test-session",
                EventDirection::Outbound,
                EventStream::Control,
                None,
                Some("v2:session.raw:0:3:digest".to_string()),
                BTreeMap::from([("origin".to_string(), "telnet-negotiation".to_string())]),
            )
            .unwrap();
        assert!(event.text.is_none());
        assert!(event.bytes_ref.is_some());
        assert_eq!(
            event.annotations.get("origin").map(String::as_str),
            Some("telnet-negotiation")
        );
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
                reconnect_delay_ms: DEFAULT_SERIAL_RECONNECT_DELAY_MS,
                receive_idle_timeout_enabled: false,
                receive_idle_timeout_seconds: DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS,
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
    fn profile_capacity_allows_updates_at_limit_and_rejects_oversized_stores() {
        let mut store = test_store();
        let profile = store.profiles[0].clone();
        store.profiles = (0..MAX_SESSION_PROFILES)
            .map(|index| {
                let mut profile = profile.clone();
                profile.id = format!("session-{index}");
                profile
            })
            .collect();

        assert!(store.validate_profile_count().is_ok());
        assert!(store.validate_profile_capacity("session-0").is_ok());
        let capacity_error = store.validate_profile_capacity("new-session").unwrap_err();
        assert!(capacity_error.contains(&MAX_SESSION_PROFILES.to_string()));

        let mut overflow = profile;
        overflow.id = "overflow-session".to_string();
        store.profiles.push(overflow);
        let count_error = store.validate_profile_count().unwrap_err();
        assert!(count_error.contains(&MAX_SESSION_PROFILES.to_string()));
        assert!(store.validate_profile_capacity("session-0").is_err());
    }

    #[test]
    fn upsert_profile_preserves_active_transport_until_disconnect() {
        let mut store = test_store();
        let mut profile = store.profile("test-session").unwrap();
        profile.kind = SessionKind::Serial;
        profile.connection = ConnectionConfig::Serial(SerialConnection {
            port: "COM7".to_string(),
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".to_string(),
            flow_control: "none".to_string(),
            dtr: false,
            rts: false,
            reconnect: true,
            reconnect_delay_ms: DEFAULT_SERIAL_RECONNECT_DELAY_MS,
            receive_idle_timeout_enabled: false,
            receive_idle_timeout_seconds: DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS,
        });

        let active = store.upsert_profile(profile);
        assert_eq!(active.profile.kind, SessionKind::Serial);
        assert_eq!(active.runtime.status, SessionStatus::Connected);
        assert_eq!(active.runtime.active_transport, SessionKind::Shell);

        let disconnected = store
            .set_runtime_status("test-session", SessionStatus::Disconnected)
            .unwrap();
        assert_eq!(disconnected.runtime.active_transport, SessionKind::Serial);
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
    fn auth_success_recording_respects_the_enabled_policy() {
        let mut store = test_store();
        let mut ssh = sensitive_ssh_connection();
        ssh.identity_policy.auth_order = vec![AuthMethod::Password];
        store.profiles[0].kind = SessionKind::Ssh;
        store.profiles[0].connection = ConnectionConfig::Ssh(ssh);

        store
            .record_auth_success("test-session", AuthMethod::PublicKey)
            .unwrap();
        let profile = store.profile("test-session").unwrap();
        let ConnectionConfig::Ssh(ssh) = profile.connection else {
            panic!("test profile must remain SSH");
        };
        assert_eq!(ssh.identity_policy.last_successful, None);

        store
            .record_auth_success("test-session", AuthMethod::Password)
            .unwrap();
        let profile = store.profile("test-session").unwrap();
        let ConnectionConfig::Ssh(ssh) = profile.connection else {
            panic!("test profile must remain SSH");
        };
        assert_eq!(
            ssh.identity_policy.last_successful,
            Some(AuthMethod::Password)
        );

        let ConnectionConfig::Ssh(ssh) = &mut store.profiles[0].connection else {
            panic!("test profile must remain SSH");
        };
        ssh.identity_policy.record_success = false;
        store
            .record_auth_success("test-session", AuthMethod::Password)
            .unwrap();
        let profile = store.profile("test-session").unwrap();
        let ConnectionConfig::Ssh(ssh) = profile.connection else {
            panic!("test profile must remain SSH");
        };
        assert_eq!(ssh.identity_policy.last_successful, None);
    }

    #[test]
    fn delete_profile_rejects_live_sessions_and_active_transfers() {
        let mut store = test_store();
        let live_error = store.delete_profile("test-session").unwrap_err();
        assert!(live_error.contains("must be disconnected"));
        assert!(store.profile("test-session").is_some());

        store.runtimes[0].status = SessionStatus::Disconnected;
        store.record_transfer(test_transfer("queued".to_string(), TransferStatus::Queued));
        let transfer_error = store.delete_profile("test-session").unwrap_err();
        assert!(transfer_error.contains("active transfer"));
        assert!(store.profile("test-session").is_some());
    }

    #[test]
    fn delete_profile_cascades_without_widening_grants_or_project_trust() {
        let mut store = test_store();
        store.runtimes[0].status = SessionStatus::Disconnected;
        let (event_sender, _event_receiver) = std::sync::mpsc::sync_channel(1);
        store.set_system_event_notifier(event_sender).unwrap();
        store.record_system_event("test-session", "queued deletion event");
        let mut other_profile = store.profiles[0].clone();
        other_profile.id = "other-session".to_string();
        other_profile.name = "other session".to_string();
        store.upsert_profile(other_profile);
        store
            .record_stream_event(
                "test-session",
                EventDirection::Inbound,
                EventStream::Stdout,
                "deleted event",
            )
            .unwrap();
        store
            .record_stream_event(
                "other-session",
                EventDirection::Inbound,
                EventStream::Stdout,
                "retained event",
            )
            .unwrap();
        store.record_transfer(test_transfer(
            "completed".to_string(),
            TransferStatus::Completed,
        ));
        let now = Utc::now();
        store.record_timeline_mark(TimelineMark {
            id: "timeline-delete".to_string(),
            session_id: "test-session".to_string(),
            ts: now,
            label: "delete me".to_string(),
            details: None,
        });
        store.record_sysmon_snapshot(SysmonSnapshot {
            session_id: "test-session".to_string(),
            ts: now,
            uptime_seconds: 1,
            cpu_percent: 0.0,
            memory_percent: 0.0,
            rx_kbps: 0.0,
            tx_kbps: 0.0,
            load_average: [0.0; 3],
            memory_total_bytes: 0,
            memory_available_bytes: 0,
            processes: Vec::new(),
            disks: Vec::new(),
            network_interfaces: Vec::new(),
        });
        store.record_audit(AuditRecord {
            id: "audit-delete".to_string(),
            ts: now,
            actor: "desktop-user".to_string(),
            action: "profile-test".to_string(),
            session_id: Some("test-session".to_string()),
            decision: "recorded".to_string(),
            details: BTreeMap::new(),
        });
        store.host_keys.keys.extend([
            TrustedHostKey {
                id: "profile-key".to_string(),
                profile_id: Some("test-session".to_string()),
                alias: "device".to_string(),
                host: "device".to_string(),
                port: 22,
                algorithm: "ssh-ed25519".to_string(),
                fingerprint_sha256: "SHA256:profile".to_string(),
                public_key_base64: "AAAAC3NzaC1lZDI1NTE5AAAAIA==".to_string(),
                scope: HostKeyScope::Profile,
                label: None,
                first_seen: now,
                last_seen: now,
            },
            TrustedHostKey {
                id: "project-key".to_string(),
                profile_id: Some("test-session".to_string()),
                alias: "shared".to_string(),
                host: "shared".to_string(),
                port: 22,
                algorithm: "ssh-ed25519".to_string(),
                fingerprint_sha256: "SHA256:project".to_string(),
                public_key_base64: "AAAAC3NzaC1lZDI1NTE5AAAAIA==".to_string(),
                scope: HostKeyScope::Project,
                label: None,
                first_seen: now,
                last_seen: now,
            },
        ]);
        store.one_keys.push(OneKeyCredential {
            id: "one-key".to_string(),
            label: "shared login".to_string(),
            kind: OneKeyKind::Ssh,
            username: "operator".to_string(),
            password_secret_ref: Some("keyring:one-key".to_string()),
            passphrase_secret_ref: None,
            identity: Some(OneKeyIdentity {
                source_profile_id: "test-session".to_string(),
                identity: IdentityRef {
                    id: "identity".to_string(),
                    label: "identity".to_string(),
                    source: IdentitySource::Agent,
                    fingerprint_sha256: Some("SHA256:identity".to_string()),
                    path: None,
                    secret_ref: None,
                },
            }),
            session_ids: vec!["test-session".to_string(), "other-session".to_string()],
            created_at: now,
            updated_at: now,
        });
        store.grants.push(McpGrant {
            client_id: "mixed".to_string(),
            name: "mixed".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: vec!["test-session".to_string(), "other-session".to_string()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let deleted = store.delete_profile("test-session").unwrap();

        assert_eq!(deleted.id, "test-session");
        assert!(store.profile("test-session").is_none());
        assert!(store.profile("other-session").is_some());
        assert!(store
            .events
            .iter()
            .all(|event| event.session_id != "test-session"));
        assert!(store.drain_system_event_outbox().is_empty());
        assert!(store
            .events
            .iter()
            .any(|event| event.session_id == "other-session"));
        assert!(store
            .transfers
            .iter()
            .all(|transfer| transfer.session_id != "test-session"));
        assert!(store.timeline.is_empty());
        assert!(store.sysmon.is_empty());
        assert!(store.audit.iter().any(|record| record.id == "audit-delete"));
        assert!(!store
            .host_keys
            .keys
            .iter()
            .any(|key| key.id == "profile-key"));
        assert!(store
            .host_keys
            .keys
            .iter()
            .any(|key| key.id == "project-key" && key.profile_id.is_none()));
        assert_eq!(store.one_keys[0].session_ids, ["other-session"]);
        assert!(store.one_keys[0].identity.is_none());
        assert!(store.one_keys[0].updated_at >= now);

        let scoped = store
            .grants
            .iter()
            .find(|grant| grant.client_id == "test-client")
            .unwrap();
        assert!(scoped.allowed_sessions.is_empty());
        assert!(scoped.revoked_at.is_some());
        assert!(!scoped.allows(McpScope::ReadLogs, Some("other-session"), Utc::now()));
        let mixed = store
            .grants
            .iter()
            .find(|grant| grant.client_id == "mixed")
            .unwrap();
        assert_eq!(mixed.allowed_sessions, ["other-session"]);
        assert!(mixed.revoked_at.is_none());
        let global = store
            .grants
            .iter()
            .find(|grant| grant.client_id == "readonly")
            .unwrap();
        assert!(global.allowed_sessions.is_empty());
        assert!(global.revoked_at.is_none());
    }

    #[test]
    fn deferred_profile_delete_keeps_shared_outbox_until_commit() {
        let mut store = test_store();
        store.runtimes[0].status = SessionStatus::Disconnected;
        let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
        store.set_system_event_notifier(sender).unwrap();
        store.record_system_event("test-session", "queued before transaction");

        let mut next_store = store.clone();
        next_store
            .delete_profile_deferred_system_event_cleanup("test-session")
            .unwrap();

        let queued_after_rollback = store.drain_system_event_outbox();
        assert_eq!(queued_after_rollback.len(), 1);
        assert_eq!(
            queued_after_rollback[0].0.text.as_deref(),
            Some("queued before transaction")
        );

        store.record_system_event("test-session", "queued before commit");
        next_store.discard_system_events_for_session("test-session");
        assert!(store.drain_system_event_outbox().is_empty());
    }

    #[test]
    fn system_events_cannot_recreate_deleted_profile_history() {
        let mut store = test_store();
        store.runtimes[0].status = SessionStatus::Disconnected;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        store.set_system_event_notifier(sender).unwrap();
        store.delete_profile("test-session").unwrap();

        store.record_system_event("test-session", "late worker diagnostic");

        assert!(store.events.is_empty());
        assert!(store.drain_system_event_outbox().is_empty());
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn system_event_notifier_coalesces_direct_and_lifecycle_events() {
        let mut store = test_store();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        store.set_system_event_notifier(sender).unwrap();

        store.record_system_event("test-session", "PortMate: direct diagnostic");
        store.open_session("test-session").unwrap();
        store.close_session("test-session").unwrap();

        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(receiver.try_recv().is_err());
        let queued = store.drain_system_event_outbox();
        assert_eq!(queued.len(), 3);
        assert!(queued.iter().all(|(event, profile)| {
            event.direction == EventDirection::System
                && event.stream == EventStream::Control
                && profile
                    .as_ref()
                    .is_some_and(|profile| profile.id == "test-session")
        }));
        let events = store.events.iter().rev().take(3).collect::<Vec<_>>();
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|event| {
            event.direction == EventDirection::System
                && event.stream == EventStream::Control
                && event.bytes_ref.is_none()
        }));
        assert!(events[2]
            .text
            .as_deref()
            .is_some_and(|text| text.contains("direct diagnostic")));
        assert!(events[1]
            .text
            .as_deref()
            .is_some_and(|text| text.contains("connected")));
        assert!(events[0]
            .text
            .as_deref()
            .is_some_and(|text| text.contains("disconnected")));
    }

    #[test]
    fn tracked_system_event_can_be_removed_from_the_outbox_exactly() {
        let mut store = test_store();
        let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
        store.set_system_event_notifier(sender).unwrap();

        let rolled_back = store
            .record_system_event_tracked("test-session", "rolled back event")
            .unwrap();
        let retained = store
            .record_system_event_tracked("test-session", "retained event")
            .unwrap();
        store.discard_queued_system_event(&rolled_back);

        let queued = store.drain_system_event_outbox();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].0.id, retained);
        assert!(store.events.iter().any(|event| event.id == rolled_back));
        assert!(store.events.iter().any(|event| event.id == retained));
    }

    #[test]
    fn system_event_outbox_is_bounded_and_reports_overflow() {
        let mut store = test_store();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        store.set_system_event_notifier(sender).unwrap();

        for index in 0..=MAX_SYSTEM_EVENT_OUTBOX {
            store.record_system_event("test-session", format!("diagnostic {index}"));
        }

        assert_eq!(
            store.drain_system_event_outbox().len(),
            MAX_SYSTEM_EVENT_OUTBOX
        );
        assert!(store.events.last().is_some_and(|event| {
            event
                .annotations
                .get("loggingError")
                .is_some_and(|error| error.contains("backlog exceeded"))
        }));

        drop(receiver);
        store.record_system_event("test-session", "disconnected worker one");
        store.record_system_event("test-session", "disconnected worker two");
        assert!(store.events.iter().rev().take(2).all(|event| {
            event
                .annotations
                .get("loggingError")
                .is_some_and(|error| error.contains("worker disconnected"))
        }));
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
    fn runtime_disconnect_reason_is_normalized_and_unicode_bounded() {
        let mut store = test_store();
        let summary = store
            .set_runtime_status_with_reason(
                "test-session",
                SessionStatus::Error,
                Some(format!("  socket\n  closed  {}  ", "界".repeat(300))),
            )
            .unwrap();
        let reason = summary.runtime.last_disconnect_reason.unwrap();

        assert!(reason.starts_with("socket closed 界"));
        assert!(reason.ends_with("..."));
        assert!(!reason.contains('\n'));
        assert_eq!(
            reason.chars().count(),
            MAX_SESSION_DISCONNECT_REASON_CHARACTERS
        );

        let fallback = store
            .set_runtime_status_with_reason(
                "test-session",
                SessionStatus::Error,
                Some(" \n\t ".to_string()),
            )
            .unwrap();
        assert_eq!(
            fallback.runtime.last_disconnect_reason.as_deref(),
            Some("connection error")
        );

        let exact = "界".repeat(MAX_SESSION_DISCONNECT_REASON_CHARACTERS);
        assert_eq!(
            normalize_session_disconnect_reason(&exact).as_deref(),
            Some(exact.as_str())
        );
        let oversized = format!(
            "{}{}",
            " \n\t".repeat(100_000),
            "界".repeat(MAX_SESSION_DISCONNECT_REASON_CHARACTERS + 1)
        );
        let bounded = normalize_session_disconnect_reason(&oversized).unwrap();
        assert_eq!(
            bounded.chars().count(),
            MAX_SESSION_DISCONNECT_REASON_CHARACTERS
        );
        assert!(bounded.ends_with("..."));
    }

    #[test]
    fn runtime_health_preserves_first_disconnect_time_during_one_outage() {
        let mut store = test_store();
        store
            .set_runtime_status("test-session", SessionStatus::Connected)
            .unwrap();
        let reconnecting = store
            .set_runtime_status_with_reason(
                "test-session",
                SessionStatus::Reconnecting,
                Some("network timeout".to_string()),
            )
            .unwrap();
        let disconnected_at = reconnecting.runtime.last_disconnect.unwrap();

        let retry_failed = store
            .set_runtime_status_with_reason(
                "test-session",
                SessionStatus::Reconnecting,
                Some("SSH reconnect failed: connection refused".to_string()),
            )
            .unwrap();
        assert_eq!(retry_failed.runtime.last_disconnect, Some(disconnected_at));
        assert_eq!(
            retry_failed.runtime.last_disconnect_reason.as_deref(),
            Some("SSH reconnect failed: connection refused")
        );

        let stopped = store
            .set_runtime_status_with_reason(
                "test-session",
                SessionStatus::Disconnected,
                Some("automatic reconnect disabled".to_string()),
            )
            .unwrap();
        assert_eq!(stopped.runtime.last_disconnect, Some(disconnected_at));
        assert_eq!(
            stopped.runtime.last_disconnect_reason.as_deref(),
            Some("automatic reconnect disabled")
        );

        let closed_again = store.close_session("test-session").unwrap();
        assert_eq!(closed_again.runtime.last_disconnect, Some(disconnected_at));
        assert_eq!(
            closed_again.runtime.last_disconnect_reason.as_deref(),
            Some("user closed session")
        );
    }

    #[test]
    fn runtime_health_records_a_new_time_after_recovery() {
        let mut store = test_store();
        let old_disconnect = Utc::now() - chrono::Duration::hours(1);
        let runtime = store
            .runtimes
            .iter_mut()
            .find(|runtime| runtime.session_id == "test-session")
            .unwrap();
        runtime.status = SessionStatus::Connected;
        runtime.last_disconnect = Some(old_disconnect);
        runtime.last_disconnect_reason = Some("older outage".to_string());

        let reconnecting = store
            .set_runtime_status_with_reason(
                "test-session",
                SessionStatus::Reconnecting,
                Some("new transport loss".to_string()),
            )
            .unwrap();
        assert!(reconnecting.runtime.last_disconnect.unwrap() > old_disconnect);
        assert_eq!(
            reconnecting.runtime.last_disconnect_reason.as_deref(),
            Some("new transport loss")
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
    fn loaded_event_histories_are_normalized_and_rebuild_the_count_cache() {
        let mut store = test_store();
        let overflow = 37;
        store.events = (0..(MAX_EVENTS_PER_SESSION + overflow))
            .map(|index| SessionEvent {
                id: format!("loaded-{index}"),
                session_id: "test-session".to_string(),
                pane_id: "test-session:main".to_string(),
                ts: Utc::now(),
                direction: EventDirection::Inbound,
                stream: EventStream::Stdout,
                bytes_ref: None,
                text: Some(format!("loaded line {index}")),
                annotations: BTreeMap::new(),
            })
            .collect();
        store
            .event_counts
            .insert("test-session".to_string(), usize::MAX);

        store.normalize_bounded_histories();

        assert_eq!(store.events.len(), MAX_EVENTS_PER_SESSION);
        assert_eq!(
            store.events.first().and_then(|event| event.text.as_deref()),
            Some("loaded line 37")
        );
        assert_eq!(
            store.event_counts.get("test-session"),
            Some(&MAX_EVENTS_PER_SESSION)
        );

        store
            .record_stream_event(
                "test-session",
                EventDirection::Inbound,
                EventStream::Stdout,
                "next line",
            )
            .unwrap();
        assert_eq!(store.events.len(), MAX_EVENTS_PER_SESSION + 1);
        assert_eq!(
            store.event_counts.get("test-session"),
            Some(&(MAX_EVENTS_PER_SESSION + 1))
        );
    }

    #[test]
    fn auxiliary_histories_are_bounded_per_scope_and_keep_active_transfers() {
        let mut store = test_store();
        let now = Utc::now();

        for index in 0..(MAX_AUDIT_RECORDS_PER_SCOPE + AUX_HISTORY_TRIM_BATCH + 2) {
            store.record_audit(AuditRecord {
                id: format!("audit-{index}"),
                ts: now + chrono::Duration::milliseconds(index as i64),
                actor: "test".to_string(),
                action: "send_text".to_string(),
                session_id: Some("test-session".to_string()),
                decision: "recorded".to_string(),
                details: BTreeMap::new(),
            });
        }
        store.record_audit(AuditRecord {
            id: "audit-global".to_string(),
            ts: now,
            actor: "test".to_string(),
            action: "global".to_string(),
            session_id: None,
            decision: "recorded".to_string(),
            details: BTreeMap::new(),
        });
        assert!(store.audit.len() <= MAX_AUDIT_RECORDS_PER_SCOPE + AUX_HISTORY_TRIM_BATCH + 1);
        assert!(!store.audit.iter().any(|record| record.id == "audit-0"));
        assert!(store.audit.iter().any(|record| record.id == "audit-global"));

        for index in 0..(MAX_TIMELINE_MARKS_PER_SESSION + AUX_HISTORY_TRIM_BATCH + 2) {
            store.record_timeline_mark(TimelineMark {
                id: format!("timeline-{index}"),
                session_id: "test-session".to_string(),
                ts: now + chrono::Duration::milliseconds(index as i64),
                label: "checkpoint".to_string(),
                details: None,
            });
        }
        assert!(store.timeline.len() <= MAX_TIMELINE_MARKS_PER_SESSION + AUX_HISTORY_TRIM_BATCH);
        assert!(!store.timeline.iter().any(|mark| mark.id == "timeline-0"));

        for index in 0..(MAX_SYSMON_SNAPSHOTS_PER_SESSION + AUX_HISTORY_TRIM_BATCH + 2) {
            store.record_sysmon_snapshot(SysmonSnapshot {
                session_id: "test-session".to_string(),
                ts: now + chrono::Duration::milliseconds(index as i64),
                uptime_seconds: index as u64,
                cpu_percent: 1.0,
                memory_percent: 2.0,
                rx_kbps: 3.0,
                tx_kbps: 4.0,
                load_average: [0.0; 3],
                memory_total_bytes: 0,
                memory_available_bytes: 0,
                processes: Vec::new(),
                disks: Vec::new(),
                network_interfaces: Vec::new(),
            });
        }
        assert!(store.sysmon.len() <= MAX_SYSMON_SNAPSHOTS_PER_SESSION + AUX_HISTORY_TRIM_BATCH);
        assert_ne!(store.sysmon[0].uptime_seconds, 0);
        let recent_sysmon = store.sysmon_history_for("test-session", 3);
        assert_eq!(recent_sysmon.len(), 3);
        assert!(recent_sysmon
            .windows(2)
            .all(|pair| pair[0].ts <= pair[1].ts));
        assert_eq!(
            recent_sysmon.last().map(|snapshot| snapshot.uptime_seconds),
            store
                .sysmon
                .iter()
                .rev()
                .find(|snapshot| snapshot.session_id == "test-session")
                .map(|snapshot| snapshot.uptime_seconds)
        );
        assert!(store.sysmon_history_for("test-session", 0).is_empty());

        for index in 0..(MAX_TERMINAL_TRANSFERS_PER_SESSION + 2) {
            store.record_transfer(test_transfer(
                format!("completed-{index}"),
                TransferStatus::Completed,
            ));
        }
        store.record_transfer(test_transfer("queued".to_string(), TransferStatus::Queued));
        store.record_transfer(test_transfer(
            "running".to_string(),
            TransferStatus::Running,
        ));
        assert_eq!(
            store
                .transfers
                .iter()
                .filter(|transfer| transfer.status == TransferStatus::Completed)
                .count(),
            MAX_TERMINAL_TRANSFERS_PER_SESSION
        );
        assert!(store.transfer_by_id("queued").is_some());
        assert!(store.transfer_by_id("running").is_some());
        assert!(store.transfer_by_id("completed-0").is_none());

        let queued = store
            .transfers
            .iter_mut()
            .find(|transfer| transfer.id == "queued")
            .unwrap();
        queued.status = TransferStatus::Completed;
        queued.finished_at = Some(now + chrono::Duration::days(1));
        store.trim_transfer_history("test-session");
        assert!(store.transfer_by_id("queued").is_some());
        assert_eq!(
            store
                .transfers
                .iter()
                .filter(|transfer| transfer.status == TransferStatus::Completed)
                .count(),
            MAX_TERMINAL_TRANSFERS_PER_SESSION
        );

        store.normalize_bounded_histories();
        assert_eq!(
            store
                .audit
                .iter()
                .filter(|record| record.session_id.as_deref() == Some("test-session"))
                .count(),
            MAX_AUDIT_RECORDS_PER_SCOPE
        );
        assert_eq!(store.timeline.len(), MAX_TIMELINE_MARKS_PER_SESSION);
        assert_eq!(store.sysmon.len(), MAX_SYSMON_SNAPSHOTS_PER_SESSION);
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
        let ConnectionConfig::Shell(shell) = &mut store.profiles[0].connection else {
            unreachable!("test store should use a shell profile");
        };
        shell.args = vec!["--password".to_string(), "opaque-shell-secret".to_string()];
        shell.cwd = Some("/home/operator/private-shell-cwd".to_string());
        store
            .record_stream_event_with_bytes_ref(
                "test-session",
                EventDirection::Inbound,
                EventStream::Stdout,
                "password=hunter2",
                Some("test.raw:0:16".to_string()),
            )
            .unwrap();
        store.record_transfer(TransferTask {
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

        assert!(!rendered.contains("test.raw:0:16"));
        assert!(rendered.contains("transfer-1"));
        assert!(rendered.contains("send_text"));
        assert!(!rendered.contains("hunter2"));
        assert!(!rendered.contains("abc123"));
        assert!(!rendered.contains("opaque-shell-secret"));
        assert!(!rendered.contains("/home/operator/private-shell-cwd"));
        assert!(bundle["summary"]["profile"]["connection"]["args"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(bundle["logShards"].as_array().unwrap().is_empty());
        assert!(bundle["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["bytesRef"].is_null()));
    }

    #[test]
    fn redacted_bundle_removes_ssh_and_tmux_credentials_and_local_paths() {
        for (kind, connection) in [
            (
                SessionKind::Ssh,
                ConnectionConfig::Ssh(sensitive_ssh_connection()),
            ),
            (
                SessionKind::Tmux,
                ConnectionConfig::Tmux(sensitive_ssh_connection()),
            ),
        ] {
            let mut store = test_store();
            store.profiles[0].kind = kind;
            store.profiles[0].connection = connection;
            store.profiles[0].logging.path_template =
                "/home/operator/private-logs/{session}.raw".to_string();
            store.profiles[0].transfer.default_local_dir =
                Some("/home/operator/private-downloads".to_string());
            store.profiles[0].triggers = vec![TriggerSpec {
                id: "sensitive-trigger".to_string(),
                label: "password=trigger-label-secret".to_string(),
                matcher: TriggerMatcher::Contains {
                    text: "token=trigger-match-secret".to_string(),
                    case_sensitive: false,
                },
                actions: vec![
                    TriggerAction::SendText {
                        text: "opaque-send-secret".to_string(),
                    },
                    TriggerAction::LocalCommand {
                        command: "/home/operator/private-scripts/deploy".to_string(),
                    },
                    TriggerAction::CustomLink {
                        url_template: "https://internal.invalid/?token=trigger-url-secret"
                            .to_string(),
                    },
                ],
                enabled: true,
            }];
            store.runtimes[0].active_transport = kind;
            store.runtimes[0].cwd = Some("/home/operator/runtime-cwd".to_string());
            store.runtimes[0].last_disconnect_reason =
                Some("password=disconnect-secret".to_string());
            store
                .record_event(
                    "test-session",
                    EventDirection::Inbound,
                    EventStream::Stdout,
                    Some("password=event-secret".to_string()),
                    Some("v2:/home/operator/private-logs/raw:0:12:digest".to_string()),
                    BTreeMap::from([(
                        "diagnostic".to_string(),
                        "token=annotation-secret".to_string(),
                    )]),
                )
                .unwrap();
            store.record_timeline_mark(TimelineMark {
                id: "timeline-sensitive".to_string(),
                session_id: "test-session".to_string(),
                ts: Utc::now(),
                label: "password=timeline-secret".to_string(),
                details: Some("token=timeline-details-secret".to_string()),
            });
            store.record_transfer(TransferTask {
                id: "transfer-sensitive".to_string(),
                session_id: "test-session".to_string(),
                protocol: TransferProtocol::Sftp,
                source: "/home/operator/source-secret.txt".to_string(),
                destination: "/srv/private/destination-secret.txt".to_string(),
                bytes_total: 12,
                bytes_done: 12,
                status: TransferStatus::Completed,
                message: Some("token=transfer-message-secret".to_string()),
                started_at: None,
                finished_at: None,
                average_bytes_per_second: None,
            });
            store.record_audit(AuditRecord {
                id: "audit-sensitive".to_string(),
                ts: Utc::now(),
                actor: "desktop-user".to_string(),
                action: "export-test".to_string(),
                session_id: Some("test-session".to_string()),
                decision: "recorded".to_string(),
                details: BTreeMap::from([(
                    "diagnostic".to_string(),
                    "password=audit-secret".to_string(),
                )]),
            });
            store.record_sysmon_snapshot(SysmonSnapshot {
                session_id: "test-session".to_string(),
                ts: Utc::now(),
                uptime_seconds: 123,
                cpu_percent: 12.5,
                memory_percent: 34.5,
                rx_kbps: 56.5,
                tx_kbps: 78.5,
                load_average: [0.5, 1.0, 1.5],
                memory_total_bytes: 1024,
                memory_available_bytes: 512,
                processes: vec![SysmonProcess {
                    pid: 4242,
                    name: "password=sysmon-process-secret".to_string(),
                    cpu_percent: 9.5,
                    memory_percent: 8.5,
                    rss_bytes: 256,
                }],
                disks: vec![SysmonDisk {
                    filesystem: "/dev/mapper/private-filesystem".to_string(),
                    mount_point: "/srv/private-mount".to_string(),
                    total_bytes: 4096,
                    available_bytes: 2048,
                    used_percent: 50.0,
                }],
                network_interfaces: vec![SysmonNetworkInterface {
                    name: "customer-private-interface".to_string(),
                    addresses: vec!["10.0.0.25/24".to_string()],
                    rx_bytes: 100,
                    tx_bytes: 200,
                    rx_kbps: 3.5,
                    tx_kbps: 4.5,
                }],
            });

            let plain = store.export_session_bundle("test-session");
            let redacted = store.export_session_bundle_redacted("test-session");
            let plain_json = serde_json::to_string(&plain).unwrap();
            let redacted_json = serde_json::to_string(&redacted).unwrap();
            let sensitive_values = [
                "keyring:target-password-ref",
                "stronghold:target-passphrase-ref",
                "keyring:proxy-password-ref",
                "/home/operator/.ssh/private-key",
                "stronghold:identity-secret-ref",
                "keyring:jump-password-ref",
                "stronghold:jump-passphrase-ref",
                "/home/operator/private-logs/{session}.raw",
                "/home/operator/private-downloads",
                "/home/operator/runtime-cwd",
                "disconnect-secret",
                "event-secret",
                "annotation-secret",
                "timeline-secret",
                "timeline-details-secret",
                "/home/operator/source-secret.txt",
                "/srv/private/destination-secret.txt",
                "transfer-message-secret",
                "audit-secret",
                "trigger-label-secret",
                "trigger-match-secret",
                "opaque-send-secret",
                "/home/operator/private-scripts/deploy",
                "trigger-url-secret",
                "v2:/home/operator/private-logs/raw:0:12:digest",
                "sysmon-process-secret",
                "/dev/mapper/private-filesystem",
                "/srv/private-mount",
                "customer-private-interface",
                "10.0.0.25/24",
            ];

            for sensitive in sensitive_values {
                assert!(
                    plain_json.contains(sensitive),
                    "plain {kind:?} bundle should retain {sensitive}"
                );
                assert!(
                    !redacted_json.contains(sensitive),
                    "redacted {kind:?} bundle leaked {sensitive}"
                );
            }
            assert!(redacted["logShards"].as_array().unwrap().is_empty());
            assert!(redacted["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|event| event["bytesRef"].is_null()));
            assert_eq!(
                redacted["summary"]["profile"]["connection"]["identityRefs"][0]
                    ["fingerprintSha256"],
                "SHA256:diagnostic-fingerprint"
            );
            assert_eq!(redacted["transfers"][0]["protocol"], "sftp");
            assert_eq!(redacted["transfers"][0]["status"], "completed");
            assert_eq!(redacted["sysmon"]["processes"][0]["pid"], 4242);
            assert_eq!(
                redacted["sysmon"]["processes"][0]["name"],
                "<redacted-process>"
            );
            assert_eq!(
                redacted["sysmon"]["disks"][0]["filesystem"],
                "<redacted-filesystem>"
            );
            assert_eq!(
                redacted["sysmon"]["disks"][0]["mountPoint"],
                "<redacted-mount-point>"
            );
            assert_eq!(
                redacted["sysmon"]["networkInterfaces"][0]["name"],
                "<redacted-interface>"
            );
            assert_eq!(
                redacted["sysmon"]["networkInterfaces"][0]["addresses"][0],
                "<redacted-address>"
            );
            assert_eq!(
                redacted["summary"]["profile"]["triggers"][0]["actions"][0]["text"],
                "<redacted>"
            );
        }
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
}

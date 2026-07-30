use super::SessionStore;
use crate::models::{AuditRecord, EventDirection, EventStream, SessionEvent, SessionProfile};
use crate::redaction::redact_secrets;
use chrono::Utc;
use std::collections::BTreeMap;
use std::sync::mpsc::SyncSender;
use uuid::Uuid;

pub(super) const MAX_EVENTS_PER_SESSION: usize = 5000;
pub(super) const EVENT_TRIM_BATCH: usize = 512;

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

    pub(super) fn push_system_event(&mut self, session_id: &str, text: String) -> Option<String> {
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
}

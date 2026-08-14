use super::SessionStore;
use crate::models::*;
use chrono::Utc;
use std::collections::HashMap;

pub const MAX_SESSION_PROFILES: usize = 10_000;
pub const MAX_SESSION_DISCONNECT_REASON_CHARACTERS: usize = 256;

impl SessionStore {
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

        for script in &mut self.custom_scripts {
            if script.allow_all_sessions {
                continue;
            }
            let previous_session_count = script.allowed_session_ids.len();
            script
                .allowed_session_ids
                .retain(|allowed_session_id| allowed_session_id != session_id);
            if previous_session_count != script.allowed_session_ids.len() {
                script.updated_at = now;
                if script.allowed_session_ids.is_empty() {
                    script.mcp_enabled = false;
                }
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

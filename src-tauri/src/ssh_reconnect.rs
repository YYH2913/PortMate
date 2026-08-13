use super::*;

pub(super) fn ssh_reconnect_enabled(profile: &SessionProfile) -> bool {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.reconnect,
        _ => false,
    }
}

pub(super) fn ssh_reconnect_delay(profile: &SessionProfile) -> Duration {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            Duration::from_millis(ssh.reconnect_delay_ms.clamp(
                portmate_core::MIN_SSH_RECONNECT_DELAY_MS,
                portmate_core::MAX_SSH_RECONNECT_DELAY_MS,
            ))
        }
        _ => Duration::from_millis(portmate_core::DEFAULT_SSH_RECONNECT_DELAY_MS),
    }
}

pub(super) fn ssh_establishment_profile_matches(
    attempt: &SessionProfile,
    latest: &SessionProfile,
) -> bool {
    let mut attempt = normalize_session_profile(attempt.clone());
    let mut latest = normalize_session_profile(latest.clone());
    ignore_host_key_last_seen_for_establishment(&mut attempt);
    ignore_host_key_last_seen_for_establishment(&mut latest);
    attempt.connection == latest.connection && attempt.terminal == latest.terminal
}

pub(super) fn ignore_host_key_last_seen_for_establishment(profile: &mut SessionProfile) {
    let ssh = match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh,
        _ => return,
    };
    for key in &mut ssh.trusted_host_keys {
        key.last_seen = key.first_seen;
    }
}

pub(super) fn ssh_reconnect_attempt_matches_profile(
    attempt: &SessionProfile,
    latest: &SessionProfile,
) -> bool {
    ssh_reconnect_enabled(latest) && ssh_establishment_profile_matches(attempt, latest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SshReconnectProfileState {
    Current,
    Changed,
    Disabled,
}

pub(super) fn ssh_reconnect_profile_state(
    store: &SessionStore,
    session_id: &str,
    attempt: &SessionProfile,
) -> SshReconnectProfileState {
    let Some(latest) = store.profile(session_id).map(normalize_session_profile) else {
        return SshReconnectProfileState::Disabled;
    };
    if !ssh_reconnect_enabled(&latest) {
        return SshReconnectProfileState::Disabled;
    }
    if !ssh_reconnect_attempt_matches_profile(attempt, &latest) {
        return SshReconnectProfileState::Changed;
    }
    SshReconnectProfileState::Current
}

pub(super) fn latest_ssh_reconnect_profile(
    state: &AppState,
    session_id: &str,
) -> Result<Option<SessionProfile>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let Some(profile) = store.profile(session_id) else {
        return Ok(None);
    };
    let profile = normalize_session_profile(profile);
    Ok(ssh_reconnect_enabled(&profile).then_some(profile))
}

pub(super) fn ssh_reconnect_pending(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    if closed.load(Ordering::SeqCst) {
        return false;
    }
    let connections = match state.ssh.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    state.store.lock().ok().is_some_and(|store| {
        store.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
        })
    })
}

pub(super) fn ssh_runtime_connected(state: &AppState, session_id: &str, runtime_id: &str) -> bool {
    let connections = match state.ssh.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if !connections.get(session_id).is_some_and(|runtime| {
        runtime.runtime_id == runtime_id && !runtime.closed.load(Ordering::SeqCst)
    }) {
        return false;
    }
    state.store.lock().ok().is_some_and(|store| {
        store.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id && runtime.status == SessionStatus::Connected
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SshReconnectFailureDisposition {
    Recorded,
    RetryLatestProfile,
    StopDisabled,
    Superseded,
}

pub(super) fn record_ssh_reconnect_failure_if_pending(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    attempt: Option<&SessionProfile>,
    error: &str,
) -> SshReconnectFailureDisposition {
    if closed.load(Ordering::SeqCst) {
        return SshReconnectFailureDisposition::Superseded;
    }
    let connections = match state.ssh.lock() {
        Ok(connections) => connections,
        Err(_) => return SshReconnectFailureDisposition::Superseded,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return SshReconnectFailureDisposition::Superseded;
    }
    let mut store = match state.store.lock() {
        Ok(store) => store,
        Err(_) => return SshReconnectFailureDisposition::Superseded,
    };
    if !store.runtimes.iter().any(|runtime| {
        runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
    }) {
        return SshReconnectFailureDisposition::Superseded;
    }
    if let Some(attempt) = attempt {
        match ssh_reconnect_profile_state(&store, session_id, attempt) {
            SshReconnectProfileState::Current => {}
            SshReconnectProfileState::Changed => {
                return SshReconnectFailureDisposition::RetryLatestProfile;
            }
            SshReconnectProfileState::Disabled => {
                return SshReconnectFailureDisposition::StopDisabled;
            }
        }
    }
    let reconnect_delay = store
        .profile(session_id)
        .map(normalize_session_profile)
        .map(|profile| ssh_reconnect_delay(&profile))
        .unwrap_or_else(|| Duration::from_millis(portmate_core::DEFAULT_SSH_RECONNECT_DELAY_MS));
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Reconnecting,
        Some(format!("SSH reconnect failed: {error}")),
    );
    store.record_system_event(
        session_id,
        format!(
            "PortMate: SSH reconnect failed: {error}; retrying in {}ms",
            reconnect_delay.as_millis()
        ),
    );
    if let Err(save_error) =
        persist_applied_store(&store, &state.store_path, "SSH reconnect failure state")
    {
        eprintln!("PortMate: failed to persist SSH reconnect failure: {save_error}");
    }
    SshReconnectFailureDisposition::Recorded
}

pub(super) fn stop_pending_ssh_reconnect_if_disabled(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    reason: &str,
) -> bool {
    let mut connections = match state.ssh.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    let mut store = match state.store.lock() {
        Ok(store) => store,
        Err(_) => return false,
    };
    let reconnect_disabled = store
        .profile(session_id)
        .map(normalize_session_profile)
        .is_none_or(|profile| !ssh_reconnect_enabled(&profile));
    if !reconnect_disabled {
        return false;
    }
    if let Some(runtime) = connections.remove(session_id) {
        runtime.closed.store(true, Ordering::SeqCst);
    }
    let stopped_tunnels = fail_session_tunnel_runtimes(&state.tunnels, session_id, reason)
        .map(|runtimes| runtimes.len())
        .unwrap_or_default();
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Disconnected,
        Some(reason.to_string()),
    );
    store.record_system_event(
        session_id,
        format!(
            "PortMate: SSH reconnect stopped: {reason}; stopped {stopped_tunnels} tunnel runtime(s)"
        ),
    );
    if let Err(error) =
        persist_applied_store(&store, &state.store_path, "stopped SSH reconnect state")
    {
        eprintln!("PortMate: failed to persist SSH reconnect stop: {error}");
    }
    true
}

pub(super) fn fail_pending_ssh_reconnect_install(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    error: &str,
) {
    closed.store(true, Ordering::SeqCst);
    let removed_current = match state.ssh.lock() {
        Ok(mut connections) => {
            if connections
                .get(session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                if let Some(runtime) = connections.remove(session_id) {
                    runtime.closed.store(true, Ordering::SeqCst);
                }
                true
            } else {
                false
            }
        }
        Err(lock_error) => {
            eprintln!(
                "PortMate: failed to clean up SSH reconnect runtime after Store failure: {lock_error}"
            );
            false
        }
    };
    if !removed_current {
        return;
    }

    clear_active_command(&state.session_io(), session_id);
    let reason = portmate_core::normalize_session_disconnect_reason(&format!(
        "SSH reconnect install failed: {error}"
    ))
    .unwrap_or_else(|| "SSH reconnect install failed".to_string());
    if let Ok(mut store) = state.store.lock() {
        let _ = store.set_runtime_status_with_reason(
            session_id,
            SessionStatus::Error,
            Some(reason.clone()),
        );
        store.record_system_event(session_id, format!("PortMate: {reason}"));
    }
}

#[cfg(test)]
pub(super) fn take_forced_ssh_reconnect_install_error(state: &AppState) -> Option<String> {
    state
        .ssh_reconnect_install_error
        .lock()
        .ok()
        .and_then(|mut error| error.take())
}

#[cfg(not(test))]
pub(super) fn take_forced_ssh_reconnect_install_error(_state: &AppState) -> Option<String> {
    None
}

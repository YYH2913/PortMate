use super::*;

pub(super) fn tcp_reconnect_enabled(profile: &SessionProfile) -> bool {
    match &profile.connection {
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => tcp.reconnect,
        _ => false,
    }
}

pub(super) fn tcp_reconnect_attempt_matches_profile(
    attempt: &SessionProfile,
    latest: &SessionProfile,
) -> bool {
    let attempt = normalize_session_profile(attempt.clone());
    let latest = normalize_session_profile(latest.clone());
    tcp_reconnect_enabled(&latest)
        && attempt.connection == latest.connection
        && attempt.terminal == latest.terminal
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TcpReconnectProfileState {
    Current,
    Changed,
    Disabled,
}

pub(super) fn tcp_reconnect_profile_state(
    store: &SessionStore,
    session_id: &str,
    attempt: &SessionProfile,
) -> TcpReconnectProfileState {
    let Some(latest) = store.profile(session_id).map(normalize_session_profile) else {
        return TcpReconnectProfileState::Disabled;
    };
    if !tcp_reconnect_enabled(&latest) {
        return TcpReconnectProfileState::Disabled;
    }
    if !tcp_reconnect_attempt_matches_profile(attempt, &latest) {
        return TcpReconnectProfileState::Changed;
    }
    TcpReconnectProfileState::Current
}

pub(super) fn latest_tcp_reconnect_profile(
    state: &AppState,
    session_id: &str,
) -> Result<Option<SessionProfile>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let Some(profile) = store.profile(session_id) else {
        return Ok(None);
    };
    let profile = normalize_session_profile(profile);
    Ok(tcp_reconnect_enabled(&profile).then_some(profile))
}

pub(super) fn tcp_reconnect_delay(profile: &SessionProfile) -> Duration {
    match &profile.connection {
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => {
            Duration::from_millis(tcp.reconnect_delay_ms)
        }
        _ => Duration::from_millis(portmate_core::DEFAULT_TCP_RECONNECT_DELAY_MS),
    }
}

pub(super) fn tcp_reconnect_pending(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    if closed.load(Ordering::SeqCst) {
        return false;
    }
    let connections = match state.tcp.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    drop(connections);
    match state.store.lock() {
        Ok(store) => store.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
        }),
        Err(error) => {
            let lock_error = error.to_string();
            drop(error);
            fail_pending_tcp_reconnect_install(
                state,
                session_id,
                runtime_id,
                closed,
                "TCP/Telnet",
                &format!("reconnect Store unavailable: {lock_error}"),
            );
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TcpReconnectFailureDisposition {
    Recorded,
    RetryLatestProfile,
    StopDisabled,
    Superseded,
}

pub(super) fn record_tcp_reconnect_failure_if_pending(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    attempt: &SessionProfile,
    label: &str,
    error: &str,
) -> TcpReconnectFailureDisposition {
    if closed.load(Ordering::SeqCst) {
        return TcpReconnectFailureDisposition::Superseded;
    }
    let connections = match state.tcp.lock() {
        Ok(connections) => connections,
        Err(_) => return TcpReconnectFailureDisposition::Superseded,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return TcpReconnectFailureDisposition::Superseded;
    }
    let mut store = match state.store.lock() {
        Ok(store) => store,
        Err(error) => {
            let lock_error = error.to_string();
            drop(error);
            drop(connections);
            fail_pending_tcp_reconnect_install(
                state,
                session_id,
                runtime_id,
                closed,
                label,
                &format!("reconnect Store unavailable: {lock_error}"),
            );
            return TcpReconnectFailureDisposition::Superseded;
        }
    };
    if !store.runtimes.iter().any(|runtime| {
        runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
    }) {
        return TcpReconnectFailureDisposition::Superseded;
    }
    match tcp_reconnect_profile_state(&store, session_id, attempt) {
        TcpReconnectProfileState::Current => {}
        TcpReconnectProfileState::Changed => {
            return TcpReconnectFailureDisposition::RetryLatestProfile;
        }
        TcpReconnectProfileState::Disabled => {
            return TcpReconnectFailureDisposition::StopDisabled;
        }
    }
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Reconnecting,
        Some(format!("{label} reconnect failed: {error}")),
    );
    store.record_system_event(
        session_id,
        format!(
            "PortMate: {label} reconnect failed: {error}; retrying in {}ms",
            tcp_reconnect_delay(attempt).as_millis()
        ),
    );
    if let Err(save_error) = persist_applied_store(
        &store,
        &state.store_path,
        "TCP/Telnet reconnect failure state",
    ) {
        eprintln!("PortMate: failed to persist {label} reconnect failure: {save_error}");
    }
    TcpReconnectFailureDisposition::Recorded
}

pub(super) fn stop_pending_tcp_reconnect_if_disabled(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    reason: &str,
) -> bool {
    let mut connections = match state.tcp.lock() {
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
        .is_none_or(|profile| !tcp_reconnect_enabled(&profile));
    if !reconnect_disabled {
        return false;
    }
    if let Some(runtime) = connections.remove(session_id) {
        runtime.closed.store(true, Ordering::SeqCst);
    }
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Disconnected,
        Some(reason.to_string()),
    );
    store.record_system_event(
        session_id,
        format!("PortMate: TCP/Telnet reconnect stopped: {reason}"),
    );
    if let Err(error) = persist_applied_store(
        &store,
        &state.store_path,
        "stopped TCP/Telnet reconnect state",
    ) {
        eprintln!("PortMate: failed to persist TCP/Telnet reconnect stop: {error}");
    }
    true
}

pub(super) fn fail_pending_tcp_reconnect_install(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    label: &str,
    error: &str,
) {
    closed.store(true, Ordering::SeqCst);
    let mut connections = match state.tcp.lock() {
        Ok(connections) => connections,
        Err(lock_error) => {
            eprintln!(
                "PortMate: failed to clean up {label} reconnect runtime after Store failure: {lock_error}"
            );
            return;
        }
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return;
    }
    if let Some(runtime) = connections.remove(session_id) {
        runtime.closed.store(true, Ordering::SeqCst);
    }

    clear_active_command(&state.session_io(), session_id);
    let reason = portmate_core::normalize_session_disconnect_reason(&format!(
        "{label} reconnect install failed: {error}"
    ))
    .unwrap_or_else(|| format!("{label} reconnect install failed"));
    let mut store = match state.store.lock() {
        Ok(store) => store,
        Err(lock_error) => {
            eprintln!(
                "PortMate: failed to record {label} reconnect install failure: {lock_error}"
            );
            return;
        }
    };
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Error,
        Some(reason.clone()),
    );
    store.record_system_event(session_id, format!("PortMate: {reason}"));
    if let Err(save_error) = persist_applied_store(
        &store,
        &state.store_path,
        "failed TCP/Telnet reconnect install state",
    ) {
        eprintln!(
            "PortMate: failed to persist {label} reconnect install failure: {save_error}"
        );
    }
}

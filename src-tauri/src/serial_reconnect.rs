use super::*;

pub(super) fn serial_reconnect_delay(profile: &SessionProfile) -> Duration {
    match &profile.connection {
        ConnectionConfig::Serial(serial) => Duration::from_millis(serial.reconnect_delay_ms.clamp(
            portmate_core::MIN_SERIAL_RECONNECT_DELAY_MS,
            portmate_core::MAX_SERIAL_RECONNECT_DELAY_MS,
        )),
        _ => Duration::from_millis(portmate_core::DEFAULT_SERIAL_RECONNECT_DELAY_MS),
    }
}

pub(super) fn serial_reconnect_enabled(profile: &SessionProfile) -> bool {
    match &profile.connection {
        ConnectionConfig::Serial(serial) => serial.reconnect,
        _ => false,
    }
}

pub(super) fn serial_reconnect_attempt_matches_profile(
    attempt: &SessionProfile,
    latest: &SessionProfile,
) -> bool {
    let attempt = normalize_session_profile(attempt.clone());
    let latest = normalize_session_profile(latest.clone());
    serial_reconnect_enabled(&latest) && attempt.connection == latest.connection
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SerialReconnectProfileState {
    Current,
    Changed,
    Disabled,
}

pub(super) fn serial_reconnect_profile_state(
    store: &SessionStore,
    session_id: &str,
    attempt: &SessionProfile,
) -> SerialReconnectProfileState {
    let Some(latest) = store.profile(session_id).map(normalize_session_profile) else {
        return SerialReconnectProfileState::Disabled;
    };
    if !serial_reconnect_enabled(&latest) {
        return SerialReconnectProfileState::Disabled;
    }
    if !serial_reconnect_attempt_matches_profile(attempt, &latest) {
        return SerialReconnectProfileState::Changed;
    }
    SerialReconnectProfileState::Current
}

pub(super) fn latest_serial_reconnect_profile(
    io: &SessionIo,
    session_id: &str,
) -> Result<Option<SessionProfile>, String> {
    let store = io.store.lock().map_err(|error| error.to_string())?;
    let Some(profile) = store.profile(session_id) else {
        return Ok(None);
    };
    let profile = normalize_session_profile(profile);
    Ok(serial_reconnect_enabled(&profile).then_some(profile))
}

fn serial_reconnect_pending(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    if closed.load(Ordering::SeqCst) {
        return false;
    }
    let connections = match io.runtimes.serial.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    io.store.lock().ok().is_some_and(|store| {
        store.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SerialReconnectFailureDisposition {
    Recorded,
    RetryLatestProfile,
    StopDisabled,
    Superseded,
}

pub(super) fn record_serial_reconnect_failure_if_pending(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    attempt: &SessionProfile,
    port_name: &str,
    error: &str,
) -> SerialReconnectFailureDisposition {
    if closed.load(Ordering::SeqCst) {
        return SerialReconnectFailureDisposition::Superseded;
    }
    let connections = match io.runtimes.serial.lock() {
        Ok(connections) => connections,
        Err(_) => return SerialReconnectFailureDisposition::Superseded,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return SerialReconnectFailureDisposition::Superseded;
    }
    let mut store = match io.store.lock() {
        Ok(store) => store,
        Err(_) => return SerialReconnectFailureDisposition::Superseded,
    };
    if !store.runtimes.iter().any(|runtime| {
        runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
    }) {
        return SerialReconnectFailureDisposition::Superseded;
    }
    match serial_reconnect_profile_state(&store, session_id, attempt) {
        SerialReconnectProfileState::Current => {}
        SerialReconnectProfileState::Changed => {
            return SerialReconnectFailureDisposition::RetryLatestProfile;
        }
        SerialReconnectProfileState::Disabled => {
            return SerialReconnectFailureDisposition::StopDisabled;
        }
    }
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Reconnecting,
        Some(format!("serial reconnect failed on {port_name}: {error}")),
    );
    store.record_system_event(
        session_id,
        format!(
            "PortMate: serial reconnect failed on {port_name}: {error}; retrying in {}ms",
            serial_reconnect_delay(attempt).as_millis()
        ),
    );
    if let Err(save_error) =
        persist_applied_store(&store, &io.store_path, "serial reconnect failure state")
    {
        eprintln!("PortMate: failed to persist serial reconnect failure: {save_error}");
    }
    SerialReconnectFailureDisposition::Recorded
}

pub(super) fn stop_pending_serial_reconnect_if_disabled(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    reason: &str,
) -> bool {
    let mut connections = match io.runtimes.serial.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    let mut store = match io.store.lock() {
        Ok(store) => store,
        Err(_) => return false,
    };
    let reconnect_disabled = store
        .profile(session_id)
        .map(normalize_session_profile)
        .is_none_or(|profile| !serial_reconnect_enabled(&profile));
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
        format!("PortMate: serial reconnect stopped: {reason}"),
    );
    if let Err(error) =
        persist_applied_store(&store, &io.store_path, "stopped serial reconnect state")
    {
        eprintln!("PortMate: failed to persist serial reconnect stop: {error}");
    }
    true
}

pub(super) fn wait_for_serial_reconnect_attempt(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    let started = Instant::now();
    loop {
        if !serial_reconnect_pending(io, session_id, runtime_id, closed) {
            return false;
        }
        let profile = match latest_serial_reconnect_profile(io, session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_serial_reconnect_if_disabled(
                    io,
                    session_id,
                    runtime_id,
                    "automatic reconnect disabled while waiting for the next attempt",
                ) {
                    return false;
                }
                std::thread::sleep(RECONNECT_DELAY_POLL_INTERVAL);
                continue;
            }
            Err(error) => {
                eprintln!(
                    "PortMate: failed to load serial reconnect delay from latest profile: {error}"
                );
                std::thread::sleep(RECONNECT_DELAY_POLL_INTERVAL);
                continue;
            }
        };
        let remaining = serial_reconnect_delay(&profile).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        std::thread::sleep(remaining.min(RECONNECT_DELAY_POLL_INTERVAL));
    }
}

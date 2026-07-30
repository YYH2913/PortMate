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

fn tcp_reconnect_pending(
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
    state.store.lock().ok().is_some_and(|store| {
        store.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TcpReconnectFailureDisposition {
    Recorded,
    RetryLatestProfile,
    StopDisabled,
    Superseded,
}

fn record_tcp_reconnect_failure_if_pending(
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
        Err(_) => return TcpReconnectFailureDisposition::Superseded,
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

fn stop_pending_tcp_reconnect_if_disabled(
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

fn fail_pending_tcp_reconnect_install(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    label: &str,
    error: &str,
) {
    closed.store(true, Ordering::SeqCst);
    let removed_current = match state.tcp.lock() {
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
                "PortMate: failed to clean up {label} reconnect runtime after Store failure: {lock_error}"
            );
            false
        }
    };
    if !removed_current {
        return;
    }

    clear_active_command(&state.session_io(), session_id);
    let reason = portmate_core::normalize_session_disconnect_reason(&format!(
        "{label} reconnect install failed: {error}"
    ))
    .unwrap_or_else(|| format!("{label} reconnect install failed"));
    if let Ok(mut store) = state.store.lock() {
        let _ = store.set_runtime_status_with_reason(
            session_id,
            SessionStatus::Error,
            Some(reason.clone()),
        );
        store.record_system_event(session_id, format!("PortMate: {reason}"));
    }
}

enum TcpReconnectInstallDecision {
    Installed,
    Retry,
    Stop,
    Superseded,
    Failed(String),
}

async fn wait_for_tcp_reconnect_attempt(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    let started = Instant::now();
    loop {
        if !tcp_reconnect_pending(state, session_id, runtime_id, closed) {
            return false;
        }
        let profile = match latest_tcp_reconnect_profile(state, session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_tcp_reconnect_if_disabled(
                    state,
                    session_id,
                    runtime_id,
                    "automatic reconnect disabled while waiting for the next attempt",
                ) {
                    return false;
                }
                tokio::time::sleep(RECONNECT_DELAY_POLL_INTERVAL).await;
                continue;
            }
            Err(error) => {
                eprintln!(
                    "PortMate: failed to load TCP/Telnet reconnect delay from latest profile: {error}"
                );
                tokio::time::sleep(RECONNECT_DELAY_POLL_INTERVAL).await;
                continue;
            }
        };
        let remaining = tcp_reconnect_delay(&profile).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        tokio::time::sleep(remaining.min(RECONNECT_DELAY_POLL_INTERVAL)).await;
    }
}

pub(super) async fn reconnect_tcp_session(
    state: AppState,
    session_id: String,
    previous_runtime_id: String,
    closed: Arc<AtomicBool>,
) {
    loop {
        if !wait_for_tcp_reconnect_attempt(
            &state,
            &session_id,
            &previous_runtime_id,
            closed.as_ref(),
        )
        .await
        {
            return;
        }

        let profile = match latest_tcp_reconnect_profile(&state, &session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_tcp_reconnect_if_disabled(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    "automatic reconnect disabled by latest profile",
                ) {
                    return;
                }
                continue;
            }
            Err(error) => {
                eprintln!("PortMate: failed to load latest TCP/Telnet reconnect profile: {error}");
                continue;
            }
        };
        let (tcp, label) = match tcp_connection_details(&profile) {
            Ok(details) => details,
            Err(error) => {
                match record_tcp_reconnect_failure_if_pending(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    &profile,
                    "TCP/Telnet",
                    &error,
                ) {
                    TcpReconnectFailureDisposition::Recorded
                    | TcpReconnectFailureDisposition::RetryLatestProfile => continue,
                    TcpReconnectFailureDisposition::StopDisabled => {
                        if stop_pending_tcp_reconnect_if_disabled(
                            &state,
                            &session_id,
                            &previous_runtime_id,
                            "automatic reconnect disabled while validating the latest profile",
                        ) {
                            return;
                        }
                        continue;
                    }
                    TcpReconnectFailureDisposition::Superseded => return,
                }
            }
        };

        let stream = match connect_tcp_transport(&tcp, label).await {
            Ok(stream) => stream,
            Err(error) => {
                match record_tcp_reconnect_failure_if_pending(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    &profile,
                    label,
                    &error,
                ) {
                    TcpReconnectFailureDisposition::Recorded
                    | TcpReconnectFailureDisposition::RetryLatestProfile => continue,
                    TcpReconnectFailureDisposition::StopDisabled => {
                        if stop_pending_tcp_reconnect_if_disabled(
                            &state,
                            &session_id,
                            &previous_runtime_id,
                            "automatic reconnect disabled while the previous attempt was running",
                        ) {
                            return;
                        }
                        continue;
                    }
                    TcpReconnectFailureDisposition::Superseded => return,
                }
            }
        };

        let runtime_id = Uuid::new_v4().to_string();
        let (read_half, write_half) = stream.split();
        let writer = Arc::new(tokio::sync::Mutex::new(write_half));
        let (tap, _) = broadcast::channel(1024);
        let next_closed = Arc::new(AtomicBool::new(false));
        let telnet = TelnetRuntimeState::from_profile(&profile);
        let install = match state.tcp.lock() {
            Err(error) => TcpReconnectInstallDecision::Failed(error.to_string()),
            Ok(mut connections) => {
                if connections
                    .get(&session_id)
                    .is_none_or(|runtime| runtime.runtime_id != previous_runtime_id)
                    || closed.load(Ordering::SeqCst)
                {
                    TcpReconnectInstallDecision::Superseded
                } else {
                    match state.store.lock() {
                        Err(error) => TcpReconnectInstallDecision::Failed(error.to_string()),
                        Ok(mut store) => {
                            match tcp_reconnect_profile_state(&store, &session_id, &profile) {
                                TcpReconnectProfileState::Changed => {
                                    TcpReconnectInstallDecision::Retry
                                }
                                TcpReconnectProfileState::Disabled => {
                                    if let Some(runtime) = connections.remove(&session_id) {
                                        runtime.closed.store(true, Ordering::SeqCst);
                                    }
                                    let reason = "automatic reconnect disabled while the previous attempt was running";
                                    let _ = store.set_runtime_status_with_reason(
                                        &session_id,
                                        SessionStatus::Disconnected,
                                        Some(reason.to_string()),
                                    );
                                    store.record_system_event(
                                        &session_id,
                                        format!("PortMate: TCP/Telnet reconnect stopped: {reason}"),
                                    );
                                    if let Err(error) = persist_applied_store(
                                        &store,
                                        &state.store_path,
                                        "stopped TCP/Telnet reconnect state",
                                    ) {
                                        eprintln!(
                                            "PortMate: failed to persist TCP/Telnet reconnect stop: {error}"
                                        );
                                    }
                                    TcpReconnectInstallDecision::Stop
                                }
                                TcpReconnectProfileState::Current => {
                                    let committed = commit_tracked_store_mutation(
                                        &mut store,
                                        &state.store_path,
                                        |next_store| {
                                            mark_session_connected_with_events(
                                                next_store,
                                                &profile,
                                                [format!("PortMate: {label} socket reconnected")],
                                            )
                                        },
                                    );
                                    match committed {
                                        Ok(_) => {
                                            connections.insert(
                                                session_id.clone(),
                                                TcpRuntime {
                                                    runtime_id: runtime_id.clone(),
                                                    writer: Arc::clone(&writer),
                                                    tap: tap.clone(),
                                                    closed: Arc::clone(&next_closed),
                                                    telnet: telnet.as_ref().map(Arc::clone),
                                                },
                                            );
                                            TcpReconnectInstallDecision::Installed
                                        }
                                        Err(error) => TcpReconnectInstallDecision::Failed(error),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };

        if !matches!(install, TcpReconnectInstallDecision::Installed) {
            let mut writer = writer.lock().await;
            let _ = writer.shutdown().await;
            drop(writer);
            match install {
                TcpReconnectInstallDecision::Retry => continue,
                TcpReconnectInstallDecision::Stop | TcpReconnectInstallDecision::Superseded => {
                    return
                }
                TcpReconnectInstallDecision::Failed(error) => {
                    fail_pending_tcp_reconnect_install(
                        &state,
                        &session_id,
                        &previous_runtime_id,
                        closed.as_ref(),
                        label,
                        &error,
                    );
                    eprintln!("PortMate: failed to install TCP/Telnet reconnect runtime: {error}");
                    return;
                }
                TcpReconnectInstallDecision::Installed => unreachable!(),
            }
        }

        tauri::async_runtime::spawn(read_tcp_stream(TcpReadTask {
            state: state.clone(),
            profile,
            runtime_id,
            label: label.to_string(),
            tap,
            writer,
            read_half,
            closed: next_closed,
            telnet,
        }));
        return;
    }
}

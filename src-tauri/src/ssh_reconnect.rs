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
    let stopped_tunnels =
        fail_session_tunnel_runtimes(&state.tunnels, session_id, reason).unwrap_or_default();
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

pub(super) enum SshReconnectInstallDecision {
    Installed(Box<SessionProfile>),
    Retry,
    Stop,
    Superseded,
    Failed(String),
}

pub(super) async fn wait_for_ssh_reconnect_attempt(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    let started = Instant::now();
    loop {
        if !ssh_reconnect_pending(state, session_id, runtime_id, closed) {
            return false;
        }
        let profile = match latest_ssh_reconnect_profile(state, session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_ssh_reconnect_if_disabled(
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
                    "PortMate: failed to load SSH reconnect delay from latest profile: {error}"
                );
                tokio::time::sleep(RECONNECT_DELAY_POLL_INTERVAL).await;
                continue;
            }
        };
        let remaining = ssh_reconnect_delay(&profile).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        tokio::time::sleep(remaining.min(RECONNECT_DELAY_POLL_INTERVAL)).await;
    }
}

pub(super) async fn reconnect_ssh_session(
    state: AppState,
    session_id: String,
    previous_runtime_id: String,
    closed: Arc<AtomicBool>,
) {
    loop {
        if !wait_for_ssh_reconnect_attempt(
            &state,
            &session_id,
            &previous_runtime_id,
            closed.as_ref(),
        )
        .await
        {
            return;
        }

        let profile = match latest_ssh_reconnect_profile(&state, &session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_ssh_reconnect_if_disabled(
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
                if record_ssh_reconnect_failure_if_pending(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    None,
                    &error,
                ) == SshReconnectFailureDisposition::Superseded
                {
                    return;
                }
                continue;
            }
        };
        let established = match establish_ssh_reconnect_runtime(&state, &profile).await {
            Ok(established) => established,
            Err(error) => {
                match record_ssh_reconnect_failure_if_pending(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    Some(&profile),
                    &error,
                ) {
                    SshReconnectFailureDisposition::Recorded
                    | SshReconnectFailureDisposition::RetryLatestProfile => continue,
                    SshReconnectFailureDisposition::StopDisabled => {
                        if stop_pending_ssh_reconnect_if_disabled(
                            &state,
                            &session_id,
                            &previous_runtime_id,
                            "automatic reconnect disabled while the previous attempt was running",
                        ) {
                            return;
                        }
                        continue;
                    }
                    SshReconnectFailureDisposition::Superseded => return,
                }
            }
        };
        let EstablishedSshRuntime {
            runtime_id,
            runtime,
            tap,
            read_half,
            auth_method,
            closed: next_closed,
            reader_finished,
        } = established;
        let mut runtime = Some(runtime);
        let install = match state.ssh.lock() {
            Err(error) => SshReconnectInstallDecision::Failed(error.to_string()),
            Ok(mut connections) => {
                if connections
                    .get(&session_id)
                    .is_none_or(|runtime| runtime.runtime_id != previous_runtime_id)
                    || closed.load(Ordering::SeqCst)
                {
                    SshReconnectInstallDecision::Superseded
                } else {
                    match state.store.lock() {
                        Err(error) => SshReconnectInstallDecision::Failed(error.to_string()),
                        Ok(mut store) => {
                            let latest = store.profile(&session_id).map(normalize_session_profile);
                            match latest {
                                Some(latest) if !ssh_reconnect_enabled(&latest) => {
                                    SshReconnectInstallDecision::Stop
                                }
                                Some(latest)
                                    if !ssh_reconnect_attempt_matches_profile(
                                        &profile, &latest,
                                    ) =>
                                {
                                    SshReconnectInstallDecision::Retry
                                }
                                Some(latest) => {
                                    let committed =
                                        match take_forced_ssh_reconnect_install_error(&state) {
                                            Some(error) => Err(error),
                                            None => commit_tracked_store_mutation(
                                                &mut store,
                                                &state.store_path,
                                                |next_store| {
                                                    next_store.record_auth_success(
                                                        &session_id,
                                                        auth_method,
                                                    )?;
                                                    mark_session_connected_with_events(
                                                        next_store,
                                                        &latest,
                                                        [],
                                                    )
                                                },
                                            ),
                                        };
                                    match committed {
                                        Ok(_) => {
                                            connections.insert(
                                                session_id.clone(),
                                                runtime.take().expect("runtime present"),
                                            );
                                            SshReconnectInstallDecision::Installed(Box::new(latest))
                                        }
                                        Err(error) => SshReconnectInstallDecision::Failed(error),
                                    }
                                }
                                None => SshReconnectInstallDecision::Stop,
                            }
                        }
                    }
                }
            }
        };

        let installed_profile = match install {
            SshReconnectInstallDecision::Installed(profile) => *profile,
            SshReconnectInstallDecision::Retry => {
                disconnect_ssh_runtime(
                    runtime.take().expect("uninstalled runtime present"),
                    "PortMate SSH reconnect profile changed",
                )
                .await;
                continue;
            }
            SshReconnectInstallDecision::Stop => {
                disconnect_ssh_runtime(
                    runtime.take().expect("uninstalled runtime present"),
                    "PortMate SSH reconnect disabled",
                )
                .await;
                if stop_pending_ssh_reconnect_if_disabled(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    "automatic reconnect disabled by latest profile",
                ) {
                    return;
                }
                continue;
            }
            SshReconnectInstallDecision::Superseded => {
                disconnect_ssh_runtime(
                    runtime.take().expect("uninstalled runtime present"),
                    "PortMate SSH reconnect superseded",
                )
                .await;
                return;
            }
            SshReconnectInstallDecision::Failed(error) => {
                disconnect_ssh_runtime(
                    runtime.take().expect("uninstalled runtime present"),
                    "PortMate SSH reconnect state unavailable",
                )
                .await;
                fail_pending_ssh_reconnect_install(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    &error,
                );
                eprintln!("PortMate: failed to install SSH reconnect runtime: {error}");
                return;
            }
        };

        tauri::async_runtime::spawn(read_ssh_channel(SshReadTask {
            state: state.clone(),
            profile: installed_profile,
            runtime_id: runtime_id.clone(),
            tap,
            read_half,
            closed: Arc::clone(&next_closed),
            reader_finished,
        }));

        let one_time_cleanup_error = take_one_time_host_keys(&state, &session_id).err();
        let (restored_tunnels, failed_tunnels) =
            restore_enabled_tunnels(&state, &session_id, &runtime_id).await;

        let connections = match state.ssh.lock() {
            Ok(connections) => connections,
            Err(_) => return,
        };
        if connections
            .get(&session_id)
            .is_none_or(|runtime| runtime.runtime_id != runtime_id)
            || next_closed.load(Ordering::SeqCst)
        {
            return;
        }
        if let Ok(mut store) = state.store.lock() {
            if !store.runtimes.iter().any(|runtime| {
                runtime.session_id == session_id && runtime.status == SessionStatus::Connected
            }) {
                return;
            }
            store.record_system_event(
                &session_id,
                format!(
                    "PortMate: SSH session reconnected via {auth_method:?}; restored {restored_tunnels} tunnel(s), {failed_tunnels} failed"
                ),
            );
            if let Some(error) = one_time_cleanup_error {
                store.record_system_event(
                    &session_id,
                    format!("PortMate: failed to consume one-time host key trust: {error}"),
                );
            }
            if let Err(error) =
                persist_applied_store(&store, &state.store_path, "completed SSH reconnect state")
            {
                eprintln!("PortMate: failed to persist SSH reconnect success: {error}");
            }
        }
        return;
    }
}

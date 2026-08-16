use super::transport_timing::RECONNECT_DELAY_POLL_INTERVAL;
use super::*;

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
            terminal_channel_open,
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
                    read_half,
                    reader_finished,
                    "PortMate SSH reconnect profile changed",
                )
                .await;
                continue;
            }
            SshReconnectInstallDecision::Stop => {
                disconnect_ssh_runtime(
                    runtime.take().expect("uninstalled runtime present"),
                    read_half,
                    reader_finished,
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
                    read_half,
                    reader_finished,
                    "PortMate SSH reconnect superseded",
                )
                .await;
                return;
            }
            SshReconnectInstallDecision::Failed(error) => {
                disconnect_ssh_runtime(
                    runtime.take().expect("uninstalled runtime present"),
                    read_half,
                    reader_finished,
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
            terminal_channel_open,
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

use super::*;

pub(super) const MAX_ACTIVE_TMUX_CONTROLS: usize = 256;
pub(super) const MAX_TMUX_CONTROLS_PER_SESSION: usize = 64;
const MAX_TMUX_CONTROL_STDERR_BYTES: usize = 64 * 1024;
const TMUX_CONTROL_EVENT_DEBOUNCE: Duration = Duration::from_millis(120);
const TMUX_CONTROL_EVENT_MAX_LATENCY: Duration = Duration::from_secs(1);

pub(super) type TmuxControlMap = Arc<Mutex<HashMap<(String, String), TmuxControlRuntime>>>;

#[derive(Clone)]
pub(super) struct TmuxControlRuntime {
    pub(super) runtime_id: String,
    pub(super) target: String,
    pub(super) cancel: Arc<AtomicBool>,
}

pub(super) enum TmuxControlInstall {
    Existing(TmuxControlRuntime),
    Installed(Option<TmuxControlRuntime>),
}

struct TmuxControlEventContext {
    session_id: String,
    target: String,
    runtime_id: String,
}

impl TmuxControlEventContext {
    fn emit(
        &self,
        app_handle: Option<&AppHandle>,
        kind: &str,
        active: bool,
        protocol_event: Option<&str>,
        error: Option<&str>,
    ) {
        let Some(app_handle) = app_handle else {
            return;
        };
        let _ = app_handle.emit(
            "portmate-tmux-control-event",
            TmuxControlEvent {
                session_id: self.session_id.clone(),
                target: self.target.clone(),
                kind: kind.to_string(),
                active,
                runtime_id: self.runtime_id.clone(),
                protocol_event: protocol_event.map(str::to_string),
                error: error.map(bounded_tmux_control_error),
            },
        );
    }
}

pub(super) fn ensure_tmux_control_capacity(
    controls: &HashMap<(String, String), TmuxControlRuntime>,
    control_key: &(String, String),
) -> Result<(), String> {
    if controls.contains_key(control_key) {
        return Ok(());
    }
    let session_count = controls
        .keys()
        .filter(|(session_id, _)| session_id == &control_key.0)
        .count();
    if session_count >= MAX_TMUX_CONTROLS_PER_SESSION {
        return Err(format!(
            "tmux control watcher count for session has reached {MAX_TMUX_CONTROLS_PER_SESSION}"
        ));
    }
    if controls.len() >= MAX_ACTIVE_TMUX_CONTROLS {
        return Err(format!(
            "tmux control watcher count has reached app limit ({MAX_ACTIVE_TMUX_CONTROLS})"
        ));
    }
    Ok(())
}

pub(super) fn install_tmux_control_runtime(
    controls: &mut HashMap<(String, String), TmuxControlRuntime>,
    control_key: &(String, String),
    runtime: TmuxControlRuntime,
) -> Result<TmuxControlInstall, String> {
    if let Some(existing) = controls
        .get(control_key)
        .filter(|existing| !existing.cancel.load(Ordering::SeqCst))
    {
        return Ok(TmuxControlInstall::Existing(existing.clone()));
    }
    ensure_tmux_control_capacity(controls, control_key)?;
    Ok(TmuxControlInstall::Installed(
        controls.insert(control_key.clone(), runtime),
    ))
}

pub(super) async fn start_tmux_control_inner(
    state: &AppState,
    session_id: &str,
    target: &str,
) -> Result<TmuxControlStatus, String> {
    let target = normalize_tmux_target(target)?.to_string();
    let control_key = (session_id.to_string(), target.clone());
    {
        let controls = state
            .tmux_controls
            .lock()
            .map_err(|error| error.to_string())?;
        if let Some(runtime) = controls
            .get(&control_key)
            .filter(|runtime| !runtime.cancel.load(Ordering::SeqCst))
        {
            return Ok(TmuxControlStatus {
                session_id: session_id.to_string(),
                target,
                active: true,
                runtime_id: Some(runtime.runtime_id.clone()),
            });
        }
        ensure_tmux_control_capacity(&controls, &control_key)?;
    }

    let control_slot = Arc::clone(&state.tmux_control_slots)
        .try_acquire_owned()
        .map_err(|_| format!("tmux control watcher limit reached ({MAX_ACTIVE_TMUX_CONTROLS})"))?;
    {
        let controls = state
            .tmux_controls
            .lock()
            .map_err(|error| error.to_string())?;
        if let Some(runtime) = controls
            .get(&control_key)
            .filter(|runtime| !runtime.cancel.load(Ordering::SeqCst))
        {
            return Ok(TmuxControlStatus {
                session_id: session_id.to_string(),
                target,
                active: true,
                runtime_id: Some(runtime.runtime_id.clone()),
            });
        }
        ensure_tmux_control_capacity(&controls, &control_key)?;
    }

    let auxiliary_lease = ssh_auxiliary_lease(state, session_id)?;
    let handle = auxiliary_lease.handle();
    let command = format!(
        "tmux -C attach-session -t {}",
        shell_quote(normalize_tmux_target(&target)?)
    );
    let mut channel = open_shared_ssh_exec_channel(
        &handle,
        &command,
        SSH_AUXILIARY_SETUP_TIMEOUT,
        "Tmux control-mode",
    )
    .await?;

    let runtime_id = Uuid::new_v4().to_string();
    let cancel = Arc::new(AtomicBool::new(false));
    let install_result = match state.tmux_controls.lock() {
        Ok(mut controls) => install_tmux_control_runtime(
            &mut controls,
            &control_key,
            TmuxControlRuntime {
                runtime_id: runtime_id.clone(),
                target: target.clone(),
                cancel: Arc::clone(&cancel),
            },
        ),
        Err(error) => Err(error.to_string()),
    };
    let previous = match install_result {
        Ok(TmuxControlInstall::Installed(previous)) => previous,
        Ok(TmuxControlInstall::Existing(existing)) => {
            close_ssh_channel_bounded(&channel).await;
            return Ok(TmuxControlStatus {
                session_id: session_id.to_string(),
                target,
                active: true,
                runtime_id: Some(existing.runtime_id),
            });
        }
        Err(error) => {
            close_ssh_channel_bounded(&channel).await;
            return Err(error);
        }
    };
    if let Some(previous) = previous {
        previous.cancel.store(true, Ordering::SeqCst);
    }

    let app_handle = state.app_handle.clone();
    let controls = Arc::clone(&state.tmux_controls);
    let event_context = TmuxControlEventContext {
        session_id: session_id.to_string(),
        target: target.clone(),
        runtime_id: runtime_id.clone(),
    };
    tauri::async_runtime::spawn(async move {
        event_context.emit(app_handle.as_ref(), "started", true, None, None);
        let mut parser = TmuxControlLineParser::default();
        let mut stderr = Vec::new();
        let mut pending_protocol_event = None;
        let mut first_pending_at = None;
        let mut last_pending_at = None;
        let mut stopped_error = None;
        let mut ticker = tokio::time::interval(Duration::from_millis(100));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let now = Instant::now();
                    let quiet = last_pending_at
                        .is_some_and(|last: Instant| now.duration_since(last) >= TMUX_CONTROL_EVENT_DEBOUNCE);
                    let overdue = first_pending_at
                        .is_some_and(|first: Instant| now.duration_since(first) >= TMUX_CONTROL_EVENT_MAX_LATENCY);
                    if pending_protocol_event.is_some() && (quiet || overdue) {
                        event_context.emit(
                            app_handle.as_ref(),
                            "state-changed",
                            true,
                            pending_protocol_event,
                            None,
                        );
                        pending_protocol_event = None;
                        first_pending_at = None;
                        last_pending_at = None;
                    }
                }
                message = channel.wait() => {
                    match message {
                        Some(SshBackendMessage::Data(data)) => match parser.push(&data) {
                            Ok(parsed) if parsed.changed => {
                                let now = Instant::now();
                                if first_pending_at.is_none() {
                                    first_pending_at = Some(now);
                                }
                                last_pending_at = Some(now);
                                pending_protocol_event = parsed.last_event;
                            }
                            Ok(_) => {}
                            Err(error) => {
                                stopped_error = Some(error);
                                break;
                            }
                        },
                        Some(SshBackendMessage::ExtendedData { data, .. }) => {
                            if let Err(error) = append_bounded_ssh_exec_data(
                                &mut stderr,
                                &data,
                                MAX_TMUX_CONTROL_STDERR_BYTES,
                                "tmux control stderr",
                            ) {
                                stopped_error = Some(error);
                                break;
                            }
                        }
                        Some(SshBackendMessage::Failure) => {
                            stopped_error = Some("远端拒绝启动 tmux control-mode".to_string());
                            break;
                        }
                        Some(SshBackendMessage::ExitStatus(exit_status)) if exit_status != 0 => {
                            let detail = String::from_utf8_lossy(&stderr);
                            stopped_error = Some(if detail.trim().is_empty() {
                                format!("tmux control-mode 返回非零状态 {exit_status}")
                            } else {
                                format!("tmux control-mode 返回非零状态 {exit_status}: {}", detail.trim())
                            });
                            break;
                        }
                        Some(SshBackendMessage::Error(error)) => {
                            stopped_error = Some(format!("tmux control-mode SSH read failed: {error}"));
                            break;
                        }
                        Some(SshBackendMessage::Eof | SshBackendMessage::Close) | None => break,
                        _ => {}
                    }
                }
            }
        }

        if pending_protocol_event.is_some() {
            event_context.emit(
                app_handle.as_ref(),
                "state-changed",
                true,
                pending_protocol_event,
                None,
            );
        }
        close_ssh_channel_bounded(&channel).await;
        if let Ok(mut controls) = controls.lock() {
            let current = controls
                .get(&control_key)
                .is_some_and(|runtime| runtime.runtime_id == event_context.runtime_id);
            if current {
                controls.remove(&control_key);
            }
        }
        event_context.emit(
            app_handle.as_ref(),
            "stopped",
            false,
            None,
            stopped_error.as_deref(),
        );
        drop(auxiliary_lease);
        drop(control_slot);
    });

    Ok(TmuxControlStatus {
        session_id: session_id.to_string(),
        target,
        active: true,
        runtime_id: Some(runtime_id),
    })
}

pub(super) fn stop_tmux_control_inner(
    state: &AppState,
    session_id: &str,
    target: Option<&str>,
) -> Result<TmuxControlStatus, String> {
    let (target, runtime_id) = if let Some(target) = target {
        let target = normalize_tmux_target(target)?.to_string();
        let runtime = cancel_tmux_control_runtime(state, session_id, &target)?;
        (target, runtime.map(|runtime| runtime.runtime_id))
    } else {
        let mut runtimes = cancel_tmux_control_runtimes_for_session(state, session_id)?;
        let target = if runtimes.len() == 1 {
            runtimes[0].target.clone()
        } else {
            String::new()
        };
        let runtime_id = if runtimes.len() == 1 {
            Some(runtimes.remove(0).runtime_id)
        } else {
            None
        };
        (target, runtime_id)
    };
    Ok(TmuxControlStatus {
        session_id: session_id.to_string(),
        target,
        active: false,
        runtime_id,
    })
}

pub(super) fn cancel_tmux_control_runtime(
    state: &AppState,
    session_id: &str,
    target: &str,
) -> Result<Option<TmuxControlRuntime>, String> {
    let controls = state
        .tmux_controls
        .lock()
        .map_err(|error| error.to_string())?;
    let runtime = controls
        .get(&(session_id.to_string(), target.to_string()))
        .cloned();
    if let Some(runtime) = &runtime {
        runtime.cancel.store(true, Ordering::SeqCst);
    }
    Ok(runtime)
}

pub(super) fn cancel_tmux_control_runtimes_for_session(
    state: &AppState,
    session_id: &str,
) -> Result<Vec<TmuxControlRuntime>, String> {
    let controls = state
        .tmux_controls
        .lock()
        .map_err(|error| error.to_string())?;
    let runtimes = controls
        .iter()
        .filter(|((candidate_session_id, _), _)| candidate_session_id == session_id)
        .map(|(_, runtime)| runtime.clone())
        .collect::<Vec<_>>();
    for runtime in &runtimes {
        runtime.cancel.store(true, Ordering::SeqCst);
    }
    Ok(runtimes)
}

pub(super) fn shutdown_tmux_controls(state: &AppState) {
    if let Ok(mut controls) = state.tmux_controls.lock() {
        for (_, runtime) in controls.drain() {
            runtime.cancel.store(true, Ordering::SeqCst);
        }
    }
}

pub(super) async fn list_tmux_state_inner(
    state: &AppState,
    session_id: &str,
) -> Result<TmuxState, String> {
    let auxiliary = ssh_auxiliary_lease(state, session_id)?;
    list_tmux_state_with_handle(auxiliary.handle()).await
}

async fn list_tmux_state_with_handle(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
) -> Result<TmuxState, String> {
    let sessions_output = exec_ssh_command_capture(
        Arc::clone(&handle),
        &format!(
            "tmux list-sessions -F '#{{session_name}}{TMUX_FIELD_SEPARATOR}#{{session_windows}}{TMUX_FIELD_SEPARATOR}#{{session_attached}}{TMUX_FIELD_SEPARATOR}#{{session_created}}' 2>/dev/null || true"
        ),
        Duration::from_secs(8),
    )
    .await?;
    let windows_output = exec_ssh_command_capture(
        Arc::clone(&handle),
        &format!(
            "tmux list-windows -a -F '#{{session_name}}{TMUX_FIELD_SEPARATOR}#{{window_index}}{TMUX_FIELD_SEPARATOR}#{{window_id}}{TMUX_FIELD_SEPARATOR}#{{window_name}}{TMUX_FIELD_SEPARATOR}#{{window_panes}}{TMUX_FIELD_SEPARATOR}#{{window_active}}' 2>/dev/null || true"
        ),
        Duration::from_secs(8),
    )
    .await?;
    let panes_output = exec_ssh_command_capture(
        handle,
        &format!(
            "tmux list-panes -a -F '#{{session_name}}{TMUX_FIELD_SEPARATOR}#{{window_index}}{TMUX_FIELD_SEPARATOR}#{{pane_index}}{TMUX_FIELD_SEPARATOR}#{{pane_id}}{TMUX_FIELD_SEPARATOR}#{{pane_active}}{TMUX_FIELD_SEPARATOR}#{{pane_current_command}}{TMUX_FIELD_SEPARATOR}#{{pane_title}}{TMUX_FIELD_SEPARATOR}#{{pane_synchronized}}' 2>/dev/null || true"
        ),
        Duration::from_secs(8),
    )
    .await?;

    let sessions = sessions_output
        .lines()
        .filter_map(parse_tmux_session)
        .collect::<Vec<_>>();
    let mut windows = windows_output
        .lines()
        .filter_map(parse_tmux_window)
        .collect::<Vec<_>>();
    let panes = panes_output
        .lines()
        .filter_map(parse_tmux_pane)
        .collect::<Vec<_>>();
    for window in &mut windows {
        let matching = panes
            .iter()
            .filter(|pane| {
                pane.session == window.session && pane.window_index == window.window_index
            })
            .collect::<Vec<_>>();
        window.synchronized = !matching.is_empty() && matching.iter().all(|pane| pane.synchronized);
    }
    Ok(TmuxState {
        sessions,
        windows,
        panes,
    })
}

pub(super) async fn set_tmux_pane_sync_inner(
    state: &AppState,
    session_id: &str,
    target: &str,
    enabled: bool,
) -> Result<TmuxState, String> {
    let auxiliary = ssh_auxiliary_lease(state, session_id)?;
    let handle = auxiliary.handle();
    let command = tmux_pane_sync_command(target, enabled)?;
    let event_message = format!(
        "PortMate: tmux pane synchronization {} ({})",
        if enabled { "enabled" } else { "disabled" },
        normalize_tmux_target(target)?
    );
    exec_ssh_command_capture(Arc::clone(&handle), &command, Duration::from_secs(8)).await?;
    record_applied_system_event(
        state,
        session_id,
        event_message,
        "tmux pane synchronization",
    );
    list_tmux_state_with_handle(handle).await
}

pub(super) async fn mutate_tmux_inner(
    state: &AppState,
    request: TmuxMutationRequest,
) -> Result<TmuxState, String> {
    let command = tmux_mutation_command(&request)?;
    let event_message = format!(
        "PortMate: tmux {} ({})",
        tmux_mutation_label(request.action),
        tmux_mutation_event_scope(&request)?
    );
    let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
    let handle = auxiliary.handle();
    exec_ssh_command_capture(Arc::clone(&handle), &command, Duration::from_secs(8)).await?;
    record_applied_system_event(state, &request.session_id, event_message, "tmux mutation");
    list_tmux_state_with_handle(handle).await
}

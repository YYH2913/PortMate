use super::*;

mod control;

pub(super) use control::*;

pub(super) async fn attach_tmux_inner(
    state: &AppState, session_id: &str, target: &str, validation: Option<CommitValidation>,
) -> Result<SessionEvent, String> {
    // Open a PTY exec channel; never type shell syntax into an unknown foreground
    // program. Replacing only this terminal channel detaches only our tmux client.
    let command = tmux_attach_command(target)?;
    let lane = session_open::session_lifecycle_lane(state, session_id)?;
    let _lifecycle = lane.lock().await;
    let io = state.session_io();
    let _outbound = acquire_outbound_lane(&io.store_path, session_id).await?;
    if let Some(validate) = validation { validate()?; }
    if state.store.lock().map_err(|e| e.to_string())?.transfers.iter().any(|transfer|
        transfer.session_id == session_id
        && matches!(transfer.protocol, TransferProtocol::Xmodem | TransferProtocol::Ymodem | TransferProtocol::Zmodem | TransferProtocol::Tftp)
        && matches!(transfer.status, TransferStatus::Queued | TransferStatus::Running)) {
        return Err("Finish or cancel the terminal transfer before attaching tmux".into());
    }
    let (runtime_id, auxiliary) = tmux_auxiliary_lease(state, session_id)?;
    let profile = state.store.lock().map_err(|e| e.to_string())?.profile(session_id)
        .ok_or_else(|| "Session no longer exists".to_string())?;
    let handle = auxiliary.handle();
    let (reader, writer) = tokio::time::timeout(SSH_RUNTIME_OPERATION_TIMEOUT, async {
        let handle = handle.lock().await;
        handle.open_terminal_exec(&command, &profile.terminal.term, profile.terminal.cols, profile.terminal.rows).await
    }).await.map_err(|_| "Tmux terminal setup timed out".to_string())??;
    let (finished_tx, finished_rx) = tokio::sync::oneshot::channel();
    let open = Arc::new(AtomicBool::new(true));
    let (old_writer, closed, tap) = {
        let mut connections = state.ssh.lock().map_err(|e| e.to_string())?;
        let runtime = connections.get_mut(session_id).filter(|r| r.runtime_id == runtime_id && !r.closed.load(Ordering::SeqCst))
            .ok_or_else(|| "SSH connection changed during tmux attach".to_string())?;
        runtime.terminal_channel_open.store(false, Ordering::SeqCst);
        runtime.terminal_channel_open = Arc::clone(&open);
        runtime.reader_finished = finished_rx;
        let old = std::mem::replace(&mut runtime.writer, Arc::new(tokio::sync::Mutex::new(writer)));
        (old, Arc::clone(&runtime.closed), runtime.tap.clone())
    };
    clear_interactive_write_queue(&io.store_path, session_id);
    clear_active_command(&io, session_id);
    if let Ok(mut store) = state.store.lock() { store.reset_terminal_screen(session_id); }
    tauri::async_runtime::spawn(read_ssh_channel(SshReadTask {
        state: state.clone(), profile, runtime_id, tap, read_half: reader, closed,
        terminal_channel_open: open, reader_finished: finished_tx,
    }));
    let _ = tokio::time::timeout(SSH_TERMINAL_WRITE_TIMEOUT, async { old_writer.lock().await.close().await }).await;
    Ok(record_outbound_control_event(&io, session_id, command.as_bytes(), "tmux-attach", None, false))
}

pub(super) async fn list_tmux_state_inner(
    state: &AppState,
    session_id: &str,
) -> Result<TmuxState, String> {
    let (runtime_id, auxiliary) = tmux_auxiliary_lease(state, session_id)?;
    let tmux_state = list_tmux_state_with_handle(auxiliary.handle()).await?;
    ensure_tmux_runtime_current(state, session_id, &runtime_id)?;
    Ok(tmux_state)
}

fn tmux_auxiliary_lease(
    state: &AppState,
    session_id: &str,
) -> Result<(String, SshAuxiliaryLease), String> {
    let (runtime_id, auxiliary) =
        ssh_auxiliary_lease_with_runtime(state, session_id, |runtime| {
            Ok(runtime.runtime_id.clone())
        })?;
    ensure_tmux_runtime_current(state, session_id, &runtime_id)?;
    Ok((runtime_id, auxiliary))
}

pub(super) fn ensure_tmux_runtime_current(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
) -> Result<(), String> {
    if ssh_runtime_connected(state, session_id, runtime_id) {
        Ok(())
    } else {
        Err("SSH runtime 在 Tmux 操作期间已变化，请重试".to_string())
    }
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
    let (runtime_id, auxiliary) = tmux_auxiliary_lease(state, session_id)?;
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
    let tmux_state = list_tmux_state_with_handle(handle).await?;
    ensure_tmux_runtime_current(state, session_id, &runtime_id)?;
    Ok(tmux_state)
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
    let (runtime_id, auxiliary) = tmux_auxiliary_lease(state, &request.session_id)?;
    let handle = auxiliary.handle();
    exec_ssh_command_capture(Arc::clone(&handle), &command, Duration::from_secs(8)).await?;
    record_applied_system_event(state, &request.session_id, event_message, "tmux mutation");
    let tmux_state = list_tmux_state_with_handle(handle).await?;
    ensure_tmux_runtime_current(state, &request.session_id, &runtime_id)?;
    Ok(tmux_state)
}

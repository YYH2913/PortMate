use super::session_open::{cancel_pending_session_opens, session_lifecycle_lane};
use super::transport_timing::SERIAL_RUNTIME_CLOSE_TIMEOUT;
use super::*;

pub(super) struct SessionCloseValidations {
    pub(super) before_pending_open_cancel: CommitValidation,
    pub(super) before_runtime_disconnect: CommitValidation,
}

pub(super) fn session_has_registered_runtime(
    state: &AppState,
    session_id: &str,
) -> Result<bool, String> {
    if state
        .ssh
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .shell
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .tcp
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .serial
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .active_commands
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .tmux_controls
        .lock()
        .map_err(|error| error.to_string())?
        .iter()
        .any(|((runtime_session_id, _), runtime)| {
            runtime_session_id == session_id && !runtime.cancel.load(Ordering::SeqCst)
        })
    {
        return Ok(true);
    }
    if state
        .tunnels
        .lock()
        .map_err(|error| error.to_string())?
        .values()
        .any(|runtime| runtime.session_id == session_id)
    {
        return Ok(true);
    }
    Ok(false)
}

pub(super) fn cleanup_deleted_session_runtime_state(
    state: &AppState,
    session_id: &str,
    transfer_ids: &[String],
) {
    clear_session_credentials(state, session_id);
    clear_active_command(&state.session_io(), session_id);
    clear_log_retention_check(&state.store_path, session_id);
    state
        .serial_captures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(session_id);
    state
        .transfer_lanes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(session_id);
    let transfer_ids = transfer_ids.iter().collect::<HashSet<_>>();
    state
        .transfer_cancellations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|transfer_id, _| !transfer_ids.contains(transfer_id));
    state
        .one_time_host_keys
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(session_id);
    clear_outbound_lane(&state.store_path, session_id);
    clear_interactive_write_queue(&state.store_path, session_id);
    clear_deferred_interactive_queue(&state.store_path, session_id);

    let mut approvals = state
        .pending_mcp_approvals
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let approval_ids = approvals
        .iter()
        .filter_map(|(id, pending)| {
            (pending.request.session_id == session_id).then_some(id.clone())
        })
        .collect::<Vec<_>>();
    for approval_id in approval_ids {
        if let Some(pending) = approvals.remove(&approval_id) {
            let _ = pending.response.send(false);
        }
    }
}

pub(super) async fn close_session_inner(
    state: &AppState,
    session_id: String,
) -> Result<SessionSummary, String> {
    close_session_inner_with_validation(state, session_id, None).await
}

pub(super) async fn close_session_inner_with_validation(
    state: &AppState,
    session_id: String,
    validations: Option<SessionCloseValidations>,
) -> Result<SessionSummary, String> {
    // Cancelling an in-flight open commits the close; otherwise revalidate after the lane wait.
    let before_runtime_disconnect = if let Some(validations) = validations {
        (validations.before_pending_open_cancel)()?;
        if cancel_pending_session_opens(state, &session_id)? > 0 {
            clear_session_credentials(state, &session_id);
            None
        } else {
            Some(validations.before_runtime_disconnect)
        }
    } else {
        clear_session_credentials(state, &session_id);
        cancel_pending_session_opens(state, &session_id)?;
        None
    };
    let lifecycle_lane = session_lifecycle_lane(state, &session_id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
    if let Some(validate) = before_runtime_disconnect {
        validate()?;
        clear_session_credentials(state, &session_id);
    }
    close_session_under_lifecycle_lock(state, session_id).await
}

pub(super) async fn close_session_under_lifecycle_lock(
    state: &AppState,
    session_id: String,
) -> Result<SessionSummary, String> {
    // Close registration first. This shares the worker counter mutex, so a
    // late interactive write or reconnect worker cannot start after the idle
    // wait has observed zero workers.
    let serial_shutdown = SerialSessionShutdownGuard::new(
        Arc::clone(&state.serial_workers),
        session_id.clone(),
    );

    // Stop queued keystrokes before removing the runtime. This prevents a
    // close/reconnect race from leaving an orphan worker holding old input.
    clear_interactive_write_queue(&state.store_path, &session_id);
    clear_deferred_interactive_queue(&state.store_path, &session_id);
    clear_active_command(&state.session_io(), &session_id);
    let _ = cancel_tmux_control_runtimes_for_session(state, &session_id);
    let existing = {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing {
        disconnect_registered_ssh_runtime(
            runtime,
            "PortMate close_session",
            "PortMate close jump session",
        )
        .await;
    }
    let existing_shell = {
        let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_shell {
        runtime.closed.store(true, Ordering::SeqCst);
        if let Ok(mut child) = runtime.child.lock() {
            let _ = child.kill();
        }
    }
    let existing_tcp = {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_tcp {
        runtime.closed.store(true, Ordering::SeqCst);
        if let Err(error) = shutdown_tcp_writer(&runtime.writer, "TCP/Telnet").await {
            eprintln!("PortMate: failed to close TCP/Telnet session {session_id}: {error}");
        }
    }
    let existing_serial = {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_serial {
        stop_serial_runtime(&runtime);
    }
    // Removing the runtime only drops the writer handle. The reader and an
    // in-flight automatic reconnect can still own cloned serial handles;
    // wait for this session's workers before allowing an immediate reopen.
    // Windows serial drivers keep the device exclusively locked until every
    // clone is closed, otherwise close-then-open reports access denied.
    let serial_workers = Arc::clone(&state.serial_workers);
    let serial_session_id = session_id.clone();
    let remaining_serial_workers = tauri::async_runtime::spawn_blocking(move || {
        serial_workers.wait_for_session_idle(
            &serial_session_id,
            SERIAL_RUNTIME_CLOSE_TIMEOUT,
        )
    })
    .await
    .unwrap_or(usize::MAX);
    if remaining_serial_workers > 0 {
        eprintln!(
            "PortMate: {remaining_serial_workers} serial worker(s) for {session_id} did not release before the close deadline"
        );
        // Keep the close barrier until the final worker drops. A background
        // cleanup removes it later; reopening during this window would still
        // be able to hit Windows' exclusive COM-port lock.
        serial_shutdown.defer_until_idle();
        if let Ok(mut store) = state.store.lock() {
            let reason = "serial handles are still releasing; retry reconnect after close finishes";
            let _ = store.set_runtime_status_with_reason(
                &session_id,
                SessionStatus::Disconnected,
                Some(reason.to_string()),
            );
            store.record_system_event(&session_id, format!("PortMate: {reason}"));
            if let Err(error) = persist_applied_store(&store, &state.store_path, "serial close timeout state") {
                eprintln!("PortMate: failed to persist serial close timeout state: {error}");
            }
        }
        return Err(format!(
            "串口会话仍有 {remaining_serial_workers} 个后台任务正在释放，请稍后重新连接"
        ));
    }
    let stopped_tunnel_runtimes = stop_session_tunnel_runtimes(&state.tunnels, &session_id)?;
    let timed_out_tunnels = await_tunnel_listener_shutdowns(&stopped_tunnel_runtimes).await;
    if !timed_out_tunnels.is_empty() {
        eprintln!(
            "PortMate: timed out waiting for tunnel listener shutdown while closing session {session_id}: {}",
            timed_out_tunnels.join(", ")
        );
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let summary = store.close_session(&session_id)?;
    persist_applied_store(&store, &state.store_path, "session disconnect state")
        .map_err(|error| format!("会话传输已在本地关闭，但断开状态无法持久化: {error}"))?;
    Ok(summary)
}

use super::*;

#[derive(Clone, Copy, Default)]
pub(crate) struct TransferRuntimeExpectations<'a> {
    pub(crate) ssh_runtime_id: Option<&'a str>,
    pub(crate) modem_binding: Option<&'a ModemRuntimeBinding>,
}

pub(crate) fn mark_transfer_running(
    state: &AppState,
    task_id: &str,
    request: &StartTransferRequest,
) -> Result<(), String> {
    let task = {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
            let task = next_store
                .transfers
                .iter_mut()
                .find(|task| task.id == task_id)
                .ok_or_else(|| format!("unknown transfer: {task_id}"))?;
            if task.status == TransferStatus::Cancelled {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            task.status = TransferStatus::Running;
            task.message = Some("running".to_string());
            task.started_at = Some(Utc::now());
            let task = task.clone();
            let event_ids = next_store
                .record_system_event_tracked(
                    &request.session_id,
                    format!(
                        "PortMate: transfer started ({:?}, {})",
                        request.protocol,
                        transfer_route_label(&request.source, &request.destination)
                    ),
                )
                .into_iter()
                .collect();
            Ok((task, event_ids))
        })?
    };
    emit_transfer_task(state, &task);
    Ok(())
}

pub(crate) fn finish_transfer_task(
    state: &AppState,
    task_id: &str,
    session_id: &str,
    status: TransferStatus,
    message: String,
    bytes: Option<u64>,
) {
    finish_transfer_task_for_runtime(
        state, task_id, session_id, status, message, bytes, None,
    );
}

pub(crate) fn finish_transfer_task_for_runtime(
    state: &AppState,
    task_id: &str,
    session_id: &str,
    status: TransferStatus,
    message: String,
    bytes: Option<u64>,
    expected_ssh_runtime_id: Option<&str>,
) {
    finish_transfer_task_for_generations(
        state,
        task_id,
        session_id,
        status,
        message,
        bytes,
        TransferRuntimeExpectations {
            ssh_runtime_id: expected_ssh_runtime_id,
            modem_binding: None,
        },
    );
}

pub(crate) fn finish_transfer_task_for_generations(
    state: &AppState,
    task_id: &str,
    session_id: &str,
    mut status: TransferStatus,
    message: String,
    mut bytes: Option<u64>,
    expectations: TransferRuntimeExpectations<'_>,
) {
    let TransferRuntimeExpectations {
        ssh_runtime_id: expected_ssh_runtime_id,
        modem_binding: expected_modem_binding,
    } = expectations;
    cleanup_mcp_content_transfer_staging(state, task_id);
    match state.transfer_cancellations.lock() {
        Ok(mut cancellations) => {
            cancellations.remove(task_id);
        }
        Err(error) => eprintln!("PortMate: failed to clean up transfer cancellation: {error}"),
    }
    let mut message = truncate_for_log(&message, 2_000);
    if status == TransferStatus::Completed
        && expected_ssh_runtime_id.is_some()
        && expected_modem_binding.is_some()
    {
        status = TransferStatus::Failed;
        bytes = None;
        message = "传输完成提交不能同时绑定 SSH 文件和 Modem runtime".to_string();
    }
    let modem_runtime = if status == TransferStatus::Completed {
        expected_modem_binding.and_then(|binding| match binding.completion_guard() {
            Ok(guard) => Some(guard),
            Err(error) => {
                status = TransferStatus::Failed;
                bytes = None;
                message = format!("无法复核 Modem 传输 runtime: {error}");
                None
            }
        })
    } else {
        None
    };
    let ssh_runtimes = if status == TransferStatus::Completed
        && expected_ssh_runtime_id.is_some()
    {
        match state.ssh.lock() {
            Ok(runtimes) => Some(runtimes),
            Err(error) => {
                status = TransferStatus::Failed;
                bytes = None;
                message = format!("无法复核 SSH 文件传输 runtime: {error}");
                None
            }
        }
    } else {
        None
    };
    let task = {
        let mut store = match state.store.lock() {
            Ok(store) => store,
            Err(error) => {
                eprintln!("PortMate: failed to lock transfer store: {error}");
                return;
            }
        };
        if status == TransferStatus::Completed {
            if let Some(binding) = expected_modem_binding {
                let runtime_current = modem_runtime
                    .as_ref()
                    .is_some_and(|guard| guard.permits_completion(binding, session_id));
                if !runtime_current {
                    status = TransferStatus::Failed;
                    bytes = None;
                    message =
                        "Modem runtime 在传输完成提交前已变化或断开，请重试".to_string();
                }
            }
            if let Some(expected_runtime_id) = expected_ssh_runtime_id {
                let runtime_current = ssh_runtimes.as_ref().is_some_and(|runtimes| {
                    runtimes.get(session_id).is_some_and(|runtime| {
                        runtime.runtime_id == expected_runtime_id
                            && !runtime.closed.load(Ordering::SeqCst)
                    })
                }) && store.runtimes.iter().any(|runtime| {
                    runtime.session_id == session_id
                        && runtime.status == SessionStatus::Connected
                });
                if !runtime_current {
                    status = TransferStatus::Failed;
                    bytes = None;
                    message =
                        "SSH runtime 在文件传输完成提交前已变化或断开，请刷新后重试"
                            .to_string();
                }
            }
        }
        let task = match store.transfers.iter_mut().find(|item| item.id == task_id) {
            Some(task) => task,
            None => return,
        };
        if task.status == TransferStatus::Cancelled {
            status = TransferStatus::Cancelled;
            message = "cancelled".to_string();
        }
        if let Some(bytes) = bytes {
            task.bytes_total = bytes;
            task.bytes_done = bytes;
        }
        task.status = status;
        task.message = Some(message.clone());
        task.finished_at = Some(Utc::now());
        task.average_bytes_per_second = transfer_average_bps(task);
        let task = task.clone();
        store.trim_transfer_history(&task.session_id);
        store.record_system_event(
            session_id,
            format!(
                "PortMate: transfer finished ({:?}, {:?})",
                task.protocol, task.status
            ),
        );
        if let Err(error) =
            persist_applied_store(&store, &state.store_path, "transfer finish state")
        {
            eprintln!("PortMate: failed to persist transfer finish: {error}");
        }
        task
    };
    emit_transfer_task(state, &task);
}

pub(crate) fn transfer_task_is_active(status: &TransferStatus) -> bool {
    matches!(status, TransferStatus::Queued | TransferStatus::Running)
}

pub(crate) fn ensure_transfer_queue_capacity(
    store: &SessionStore,
    session_id: &str,
    additional: usize,
) -> Result<(), String> {
    let mut total = 0_usize;
    let mut session = 0_usize;
    for transfer in &store.transfers {
        if !transfer_task_is_active(&transfer.status) {
            continue;
        }
        total = total.saturating_add(1);
        if transfer.session_id == session_id {
            session = session.saturating_add(1);
        }
    }
    if session
        .checked_add(additional)
        .is_none_or(|count| count > MAX_ACTIVE_TRANSFERS_PER_SESSION)
    {
        return Err(format!(
            "active transfer count for session has reached {MAX_ACTIVE_TRANSFERS_PER_SESSION}"
        ));
    }
    if total
        .checked_add(additional)
        .is_none_or(|count| count > MAX_ACTIVE_TRANSFER_TASKS)
    {
        return Err(format!(
            "active transfer count has reached app limit ({MAX_ACTIVE_TRANSFER_TASKS})"
        ));
    }
    Ok(())
}

pub(crate) fn ensure_transfer_batch_capacity(
    state: &AppState,
    session_id: &str,
    additional: usize,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    ensure_transfer_queue_capacity(&store, session_id, additional)?;
    if state.transfer_task_slots.available_permits() < additional {
        return Err(format!(
            "transfer runner limit would be exceeded ({MAX_ACTIVE_TRANSFER_TASKS})"
        ));
    }
    Ok(())
}

pub(crate) fn cancel_transfer_inner(
    state: &AppState,
    transfer_id: &str,
) -> Result<TransferTask, String> {
    let cancel = state
        .transfer_cancellations
        .lock()
        .map_err(|error| error.to_string())?
        .get(transfer_id)
        .cloned();

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let task = store
        .transfers
        .iter_mut()
        .find(|task| task.id == transfer_id)
        .ok_or_else(|| format!("unknown transfer: {transfer_id}"))?;
    let abort_modem_session = task.status == TransferStatus::Running
        && matches!(
            task.protocol,
            TransferProtocol::Xmodem | TransferProtocol::Ymodem | TransferProtocol::Zmodem
        );
    let was_active = transfer_task_is_active(&task.status);
    if was_active {
        if let Some(cancel) = cancel.as_ref() {
            cancel.cancel();
        }
        task.status = TransferStatus::Cancelled;
        task.message = Some("cancelling".to_string());
        task.finished_at = Some(Utc::now());
        task.average_bytes_per_second = transfer_average_bps(task);
    }
    let task = task.clone();
    store.trim_transfer_history(&task.session_id);
    if let Err(error) =
        persist_applied_store(&store, &state.store_path, "transfer cancellation state")
    {
        eprintln!(
            "PortMate: transfer cancellation was accepted but could not be persisted: {error}"
        );
    }
    drop(store);
    if abort_modem_session {
        match cancel
            .as_ref()
            .map(|cancel| cancel.modem_runtime_binding())
            .transpose()
        {
            Ok(Some(Some(binding))) => {
                let state = state.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = binding
                        .write_runtime_bytes(&state, &[MODEM_CAN, MODEM_CAN, MODEM_CAN])
                        .await;
                });
            }
            Ok(Some(None) | None) => {}
            Err(error) => {
                eprintln!("PortMate: failed to resolve modem cancellation binding: {error}")
            }
        }
    }
    emit_transfer_task(state, &task);
    Ok(task)
}

pub(crate) fn emit_transfer_task(state: &AppState, task: &TransferTask) {
    if let Some(app_handle) = &state.app_handle {
        let _ = app_handle.emit("portmate-transfer-task", task.clone());
    }
}

use super::*;

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
    match state.transfer_cancellations.lock() {
        Ok(mut cancellations) => {
            cancellations.remove(task_id);
        }
        Err(error) => eprintln!("PortMate: failed to clean up transfer cancellation: {error}"),
    }
    let mut status = status;
    let mut message = truncate_for_log(&message, 2_000);
    let task = {
        let mut store = match state.store.lock() {
            Ok(store) => store,
            Err(error) => {
                eprintln!("PortMate: failed to lock transfer store: {error}");
                return;
            }
        };
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
    let abort_modem_session = (task.status == TransferStatus::Running
        && matches!(
            task.protocol,
            TransferProtocol::Xmodem | TransferProtocol::Ymodem | TransferProtocol::Zmodem
        ))
    .then(|| task.session_id.clone());
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
    if let Some(session_id) = abort_modem_session {
        let state = state.clone();
        tauri::async_runtime::spawn(async move {
            let _ =
                write_runtime_bytes(&state, &session_id, &[MODEM_CAN, MODEM_CAN, MODEM_CAN]).await;
        });
    }
    emit_transfer_task(state, &task);
    Ok(task)
}

pub(crate) fn emit_transfer_task(state: &AppState, task: &TransferTask) {
    if let Some(app_handle) = &state.app_handle {
        let _ = app_handle.emit("portmate-transfer-task", task.clone());
    }
}

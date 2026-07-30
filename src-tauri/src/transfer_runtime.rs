use super::*;

pub(super) const TRANSFER_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const TRANSFER_CANCELLED_MESSAGE: &str = "transfer cancelled";
pub(super) const MAX_ACTIVE_TRANSFER_TASKS: usize = 5_000;
pub(super) const MAX_ACTIVE_TRANSFERS_PER_SESSION: usize = 5_000;

pub(super) struct TransferCancellation {
    cancelled: Arc<AtomicBool>,
    changed: tokio::sync::Notify,
}

impl TransferCancellation {
    pub(super) fn new() -> Self {
        Self::with_flag(Arc::new(AtomicBool::new(false)))
    }

    pub(super) fn with_flag(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            changed: tokio::sync::Notify::new(),
        }
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.changed.notify_one();
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(super) async fn wait(&self) {
        let changed = self.changed.notified();
        if self.is_cancelled() {
            return;
        }
        changed.await;
    }
}

#[derive(Clone)]
pub(super) struct TransferProgressContext {
    pub(super) state: AppState,
    pub(super) task_id: String,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) last_emit: Arc<Mutex<Instant>>,
    pub(super) started: Instant,
    pub(super) rate_baseline_bytes: Arc<AtomicU64>,
    pub(super) rate_limit_bytes_per_second: Option<u64>,
}

pub(super) async fn retry_transfer_inner(
    state: &AppState,
    transfer_id: &str,
) -> Result<TransferTask, String> {
    let previous = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store
            .transfer_by_id(transfer_id)
            .ok_or_else(|| format!("unknown transfer: {transfer_id}"))?
    };
    start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: previous.session_id,
            protocol: previous.protocol,
            source: previous.source,
            destination: previous.destination,
        },
    )
    .await
}

pub(super) async fn start_transfer_inner(
    state: &AppState,
    request: StartTransferRequest,
) -> Result<TransferTask, String> {
    let profile = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store
            .profile(&request.session_id)
            .ok_or_else(|| format!("unknown session: {}", request.session_id))?
    };
    let request = prepare_transfer_request(&profile, request)?;
    let task_permit = Arc::clone(&state.transfer_task_slots)
        .try_acquire_owned()
        .map_err(|_| format!("transfer runner limit reached ({MAX_ACTIVE_TRANSFER_TASKS})"))?;

    let task = TransferTask {
        id: Uuid::new_v4().to_string(),
        session_id: request.session_id.clone(),
        protocol: request.protocol.clone(),
        source: request.source.clone(),
        destination: request.destination.clone(),
        bytes_total: 0,
        bytes_done: 0,
        status: TransferStatus::Queued,
        message: Some("queued".to_string()),
        started_at: None,
        finished_at: None,
        average_bytes_per_second: None,
    };
    let lane = transfer_lane(state, &request.session_id)?;
    let cancel = Arc::new(TransferCancellation::new());
    {
        let mut cancellations = state
            .transfer_cancellations
            .lock()
            .map_err(|error| error.to_string())?;
        cancellations.insert(task.id.clone(), Arc::clone(&cancel));
    }

    let queue_result = match state.store.lock() {
        Ok(mut store) => {
            commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
                ensure_transfer_queue_capacity(next_store, &request.session_id, 1)?;
                next_store.record_transfer(task.clone());
                let event_ids = next_store
                    .record_system_event_tracked(
                        &request.session_id,
                        format!(
                            "PortMate: transfer queued ({:?}) {} -> {}",
                            request.protocol, request.source, request.destination
                        ),
                    )
                    .into_iter()
                    .collect();
                Ok(((), event_ids))
            })
        }
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = queue_result {
        let cleanup_error = state
            .transfer_cancellations
            .lock()
            .map_err(|lock_error| lock_error.to_string())
            .map(|mut cancellations| {
                if cancellations
                    .get(&task.id)
                    .is_some_and(|registered| Arc::ptr_eq(registered, &cancel))
                {
                    cancellations.remove(&task.id);
                }
            })
            .err();
        return Err(match cleanup_error {
            Some(cleanup_error) => {
                format!("{error}; transfer cancellation cleanup failed: {cleanup_error}")
            }
            None => error,
        });
    }
    emit_transfer_task(state, &task);

    let runner_state = state.clone();
    let task_id = task.id.clone();
    tauri::async_runtime::spawn(async move {
        let _task_permit = task_permit;
        run_queued_transfer(runner_state, request, task_id, cancel, lane).await;
    });

    Ok(task)
}

pub(super) fn transfer_lane(
    state: &AppState,
    session_id: &str,
) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let mut lanes = state
        .transfer_lanes
        .lock()
        .map_err(|error| error.to_string())?;
    Ok(Arc::clone(
        lanes
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    ))
}

pub(super) async fn run_queued_transfer(
    state: AppState,
    request: StartTransferRequest,
    task_id: String,
    cancel: Arc<TransferCancellation>,
    lane: Arc<tokio::sync::Mutex<()>>,
) {
    let lane_guard = tokio::select! {
        guard = lane.lock() => Some(guard),
        () = cancel.wait() => None,
    };
    let Some(_lane_guard) = lane_guard else {
        finish_transfer_task(
            &state,
            &task_id,
            &request.session_id,
            TransferStatus::Cancelled,
            "cancelled".to_string(),
            None,
        );
        return;
    };
    if cancel.is_cancelled() {
        finish_transfer_task(
            &state,
            &task_id,
            &request.session_id,
            TransferStatus::Cancelled,
            "cancelled".to_string(),
            None,
        );
        return;
    }

    if let Err(error) = validate_current_transfer_protocol(&state, &request) {
        finish_transfer_task(
            &state,
            &task_id,
            &request.session_id,
            TransferStatus::Failed,
            error,
            None,
        );
        return;
    }

    let progress = TransferProgressContext {
        state: state.clone(),
        task_id: task_id.clone(),
        cancel: Arc::clone(&cancel.cancelled),
        last_emit: Arc::new(Mutex::new(Instant::now())),
        started: Instant::now(),
        rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
        rate_limit_bytes_per_second: transfer_rate_limit_bytes_per_second(
            &state,
            &request.session_id,
        ),
    };

    if let Err(error) = mark_transfer_running(&state, &task_id, &request) {
        let status = if error == TRANSFER_CANCELLED_MESSAGE {
            TransferStatus::Cancelled
        } else {
            TransferStatus::Failed
        };
        finish_transfer_task(&state, &task_id, &request.session_id, status, error, None);
        return;
    }

    let result = match request.protocol {
        TransferProtocol::Sftp => transfer_file_via_sftp(&state, &request, &progress).await,
        TransferProtocol::Scp => transfer_file_via_local_or_scp(&state, &request, &progress).await,
        TransferProtocol::Xmodem => transfer_file_via_xmodem(&state, &request, &progress).await,
        TransferProtocol::Ymodem => transfer_file_via_ymodem(&state, &request, &progress).await,
        TransferProtocol::Zmodem => transfer_file_via_zmodem(&state, &request, &progress).await,
    };

    let (status, message, bytes) = match result {
        Ok(bytes) if cancel.is_cancelled() => (
            TransferStatus::Cancelled,
            "cancelled".to_string(),
            Some(bytes),
        ),
        Ok(bytes) => (
            TransferStatus::Completed,
            "completed".to_string(),
            Some(bytes),
        ),
        Err(error) if error == TRANSFER_CANCELLED_MESSAGE => {
            (TransferStatus::Cancelled, "cancelled".to_string(), None)
        }
        Err(error) => (TransferStatus::Failed, error, None),
    };
    finish_transfer_task(
        &state,
        &task_id,
        &request.session_id,
        status,
        message,
        bytes,
    );
}

pub(super) fn validate_current_transfer_protocol(
    state: &AppState,
    request: &StartTransferRequest,
) -> Result<(), String> {
    let profile = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .profile(&request.session_id)
        .ok_or_else(|| format!("unknown session: {}", request.session_id))?;
    let accesses_remote = has_remote_transfer_prefix(&request.source)
        || has_remote_transfer_prefix(&request.destination);
    validate_transfer_protocol(&profile, &request.protocol, accesses_remote)
}

pub(super) fn mark_transfer_running(
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
                        "PortMate: transfer started ({:?}) {} -> {}",
                        request.protocol, request.source, request.destination
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

pub(super) fn finish_transfer_task(
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
                "PortMate: transfer finished ({:?}, {:?}): {}",
                task.protocol,
                task.status,
                truncate_for_log(&message, 800)
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

pub(super) fn transfer_task_is_active(status: &TransferStatus) -> bool {
    matches!(status, TransferStatus::Queued | TransferStatus::Running)
}

pub(super) fn ensure_transfer_queue_capacity(
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

pub(super) fn ensure_transfer_batch_capacity(
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

pub(super) fn transfer_rate_limit_bytes_per_second(
    state: &AppState,
    session_id: &str,
) -> Option<u64> {
    state
        .store
        .lock()
        .ok()
        .and_then(|store| store.profile(session_id))
        .and_then(|profile| profile.transfer.rate_limit_bytes_per_second)
        .filter(|limit| *limit > 0)
}

pub(super) fn transfer_throttle_delay(
    rate_limit_bytes_per_second: Option<u64>,
    bytes_done: u64,
    elapsed: Duration,
) -> Option<Duration> {
    let limit = rate_limit_bytes_per_second.filter(|limit| *limit > 0)?;
    if bytes_done == 0 {
        return None;
    }
    Duration::from_secs_f64(bytes_done as f64 / limit as f64)
        .checked_sub(elapsed)
        .filter(|delay| !delay.is_zero())
}

pub(super) fn transfer_average_bps(task: &TransferTask) -> Option<f64> {
    let started = task.started_at?;
    let finished = task.finished_at?;
    let elapsed_ms = (finished - started).num_milliseconds().max(1) as f64;
    if task.bytes_done == 0 {
        return None;
    }
    Some((task.bytes_done as f64) * 1000.0 / elapsed_ms)
}

pub(super) fn cancel_transfer_inner(
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

pub(super) fn emit_transfer_task(state: &AppState, task: &TransferTask) {
    if let Some(app_handle) = &state.app_handle {
        let _ = app_handle.emit("portmate-transfer-task", task.clone());
    }
}

pub(super) fn record_applied_transfer_progress_with<Persist, VerifyAfterError>(
    store: &mut SessionStore,
    task_id: &str,
    bytes_done: u64,
    bytes_total: u64,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<TransferTask, String>
where
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    let task = store
        .transfers
        .iter_mut()
        .find(|task| task.id == task_id)
        .ok_or_else(|| format!("unknown transfer: {task_id}"))?;
    task.bytes_done = bytes_done;
    if bytes_total > 0 {
        task.bytes_total = bytes_total;
    }
    task.message = Some("running".to_string());
    let task = task.clone();
    persist_applied_store_with(store, "transfer progress", persist, verify_after_error)?;
    Ok(task)
}

impl TransferProgressContext {
    pub(super) fn check_cancelled(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(TRANSFER_CANCELLED_MESSAGE.to_string())
        } else {
            Ok(())
        }
    }

    pub(super) fn set_rate_baseline(&self, bytes_done: u64) {
        self.rate_baseline_bytes.store(bytes_done, Ordering::SeqCst);
    }

    pub(super) async fn throttle(&self, bytes_done: u64) -> Result<(), String> {
        let transferred_this_run =
            bytes_done.saturating_sub(self.rate_baseline_bytes.load(Ordering::SeqCst));
        if let Some(delay) = transfer_throttle_delay(
            self.rate_limit_bytes_per_second,
            transferred_this_run,
            self.started.elapsed(),
        ) {
            let started = Instant::now();
            loop {
                self.check_cancelled()?;
                let remaining = delay.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    break;
                }
                tokio::time::sleep(remaining.min(TRANSFER_CANCEL_POLL_INTERVAL)).await;
            }
        }
        Ok(())
    }

    pub(super) async fn update(&self, bytes_done: u64, bytes_total: u64) -> Result<(), String> {
        self.check_cancelled()?;
        self.throttle(bytes_done).await?;
        let should_emit = {
            let mut last_emit = self.last_emit.lock().map_err(|error| error.to_string())?;
            if last_emit.elapsed() < Duration::from_millis(300) && bytes_done < bytes_total {
                false
            } else {
                *last_emit = Instant::now();
                true
            }
        };
        if !should_emit {
            return Ok(());
        }
        let task = {
            let mut store = self.state.store.lock().map_err(|error| error.to_string())?;
            record_applied_transfer_progress_with(
                &mut store,
                &self.task_id,
                bytes_done,
                bytes_total,
                |next_store| save_store(&self.state.store_path, next_store),
                |next_store| verify_persisted_store_commit(&self.state.store_path, next_store),
            )?
        };
        emit_transfer_task(&self.state, &task);
        Ok(())
    }
}

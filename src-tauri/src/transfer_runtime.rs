use super::*;

mod state;
pub(super) use self::state::*;

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

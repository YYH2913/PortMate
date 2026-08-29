use super::*;

mod state;
pub(super) use self::state::*;

pub(super) const MAX_ACTIVE_TRANSFER_TASKS: usize = 5_000;
pub(super) const MAX_ACTIVE_TRANSFERS_PER_SESSION: usize = 5_000;

pub(super) struct TransferCancellation {
    cancelled: Arc<AtomicBool>,
    changed: tokio::sync::Notify,
    modem_binding: Mutex<Option<ModemRuntimeBinding>>,
}

impl TransferCancellation {
    pub(super) fn new() -> Self {
        Self::with_flag(Arc::new(AtomicBool::new(false)))
    }

    pub(super) fn with_flag(cancelled: Arc<AtomicBool>) -> Self {
        Self {
            cancelled,
            changed: tokio::sync::Notify::new(),
            modem_binding: Mutex::new(None),
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

    pub(super) fn bind_modem_runtime(
        &self,
        binding: ModemRuntimeBinding,
    ) -> Result<(), String> {
        *self
            .modem_binding
            .lock()
            .map_err(|error| error.to_string())? = Some(binding);
        Ok(())
    }

    pub(super) fn modem_runtime_binding(&self) -> Result<Option<ModemRuntimeBinding>, String> {
        Ok(self
            .modem_binding
            .lock()
            .map_err(|error| error.to_string())?
            .clone())
    }
}

pub(super) async fn retry_transfer_inner(
    state: &AppState,
    transfer_id: &str,
) -> Result<TransferTask, String> {
    retry_transfer_inner_with_validation(state, transfer_id, None).await
}

pub(super) async fn retry_transfer_inner_with_validation(
    state: &AppState,
    transfer_id: &str,
    commit_validation: Option<CommitValidation>,
) -> Result<TransferTask, String> {
    let previous = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store
            .transfer_by_id(transfer_id)
            .ok_or_else(|| format!("unknown transfer: {transfer_id}"))?
    };
    if is_mcp_content_transfer_staging_source(state, &previous.source) {
        return Err(MCP_CONTENT_TRANSFER_RETRY_ERROR.to_string());
    }
    start_transfer_inner_with_validation(
        state,
        StartTransferRequest {
            session_id: previous.session_id,
            protocol: previous.protocol,
            source: previous.source,
            destination: previous.destination,
        },
        commit_validation,
    )
    .await
}

pub(super) async fn start_transfer_inner(
    state: &AppState,
    request: StartTransferRequest,
) -> Result<TransferTask, String> {
    start_transfer_inner_with_context(state, request, None, None, None).await
}

pub(super) async fn start_transfer_inner_with_validation(
    state: &AppState,
    request: StartTransferRequest,
    commit_validation: Option<CommitValidation>,
) -> Result<TransferTask, String> {
    start_transfer_inner_with_context(state, request, None, None, commit_validation).await
}

pub(super) async fn start_transfer_inner_with_staging(
    state: &AppState,
    request: StartTransferRequest,
    staging_path: Option<PathBuf>,
    commit_validation: Option<CommitValidation>,
) -> Result<TransferTask, String> {
    start_transfer_inner_with_context(state, request, staging_path, None, commit_validation).await
}

pub(super) async fn start_transfer_inner_for_ssh_runtime(
    state: &AppState,
    request: StartTransferRequest,
    expected_ssh_runtime_id: &str,
) -> Result<TransferTask, String> {
    start_transfer_inner_with_context(
        state,
        request,
        None,
        Some(expected_ssh_runtime_id),
        None,
    )
    .await
}

async fn start_transfer_inner_with_context(
    state: &AppState,
    request: StartTransferRequest,
    staging_path: Option<PathBuf>,
    required_ssh_runtime_id: Option<&str>,
    commit_validation: Option<CommitValidation>,
) -> Result<TransferTask, String> {
    let profile = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store
            .profile(&request.session_id)
            .ok_or_else(|| format!("unknown session: {}", request.session_id))?
    };
    let request = prepare_transfer_request(&profile, request)?;
    let expected_ssh_runtime_id =
        transfer_ssh_runtime_id(state, &request, required_ssh_runtime_id)?;
    let task_permit = Arc::clone(&state.transfer_task_slots)
        .try_acquire_owned()
        .map_err(|_| format!("transfer runner limit reached ({MAX_ACTIVE_TRANSFER_TASKS})"))?;
    if let Some(validate) = commit_validation {
        validate()?;
    }

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
    if let Some(staging_path) = staging_path.as_ref() {
        let mut staging = state
            .mcp_content_transfer_staging
            .lock()
            .map_err(|error| error.to_string())?;
        staging.insert(task.id.clone(), staging_path.clone());
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
                            "PortMate: transfer queued ({:?}, {})",
                            request.protocol,
                            transfer_route_label(&request.source, &request.destination)
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
        cleanup_mcp_content_transfer_staging(state, &task.id);
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
        run_queued_transfer(
            runner_state,
            request,
            task_id,
            cancel,
            lane,
            expected_ssh_runtime_id,
        )
        .await;
    });

    Ok(task)
}

fn transfer_ssh_runtime_id(
    state: &AppState,
    request: &StartTransferRequest,
    required_runtime_id: Option<&str>,
) -> Result<Option<String>, String> {
    if !transfer_uses_ssh_files(request) {
        return Ok(None);
    }

    let current_runtime_id = state
        .ssh
        .lock()
        .map_err(|error| error.to_string())?
        .get(&request.session_id)
        .filter(|runtime| !runtime.closed.load(Ordering::SeqCst))
        .map(|runtime| runtime.runtime_id.clone());
    if let Some(required_runtime_id) = required_runtime_id {
        if current_runtime_id.as_deref() != Some(required_runtime_id)
            || !ssh_runtime_connected(state, &request.session_id, required_runtime_id)
        {
            return Err(
                "SSH runtime 在文件批次规划后已变化或断开，请刷新目录后重试".to_string(),
            );
        }
        return Ok(Some(required_runtime_id.to_string()));
    }

    Ok(current_runtime_id.filter(|runtime_id| {
        ssh_runtime_connected(state, &request.session_id, runtime_id)
    }))
}

fn transfer_uses_ssh_files(request: &StartTransferRequest) -> bool {
    matches!(
        request.protocol,
        TransferProtocol::Sftp | TransferProtocol::Scp
    ) && (remote_path(&request.source).is_some() || remote_path(&request.destination).is_some())
}

pub(super) fn cleanup_mcp_content_transfer_staging(state: &AppState, task_id: &str) {
    let path = match state.mcp_content_transfer_staging.lock() {
        Ok(mut staging) => staging.remove(task_id),
        Err(error) => {
            eprintln!(
                "PortMate: failed to lock MCP content staging cleanup for {task_id}: {error}"
            );
            None
        }
    };
    if let Some(path) = path {
        if let Err(error) = fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "PortMate: failed to remove MCP content staging file after transfer {task_id}: {error}"
                );
            }
        }
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

pub(super) fn transfer_route_label(source: &str, destination: &str) -> &'static str {
    match (
        is_nonlocal_transfer_endpoint(source),
        is_nonlocal_transfer_endpoint(destination),
    ) {
        (false, true) => "upload",
        (true, false) => "download",
        (true, true) => "remote-copy",
        (false, false) => "local-copy",
    }
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
    mut expected_ssh_runtime_id: Option<String>,
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

    if let Some(runtime_id) = expected_ssh_runtime_id.as_deref() {
        if let Err(error) = ensure_ssh_runtime_current_for_operation(
            &state,
            &request.session_id,
            runtime_id,
            "文件传输等待",
        ) {
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

    if transfer_uses_ssh_files(&request) && expected_ssh_runtime_id.is_none() {
        match transfer_ssh_runtime_id(&state, &request, None) {
            Ok(Some(runtime_id)) => expected_ssh_runtime_id = Some(runtime_id),
            Ok(None) => {
                finish_transfer_task(
                    &state,
                    &task_id,
                    &request.session_id,
                    TransferStatus::Failed,
                    "需要先连接 SSH/Tmux 会话才能执行远端文件传输".to_string(),
                    None,
                );
                return;
            }
            Err(error) => {
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
        }
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
        TransferProtocol::Sftp => {
            transfer_file_via_sftp(
                &state,
                &request,
                &progress,
                expected_ssh_runtime_id.as_deref(),
            )
            .await
        }
        TransferProtocol::Scp => {
            transfer_file_via_local_or_scp(
                &state,
                &request,
                &progress,
                expected_ssh_runtime_id.as_deref(),
            )
            .await
        }
        TransferProtocol::Tftp => transfer_file_via_tftp(&state, &request, &progress).await,
        TransferProtocol::Xmodem => transfer_file_via_xmodem(&state, &request, &progress).await,
        TransferProtocol::Ymodem => transfer_file_via_ymodem(&state, &request, &progress).await,
        TransferProtocol::Zmodem => transfer_file_via_zmodem(&state, &request, &progress).await,
    };

    let (mut status, mut message, mut bytes) = match result {
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
    let modem_binding = if status == TransferStatus::Completed
        && matches!(
            request.protocol,
            TransferProtocol::Tftp
                | TransferProtocol::Xmodem
                | TransferProtocol::Ymodem
                | TransferProtocol::Zmodem
        )
    {
        match cancel.modem_runtime_binding() {
            Ok(Some(binding)) => Some(binding),
            Ok(None) => {
                status = TransferStatus::Failed;
                message = "Modem 传输完成时缺少 runtime binding".to_string();
                bytes = None;
                None
            }
            Err(error) => {
                status = TransferStatus::Failed;
                message = format!("无法读取 Modem 传输 runtime binding: {error}");
                bytes = None;
                None
            }
        }
    } else {
        None
    };
    finish_transfer_task_for_generations(
        &state,
        &task_id,
        &request.session_id,
        status,
        message,
        bytes,
        TransferRuntimeExpectations {
            ssh_runtime_id: expected_ssh_runtime_id.as_deref(),
            modem_binding: modem_binding.as_ref(),
        },
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
    let accesses_remote = is_nonlocal_transfer_endpoint(&request.source)
        || is_nonlocal_transfer_endpoint(&request.destination);
    validate_transfer_protocol(&profile, &request.protocol, accesses_remote)
}

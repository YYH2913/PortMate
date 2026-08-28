use super::*;

struct DeferredInteractiveEvent {
    io: SessionIo,
    session_id: String,
    text: String,
    wire_bytes: Vec<u8>,
}

struct DeferredInteractiveBatch {
    request: DeferredInteractiveEvent,
    request_count: usize,
}

impl DeferredInteractiveBatch {
    fn new(request: DeferredInteractiveEvent) -> Self {
        Self {
            request,
            request_count: 1,
        }
    }

    fn can_append(&self, request: &DeferredInteractiveEvent) -> bool {
        self.request
            .wire_bytes
            .len()
            .saturating_add(request.wire_bytes.len())
            <= DEFERRED_INTERACTIVE_BATCH_MAX_BYTES
    }

    fn append(&mut self, request: DeferredInteractiveEvent) {
        self.request.text.push_str(&request.text);
        self.request.wire_bytes.extend_from_slice(&request.wire_bytes);
        self.request_count = self.request_count.saturating_add(1);
    }

    fn should_flush(&self) -> bool {
        self.request.wire_bytes.len() >= DEFERRED_INTERACTIVE_BATCH_MAX_BYTES
            || deferred_interactive_input_boundary(&self.request.text)
    }
}

type DeferredInteractiveQueues =
    Mutex<HashMap<(PathBuf, String), DeferredInteractiveQueue>>;

static DEFERRED_INTERACTIVE_QUEUES: OnceLock<DeferredInteractiveQueues> = OnceLock::new();
static DEFERRED_INTERACTIVE_ACCEPTING: AtomicBool = AtomicBool::new(true);
const DEFERRED_INTERACTIVE_QUEUE_CAPACITY: usize = 1024;
const DEFERRED_INTERACTIVE_BATCH_MAX_BYTES: usize = 16 * 1024;

struct DeferredInteractiveQueueState {
    pending: AtomicUsize,
    changed: Condvar,
    lock: Mutex<()>,
}

impl DeferredInteractiveQueueState {
    fn new() -> Self {
        Self {
            pending: AtomicUsize::new(0),
            changed: Condvar::new(),
            lock: Mutex::new(()),
        }
    }

    fn complete(&self) {
        self.pending.fetch_sub(1, Ordering::SeqCst);
        self.changed.notify_all();
    }

    fn wait_empty(&self, deadline: Instant) -> bool {
        let mut guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while self.pending.load(Ordering::SeqCst) > 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, result) = self
                .changed
                .wait_timeout(guard, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = next;
            if result.timed_out() && self.pending.load(Ordering::SeqCst) > 0 {
                return false;
            }
        }
        true
    }
}

#[derive(Clone)]
struct DeferredInteractiveQueue {
    sender: mpsc::Sender<DeferredInteractiveEvent>,
    state: Arc<DeferredInteractiveQueueState>,
}

const INTERACTIVE_WRITE_QUEUE_CAPACITY: usize = 256;
const INTERACTIVE_WRITE_BATCH_MAX_BYTES: usize = 16 * 1024;

struct InteractiveQueueCancellation {
    cancelled: AtomicBool,
    changed: tokio::sync::Notify,
}

impl InteractiveQueueCancellation {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            changed: tokio::sync::Notify::new(),
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.changed.notify_one();
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    async fn wait(&self) {
        let notified = self.changed.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

struct InteractiveWriteRequest {
    io: SessionIo,
    session_id: String,
    runtime_id: String,
    text: String,
    wire_bytes: Vec<u8>,
    coalesce: bool,
    completion: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
}

type InteractiveWriteQueues =
    Mutex<HashMap<(PathBuf, String), InteractiveWriteQueue>>;

#[derive(Clone)]
struct InteractiveWriteQueue {
    sender: mpsc::Sender<InteractiveWriteRequest>,
    cancellation: Arc<InteractiveQueueCancellation>,
    completion: Arc<InteractiveWorkerCompletion>,
}

struct InteractiveWorkerCompletion {
    done: AtomicBool,
    changed: Condvar,
    lock: Mutex<()>,
}

impl InteractiveWorkerCompletion {
    fn new() -> Self {
        Self {
            done: AtomicBool::new(false),
            changed: Condvar::new(),
            lock: Mutex::new(()),
        }
    }

    fn finish(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.changed.notify_all();
    }

    fn wait(&self, deadline: Instant) -> bool {
        let mut guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !self.done.load(Ordering::SeqCst) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, result) = self
                .changed
                .wait_timeout(guard, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = next;
            if result.timed_out() && !self.done.load(Ordering::SeqCst) {
                return false;
            }
        }
        true
    }
}

static INTERACTIVE_WRITE_QUEUES: OnceLock<InteractiveWriteQueues> = OnceLock::new();
static INTERACTIVE_WRITE_ACCEPTING: AtomicBool = AtomicBool::new(true);

/// Enqueues desktop input without making the webview wait for the transport
/// writer. Printable input may coalesce; control keys and paste requests are
/// explicit ordering barriers in the same per-session queue.
pub(super) fn enqueue_interactive_text(
    io: SessionIo,
    session_id: String,
    text: String,
    coalesce: bool,
) -> Result<(), String> {
    enqueue_interactive_text_with_completion(io, session_id, text, coalesce, None)
}

/// Enqueue an atomic payload and wait until the per-session writer has
/// completed the transport write. The regular keyboard path intentionally
/// remains fire-and-forget; this acknowledgement is used by paced senders
/// that must measure their interval from an actual write rather than from
/// queue admission.
pub(super) async fn enqueue_interactive_text_and_wait(
    io: SessionIo,
    session_id: String,
    text: String,
    coalesce: bool,
) -> Result<(), String> {
    if session_id.is_empty() || text.is_empty() {
        return Ok(());
    }
    let (completion, result) = tokio::sync::oneshot::channel();
    enqueue_interactive_text_with_completion(
        io,
        session_id,
        text,
        coalesce,
        Some(completion),
    )?;
    result
        .await
        .map_err(|_| "终端输入队列已关闭，未能确认写入".to_string())?
}

fn enqueue_interactive_text_with_completion(
    io: SessionIo,
    session_id: String,
    text: String,
    coalesce: bool,
    completion: Option<tokio::sync::oneshot::Sender<Result<(), String>>>,
) -> Result<(), String> {
    if session_id.is_empty() || text.is_empty() {
        return Ok(());
    }
    let runtime_id = current_session_runtime_id(&io.runtimes, &session_id)?
        .ok_or_else(|| "会话尚未连接，无法发送输入".to_string())?;
    let wire_bytes = outbound_text_for_active_runtime(&io.runtimes, &session_id, &text)?.into_bytes();
    let key = (io.store_path.clone(), session_id.clone());
    let sender = {
        let mut queues = INTERACTIVE_WRITE_QUEUES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !INTERACTIVE_WRITE_ACCEPTING.load(Ordering::SeqCst) {
            return Err("终端输入队列正在关闭".to_string());
        }
        if let Some(queue) = queues.get(&key) {
            queue.clone()
        } else {
            let (sender, mut receiver) = mpsc::channel::<InteractiveWriteRequest>(
                INTERACTIVE_WRITE_QUEUE_CAPACITY,
            );
            let cancellation = Arc::new(InteractiveQueueCancellation::new());
            let worker_cancellation = Arc::clone(&cancellation);
            let completion = Arc::new(InteractiveWorkerCompletion::new());
            let worker_completion = Arc::clone(&completion);
            tauri::async_runtime::spawn(async move {
                let mut pending = None;
                loop {
                    if worker_cancellation.is_cancelled() {
                        break;
                    }
                    let first = tokio::select! {
                        _ = worker_cancellation.wait() => break,
                        request = async {
                            match pending.take() {
                                Some(request) => Some(request),
                                None => receiver.recv().await,
                            }
                        } => request,
                    };
                    let Some(first) = first else { break };
                    let mut text = first.text;
                    let io = first.io;
                    let session_id = first.session_id;
                    let runtime_id = first.runtime_id;
                    let mut wire_bytes = first.wire_bytes;
                    let coalesce = first.coalesce;
                    let completion = first.completion;
                    // Drain anything already queued, but never delay the first
                    // byte solely to enlarge a batch. The frontend coalesces
                    // bursts while an IPC call is in flight.
                    while coalesce
                        && text.len() < INTERACTIVE_WRITE_BATCH_MAX_BYTES
                        && !worker_cancellation.is_cancelled()
                    {
                        let Ok(next) = receiver.try_recv() else {
                            break;
                        };
                        let remaining = INTERACTIVE_WRITE_BATCH_MAX_BYTES - text.len();
                        if completion.is_none()
                            && next.completion.is_none()
                            && next.coalesce
                            && next.runtime_id == runtime_id
                            && next.text.len() <= remaining
                        {
                            text.push_str(&next.text);
                            wire_bytes.extend_from_slice(&next.wire_bytes);
                        } else {
                            // Keep the next request intact for the following
                            // batch rather than splitting UTF-8 text.
                            pending = Some(next);
                            break;
                        }
                    }
                    if worker_cancellation.is_cancelled() {
                        if let Some(completion) = completion {
                            let _ = completion.send(Err("终端输入队列已关闭".to_string()));
                        }
                        break;
                    }
                    match current_session_runtime_id(&io.runtimes, &session_id) {
                        Ok(Some(current)) if current == runtime_id => {
                            let result = if worker_cancellation.is_cancelled() {
                                Err("终端输入队列已关闭".to_string())
                            } else {
                                send_text_interactive_inner_for_runtime(
                                    io.clone(),
                                    session_id.clone(),
                                    text,
                                    &runtime_id,
                                    wire_bytes,
                                )
                                .await
                                .map(|_| ())
                            };
                            if completion.is_none() {
                                if let Err(error) = &result {
                                    publish_interactive_write_error(&io, &session_id, error.clone());
                                }
                            }
                            if let Some(completion) = completion {
                                let _ = completion.send(result);
                            }
                        }
                        Ok(_) => {
                            // The session was closed or replaced while the
                            // request waited in the queue. Never replay stale
                            // keystrokes into a newly connected runtime.
                            if let Some(completion) = completion {
                                let _ = completion.send(Err(
                                    "会话已关闭或被新连接替换".to_string(),
                                ));
                            }
                        }
                        Err(error) => {
                            if let Some(completion) = completion {
                                let _ = completion.send(Err(error));
                            } else {
                                publish_interactive_write_error(&io, &session_id, error);
                            }
                        }
                    }
                }
                worker_completion.finish();
            });
            let queue = InteractiveWriteQueue {
                sender,
                cancellation,
                completion,
            };
            queues.insert(key.clone(), queue.clone());
            queue
        }
    };
    if sender.cancellation.is_cancelled() {
        return Err("终端输入队列已关闭".to_string());
    }
    let result = sender.sender.try_send(InteractiveWriteRequest {
        io,
        session_id,
        runtime_id,
        text,
        wire_bytes,
        coalesce,
        completion,
    });
    match result {
        Ok(()) => Ok(()),
        Err(mpsc::error::TrySendError::Full(request)) => {
            if let Some(completion) = request.completion {
                let _ = completion.send(Err("终端输入队列已满，请稍后重试".to_string()));
            }
            Err("终端输入队列已满，请稍后重试".to_string())
        }
        Err(mpsc::error::TrySendError::Closed(request)) => {
            if let Some(completion) = request.completion {
                let _ = completion.send(Err("终端输入队列已关闭".to_string()));
            }
            Err("终端输入队列已关闭".to_string())
        }
    }
}

fn publish_interactive_write_error(io: &SessionIo, session_id: &str, error: String) {
    eprintln!("PortMate: interactive write failed for {session_id}: {error}");
    let event = SessionEvent {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        pane_id: format!("{session_id}:main"),
        ts: Utc::now(),
        direction: EventDirection::System,
        stream: EventStream::Audit,
        bytes_ref: None,
        text: Some(format!("PortMate: 交互输入发送失败: {error}")),
        annotations: BTreeMap::from([(
            "origin".to_string(),
            "interactive-write-worker".to_string(),
        )]),
    };
    if let Some(app_handle) = &io.app_handle {
        let _ = app_handle.emit("portmate-session-event", event);
    }
}

pub(super) fn clear_interactive_write_queue(store_path: &Path, session_id: &str) {
    if let Some(queues) = INTERACTIVE_WRITE_QUEUES.get() {
        let queue = queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(store_path.to_path_buf(), session_id.to_string()));
        if let Some(queue) = queue {
            queue.cancellation.cancel();
        }
    }
}

pub(super) fn clear_deferred_interactive_queue(store_path: &Path, session_id: &str) {
    let queue = DEFERRED_INTERACTIVE_QUEUES.get().and_then(|queues| {
        queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(store_path.to_path_buf(), session_id.to_string()))
    });
    if let Some(queue) = queue {
        drop(queue.sender);
        if !queue.state.wait_empty(Instant::now() + Duration::from_secs(1)) {
            eprintln!("PortMate: deferred interactive event queue did not drain while closing session");
        }
    }
}

pub(super) fn shutdown_interactive_write_queues() {
    INTERACTIVE_WRITE_ACCEPTING.store(false, Ordering::SeqCst);
    DEFERRED_INTERACTIVE_ACCEPTING.store(false, Ordering::SeqCst);
    if let Some(queues) = INTERACTIVE_WRITE_QUEUES.get() {
        let queues = {
            let mut queues = queues
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *queues)
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        for queue in queues.into_values() {
            queue.cancellation.cancel();
            if !queue.completion.wait(deadline) {
                eprintln!("PortMate: interactive write worker did not stop before shutdown");
            }
        }
    }
    if let Some(queues) = DEFERRED_INTERACTIVE_QUEUES.get() {
        let queues = {
            let mut queues = queues
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *queues)
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        for queue in queues.into_values() {
            drop(queue.sender);
            if !queue.state.wait_empty(deadline) {
                eprintln!("PortMate: deferred interactive event queue did not flush before shutdown");
            }
        }
    }
}

/// Queue interactive event persistence away from the transport write path.
/// The per-session worker preserves event order while keeping keystrokes from
/// waiting on full-store snapshots and log shard writes.
pub(super) fn enqueue_deferred_interactive_event(
    io: SessionIo,
    session_id: String,
    text: String,
    wire_bytes: Vec<u8>,
) {
    let key = (io.store_path.clone(), session_id.clone());
    let result = {
        let mut queues = DEFERRED_INTERACTIVE_QUEUES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !DEFERRED_INTERACTIVE_ACCEPTING.load(Ordering::SeqCst) {
            return;
        }
        let queue = if let Some(queue) = queues.get(&key) {
            queue.clone()
        } else {
            let (sender, mut receiver) = mpsc::channel::<DeferredInteractiveEvent>(
                DEFERRED_INTERACTIVE_QUEUE_CAPACITY,
            );
            let state = Arc::new(DeferredInteractiveQueueState::new());
            let worker_state = Arc::clone(&state);
            tauri::async_runtime::spawn(async move {
                let mut batch = None;
                while let Some(request) = receiver.recv().await {
                    if batch
                        .as_ref()
                        .is_some_and(|current: &DeferredInteractiveBatch| !current.can_append(&request))
                    {
                        persist_deferred_interactive_batch(
                            batch.take().expect("deferred batch exists"),
                            &worker_state,
                        )
                        .await;
                    }
                    match &mut batch {
                        Some(current) => current.append(request),
                        None => batch = Some(DeferredInteractiveBatch::new(request)),
                    }
                    if batch
                        .as_ref()
                        .is_some_and(DeferredInteractiveBatch::should_flush)
                    {
                        persist_deferred_interactive_batch(
                            batch.take().expect("deferred batch exists"),
                            &worker_state,
                        )
                        .await;
                    }
                }
                if let Some(batch) = batch {
                    persist_deferred_interactive_batch(batch, &worker_state).await;
                }
            });
            let queue = DeferredInteractiveQueue { sender, state };
            queues.insert(key.clone(), queue.clone());
            queue
        };
        queue.state.pending.fetch_add(1, Ordering::SeqCst);
        let result = queue.sender.try_send(DeferredInteractiveEvent {
            io,
            session_id,
            text,
            wire_bytes,
        });
        if result.is_err() {
            queue.state.complete();
        }
        result
    };
    if result.is_err() {
        eprintln!("PortMate: deferred interactive event queue is unavailable");
    }
}

fn deferred_interactive_input_boundary(text: &str) -> bool {
    text.chars().any(|character| {
        matches!(
            character,
            '\r' | '\n' | '\u{0003}' | '\u{0004}'
        )
    })
}

async fn persist_deferred_interactive_batch(
    batch: DeferredInteractiveBatch,
    state: &DeferredInteractiveQueueState,
) {
    let request_count = batch.request_count;
    let request = batch.request;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _ = record_outbound_user_event_with_context(
            &request.io,
            &request.session_id,
            &request.text,
            &request.wire_bytes,
            "desktop-user",
            Some("send_text"),
            BTreeMap::new(),
        );
    })
    .await;
    for _ in 0..request_count {
        state.complete();
    }
    if result.is_err() {
        eprintln!("PortMate: deferred interactive event persistence worker failed");
    }
}

pub(super) fn deferred_outbound_event(
    session_id: &str,
    text: &str,
    wire_bytes: &[u8],
) -> SessionEvent {
    SessionEvent {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        pane_id: format!("{session_id}:main"),
        ts: Utc::now(),
        direction: EventDirection::Outbound,
        stream: EventStream::Stdout,
        bytes_ref: None,
        text: Some(redact_secrets(text)),
        annotations: BTreeMap::from([
            ("origin".to_string(), "interactive".to_string()),
            ("wireBytes".to_string(), wire_bytes.len().to_string()),
            ("persistence".to_string(), "queued".to_string()),
        ]),
    }
}

pub(super) async fn send_text_interactive_inner(
    io: SessionIo,
    session_id: String,
    text: String,
) -> Result<SessionEvent, String> {
    send_text_interactive_inner_for_optional_runtime(io, session_id, text, None, None).await
}

async fn send_text_interactive_inner_for_runtime(
    io: SessionIo,
    session_id: String,
    text: String,
    expected_runtime_id: &str,
    wire_bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    send_text_interactive_inner_for_optional_runtime(
        io,
        session_id,
        text,
        Some(expected_runtime_id),
        Some(wire_bytes),
    )
    .await
}

async fn send_text_interactive_inner_for_optional_runtime(
    io: SessionIo,
    session_id: String,
    text: String,
    expected_runtime_id: Option<&str>,
    provided_wire_bytes: Option<Vec<u8>>,
) -> Result<SessionEvent, String> {
    let lane_guard = acquire_outbound_lane(&io.store_path, &session_id).await?;
    let wire_bytes = match provided_wire_bytes {
        Some(bytes) => bytes,
        None => outbound_text_for_session(&io.store, &io.runtimes.tcp, &session_id, &text)?
            .into_bytes(),
    };
    clear_active_command(&io, &session_id);
    write_session_bytes_for_runtime(
        &io.store,
        &io.runtimes,
        &io.serial_workers,
        &session_id,
        &wire_bytes,
        expected_runtime_id,
    )
    .await?;
    drop(lane_guard);

    enqueue_deferred_interactive_event(
        io,
        session_id.clone(),
        text.clone(),
        wire_bytes.clone(),
    );
    Ok(deferred_outbound_event(
        &session_id,
        &text,
        &wire_bytes,
    ))
}

pub(super) async fn send_one_key_value(
    io: SessionIo,
    session_id: &str,
    value: &str,
    origin: &str,
    prompt_event_id: Option<&str>,
    prompt_validation: Option<&OneKeyPromptValidation>,
) -> Result<SessionEvent, String> {
    let _lane_guard = acquire_outbound_lane(&io.store_path, session_id).await?;
    if let Some(validation) = prompt_validation {
        let store = io.store.lock().map_err(|error| error.to_string())?;
        let one_key = store
            .one_keys
            .iter()
            .find(|one_key| one_key.id == validation.one_key_id)
            .ok_or_else(|| "OneKey 已被删除，请刷新后重试".to_string())?;
        if one_key.updated_at != validation.one_key_updated_at {
            return Err("OneKey 已在补全等待期间更新，请重新选择".to_string());
        }
        if !one_key
            .session_ids
            .iter()
            .any(|bound_session_id| bound_session_id == session_id)
        {
            return Err("OneKey 未绑定当前会话".to_string());
        }
        validate_one_key_prompt_completion(
            &store,
            one_key,
            session_id,
            validation.field,
            &validation.prompt_event_id,
        )?;
    }
    let text = Zeroizing::new(format!("{value}\r"));
    let wire_text = Zeroizing::new(outbound_text_for_session(
        &io.store,
        &io.runtimes.tcp,
        session_id,
        text.as_str(),
    )?);
    clear_active_command(&io, session_id);
    write_session_bytes_for_runtime(
        &io.store,
        &io.runtimes,
        &io.serial_workers,
        session_id,
        wire_text.as_bytes(),
        None,
    )
    .await?;
    Ok(record_outbound_control_event(
        &io,
        session_id,
        wire_text.as_bytes(),
        origin,
        prompt_event_id,
        true,
    ))
}

pub(super) async fn send_text_inner(
    io: SessionIo,
    session_id: String,
    text: String,
) -> Result<SessionEvent, String> {
    send_text_inner_with_context(io, session_id, text, "desktop-user", Some("send_text")).await
}

pub(super) async fn send_text_inner_with_context(
    io: SessionIo,
    session_id: String,
    text: String,
    actor: &str,
    audit_action: Option<&str>,
) -> Result<SessionEvent, String> {
    send_text_inner_with_context_and_validation(
        io,
        session_id,
        text,
        actor,
        audit_action,
        None,
    )
    .await
}

pub(super) async fn send_text_inner_with_context_and_validation(
    io: SessionIo,
    session_id: String,
    text: String,
    actor: &str,
    audit_action: Option<&str>,
    commit_validation: Option<CommitValidation>,
) -> Result<SessionEvent, String> {
    let _lane_guard = acquire_outbound_lane(&io.store_path, &session_id).await?;
    if let Some(validate) = commit_validation {
        validate()?;
    }
    send_text_under_outbound_lane(&io, &session_id, &text, actor, audit_action, None).await
}

pub(super) async fn send_text_inner_for_runtime(
    io: SessionIo,
    session_id: String,
    text: String,
    expected_runtime_id: &str,
    actor: &str,
    audit_action: Option<&str>,
) -> Result<SessionEvent, String> {
    let _lane_guard = acquire_outbound_lane(&io.store_path, &session_id).await?;
    send_text_under_outbound_lane(
        &io,
        &session_id,
        &text,
        actor,
        audit_action,
        Some(expected_runtime_id),
    )
    .await
}

pub(super) async fn send_text_under_outbound_lane(
    io: &SessionIo,
    session_id: &str,
    text: &str,
    actor: &str,
    audit_action: Option<&str>,
    expected_runtime_id: Option<&str>,
) -> Result<SessionEvent, String> {
    let wire_text = outbound_text_for_session(&io.store, &io.runtimes.tcp, session_id, text)?;
    if expected_runtime_id.is_none() {
        clear_active_command(io, session_id);
    }
    write_session_bytes_for_runtime(
        &io.store,
        &io.runtimes,
        &io.serial_workers,
        session_id,
        wire_text.as_bytes(),
        expected_runtime_id,
    )
    .await?;
    if expected_runtime_id.is_some() {
        clear_active_command(io, session_id);
    }
    Ok(record_outbound_user_event_with_context(
        io,
        session_id,
        text,
        wire_text.as_bytes(),
        actor,
        audit_action,
        BTreeMap::new(),
    ))
}

pub(super) async fn run_command_inner_with_context(
    io: SessionIo,
    session_id: String,
    text: String,
    actor: &str,
    audit_action: Option<&str>,
) -> Result<SessionEvent, String> {
    run_command_inner_with_annotations(
        io,
        session_id,
        text,
        actor,
        audit_action,
        BTreeMap::new(),
    )
    .await
}

pub(super) async fn run_command_inner_with_context_and_validation(
    io: SessionIo,
    session_id: String,
    text: String,
    actor: &str,
    audit_action: Option<&str>,
    commit_validation: Option<CommitValidation>,
) -> Result<SessionEvent, String> {
    run_command_inner_with_annotations_impl(
        io,
        session_id,
        text,
        actor,
        audit_action,
        BTreeMap::new(),
        commit_validation,
    )
    .await
}

pub(super) async fn run_command_inner_with_annotations(
    io: SessionIo,
    session_id: String,
    text: String,
    actor: &str,
    audit_action: Option<&str>,
    additional_annotations: BTreeMap<String, String>,
) -> Result<SessionEvent, String> {
    run_command_inner_with_annotations_impl(
        io,
        session_id,
        text,
        actor,
        audit_action,
        additional_annotations,
        None,
    )
    .await
}

async fn run_command_inner_with_annotations_impl(
    io: SessionIo,
    session_id: String,
    text: String,
    actor: &str,
    audit_action: Option<&str>,
    additional_annotations: BTreeMap<String, String>,
    commit_validation: Option<CommitValidation>,
) -> Result<SessionEvent, String> {
    let _lane_guard = acquire_outbound_lane(&io.store_path, &session_id).await?;
    run_command_under_outbound_lane_with_annotations_and_display_text_for_runtime(
        &io,
        &session_id,
        &text,
        RunCommandContext {
            display_text: None,
            actor,
            audit_action,
            additional_annotations,
            expected_runtime_id: None,
            commit_validation,
        },
    )
    .await
}

pub(super) struct RunCommandContext<'a> {
    pub(super) display_text: Option<&'a str>,
    pub(super) actor: &'a str,
    pub(super) audit_action: Option<&'a str>,
    pub(super) additional_annotations: BTreeMap<String, String>,
    pub(super) expected_runtime_id: Option<&'a str>,
    pub(super) commit_validation: Option<CommitValidation>,
}

pub(super) async fn run_command_under_outbound_lane_with_annotations_and_display_text_for_runtime(
    io: &SessionIo,
    session_id: &str,
    text: &str,
    context: RunCommandContext<'_>,
) -> Result<SessionEvent, String> {
    let RunCommandContext {
        display_text,
        actor,
        audit_action,
        mut additional_annotations,
        expected_runtime_id,
        commit_validation,
    } = context;
    if let Some(validate) = commit_validation {
        validate()?;
    }
    let wire_text = outbound_text_for_session(&io.store, &io.runtimes.tcp, session_id, text)?;
    let command_id = Uuid::new_v4().to_string();
    set_active_command(io, session_id, &command_id);
    if let Err(error) = write_session_bytes_for_runtime(
        &io.store,
        &io.runtimes,
        &io.serial_workers,
        session_id,
        wire_text.as_bytes(),
        expected_runtime_id,
    )
    .await
    {
        clear_active_command_if(io, session_id, &command_id);
        return Err(error);
    }
    additional_annotations.extend([
        ("commandId".to_string(), command_id.clone()),
        ("commandState".to_string(), "started".to_string()),
    ]);
    let record = || {
        record_outbound_user_event_with_display_text(
            io,
            session_id,
            display_text.unwrap_or(text),
            wire_text.as_bytes(),
            actor,
            audit_action,
            additional_annotations,
        )
    };
    match expected_runtime_id {
        Some(runtime_id) => {
            match with_current_session_runtime_generation(
                &io.runtimes,
                session_id,
                runtime_id,
                record,
            )? {
                Some(event) => Ok(event),
                None => {
                    clear_active_command_if(io, session_id, &command_id);
                    Err("命令来源连接已关闭或被新连接替换".to_string())
                }
            }
        }
        None => Ok(record()),
    }
}

pub(super) async fn send_bytes_inner(
    io: SessionIo,
    session_id: String,
    bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    send_bytes_inner_with_context(
        io,
        session_id,
        bytes,
        "desktop-user",
        Some("send_bytes"),
        None,
    )
    .await
}

pub(super) async fn send_bytes_inner_with_context(
    io: SessionIo,
    session_id: String,
    bytes: Vec<u8>,
    actor: &str,
    audit_action: Option<&str>,
    commit_validation: Option<CommitValidation>,
) -> Result<SessionEvent, String> {
    let _lane_guard = acquire_outbound_lane(&io.store_path, &session_id).await?;
    if let Some(validate) = commit_validation {
        validate()?;
    }
    let wire_bytes = outbound_bytes_for_session(&io.store, &session_id, &bytes)?;
    clear_active_command(&io, &session_id);
    write_session_bytes_for_runtime(
        &io.store,
        &io.runtimes,
        &io.serial_workers,
        &session_id,
        &wire_bytes,
        None,
    )
    .await?;
    let text = format_outbound_byte_summary(&bytes);
    Ok(record_outbound_user_event_with_context(
        &io,
        &session_id,
        &text,
        &wire_bytes,
        actor,
        audit_action,
        BTreeMap::new(),
    ))
}

pub(super) fn record_outbound_user_event_with_context(
    io: &SessionIo,
    session_id: &str,
    text: &str,
    wire_bytes: &[u8],
    actor: &str,
    audit_action: Option<&str>,
    additional_annotations: BTreeMap<String, String>,
) -> SessionEvent {
    record_outbound_user_event_with_display_text(
        io,
        session_id,
        text,
        wire_bytes,
        actor,
        audit_action,
        additional_annotations,
    )
}

fn record_outbound_user_event_with_display_text(
    io: &SessionIo,
    session_id: &str,
    display_text: &str,
    wire_bytes: &[u8],
    actor: &str,
    audit_action: Option<&str>,
    additional_annotations: BTreeMap<String, String>,
) -> SessionEvent {
    let PendingEventLogs {
        profile,
        bytes_ref,
        errors,
    } = begin_event_log_shards(io, session_id, wire_bytes);
    let mut event = match io.store.lock() {
        Ok(mut store) => {
            let event = store.send_text_with_bytes_ref_and_audit_action(
                actor,
                session_id,
                display_text,
                bytes_ref.clone(),
                audit_action,
                wire_bytes.len(),
            );
            match event {
                Ok(mut event) => {
                    event.annotations.extend(additional_annotations.clone());
                    append_logging_errors(&mut event, &errors);
                    sync_stored_event(&mut store, &event);
                    if let Err(error) =
                        persist_applied_store(&store, &io.store_path, "outbound user event")
                    {
                        eprintln!(
                            "PortMate: outbound transport succeeded but store save failed: {error}"
                        );
                        append_logging_error(&mut event, format!("store save failed: {error}"));
                        sync_stored_event(&mut store, &event);
                    }
                    event
                }
                Err(error) => fallback_outbound_event(
                    session_id,
                    display_text,
                    bytes_ref.clone(),
                    actor,
                    additional_annotations.clone(),
                    merge_logging_error_messages(&errors, error),
                ),
            }
        }
        Err(error) => fallback_outbound_event(
            session_id,
            display_text,
            bytes_ref,
            actor,
            additional_annotations,
            merge_logging_error_messages(&errors, format!("session store lock poisoned: {error}")),
        ),
    };
    if profile.as_ref().is_some_and(|profile| {
        append_event_text_and_jsonl_log_shards(&io.store_path, profile, &mut event)
    }) {
        if let Ok(mut store) = io.store.lock() {
            sync_stored_event(&mut store, &event);
            if let Err(error) =
                persist_applied_store(&store, &io.store_path, "outbound user logging state")
            {
                append_logging_error(
                    &mut event,
                    format!("store save after file log failure failed: {error}"),
                );
                sync_stored_event(&mut store, &event);
            }
        }
    }
    publish_terminal_live_event(io.app_handle.as_ref(), &event, wire_bytes);
    if let Some(app_handle) = &io.app_handle {
        let _ = app_handle.emit("portmate-session-event", event.clone());
    }
    event
}

pub(super) fn record_outbound_control_event(
    io: &SessionIo,
    session_id: &str,
    wire_bytes: &[u8],
    origin: &str,
    related_event_id: Option<&str>,
    persist_store: bool,
) -> SessionEvent {
    let PendingEventLogs {
        profile,
        bytes_ref,
        errors,
    } = begin_event_log_shards(io, session_id, wire_bytes);
    let mut annotations = BTreeMap::from([
        ("origin".to_string(), origin.to_string()),
        ("wireBytes".to_string(), wire_bytes.len().to_string()),
    ]);
    if let Some(related_event_id) = related_event_id {
        annotations.insert("relatedEventId".to_string(), related_event_id.to_string());
    }
    let mut event = match io.store.lock() {
        Ok(mut store) => match store.record_event(
            session_id,
            EventDirection::Outbound,
            EventStream::Control,
            None,
            bytes_ref.clone(),
            annotations.clone(),
        ) {
            Ok(mut event) => {
                append_logging_errors(&mut event, &errors);
                sync_stored_event(&mut store, &event);
                if persist_store {
                    if let Err(error) =
                        persist_applied_store(&store, &io.store_path, "outbound control event")
                    {
                        eprintln!(
                            "PortMate: outbound control succeeded but store save failed: {error}"
                        );
                        append_logging_error(&mut event, format!("store save failed: {error}"));
                        sync_stored_event(&mut store, &event);
                    }
                }
                event
            }
            Err(error) => fallback_outbound_control_event(
                session_id,
                bytes_ref.clone(),
                annotations,
                merge_logging_error_messages(&errors, error),
            ),
        },
        Err(error) => fallback_outbound_control_event(
            session_id,
            bytes_ref,
            annotations,
            merge_logging_error_messages(&errors, format!("session store lock poisoned: {error}")),
        ),
    };
    if profile.as_ref().is_some_and(|profile| {
        append_event_text_and_jsonl_log_shards(&io.store_path, profile, &mut event)
    }) {
        if let Ok(mut store) = io.store.lock() {
            sync_stored_event(&mut store, &event);
            if persist_store {
                if let Err(error) =
                    persist_applied_store(&store, &io.store_path, "outbound control logging state")
                {
                    append_logging_error(
                        &mut event,
                        format!("store save after file log failure failed: {error}"),
                    );
                    sync_stored_event(&mut store, &event);
                }
            }
        }
    }
    publish_terminal_live_event(io.app_handle.as_ref(), &event, wire_bytes);
    if let Some(app_handle) = &io.app_handle {
        let _ = app_handle.emit("portmate-session-event", event.clone());
    }
    event
}

pub(super) fn record_outbound_control_event_for_runtime(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    wire_bytes: &[u8],
    origin: &str,
    persist_store: bool,
) -> Option<SessionEvent> {
    record_outbound_control_event_for_optional_runtime_with_accepted_side_effect(
        io,
        session_id,
        Some(runtime_id),
        wire_bytes,
        origin,
        persist_store,
        || {},
    )
}

pub(super) fn record_outbound_control_event_for_optional_runtime_with_accepted_side_effect(
    io: &SessionIo,
    session_id: &str,
    runtime_id: Option<&str>,
    wire_bytes: &[u8],
    origin: &str,
    persist_store: bool,
    accepted_side_effect: impl FnOnce(),
) -> Option<SessionEvent> {
    let accepted = match runtime_id {
        Some(runtime_id) => match with_current_session_runtime_generation(
            &io.runtimes,
            session_id,
            runtime_id,
            accepted_side_effect,
        ) {
            Ok(Some(())) => true,
            Ok(None) => false,
            Err(error) => {
                eprintln!(
                    "PortMate: runtime registry unavailable; dropping outbound control event for {session_id}: {error}"
                );
                false
            }
        },
        None => {
            accepted_side_effect();
            true
        }
    };
    if !accepted {
        return None;
    }
    Some(record_outbound_control_event(
        io,
        session_id,
        wire_bytes,
        origin,
        None,
        persist_store,
    ))
}

fn fallback_outbound_control_event(
    session_id: &str,
    bytes_ref: Option<String>,
    mut annotations: BTreeMap<String, String>,
    error: String,
) -> SessionEvent {
    eprintln!("PortMate: outbound control succeeded but event persistence degraded: {error}");
    annotations.insert("loggingError".to_string(), error);
    SessionEvent {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        pane_id: format!("{session_id}:main"),
        ts: Utc::now(),
        direction: EventDirection::Outbound,
        stream: EventStream::Control,
        bytes_ref,
        text: None,
        annotations,
    }
}

fn fallback_outbound_event(
    session_id: &str,
    text: &str,
    bytes_ref: Option<String>,
    actor: &str,
    additional_annotations: BTreeMap<String, String>,
    error: String,
) -> SessionEvent {
    eprintln!("PortMate: outbound transport succeeded but event persistence degraded: {error}");
    SessionEvent {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        pane_id: format!("{session_id}:main"),
        ts: Utc::now(),
        direction: EventDirection::Outbound,
        stream: EventStream::Stdout,
        bytes_ref,
        text: Some(redact_secrets(text)),
        annotations: BTreeMap::from([
            ("actor".to_string(), actor.to_string()),
            ("loggingError".to_string(), error),
        ])
        .into_iter()
        .chain(additional_annotations)
        .collect(),
    }
}

pub(super) fn set_active_command(io: &SessionIo, session_id: &str, command_id: &str) {
    io.active_commands
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(session_id.to_string(), command_id.to_string());
}

pub(super) fn active_command_id(io: &SessionIo, session_id: &str) -> Option<String> {
    io.active_commands
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(session_id)
        .cloned()
}

pub(super) fn clear_active_command(io: &SessionIo, session_id: &str) {
    io.active_commands
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(session_id);
}

fn clear_active_command_if(io: &SessionIo, session_id: &str, command_id: &str) {
    let mut active_commands = io
        .active_commands
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if active_commands.get(session_id).map(String::as_str) == Some(command_id) {
        active_commands.remove(session_id);
    }
}

pub(super) fn format_outbound_byte_summary(bytes: &[u8]) -> String {
    format!("Binary payload: {} bytes", bytes.len())
}

fn merge_logging_error_messages(errors: &[String], error: String) -> String {
    let mut messages = errors.to_vec();
    messages.push(error);
    messages.join("; ")
}

use super::transport_timing::STREAM_PERSIST_INTERVAL;
use super::*;

pub(super) fn append_logging_error(event: &mut SessionEvent, error: impl Into<String>) {
    let error = error.into();
    if error.is_empty() {
        return;
    }
    let current = event
        .annotations
        .entry("loggingError".to_string())
        .or_default();
    if !current.is_empty() {
        current.push_str("; ");
    }
    current.push_str(&error);
}

pub(super) fn append_logging_errors(event: &mut SessionEvent, errors: &[String]) {
    for error in errors {
        append_logging_error(event, error.clone());
    }
}

pub(super) fn sync_stored_event(store: &mut SessionStore, event: &SessionEvent) {
    if let Some(stored) = store.events.iter_mut().find(|stored| stored.id == event.id) {
        *stored = event.clone();
    }
}

#[cfg(test)]
pub(super) fn record_channel_bytes(
    io: &SessionIo,
    session_id: &str,
    source_runtime_id: Option<&str>,
    stream: EventStream,
    raw_bytes: &[u8],
    text: String,
) {
    record_channel_bytes_with_accepted_side_effect(
        io,
        session_id,
        source_runtime_id,
        stream,
        ChannelByteViews::same(raw_bytes),
        text,
        || {},
    );
}

/// Original wire bytes for raw audit logs and application bytes for the
/// terminal renderer. These views differ for negotiated transports such as Telnet.
pub(super) struct ChannelByteViews<'a> {
    pub(super) raw_log: &'a [u8],
    pub(super) terminal: &'a [u8],
}

impl<'a> ChannelByteViews<'a> {
    pub(super) fn same(bytes: &'a [u8]) -> Self {
        Self {
            raw_log: bytes,
            terminal: bytes,
        }
    }
}

pub(super) fn record_channel_bytes_with_accepted_side_effect(
    io: &SessionIo,
    session_id: &str,
    source_runtime_id: Option<&str>,
    stream: EventStream,
    bytes: ChannelByteViews<'_>,
    text: String,
    accepted_side_effect: impl FnOnce(),
) -> bool {
    let ChannelByteViews {
        raw_log,
        terminal,
    } = bytes;
    let Some(source_runtime_id) = source_runtime_id else {
        accepted_side_effect();
        record_accepted_channel_bytes(
            io,
            session_id,
            None,
            stream,
            raw_log,
            terminal,
            text,
        );
        return true;
    };
    match with_current_session_runtime_generation(
        &io.runtimes,
        session_id,
        source_runtime_id,
        || {
            accepted_side_effect();
            record_accepted_channel_bytes(
                io,
                session_id,
                Some(source_runtime_id),
                stream,
                raw_log,
                terminal,
                text,
            );
        },
    ) {
        Ok(Some(())) => true,
        Ok(None) => false,
        Err(error) => {
            eprintln!(
                "PortMate: runtime registry unavailable; dropping channel bytes for {session_id}: {error}"
            );
            false
        }
    }
}

fn record_accepted_channel_bytes(
    io: &SessionIo,
    session_id: &str,
    source_runtime_id: Option<&str>,
    stream: EventStream,
    raw_log_bytes: &[u8],
    terminal_bytes: &[u8],
    text: String,
) {
    if text.is_empty() {
        // Binary/control-only traffic still gets a raw log entry. Publish the
        // same canonical live packet as text output so mixed binary/text
        // chunks share one ordered frontend channel.
        let event = SessionEvent {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            pane_id: format!("{session_id}:main"),
            ts: Utc::now(),
            direction: EventDirection::Inbound,
            stream,
            bytes_ref: None,
            text: None,
            annotations: BTreeMap::from([(
                "binaryOnly".to_string(),
                "true".to_string(),
            )]),
        };
        publish_terminal_live_event(io.app_handle.as_ref(), &event, terminal_bytes);
        if let Err(error) = enqueue_inbound_log_persistence(
            io.clone(),
            session_id.to_string(),
            source_runtime_id.map(str::to_string),
            None,
            raw_log_bytes.to_vec(),
        ) {
            eprintln!("PortMate: inbound raw log queue unavailable: {error}");
        }
        return;
    }
    let event_id = Uuid::new_v4().to_string();
    let event_timestamp = Utc::now();
    let annotations = active_command_id(io, session_id)
        .map(|command_id| BTreeMap::from([("commandId".to_string(), command_id)]))
        .unwrap_or_default();
    let prepared_event = SessionEvent {
        id: event_id,
        session_id: session_id.to_string(),
        pane_id: format!("{session_id}:main"),
        ts: event_timestamp,
        direction: EventDirection::Inbound,
        stream,
        bytes_ref: None,
        text: Some(text.clone()),
        annotations,
    };
    publish_terminal_live_event(
        io.app_handle.as_ref(),
        &prepared_event,
        terminal_bytes,
    );
    if let Err(error) = enqueue_inbound_log_persistence(
        io.clone(),
        session_id.to_string(),
        source_runtime_id.map(str::to_string),
        Some(prepared_event),
        raw_log_bytes.to_vec(),
    ) {
        eprintln!("PortMate: inbound event queue unavailable: {error}");
    }
}

pub(super) fn with_current_session_runtime_generation<T>(
    runtimes: &RuntimeRegistry,
    session_id: &str,
    runtime_id: &str,
    operation: impl FnOnce() -> T,
) -> Result<Option<T>, String> {
    let mut operation = Some(operation);
    let ssh = runtimes.ssh.lock().map_err(|error| error.to_string())?;
    if ssh
        .get(session_id)
        .is_some_and(|runtime| runtime.runtime_id == runtime_id)
    {
        return Ok(operation.take().map(|operation| operation()));
    }
    drop(ssh);

    let shell = runtimes.shell.lock().map_err(|error| error.to_string())?;
    if shell
        .get(session_id)
        .is_some_and(|runtime| runtime.runtime_id == runtime_id)
    {
        return Ok(operation.take().map(|operation| operation()));
    }
    drop(shell);

    let tcp = runtimes.tcp.lock().map_err(|error| error.to_string())?;
    if tcp
        .get(session_id)
        .is_some_and(|runtime| runtime.runtime_id == runtime_id)
    {
        return Ok(operation.take().map(|operation| operation()));
    }
    drop(tcp);

    let serial = runtimes.serial.lock().map_err(|error| error.to_string())?;
    if serial
        .get(session_id)
        .is_some_and(|runtime| runtime.runtime_id == runtime_id)
    {
        return Ok(operation.take().map(|operation| operation()));
    }
    Ok(None)
}

pub(super) fn with_current_session_runtime_store<T>(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    operation: impl FnOnce(&mut SessionStore) -> T,
) -> Result<Option<T>, String> {
    match with_current_session_runtime_generation(&io.runtimes, session_id, runtime_id, || {
        let mut store = io.store.lock().map_err(|error| error.to_string())?;
        Ok(operation(&mut store))
    })? {
        Some(result) => result.map(Some),
        None => Ok(None),
    }
}

pub(super) fn record_runtime_system_event(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    text: String,
    persistence_context: &str,
) -> bool {
    match with_current_session_runtime_generation(&io.runtimes, session_id, runtime_id, || match io
        .store
        .lock()
    {
        Ok(mut store) => {
            store.record_system_event(session_id, text);
            if let Err(error) = persist_applied_store(&store, &io.store_path, persistence_context) {
                eprintln!("PortMate: failed to persist {persistence_context}: {error}");
            }
            true
        }
        Err(_) => false,
    }) {
        Ok(Some(recorded)) => recorded,
        Ok(None) => false,
        Err(error) => {
            eprintln!(
                "PortMate: runtime registry unavailable; dropping system event for {session_id}: {error}"
            );
            false
        }
    }
}

pub(super) fn publish_system_event(
    store: &Weak<Mutex<SessionStore>>,
    store_path: &Path,
    app_handle: Option<&AppHandle>,
    mut event: SessionEvent,
    profile: Option<SessionProfile>,
) {
    if let Some(profile) = profile {
        append_event_text_and_jsonl_log_shards(store_path, &profile, &mut event);
    } else {
        let session_id = event.session_id.clone();
        append_logging_error(
            &mut event,
            format!("system event profile unavailable for session {session_id}"),
        );
    }

    if event.annotations.contains_key("loggingError") {
        if let Some(store) = store.upgrade() {
            if let Ok(mut store) = store.lock() {
                sync_stored_event(&mut store, &event);
                if let Err(error) =
                    persist_applied_store(&store, store_path, "system event logging state")
                {
                    append_logging_error(
                        &mut event,
                        format!("store save after system log failure failed: {error}"),
                    );
                    sync_stored_event(&mut store, &event);
                }
            }
        }
    }

    if let Some(app_handle) = app_handle {
        let _ = app_handle.emit("portmate-session-event", event);
    }
}

#[derive(Default)]
pub(super) struct PendingEventLogs {
    pub(super) profile: Option<SessionProfile>,
    pub(super) bytes_ref: Option<String>,
    pub(super) errors: Vec<String>,
}

const INBOUND_LOG_QUEUE_CAPACITY: usize = 1024;
const INBOUND_LOG_BATCH_MAX_REQUESTS: usize = 64;
const INBOUND_LOG_BATCH_MAX_BYTES: usize = 256 * 1024;

struct InboundLogPersistenceRequest {
    io: SessionIo,
    session_id: String,
    source_runtime_id: Option<String>,
    event: Option<SessionEvent>,
    raw_bytes: Vec<u8>,
}

struct InboundLogQueueState {
    finished: AtomicBool,
    changed: Condvar,
    lock: Mutex<()>,
}

impl InboundLogQueueState {
    fn new() -> Self {
        Self {
            finished: AtomicBool::new(false),
            changed: Condvar::new(),
            lock: Mutex::new(()),
        }
    }

    fn finish(&self) {
        let _guard = self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        self.finished.store(true, Ordering::SeqCst);
        self.changed.notify_all();
    }

    fn wait_finished(&self, deadline: Instant) -> bool {
        let mut guard = self
            .lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !self.finished.load(Ordering::SeqCst) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, result) = self
                .changed
                .wait_timeout(guard, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard = next;
            if result.timed_out() && !self.finished.load(Ordering::SeqCst) {
                return false;
            }
        }
        true
    }
}

#[derive(Clone)]
struct InboundLogQueue {
    sender: mpsc::Sender<InboundLogPersistenceRequest>,
    state: Arc<InboundLogQueueState>,
}

type InboundLogQueues = Mutex<HashMap<(PathBuf, String), InboundLogQueue>>;

static INBOUND_LOG_QUEUES: OnceLock<InboundLogQueues> = OnceLock::new();
static INBOUND_LOG_ACCEPTING: AtomicBool = AtomicBool::new(true);

fn enqueue_inbound_log_persistence(
    io: SessionIo,
    session_id: String,
    source_runtime_id: Option<String>,
    event: Option<SessionEvent>,
    raw_bytes: Vec<u8>,
) -> Result<(), String> {
    #[cfg(test)]
    if io.synchronous_inbound_logs {
        persist_inbound_log_request(InboundLogPersistenceRequest {
            io,
            session_id,
            source_runtime_id,
            event,
            raw_bytes,
        });
        return Ok(());
    }
    let key = (io.store_path.clone(), session_id.clone());
    let result = {
        let mut queues = INBOUND_LOG_QUEUES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !INBOUND_LOG_ACCEPTING.load(Ordering::SeqCst) {
            return Err("终端日志 worker 正在关闭".to_string());
        }
        let queue = if let Some(queue) = queues.get(&key) {
            queue.clone()
        } else {
            let (sender, receiver) =
                mpsc::channel::<InboundLogPersistenceRequest>(INBOUND_LOG_QUEUE_CAPACITY);
            let state = Arc::new(InboundLogQueueState::new());
            tauri::async_runtime::spawn(run_inbound_log_queue(
                receiver,
                Arc::clone(&state),
                Arc::downgrade(&io.store),
                io.store_path.clone(),
            ));
            let queue = InboundLogQueue { sender, state };
            queues.insert(key, queue.clone());
            queue
        };
        // Shutdown takes this registry lock before dropping every sender,
        // so the worker drains every request admitted here before finishing.
        queue.sender.try_send(InboundLogPersistenceRequest {
            io,
            session_id,
            source_runtime_id,
            event,
            raw_bytes,
        }).map_err(|error| {
            match error {
                mpsc::error::TrySendError::Full(_) =>
                    "终端事件队列已满，已跳过本次持久化和触发器处理".to_string(),
                mpsc::error::TrySendError::Closed(_) => "终端日志 worker 已关闭".to_string(),
            }
        })
    };
    result.map(|_| ())
}

async fn run_inbound_log_queue(
    mut receiver: mpsc::Receiver<InboundLogPersistenceRequest>,
    state: Arc<InboundLogQueueState>,
    store: Weak<Mutex<SessionStore>>,
    path: PathBuf,
) {
    let mut dirty = false;
    let mut last_persist = Instant::now();
    loop {
        let request = if dirty {
            match tokio::time::timeout(
                STREAM_PERSIST_INTERVAL.saturating_sub(last_persist.elapsed()),
                receiver.recv(),
            ).await {
                Ok(request) => request,
                Err(_) => {
                    dirty = !persist_inbound_log_checkpoint(&store, &path).await;
                    last_persist = Instant::now();
                    continue;
                }
            }
        } else {
            receiver.recv().await
        };
        let Some(first) = request else { break };
        let mut batch_bytes = first.raw_bytes.len();
        let mut batch = vec![first];
        while batch.len() < INBOUND_LOG_BATCH_MAX_REQUESTS
            && batch_bytes < INBOUND_LOG_BATCH_MAX_BYTES
        {
            let Ok(next) = receiver.try_recv() else { break };
            batch_bytes = batch_bytes.saturating_add(next.raw_bytes.len());
            batch.push(next);
        }
        dirty |= batch.iter().any(|request| request.event.is_some());
        if tauri::async_runtime::spawn_blocking(move || {
            for request in batch {
                persist_inbound_log_request(request);
            }
        }).await.is_err() {
            eprintln!("PortMate: inbound log persistence worker failed");
        }
        // Checkpoint only after recording the batch. Readers keep publishing
        // live bytes while this worker waits for the Store or the disk.
        if dirty && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
            dirty = !persist_inbound_log_checkpoint(&store, &path).await;
            last_persist = Instant::now();
        }
    }
    if dirty {
        persist_inbound_log_checkpoint(&store, &path).await;
    }
    state.finish();
}

async fn persist_inbound_log_checkpoint(store: &Weak<Mutex<SessionStore>>, path: &Path) -> bool {
    let Some(store) = store.upgrade() else { return true };
    let path = path.to_path_buf();
    match tauri::async_runtime::spawn_blocking(move || persist_store_arc(&path, &store)).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            eprintln!("PortMate: failed to persist runtime event stream: {error}");
            false
        }
        Err(error) => {
            eprintln!("PortMate: runtime checkpoint worker failed: {error}");
            false
        }
    }
}

#[cfg(test)]
pub(super) fn finish_inbound_log_queue(store_path: &Path, session_id: &str, timeout: Duration) -> bool {
    let queue = INBOUND_LOG_QUEUES.get().and_then(|queues| {
        queues.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(store_path.to_path_buf(), session_id.to_string()))
    });
    let Some(queue) = queue else { return true };
    drop(queue.sender);
    queue.state.wait_finished(Instant::now() + timeout)
}

pub(super) fn shutdown_inbound_log_queues(timeout: Duration) {
    let queues = INBOUND_LOG_QUEUES.get().map(|queues| {
        let mut queues = queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        INBOUND_LOG_ACCEPTING.store(false, Ordering::SeqCst);
        std::mem::take(&mut *queues)
    }).unwrap_or_else(|| {
        INBOUND_LOG_ACCEPTING.store(false, Ordering::SeqCst);
        HashMap::new()
    });
    let deadline = Instant::now() + timeout;
    let completions = queues.into_values().map(|queue| {
        drop(queue.sender);
        queue.state
    }).collect::<Vec<_>>();
    for completion in completions {
        if !completion.wait_finished(deadline) {
            eprintln!("PortMate: inbound log queue did not flush before shutdown");
        }
    }
}

fn persist_inbound_log_request(request: InboundLogPersistenceRequest) {
    let InboundLogPersistenceRequest {
        io,
        session_id,
        source_runtime_id,
        mut event,
        raw_bytes,
    } = request;
    let mut event_recorded = false;
    if let Some(prepared_event) = event.as_ref().cloned() {
        if let Ok(mut store) = io.store.lock() {
            match store.record_prepared_event(prepared_event) {
                Ok(recorded) => {
                    event = Some(recorded);
                    event_recorded = true;
                }
                Err(error) => {
                    eprintln!(
                        "PortMate: live output continued but event recording failed for \
                         {session_id}: {error}"
                    );
                }
            }
        } else {
            eprintln!(
                "PortMate: session store lock poisoned; live output continued but persistence \
                 degraded for {session_id} until the app restarts"
            );
        }
    }
    if let Some(live_event) = event.as_ref() {
        if let Some(app_handle) = &io.app_handle {
            let _ = app_handle.emit("portmate-session-event", live_event.clone());
        }
    }
    let PendingEventLogs {
        profile,
        bytes_ref,
        errors,
    } = begin_event_log_shards(&io, &session_id, &raw_bytes);
    let Some(mut event) = event.take() else {
        return;
    };
    let previous_bytes_ref = event.bytes_ref.clone();
    let previous_annotations = event.annotations.clone();
    event.bytes_ref = bytes_ref;
    append_logging_errors(&mut event, &errors);
    if let Some(profile) = profile {
        append_event_text_and_jsonl_log_shards(&io.store_path, &profile, &mut event);
    }
    let trigger_text = event.text.clone().unwrap_or_default();
    if event.bytes_ref != previous_bytes_ref || event.annotations != previous_annotations {
        if event_recorded {
            if let Ok(mut store) = io.store.lock() {
                sync_stored_event(&mut store, &event);
            }
        };
        if let Some(app_handle) = &io.app_handle {
            let _ = app_handle.emit("portmate-session-event-updated", event.clone());
        }
    }

    let trigger_dispatch = if event_recorded {
        if let Ok(mut store) = io.store.lock() {
            let (trigger_dispatch, trigger_changed_store) =
                apply_trigger_actions_locked(&mut store, &session_id, &trigger_text);
            if trigger_changed_store {
                if let Err(error) =
                    persist_applied_store(&store, &io.store_path, "trigger action state")
                {
                    eprintln!("PortMate: failed to persist trigger actions: {error}");
                }
            }
            trigger_dispatch
        } else {
            eprintln!(
                "PortMate: session store lock poisoned; dropping trigger actions for {session_id}"
            );
            TriggerDispatch::default()
        }
    } else {
        TriggerDispatch::default()
    };
    if let Some(app_handle) = &io.app_handle {
        for effect in trigger_dispatch.effects {
            let _ = app_handle.emit("portmate-trigger-effect", effect);
        }
    }
    spawn_trigger_commands(
        io.clone(),
        session_id.clone(),
        source_runtime_id.clone(),
        trigger_dispatch.local_commands,
    );
    if let Some(source_runtime_id) = source_runtime_id {
        spawn_trigger_send_text_batch(
            io,
            session_id,
            source_runtime_id,
            trigger_dispatch.send_texts,
        );
    }
}

pub(super) fn begin_event_log_shards(
    io: &SessionIo,
    session_id: &str,
    raw_bytes: &[u8],
) -> PendingEventLogs {
    let profile = match logging_profile(io, session_id) {
        Ok(profile) => profile,
        Err(error) => {
            return PendingEventLogs {
                errors: vec![error],
                ..PendingEventLogs::default()
            };
        }
    };
    if !profile.logging.enabled {
        return PendingEventLogs {
            profile: Some(profile),
            ..PendingEventLogs::default()
        };
    }

    let mut result = PendingEventLogs::default();
    if profile.logging.raw && !raw_bytes.is_empty() {
        match append_log_bytes(&io.store_path, &profile, "raw", raw_bytes) {
            Ok(reference) => result.bytes_ref = Some(reference),
            Err(error) => {
                let error = format!("raw shard append failed: {error}");
                eprintln!("PortMate: {error}");
                result.errors.push(error);
            }
        }
    }
    result.profile = Some(profile);
    result
}

pub(super) fn text_log_field(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            ']' => escaped.push_str("\\]"),
            character if character.is_control() => escaped.extend(character.escape_default()),
            character => escaped.push(character),
        }
    }
    escaped
}

pub(super) fn format_text_log_event(event: &SessionEvent, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let direction = match event.direction {
        EventDirection::Inbound => "inbound",
        EventDirection::Outbound => "outbound",
        EventDirection::System => "system",
    };
    let stream = match event.stream {
        EventStream::Stdout => "stdout",
        EventStream::Stderr => "stderr",
        EventStream::Control => "control",
        EventStream::Audit => "audit",
    };
    let command_id = event
        .annotations
        .get("commandId")
        .map(String::as_str)
        .unwrap_or("-");
    let prefix = format!(
        "[{}] [{direction}/{stream}] [session={}] [pane={}] [command={}] ",
        event
            .ts
            .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        text_log_field(&event.session_id),
        text_log_field(&event.pane_id),
        text_log_field(command_id),
    );
    let mut rendered = String::with_capacity(text.len().saturating_add(prefix.len()));
    for line in text.split_inclusive('\n') {
        rendered.push_str(&prefix);
        rendered.push_str(line);
        if !line.ends_with('\n') {
            rendered.push('\n');
        }
    }
    rendered
}

pub(super) fn serialize_jsonl_log_event(event: &SessionEvent) -> Result<Vec<u8>, String> {
    let mut serialized = serde_json::to_value(event)
        .map_err(|error| format!("JSONL serialization failed: {error}"))?;
    let fields = serialized
        .as_object_mut()
        .ok_or_else(|| "JSONL serialization did not produce an event object".to_string())?;
    fields.insert(
        "ts".to_string(),
        serde_json::Value::String(
            event
                .ts
                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
        ),
    );
    let mut line = serde_json::to_vec(&serialized)
        .map_err(|error| format!("JSONL serialization failed: {error}"))?;
    line.push(b'\n');
    Ok(line)
}

fn append_text_log_shard_for_profile(
    store_path: &Path,
    profile: &SessionProfile,
    event: &SessionEvent,
) -> Result<(), String> {
    if !profile.logging.enabled || !profile.logging.text {
        return Ok(());
    }
    let Some(text) = event.text.as_deref() else {
        return Ok(());
    };
    if text.is_empty() {
        return Ok(());
    }
    let text = if profile.logging.redact_secrets {
        redact_secrets(text)
    } else {
        text.to_string()
    };
    let rendered = format_text_log_event(event, &text);
    append_log_bytes(store_path, profile, "txt", rendered.as_bytes())
        .map(|_| ())
        .map_err(|error| {
            let error = format!("text shard append failed: {error}");
            eprintln!("PortMate: {error}");
            error
        })
}

fn append_jsonl_log_shard_for_profile(
    store_path: &Path,
    profile: &SessionProfile,
    event: &SessionEvent,
) -> Result<(), String> {
    if !profile.logging.enabled || !profile.logging.jsonl {
        return Ok(());
    }
    let mut event = event.clone();
    if profile.logging.redact_secrets {
        event.text = event.text.map(|text| redact_secrets(&text));
    }
    let line = serialize_jsonl_log_event(&event)?;
    append_log_bytes(store_path, profile, "jsonl", &line)
        .map(|_| ())
        .map_err(|error| {
            let error = format!("JSONL shard append failed: {error}");
            eprintln!("PortMate: {error}");
            error
        })
}

pub(super) fn append_event_text_and_jsonl_log_shards(
    store_path: &Path,
    profile: &SessionProfile,
    event: &mut SessionEvent,
) -> bool {
    let mut failed = false;
    if let Err(error) = append_text_log_shard_for_profile(store_path, profile, event) {
        append_logging_error(event, error);
        failed = true;
    }
    if let Err(error) = append_jsonl_log_shard_for_profile(store_path, profile, event) {
        append_logging_error(event, error);
        failed = true;
    }
    failed
}

fn logging_profile(io: &SessionIo, session_id: &str) -> Result<SessionProfile, String> {
    io.store
        .lock()
        .map_err(|error| format!("session store lock poisoned while resolving logging: {error}"))?
        .profile(session_id)
        .ok_or_else(|| format!("unknown session while resolving logging: {session_id}"))
}

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
        raw_bytes,
        text,
        || {},
    );
}

pub(super) fn record_channel_bytes_with_accepted_side_effect(
    io: &SessionIo,
    session_id: &str,
    source_runtime_id: Option<&str>,
    stream: EventStream,
    raw_bytes: &[u8],
    text: String,
    accepted_side_effect: impl FnOnce(),
) -> bool {
    let Some(source_runtime_id) = source_runtime_id else {
        accepted_side_effect();
        record_accepted_channel_bytes(io, session_id, None, stream, raw_bytes, text);
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
                raw_bytes,
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
    raw_bytes: &[u8],
    text: String,
) {
    let command_id = active_command_id(io, session_id);
    if text.is_empty() {
        // Binary/control-only traffic still gets a raw log entry, but it does
        // not need to enter the text terminal event stream.
        let raw_bytes = raw_bytes.to_vec();
        let raw_bytes_for_publish = raw_bytes.clone();
        if let Err(error) = enqueue_inbound_log_persistence(
            io.clone(),
            session_id.to_string(),
            None,
            raw_bytes,
        ) {
            eprintln!("PortMate: inbound raw log queue unavailable: {error}");
        }
        publish_terminal_bytes(
            io.app_handle.as_ref(),
            session_id,
            EventDirection::Inbound,
            stream,
            &raw_bytes_for_publish,
            None,
            Utc::now(),
        );
        return;
    }
    let live_event = if let Ok(mut store) = io.store.lock() {
        let annotations = command_id
            .map(|command_id| BTreeMap::from([("commandId".to_string(), command_id)]))
            .unwrap_or_default();
        let event = store
            .record_event(
                session_id,
                EventDirection::Inbound,
                stream,
                Some(text.clone()),
                None,
                annotations,
            )
            .ok();
        if let Some(event) = event.as_ref() {
            sync_stored_event(&mut store, event);
        }
        event
    } else {
        eprintln!(
            "PortMate: session store lock poisoned; dropping event for {session_id} \
             (live push and persistence degraded until the app restarts)"
        );
        None
    };
    let terminal_event_id = live_event.as_ref().map(|event| event.id.as_str());
    let terminal_event_timestamp = live_event
        .as_ref()
        .map(|event| event.ts.to_owned())
        .unwrap_or_else(Utc::now);
    publish_terminal_bytes(
        io.app_handle.as_ref(),
        session_id,
        EventDirection::Inbound,
        stream,
        raw_bytes,
        terminal_event_id,
        terminal_event_timestamp,
    );
    if let Some(mut event) = live_event {
        if let Err(error) = enqueue_inbound_log_persistence(
            io.clone(),
            session_id.to_string(),
            Some(event.clone()),
            raw_bytes.to_vec(),
        ) {
            append_logging_error(&mut event, error);
            if let Ok(mut store) = io.store.lock() {
                sync_stored_event(&mut store, &event);
            }
        }
        if let Some(app_handle) = &io.app_handle {
            let _ = app_handle.emit("portmate-session-event", event);
        }
    }
    let trigger_dispatch = if let Ok(mut store) = io.store.lock() {
        let (trigger_dispatch, trigger_changed_store) =
            apply_trigger_actions_locked(&mut store, session_id, &text);
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
    };
    if let Some(app_handle) = &io.app_handle {
        for effect in trigger_dispatch.effects {
            let _ = app_handle.emit("portmate-trigger-effect", effect);
        }
    }
    spawn_trigger_commands(
        io.clone(),
        session_id.to_string(),
        source_runtime_id.map(str::to_string),
        trigger_dispatch.local_commands,
    );
    if let Some(source_runtime_id) = source_runtime_id {
        spawn_trigger_send_text_batch(
            io.clone(),
            session_id.to_string(),
            source_runtime_id.to_string(),
            trigger_dispatch.send_texts,
        );
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

struct InboundLogPersistenceRequest {
    io: SessionIo,
    session_id: String,
    event: Option<SessionEvent>,
    raw_bytes: Vec<u8>,
}

static INBOUND_LOG_QUEUE: OnceLock<mpsc::Sender<InboundLogPersistenceRequest>> = OnceLock::new();

fn enqueue_inbound_log_persistence(
    io: SessionIo,
    session_id: String,
    event: Option<SessionEvent>,
    raw_bytes: Vec<u8>,
) -> Result<(), String> {
    if cfg!(test) {
        persist_inbound_log_request(InboundLogPersistenceRequest {
            io,
            session_id,
            event,
            raw_bytes,
        });
        return Ok(());
    }
    let sender = INBOUND_LOG_QUEUE.get_or_init(|| {
        let (sender, mut receiver) = mpsc::channel(INBOUND_LOG_QUEUE_CAPACITY);
        tauri::async_runtime::spawn(async move {
            while let Some(request) = receiver.recv().await {
                let _ = tauri::async_runtime::spawn_blocking(|| {
                    persist_inbound_log_request(request);
                })
                .await;
            }
        });
        sender
    });
    sender
        .try_send(InboundLogPersistenceRequest {
            io,
            session_id,
            event,
            raw_bytes,
        })
        .map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) =>
                "终端日志队列已满，已跳过本次磁盘日志写入".to_string(),
            mpsc::error::TrySendError::Closed(_) => "终端日志 worker 已关闭".to_string(),
        })
}

fn persist_inbound_log_request(request: InboundLogPersistenceRequest) {
    let InboundLogPersistenceRequest {
        io,
        session_id,
        mut event,
        raw_bytes,
    } = request;
    let PendingEventLogs {
        profile,
        bytes_ref,
        errors,
    } = begin_event_log_shards(&io, &session_id, &raw_bytes);
    let Some(mut event) = event.take() else {
        return;
    };
    event.bytes_ref = bytes_ref;
    append_logging_errors(&mut event, &errors);
    if let Some(profile) = profile {
        append_event_text_and_jsonl_log_shards(&io.store_path, &profile, &mut event);
    }
    if let Ok(mut store) = io.store.lock() {
        sync_stored_event(&mut store, &event);
    };
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

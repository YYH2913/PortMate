use super::*;

struct DeferredInteractiveEvent {
    io: SessionIo,
    session_id: String,
    text: String,
    wire_bytes: Vec<u8>,
}

type DeferredInteractiveQueues =
    Mutex<HashMap<(PathBuf, String), mpsc::UnboundedSender<DeferredInteractiveEvent>>>;

static DEFERRED_INTERACTIVE_QUEUES: OnceLock<DeferredInteractiveQueues> = OnceLock::new();

const INTERACTIVE_WRITE_QUEUE_CAPACITY: usize = 256;
const INTERACTIVE_WRITE_BATCH_MAX_BYTES: usize = 16 * 1024;

struct InteractiveWriteRequest {
    io: SessionIo,
    session_id: String,
    runtime_id: String,
    text: String,
}

type InteractiveWriteQueues =
    Mutex<HashMap<(PathBuf, String), mpsc::Sender<InteractiveWriteRequest>>>;

static INTERACTIVE_WRITE_QUEUES: OnceLock<InteractiveWriteQueues> = OnceLock::new();

/// Enqueues desktop keystrokes without making the webview wait for the
/// transport writer. Each session owns one bounded worker, so ordering is
/// preserved while unrelated sessions remain independent.
pub(super) fn enqueue_interactive_text(
    io: SessionIo,
    session_id: String,
    text: String,
) -> Result<Vec<u8>, String> {
    if session_id.is_empty() || text.is_empty() {
        return Ok(Vec::new());
    }
    let runtime_id = current_session_runtime_id(&io.runtimes, &session_id)?
        .ok_or_else(|| "会话尚未连接，无法发送输入".to_string())?;
    let wire_bytes = outbound_text_for_session(&io.store, &io.runtimes.tcp, &session_id, &text)?
        .into_bytes();
    let key = (io.store_path.clone(), session_id.clone());
    let sender = {
        let mut queues = INTERACTIVE_WRITE_QUEUES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(sender) = queues.get(&key) {
            sender.clone()
        } else {
            let (sender, mut receiver) = mpsc::channel::<InteractiveWriteRequest>(
                INTERACTIVE_WRITE_QUEUE_CAPACITY,
            );
            tauri::async_runtime::spawn(async move {
                let mut pending = None;
                loop {
                    let Some(first) = (match pending.take() {
                        Some(request) => Some(request),
                        None => receiver.recv().await,
                    }) else {
                        break;
                    };
                    let mut text = first.text;
                    let io = first.io;
                    let session_id = first.session_id;
                    let runtime_id = first.runtime_id;
                    while text.len() < INTERACTIVE_WRITE_BATCH_MAX_BYTES {
                        let Ok(next) = receiver.try_recv() else {
                            break;
                        };
                        let remaining = INTERACTIVE_WRITE_BATCH_MAX_BYTES - text.len();
                        if next.runtime_id == runtime_id && next.text.len() <= remaining {
                            text.push_str(&next.text);
                        } else {
                            // Keep the next request intact for the following
                            // batch rather than splitting UTF-8 text.
                            pending = Some(next);
                            break;
                        }
                    }
                    match current_session_runtime_id(&io.runtimes, &session_id) {
                        Ok(Some(current)) if current == runtime_id => {
                            if let Err(error) = send_text_interactive_inner_for_runtime(
                                io.clone(),
                                session_id.clone(),
                                text,
                                &runtime_id,
                            )
                            .await
                            {
                                publish_interactive_write_error(&io, &session_id, error);
                            }
                        }
                        Ok(_) => {
                            // The session was closed or replaced while the
                            // request waited in the queue. Never replay stale
                            // keystrokes into a newly connected runtime.
                        }
                        Err(error) => {
                            publish_interactive_write_error(&io, &session_id, error);
                        }
                    }
                }
            });
            queues.insert(key.clone(), sender.clone());
            sender
        }
    };
    sender.try_send(InteractiveWriteRequest {
        io,
        session_id,
        runtime_id,
        text,
    })
    .map_err(|error| match error {
        mpsc::error::TrySendError::Full(_) => "终端输入队列已满，请稍后重试".to_string(),
        mpsc::error::TrySendError::Closed(_) => "终端输入队列已关闭".to_string(),
    })?;
    Ok(wire_bytes)
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
        queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(store_path.to_path_buf(), session_id.to_string()));
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
    let sender = {
        let mut queues = DEFERRED_INTERACTIVE_QUEUES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(sender) = queues.get(&key) {
            sender.clone()
        } else {
            let (sender, mut receiver) = mpsc::unbounded_channel::<DeferredInteractiveEvent>();
            tauri::async_runtime::spawn(async move {
                while let Some(request) = receiver.recv().await {
                    let _ = tauri::async_runtime::spawn_blocking(move || {
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
                }
            });
            queues.insert(key.clone(), sender.clone());
            sender
        }
    };
    if sender
        .send(DeferredInteractiveEvent {
            io,
            session_id,
            text,
            wire_bytes,
        })
        .is_err()
    {
        eprintln!("PortMate: deferred interactive event queue is unavailable");
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
    send_text_interactive_inner_for_optional_runtime(io, session_id, text, None).await
}

async fn send_text_interactive_inner_for_runtime(
    io: SessionIo,
    session_id: String,
    text: String,
    expected_runtime_id: &str,
) -> Result<SessionEvent, String> {
    send_text_interactive_inner_for_optional_runtime(
        io,
        session_id,
        text,
        Some(expected_runtime_id),
    )
    .await
}

async fn send_text_interactive_inner_for_optional_runtime(
    io: SessionIo,
    session_id: String,
    text: String,
    expected_runtime_id: Option<&str>,
) -> Result<SessionEvent, String> {
    let lane_guard = acquire_outbound_lane(&io.store_path, &session_id).await?;
    let wire_text = outbound_text_for_session(&io.store, &io.runtimes.tcp, &session_id, &text)?;
    clear_active_command(&io, &session_id);
    write_session_bytes_for_runtime(
        &io.store,
        &io.runtimes,
        &session_id,
        wire_text.as_bytes(),
        expected_runtime_id,
    )
    .await?;
    drop(lane_guard);

    enqueue_deferred_interactive_event(
        io,
        session_id.clone(),
        text.clone(),
        wire_text.as_bytes().to_vec(),
    );
    Ok(deferred_outbound_event(
        &session_id,
        &text,
        wire_text.as_bytes(),
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
    write_session_bytes(
        &io.store,
        &io.runtimes.ssh,
        &io.runtimes.shell,
        &io.runtimes.tcp,
        &io.runtimes.serial,
        session_id,
        wire_text.as_bytes(),
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
    write_session_bytes(
        &io.store,
        &io.runtimes.ssh,
        &io.runtimes.shell,
        &io.runtimes.tcp,
        &io.runtimes.serial,
        &session_id,
        &wire_bytes,
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
    publish_terminal_bytes(
        io.app_handle.as_ref(),
        session_id,
        event.direction,
        event.stream,
        wire_bytes,
        Some(&event.id),
        event.ts.to_owned(),
    );
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
    publish_terminal_bytes(
        io.app_handle.as_ref(),
        session_id,
        event.direction,
        event.stream,
        wire_bytes,
        Some(&event.id),
        event.ts.to_owned(),
    );
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

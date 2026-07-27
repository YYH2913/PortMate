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

pub(super) fn record_channel_bytes(
    io: &SessionIo,
    session_id: &str,
    source_runtime_id: Option<&str>,
    stream: EventStream,
    raw_bytes: &[u8],
    text: String,
) {
    let command_id = active_command_id(io, session_id);
    let PendingEventLogs {
        profile,
        bytes_ref,
        errors,
    } = begin_event_log_shards(io, session_id, raw_bytes);
    if text.is_empty() {
        return;
    }
    let live_event = if let Ok(mut store) = io.store.lock() {
        let annotations = command_id
            .map(|command_id| BTreeMap::from([("commandId".to_string(), command_id)]))
            .unwrap_or_default();
        let mut event = store
            .record_event(
                session_id,
                EventDirection::Inbound,
                stream,
                Some(text.clone()),
                bytes_ref,
                annotations,
            )
            .ok();
        if let Some(event) = event.as_mut() {
            append_logging_errors(event, &errors);
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
    if let Some(mut event) = live_event {
        if profile.as_ref().is_some_and(|profile| {
            append_event_text_and_jsonl_log_shards(&io.store_path, profile, &mut event)
        }) {
            if let Ok(mut store) = io.store.lock() {
                sync_stored_event(&mut store, &event);
            }
        }
        if let Some(app_handle) = &io.app_handle {
            let _ = app_handle.emit("portmate-session-event", event);
        }
    }
    let source_is_current = match source_runtime_id {
        Some(source_runtime_id) => {
            match session_runtime_generation_is_current(&io.runtimes, session_id, source_runtime_id)
            {
                Ok(current) => current,
                Err(error) => {
                    eprintln!(
                        "PortMate: runtime registry unavailable; dropping trigger actions for {session_id}: {error}"
                    );
                    false
                }
            }
        }
        None => true,
    };
    let trigger_dispatch = if !source_is_current {
        TriggerDispatch::default()
    } else if let Ok(mut store) = io.store.lock() {
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
        Arc::clone(&io.store),
        io.store_path.clone(),
        Arc::clone(&io.trigger_command_slots),
        session_id.to_string(),
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

fn session_runtime_generation_is_current(
    runtimes: &RuntimeRegistry,
    session_id: &str,
    runtime_id: &str,
) -> Result<bool, String> {
    let ssh_matches = runtimes
        .ssh
        .lock()
        .map_err(|error| error.to_string())?
        .get(session_id)
        .is_some_and(|runtime| runtime.runtime_id == runtime_id);
    let shell_matches = runtimes
        .shell
        .lock()
        .map_err(|error| error.to_string())?
        .get(session_id)
        .is_some_and(|runtime| runtime.runtime_id == runtime_id);
    let tcp_matches = runtimes
        .tcp
        .lock()
        .map_err(|error| error.to_string())?
        .get(session_id)
        .is_some_and(|runtime| runtime.runtime_id == runtime_id);
    let serial_matches = runtimes
        .serial
        .lock()
        .map_err(|error| error.to_string())?
        .get(session_id)
        .is_some_and(|runtime| runtime.runtime_id == runtime_id);
    Ok(ssh_matches || shell_matches || tcp_matches || serial_matches)
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
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
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
    let mut line = serde_json::to_vec(&event)
        .map_err(|error| format!("JSONL serialization failed: {error}"))?;
    line.push(b'\n');
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

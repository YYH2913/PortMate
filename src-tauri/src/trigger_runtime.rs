use super::*;

mod trigger_command;

use trigger_command::run_shell_command;
#[cfg(test)]
pub(super) use trigger_command::run_shell_command_bounded;

pub(super) const MAX_TRIGGER_COMMAND_CONCURRENCY: usize = 4;
pub(super) const MAX_TRIGGER_LOCAL_COMMANDS_PER_BATCH: usize = 8;
pub(super) const MAX_TRIGGER_SEND_BATCH_CONCURRENCY: usize = 8;
pub(super) const MAX_TRIGGER_SEND_TEXTS_PER_BATCH: usize = 32;
pub(super) const MAX_TRIGGER_CUSTOM_LINK_CHARACTERS: usize = 8_192;

#[derive(Default)]
pub(super) struct TriggerDispatch {
    pub(super) local_commands: Vec<String>,
    pub(super) send_texts: Vec<String>,
    pub(super) effects: Vec<TriggerEffect>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TriggerEffect {
    pub(super) session_id: String,
    pub(super) trigger_id: String,
    pub(super) trigger_label: String,
    pub(super) kind: String,
    pub(super) value: String,
}

pub(super) fn apply_trigger_actions_locked(
    store: &mut SessionStore,
    session_id: &str,
    text: &str,
) -> (TriggerDispatch, bool) {
    let Some(profile) = store.profile(session_id) else {
        return (TriggerDispatch::default(), false);
    };

    let matches = portmate_core::triggers::evaluate_triggers(&profile.triggers, text);
    let mut dispatch = TriggerDispatch::default();
    let mut changed = false;
    let mut dropped_local_commands = 0_usize;
    let mut dropped_send_texts = 0_usize;
    for trigger_match in matches {
        changed = true;
        store.record_system_event(
            session_id,
            format!("PortMate: trigger matched ({})", trigger_match.label),
        );
        for action in trigger_match.actions {
            match action {
                TriggerAction::Highlight { color } => {
                    store.record_system_event(
                        session_id,
                        format!(
                            "PortMate: trigger highlight action ({}, color={color})",
                            trigger_match.label
                        ),
                    );
                    dispatch.effects.push(TriggerEffect {
                        session_id: session_id.to_string(),
                        trigger_id: trigger_match.trigger_id.clone(),
                        trigger_label: trigger_match.label.clone(),
                        kind: "highlight".to_string(),
                        value: color,
                    });
                }
                TriggerAction::SendText { text } => {
                    if dispatch.send_texts.len() < MAX_TRIGGER_SEND_TEXTS_PER_BATCH {
                        store.record_system_event(
                            session_id,
                            format!(
                                "PortMate: trigger send_text action queued ({}) bytes={}",
                                trigger_match.label,
                                text.len()
                            ),
                        );
                        dispatch.send_texts.push(text);
                    } else {
                        dropped_send_texts = dropped_send_texts.saturating_add(1);
                    }
                }
                TriggerAction::LocalCommand { command } => {
                    if dispatch.local_commands.len() < MAX_TRIGGER_LOCAL_COMMANDS_PER_BATCH {
                        dispatch.local_commands.push(command);
                    } else {
                        dropped_local_commands = dropped_local_commands.saturating_add(1);
                    }
                }
                TriggerAction::Notification { message } => {
                    store.record_system_event(
                        session_id,
                        format!("PortMate notification: {message}"),
                    );
                    dispatch.effects.push(TriggerEffect {
                        session_id: session_id.to_string(),
                        trigger_id: trigger_match.trigger_id.clone(),
                        trigger_label: trigger_match.label.clone(),
                        kind: "notification".to_string(),
                        value: message,
                    });
                }
                TriggerAction::TimelineMark { label } => {
                    store.record_timeline_mark(TimelineMark {
                        id: Uuid::new_v4().to_string(),
                        session_id: session_id.to_string(),
                        ts: Utc::now(),
                        label,
                        details: Some(format!("trigger: {}", trigger_match.label)),
                    });
                }
                TriggerAction::CustomLink { url_template } => {
                    let (url, truncated) = render_trigger_custom_link(&url_template, text);
                    store.record_system_event(
                        session_id,
                        format!(
                            "PortMate: trigger custom link ({url}){}",
                            if truncated { " [truncated]" } else { "" }
                        ),
                    );
                    dispatch.effects.push(TriggerEffect {
                        session_id: session_id.to_string(),
                        trigger_id: trigger_match.trigger_id.clone(),
                        trigger_label: trigger_match.label.clone(),
                        kind: "custom-link".to_string(),
                        value: url,
                    });
                }
                TriggerAction::Sound { name } => {
                    store.record_system_event(
                        session_id,
                        format!("PortMate: trigger sound ({name})"),
                    );
                    dispatch.effects.push(TriggerEffect {
                        session_id: session_id.to_string(),
                        trigger_id: trigger_match.trigger_id.clone(),
                        trigger_label: trigger_match.label.clone(),
                        kind: "sound".to_string(),
                        value: name,
                    });
                }
            }
        }
    }
    if dropped_local_commands > 0 {
        store.record_system_event(
            session_id,
            format!(
                "PortMate: trigger local-command batch limit reached ({MAX_TRIGGER_LOCAL_COMMANDS_PER_BATCH}); skipped {dropped_local_commands} actions"
            ),
        );
    }
    if dropped_send_texts > 0 {
        store.record_system_event(
            session_id,
            format!(
                "PortMate: trigger send_text batch limit reached ({MAX_TRIGGER_SEND_TEXTS_PER_BATCH}); skipped {dropped_send_texts} actions"
            ),
        );
    }
    (dispatch, changed)
}

pub(super) fn render_trigger_custom_link(template: &str, matched_text: &str) -> (String, bool) {
    let mut output = String::new();
    let mut remaining = MAX_TRIGGER_CUSTOM_LINK_CHARACTERS;
    let replacement = matched_text.trim();
    let mut parts = template.split("{text}").peekable();
    while let Some(literal) = parts.next() {
        if append_bounded_trigger_text(&mut output, literal, &mut remaining) {
            return (output, true);
        }
        if parts.peek().is_some()
            && append_bounded_trigger_text(&mut output, replacement, &mut remaining)
        {
            return (output, true);
        }
    }
    (output, false)
}

fn append_bounded_trigger_text(output: &mut String, value: &str, remaining: &mut usize) -> bool {
    for character in value.chars() {
        if *remaining == 0 {
            return true;
        }
        output.push(character);
        *remaining -= 1;
    }
    false
}

pub(super) fn spawn_trigger_commands(
    store: Arc<Mutex<SessionStore>>,
    store_path: PathBuf,
    command_slots: Arc<tokio::sync::Semaphore>,
    session_id: String,
    commands: Vec<String>,
) {
    let mut skipped = 0_usize;
    for command in commands {
        let permit = match Arc::clone(&command_slots).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                skipped = skipped.saturating_add(1);
                continue;
            }
        };
        let command_store = Arc::clone(&store);
        let command_store_path = store_path.clone();
        let command_session_id = session_id.clone();
        tauri::async_runtime::spawn(async move {
            let output = run_shell_command(&command).await;
            drop(permit);
            let message = match output {
                Ok((code, stdout, stderr)) => format!(
                    "PortMate: trigger command exited code={code}: {}{}",
                    truncate_for_log(&stdout, 1600),
                    if stderr.trim().is_empty() {
                        String::new()
                    } else {
                        format!(" stderr={}", truncate_for_log(&stderr, 1600))
                    }
                ),
                Err(error) => format!("PortMate: trigger command failed: {error}"),
            };
            let persist = tauri::async_runtime::spawn_blocking(move || {
                record_trigger_command_event(
                    &command_store,
                    &command_store_path,
                    &command_session_id,
                    message,
                );
            });
            if let Err(error) = persist.await {
                eprintln!("PortMate: trigger command result task failed: {error}");
            }
        });
    }
    if skipped > 0 {
        record_trigger_command_event(
            &store,
            &store_path,
            &session_id,
            format!(
                "PortMate: trigger commands skipped: concurrent command limit reached ({MAX_TRIGGER_COMMAND_CONCURRENCY}); skipped {skipped} actions"
            ),
        );
    }
}

fn record_trigger_command_event(
    store: &Arc<Mutex<SessionStore>>,
    store_path: &Path,
    session_id: &str,
    message: String,
) {
    let Ok(mut store) = store.lock() else {
        eprintln!("PortMate: session store lock poisoned; dropping trigger command result");
        return;
    };
    store.record_system_event(session_id, message);
    if let Err(error) = persist_applied_store(&store, store_path, "trigger command result event") {
        eprintln!("PortMate: failed to persist trigger command output: {error}");
    }
}

pub(super) fn spawn_trigger_send_text_batch(
    io: SessionIo,
    session_id: String,
    source_runtime_id: String,
    texts: Vec<String>,
) {
    if texts.is_empty() {
        return;
    }
    let permit = match Arc::clone(&io.trigger_send_batch_slots).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            record_trigger_send_text_event(
                &io.store,
                &io.store_path,
                &session_id,
                format!(
                    "PortMate: trigger send_text batch skipped: concurrent batch limit reached ({MAX_TRIGGER_SEND_BATCH_CONCURRENCY})"
                ),
            );
            return;
        }
    };
    tauri::async_runtime::spawn(async move {
        let total = texts.len();
        let mut failure = None;
        for (index, text) in texts.into_iter().enumerate() {
            if let Err(error) =
                send_trigger_text_inner(&io, &session_id, &source_runtime_id, &text).await
            {
                failure = Some(format!(
                    "PortMate: trigger send_text batch failed at {}/{}; skipped {} remaining actions: {error}",
                    index + 1,
                    total,
                    total.saturating_sub(index + 1)
                ));
                break;
            }
        }
        drop(permit);
        if let Some(message) = failure {
            let store = Arc::clone(&io.store);
            let store_path = io.store_path.clone();
            let persist_session_id = session_id.clone();
            let persist = tauri::async_runtime::spawn_blocking(move || {
                record_trigger_send_text_event(&store, &store_path, &persist_session_id, message);
            });
            if let Err(error) = persist.await {
                eprintln!("PortMate: trigger send_text result task failed: {error}");
            }
        }
    });
}

pub(super) async fn send_trigger_text_inner(
    io: &SessionIo,
    session_id: &str,
    source_runtime_id: &str,
    text: &str,
) -> Result<SessionEvent, String> {
    let lane = outbound_lane(&io.store_path, session_id)?;
    let _lane_guard = lane.lock().await;
    send_text_under_outbound_lane(
        io,
        session_id,
        text,
        "trigger",
        Some("trigger_send_text"),
        Some(source_runtime_id),
    )
    .await
}

fn record_trigger_send_text_event(
    store: &Arc<Mutex<SessionStore>>,
    store_path: &Path,
    session_id: &str,
    message: String,
) {
    let Ok(mut store) = store.lock() else {
        eprintln!("PortMate: session store lock poisoned; dropping trigger send_text result");
        return;
    };
    store.record_system_event(session_id, message);
    if let Err(error) = persist_applied_store(&store, store_path, "trigger send_text result event")
    {
        eprintln!("PortMate: failed to persist trigger send_text result: {error}");
    }
}

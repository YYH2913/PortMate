use super::*;

pub(super) const COMMAND_HISTORY_UPDATED_EVENT: &str = "portmate-command-history-updated";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommandHistorySnapshot {
    entries: Vec<portmate_core::CommandHistoryEntry>,
    migrated: bool,
    revision: u64,
}

fn snapshot(
    store: &SessionStore,
    entries: Vec<portmate_core::CommandHistoryEntry>,
) -> CommandHistorySnapshot {
    CommandHistorySnapshot {
        entries,
        migrated: store.command_history_migrated,
        revision: store.command_history_revision,
    }
}

fn emit_snapshot(state: &AppState, snapshot: &CommandHistorySnapshot) {
    if let Some(app) = &state.app_handle {
        if let Err(error) = app.emit(COMMAND_HISTORY_UPDATED_EVENT, snapshot) {
            eprintln!("PortMate: failed to emit command history update: {error}");
        }
    }
}

fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

#[tauri::command]
pub(crate) fn list_command_history(
    state: State<'_, AppState>,
) -> Result<CommandHistorySnapshot, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let entries = SessionStore::normalized_command_history(
        &store.command_history,
        store.command_history_policy.limit,
        store.command_history_policy.retention_days,
        now_millis(),
    )?;
    Ok(snapshot(&store, entries))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum CommandSubmissionSource {
    Interactive,
    Paste,
    FreeInput,
    QuickCommand,
    SendPanel,
    McpRunCommand,
    McpSendText,
    SyncBroadcast,
}

#[tauri::command]
pub(crate) fn configure_command_history(
    state: State<'_, AppState>,
    enabled: bool,
    limit: usize,
    retention_days: u32,
) -> Result<CommandHistorySnapshot, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let result = commit_store_mutation(&mut store, &state.store_path, |next_store| {
        configure_history_policy(next_store, enabled, limit, retention_days, now_millis())?;
        Ok(snapshot(next_store, next_store.command_history.clone()))
    })?;
    emit_snapshot(&state, &result);
    Ok(result)
}

fn configure_history_policy(
    store: &mut SessionStore,
    enabled: bool,
    limit: usize,
    retention_days: u32,
    now: i64,
) -> Result<(), String> {
    // Validate and prune before changing policy, within the same disk transaction.
    let pending_migration =
        enabled && !store.command_history_migrated && store.command_history.is_empty();
    let entries = if enabled {
        store.command_history.clone()
    } else {
        Vec::new()
    };
    store.replace_command_history(&entries, limit, retention_days, now)?;
    if pending_migration {
        store.command_history_migrated = false;
    }
    store.command_history_policy = portmate_core::CommandHistoryPolicy {
        enabled,
        limit,
        retention_days,
    };
    Ok(())
}

#[tauri::command]
pub(crate) fn migrate_command_history(
    state: State<'_, AppState>,
    entries: Vec<portmate_core::CommandHistoryEntry>,
) -> Result<CommandHistorySnapshot, String> {
    if entries.len() > MAX_COMMAND_HISTORY_ENTRIES {
        return Err(format!(
            "command history migration exceeds {MAX_COMMAND_HISTORY_ENTRIES} entries"
        ));
    }
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if !store.command_history_policy.enabled {
        return Ok(snapshot(&store, Vec::new()));
    }
    let limit = store.command_history_policy.limit;
    let retention_days = store.command_history_policy.retention_days;
    let now = now_millis();
    if should_skip_empty_migration(&store, &entries) {
        let entries = SessionStore::normalized_command_history(&[], limit, retention_days, now)?;
        let result = snapshot(&store, entries);
        emit_snapshot(&state, &result);
        return Ok(result);
    }
    let result = commit_store_mutation(&mut store, &state.store_path, |next_store| {
        let source = if next_store.command_history_migrated {
            next_store.command_history.clone()
        } else {
            entries
        };
        let entries = next_store.replace_command_history(&source, limit, retention_days, now)?;
        Ok(snapshot(next_store, entries))
    })?;
    emit_snapshot(&state, &result);
    Ok(result)
}

fn should_skip_empty_migration(
    store: &SessionStore,
    entries: &[portmate_core::CommandHistoryEntry],
) -> bool {
    !store.command_history_migrated && store.command_history.is_empty() && entries.is_empty()
}

#[tauri::command]
pub(crate) fn record_command_history(
    state: State<'_, AppState>,
    command: String,
    session_id: Option<String>,
    source: CommandSubmissionSource,
) -> Result<CommandHistorySnapshot, String> {
    record_command_submission(&state, command, session_id, source)
}

// All desktop and MCP writers use the canonical policy held by the backend.
// MCP send_text is deliberately stateless: only complete lines in this payload
// are submissions. Raw fragments, cursor keys and bracketed paste aren't commands.
fn submitted_commands(text: &str, source: CommandSubmissionSource) -> Vec<String> {
    if matches!(source, CommandSubmissionSource::McpSendText | CommandSubmissionSource::SendPanel)
        && text
            .chars()
            .any(|c| c.is_control() && c != '\r' && c != '\n')
    {
        return Vec::new();
    }
    match source {
        CommandSubmissionSource::McpSendText => text
            .split_inclusive(['\r', '\n'])
            .filter(|line| line.ends_with(['\r', '\n']))
            .map(|line| line.trim_end_matches(['\r', '\n']))
            .filter(|line| !line.trim().is_empty() && !line.chars().any(char::is_control))
            .map(str::to_string)
            .collect(),
        CommandSubmissionSource::SendPanel => text
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
        _ => {
            let command = text.trim_end_matches(['\r', '\n']);
            if command.trim().is_empty()
                || command
                    .chars()
                    .any(|c| c.is_control() && c != '\n' && c != '\t')
            {
                Vec::new()
            } else {
                vec![command.to_string()]
            }
        }
    }
}

fn append_submissions(
    store: &mut SessionStore,
    commands: Vec<String>,
    session_id: Option<String>,
    now: i64,
) -> Result<(), String> {
    let policy = store.command_history_policy.clone();
    if !policy.enabled {
        return Ok(());
    }
    for command in commands {
        store.record_command_history(
            command,
            session_id.clone(),
            policy.limit,
            policy.retention_days,
            now,
        )?;
    }
    Ok(())
}

pub(super) fn record_command_submission(
    state: &AppState,
    text: String,
    session_id: Option<String>,
    source: CommandSubmissionSource,
) -> Result<CommandHistorySnapshot, String> {
    let commands = submitted_commands(&text, source);
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if !store.command_history_policy.enabled || commands.is_empty() {
        return Ok(snapshot(&store, store.command_history.clone()));
    }
    if let Some(id) = session_id.as_deref() {
        if store.profile(id).is_none() {
            return Err(format!("unknown session: {id}"));
        }
    }
    let result = commit_store_mutation(&mut store, &state.store_path, |next_store| {
        append_submissions(next_store, commands, session_id, now_millis())?;
        Ok(snapshot(next_store, next_store.command_history.clone()))
    })?;
    emit_snapshot(state, &result);
    Ok(result)
}

#[tauri::command]
pub(crate) fn merge_command_history(
    state: State<'_, AppState>,
    entries: Vec<portmate_core::CommandHistoryEntry>,
) -> Result<CommandHistorySnapshot, String> {
    if entries.len() > MAX_COMMAND_HISTORY_ENTRIES {
        return Err(format!(
            "command history merge exceeds {MAX_COMMAND_HISTORY_ENTRIES} entries"
        ));
    }
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if !store.command_history_policy.enabled {
        return Ok(snapshot(&store, Vec::new()));
    }
    let limit = store.command_history_policy.limit;
    let retention_days = store.command_history_policy.retention_days;
    let now = now_millis();
    let result = commit_store_mutation(&mut store, &state.store_path, |next_store| {
        let normalized = next_store.merge_command_history(&entries, limit, retention_days, now)?;
        Ok(snapshot(next_store, normalized))
    })?;
    emit_snapshot(&state, &result);
    Ok(result)
}

#[tauri::command]
pub(crate) fn normalize_command_history(
    state: State<'_, AppState>,
) -> Result<CommandHistorySnapshot, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let limit = store.command_history_policy.limit;
    let retention_days = store.command_history_policy.retention_days;
    let now = now_millis();
    let result = commit_store_mutation(&mut store, &state.store_path, |next_store| {
        let current = next_store.command_history.clone();
        let entries = next_store.replace_command_history(&current, limit, retention_days, now)?;
        Ok(snapshot(next_store, entries))
    })?;
    emit_snapshot(&state, &result);
    Ok(result)
}

#[tauri::command]
pub(crate) fn clear_command_history(
    state: State<'_, AppState>,
) -> Result<CommandHistorySnapshot, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let result = commit_store_mutation(&mut store, &state.store_path, |next_store| {
        let entries = next_store.replace_command_history(
            &[],
            MAX_COMMAND_HISTORY_ENTRIES,
            0,
            now_millis(),
        )?;
        Ok(snapshot(next_store, entries))
    })?;
    emit_snapshot(&state, &result);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_text_requires_complete_submissions_and_rejects_editor_input() {
        use CommandSubmissionSource::*;
        for text in [
            "echo partial",
            "\r",
            "   \n",
            "abc\x1b[D\n",
            "\x1b[200~one\ntwo\n\x1b[201~",
        ] {
            assert!(submitted_commands(text, McpSendText).is_empty(), "{text:?}");
        }
        assert_eq!(
            submitted_commands("one\r\ntwo\nunfinished", McpSendText),
            ["one", "two"]
        );
        assert_eq!(
            submitted_commands("echo submitted", McpRunCommand),
            ["echo submitted"]
        );
    }

    #[test]
    fn all_submission_sources_share_disabled_limit_and_retention_policy() {
        let mut store = SessionStore::default();
        let now = 10 * 86_400_000;
        configure_history_policy(&mut store, true, 2, 1, now).unwrap();
        for (command, timestamp) in [
            ("expired", now - 2 * 86_400_000),
            ("first", now - 1),
            ("second", now),
            ("third", now),
        ] {
            append_submissions(
                &mut store,
                submitted_commands(command, CommandSubmissionSource::McpRunCommand),
                None,
                timestamp,
            )
            .unwrap();
        }
        assert_eq!(
            store
                .command_history
                .iter()
                .map(|e| e.command.as_str())
                .collect::<Vec<_>>(),
            ["third", "second"]
        );
        configure_history_policy(&mut store, false, 2, 1, now).unwrap();
        let revision = store.command_history_revision;
        for source in [
            CommandSubmissionSource::Interactive,
            CommandSubmissionSource::Paste,
            CommandSubmissionSource::FreeInput,
            CommandSubmissionSource::QuickCommand,
            CommandSubmissionSource::SendPanel,
            CommandSubmissionSource::McpRunCommand,
            CommandSubmissionSource::McpSendText,
            CommandSubmissionSource::SyncBroadcast,
        ] {
            append_submissions(
                &mut store,
                submitted_commands("private\r", source),
                None,
                now,
            )
            .unwrap();
        }
        assert!(store.command_history.is_empty());
        assert_eq!(store.command_history_revision, revision);
        let restored: SessionStore =
            serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
        assert_eq!(
            restored.command_history_policy,
            store.command_history_policy
        );
    }

    #[test]
    fn retention_zero_keeps_old_entries_and_policy_update_prunes_them() {
        let mut store = SessionStore::default();
        configure_history_policy(&mut store, true, 10, 0, 1).unwrap();
        assert!(!store.command_history_migrated);
        append_submissions(&mut store, vec!["old".into()], None, 1).unwrap();
        let now = 3 * 86_400_000;
        append_submissions(&mut store, vec!["new".into()], None, now).unwrap();
        assert_eq!(store.command_history.len(), 2);
        configure_history_policy(&mut store, true, 10, 1, now).unwrap();
        assert_eq!(store.command_history.len(), 1);
        assert_eq!(store.command_history[0].command, "new");
        let policy = store.command_history_policy.clone();
        assert!(configure_history_policy(&mut store, true, 0, 1, now).is_err());
        assert_eq!(store.command_history_policy, policy);
    }

    #[test]
    fn empty_legacy_history_migration_is_persistence_free() {
        let store = SessionStore::default();
        assert!(should_skip_empty_migration(&store, &[]));

        let mut store_with_history = SessionStore::default();
        store_with_history
            .command_history
            .push(portmate_core::CommandHistoryEntry {
                command: "git status".to_string(),
                recorded_at: 1,
                session_id: None,
            });
        assert!(!should_skip_empty_migration(&store_with_history, &[]));

        let mut migrated_store = SessionStore::default();
        migrated_store.command_history_migrated = true;
        assert!(!should_skip_empty_migration(&migrated_store, &[]));
    }
}

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
    limit: usize,
    retention_days: u32,
) -> Result<CommandHistorySnapshot, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let entries = SessionStore::normalized_command_history(
        &store.command_history,
        limit,
        retention_days,
        now_millis(),
    )?;
    Ok(snapshot(&store, entries))
}

#[tauri::command]
pub(crate) fn migrate_command_history(
    state: State<'_, AppState>,
    entries: Vec<portmate_core::CommandHistoryEntry>,
    limit: usize,
    retention_days: u32,
) -> Result<CommandHistorySnapshot, String> {
    if entries.len() > MAX_COMMAND_HISTORY_ENTRIES {
        return Err(format!(
            "command history migration exceeds {MAX_COMMAND_HISTORY_ENTRIES} entries"
        ));
    }
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
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
    limit: usize,
    retention_days: u32,
) -> Result<CommandHistorySnapshot, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let now = now_millis();
    let result = commit_store_mutation(&mut store, &state.store_path, |next_store| {
        if let Some(session_id) = session_id.as_deref() {
            if next_store.profile(session_id).is_none() {
                return Err(format!("unknown session: {session_id}"));
            }
        }
        let entries = next_store.record_command_history(
            command,
            session_id,
            limit,
            retention_days,
            now,
        )?;
        Ok(snapshot(next_store, entries))
    })?;
    emit_snapshot(&state, &result);
    Ok(result)
}

#[tauri::command]
pub(crate) fn merge_command_history(
    state: State<'_, AppState>,
    entries: Vec<portmate_core::CommandHistoryEntry>,
    limit: usize,
    retention_days: u32,
) -> Result<CommandHistorySnapshot, String> {
    if entries.len() > MAX_COMMAND_HISTORY_ENTRIES {
        return Err(format!(
            "command history merge exceeds {MAX_COMMAND_HISTORY_ENTRIES} entries"
        ));
    }
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
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
    limit: usize,
    retention_days: u32,
) -> Result<CommandHistorySnapshot, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
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

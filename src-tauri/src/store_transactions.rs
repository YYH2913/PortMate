use super::*;

/// Runs copy-on-write mutations that do not enqueue system events. SessionStore
/// clones share the event sink, so event-producing transactions need a dedicated
/// commit path that cannot publish rolled-back events.
pub(super) fn commit_store_mutation<ResultValue, Mutate>(
    store: &mut SessionStore,
    store_path: &Path,
    mutate: Mutate,
) -> Result<ResultValue, String>
where
    Mutate: FnOnce(&mut SessionStore) -> Result<ResultValue, String>,
{
    commit_store_mutation_with(
        store,
        mutate,
        |next_store| save_store(store_path, next_store),
        |next_store| verify_persisted_store_commit(store_path, next_store),
    )
}

pub(super) fn commit_store_mutation_with<ResultValue, Mutate, Persist, VerifyAfterError>(
    store: &mut SessionStore,
    mutate: Mutate,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<ResultValue, String>
where
    Mutate: FnOnce(&mut SessionStore) -> Result<ResultValue, String>,
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    let mut next_store = store.clone();
    let result = mutate(&mut next_store)?;
    if let Err(save_error) = persist(&next_store) {
        match verify_after_error(&next_store) {
            Ok(true) => {
                eprintln!(
                    "PortMate: store save returned an error, but the intended snapshot was verified on disk: {save_error}"
                );
            }
            Ok(false) => return Err(save_error),
            Err(verify_error) => {
                return Err(format!(
                    "{save_error}; 无法判定 Store 提交是否生效，请重启应用: {verify_error}"
                ));
            }
        }
    }
    *store = next_store;
    Ok(result)
}

pub(super) fn verify_persisted_store_commit(
    path: &Path,
    expected: &SessionStore,
) -> Result<bool, String> {
    let persisted = read_persisted_store_for_migration(path)?;
    let persisted = serde_json::to_value(persisted)
        .map_err(|error| format!("failed to encode persisted Store for verification: {error}"))?;
    let expected = serde_json::to_value(expected)
        .map_err(|error| format!("failed to encode expected Store for verification: {error}"))?;
    if persisted != expected {
        return Ok(false);
    }

    let version = store_snapshot_version(path)?;
    if !matches!(version, StoreSnapshotVersion::Sha256(_)) {
        return Err("persisted Store exists but has no verifiable snapshot version".to_string());
    }
    STORE_SNAPSHOT_VERSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?
        .insert(path.to_path_buf(), version);
    Ok(true)
}

pub(super) fn commit_tracked_store_mutation<ResultValue, Mutate>(
    store: &mut SessionStore,
    store_path: &Path,
    mutate: Mutate,
) -> Result<ResultValue, String>
where
    Mutate: FnOnce(&mut SessionStore) -> Result<(ResultValue, Vec<String>), String>,
{
    commit_tracked_store_mutation_with(
        store,
        mutate,
        |next_store| save_store(store_path, next_store),
        |next_store| verify_persisted_store_commit(store_path, next_store),
    )
}

pub(super) fn commit_tracked_store_mutation_with<ResultValue, Mutate, Persist, VerifyAfterError>(
    store: &mut SessionStore,
    mutate: Mutate,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<ResultValue, String>
where
    Mutate: FnOnce(&mut SessionStore) -> Result<(ResultValue, Vec<String>), String>,
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    let before = store.clone();
    let (result, event_ids) = mutate(store)?;
    if let Err(save_error) = persist(store) {
        match verify_after_error(store) {
            Ok(true) => {
                eprintln!(
                    "PortMate: tracked Store save returned an error, but the intended snapshot was verified on disk: {save_error}"
                );
            }
            Ok(false) => {
                for event_id in &event_ids {
                    store.discard_queued_system_event(event_id);
                }
                *store = before;
                return Err(save_error);
            }
            Err(verify_error) => {
                for event_id in &event_ids {
                    store.discard_queued_system_event(event_id);
                }
                *store = before;
                return Err(format!(
                    "{save_error}; 无法判定 Store 提交是否生效，请重启应用: {verify_error}"
                ));
            }
        }
    }
    Ok(result)
}

pub(super) fn persist_applied_store(
    store: &SessionStore,
    store_path: &Path,
    operation: &str,
) -> Result<(), String> {
    persist_applied_store_with(
        store,
        operation,
        |next_store| save_store(store_path, next_store),
        |next_store| verify_persisted_store_commit(store_path, next_store),
    )
}

pub(super) fn persist_applied_store_with<Persist, VerifyAfterError>(
    store: &SessionStore,
    operation: &str,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<(), String>
where
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    let Err(save_error) = persist(store) else {
        return Ok(());
    };
    match verify_after_error(store) {
        Ok(true) => {
            eprintln!(
                "PortMate: {operation} save returned an error, but the intended snapshot was verified on disk: {save_error}"
            );
            Ok(())
        }
        Ok(false) => Err(save_error),
        Err(verify_error) => Err(format!(
            "{save_error}; 无法判定已应用的 {operation} 是否保存，请重启应用: {verify_error}"
        )),
    }
}

pub(super) fn record_applied_system_event(
    state: &AppState,
    session_id: &str,
    message: String,
    operation: &str,
) {
    let mut store = match state.store.lock() {
        Ok(store) => store,
        Err(error) => {
            eprintln!("PortMate: {operation} succeeded but the system event lock failed: {error}");
            return;
        }
    };
    if let Err(error) = record_applied_system_event_with(
        &mut store,
        session_id,
        message,
        |next_store| save_store(&state.store_path, next_store),
        |next_store| verify_persisted_store_commit(&state.store_path, next_store),
    ) {
        eprintln!("PortMate: {operation} succeeded but system event persistence degraded: {error}");
    }
}

pub(super) fn record_applied_system_event_with<Persist, VerifyAfterError>(
    store: &mut SessionStore,
    session_id: &str,
    message: String,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<(), String>
where
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    let event_id = store
        .record_system_event_tracked(session_id, message)
        .ok_or_else(|| {
            format!("session profile unavailable after applied operation: {session_id}")
        })?;
    if let Err(error) = persist_applied_store_with(
        store,
        "applied operation event",
        persist,
        verify_after_error,
    ) {
        if let Some(event) = store.events.iter_mut().find(|event| event.id == event_id) {
            append_logging_error(event, format!("store save failed: {error}"));
        }
        return Err(error);
    }
    Ok(())
}

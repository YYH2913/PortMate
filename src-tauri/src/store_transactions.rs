use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoreCommitState {
    NotCommitted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StoreCommitError {
    pub(super) state: StoreCommitState,
    pub(super) message: String,
}

impl StoreCommitError {
    pub(super) fn not_committed(message: String) -> Self {
        Self {
            state: StoreCommitState::NotCommitted,
            message,
        }
    }

    fn unknown(message: String) -> Self {
        Self {
            state: StoreCommitState::Unknown,
            message,
        }
    }
}

impl std::fmt::Display for StoreCommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

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
    commit_store_mutation_with_state(
        store,
        mutate,
        |next_store| save_store(store_path, next_store),
        |next_store| verify_persisted_store_commit(store_path, next_store),
    )
    .map_err(|error| error.message)
}

pub(super) fn commit_store_mutation_state<ResultValue, Mutate>(
    store: &mut SessionStore,
    store_path: &Path,
    mutate: Mutate,
) -> Result<ResultValue, StoreCommitError>
where
    Mutate: FnOnce(&mut SessionStore) -> Result<ResultValue, String>,
{
    commit_store_mutation_with_state(
        store,
        mutate,
        |next_store| save_store(store_path, next_store),
        |next_store| verify_persisted_store_commit(store_path, next_store),
    )
}

#[cfg(test)]
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
    commit_store_mutation_with_state(store, mutate, persist, verify_after_error)
        .map_err(|error| error.message)
}

pub(super) fn commit_store_mutation_with_state<
    ResultValue,
    Mutate,
    Persist,
    VerifyAfterError,
>(
    store: &mut SessionStore,
    mutate: Mutate,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<ResultValue, StoreCommitError>
where
    Mutate: FnOnce(&mut SessionStore) -> Result<ResultValue, String>,
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    let mut next_store = store.clone();
    let result = mutate(&mut next_store).map_err(StoreCommitError::not_committed)?;
    if let Err(save_error) = persist(&next_store) {
        match verify_after_error(&next_store) {
            Ok(true) => {
                eprintln!(
                    "PortMate: store save returned an error, but the intended snapshot was verified on disk: {save_error}"
                );
            }
            Ok(false) => return Err(StoreCommitError::not_committed(save_error)),
            Err(verify_error) => {
                return Err(StoreCommitError::unknown(format!(
                    "{save_error}; 无法判定 Store 提交是否生效，请重启应用: {verify_error}"
                )));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_secrets_are_retained_when_store_commit_cannot_be_verified() {
        for caller in ["profile_proxy", "identity_rotation", "one_key"] {
            let mut store = SessionStore::default();
            let durable = Mutex::new(None);
            let new_secret_ref = format!("secret:{caller}");
            let mut secrets = HashSet::from([new_secret_ref.clone()]);
            let error = commit_store_mutation_with_state(
                &mut store,
                |next_store| {
                    next_store.one_keys.push(OneKeyCredential {
                        id: caller.to_string(),
                        label: caller.to_string(),
                        kind: OneKeyKind::Account,
                        username: "user".to_string(),
                        password_secret_ref: Some(new_secret_ref.clone()),
                        passphrase_secret_ref: None,
                        identity: None,
                        session_ids: Vec::new(),
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                    });
                    Ok(())
                },
                |next_store| {
                    *durable.lock().unwrap() = Some(next_store.clone());
                    Err("post-commit read failed".to_string())
                },
                |_| Err("verification unavailable".to_string()),
            )
            .unwrap_err();
            if error.state == StoreCommitState::NotCommitted {
                secrets.remove(&new_secret_ref);
            }
            assert_eq!(error.state, StoreCommitState::Unknown);
            let durable = durable.lock().unwrap();
            let persisted_ref = durable.as_ref().unwrap().one_keys[0]
                .password_secret_ref
                .as_ref()
                .unwrap();
            assert!(secrets.contains(persisted_ref));
            assert!(store.one_keys.is_empty());
        }
    }

    #[test]
    fn definite_store_failures_allow_generated_secret_cleanup() {
        let mut store = SessionStore::default();
        let error = commit_store_mutation_with_state(
            &mut store,
            |_| Ok(()),
            |_| Err("write rejected".to_string()),
            |_| Ok(false),
        )
        .unwrap_err();
        assert_eq!(error.state, StoreCommitState::NotCommitted);
        assert_eq!(error.message, "write rejected");

        let error = commit_store_mutation_with_state(
            &mut store,
            |_| Err::<(), _>("mutation rejected".to_string()),
            |_| panic!("invalid mutations cannot persist"),
            |_| panic!("invalid mutations cannot need verification"),
        )
        .unwrap_err();
        assert_eq!(error.state, StoreCommitState::NotCommitted);
    }
}

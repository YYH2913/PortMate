use super::session_commands::{
    cancel_pending_session_opens, cleanup_deleted_session_runtime_state,
    close_session_under_lifecycle_lock, session_has_registered_runtime, session_lifecycle_lane,
};
use super::*;

pub(super) async fn delete_session_profile_inner(
    state: &AppState,
    session_id: String,
) -> Result<DeleteSessionProfileResponse, String> {
    cancel_pending_session_opens(state, &session_id)?;
    let lifecycle_lane = session_lifecycle_lane(state, &session_id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
    delete_session_profile_under_lifecycle_lock(state, session_id).await
}

pub(super) async fn delete_session_profile_under_lifecycle_lock(
    state: &AppState,
    session_id: String,
) -> Result<DeleteSessionProfileResponse, String> {
    let (status, has_active_transfer) = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        let profile = store
            .profiles
            .iter()
            .find(|profile| profile.id == session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        let status = store
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == profile.id)
            .map(|runtime| runtime.status)
            .unwrap_or(SessionStatus::Disconnected);
        let has_active_transfer = store.transfers.iter().any(|transfer| {
            transfer.session_id == profile.id
                && matches!(
                    transfer.status,
                    TransferStatus::Queued | TransferStatus::Running
                )
        });
        (status, has_active_transfer)
    };
    if has_active_transfer {
        return Err("会话存在排队中或运行中的传输任务，取消或等待任务结束后才能删除".to_string());
    }

    if !matches!(
        status,
        SessionStatus::Disconnected | SessionStatus::Blocked | SessionStatus::Error
    ) || session_has_registered_runtime(state, &session_id)?
    {
        close_session_under_lifecycle_lock(state, session_id.clone()).await?;
    }

    let _credential_guard = lock_credential_operations(state)?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profile(&session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    let mut orphan_secret_candidates = profile_secret_refs(&profile);
    for one_key in &store.one_keys {
        let Some(identity) = one_key
            .identity
            .as_ref()
            .filter(|identity| identity.source_profile_id == session_id)
        else {
            continue;
        };
        if let Some(secret_ref) = identity
            .identity
            .secret_ref
            .as_deref()
            .and_then(canonical_secret_ref)
        {
            orphan_secret_candidates.insert(secret_ref);
        }
    }
    let transfer_ids = store
        .transfers
        .iter()
        .filter(|transfer| transfer.session_id == session_id)
        .map(|transfer| transfer.id.clone())
        .collect::<Vec<_>>();

    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        let deleted = next_store.delete_profile_deferred_system_event_cleanup(&session_id)?;
        next_store.record_audit(AuditRecord {
            id: Uuid::new_v4().to_string(),
            ts: Utc::now(),
            actor: "desktop-user".to_string(),
            action: "delete_session_profile".to_string(),
            session_id: Some(session_id.clone()),
            decision: "recorded".to_string(),
            details: BTreeMap::from([
                ("profileName".to_string(), deleted.name),
                ("diskLogs".to_string(), "retained".to_string()),
            ]),
        });
        Ok(())
    })?;
    store.discard_system_events_for_session(&session_id);

    for secret_ref in orphan_secret_candidates {
        if secret_ref_usage_count(&store, &secret_ref) == 0 {
            if let Err(error) = delete_secret_from_store(&secret_ref) {
                eprintln!(
                    "PortMate: profile deleted but orphan secret cleanup failed ({secret_ref}): {error}"
                );
            }
        }
    }
    let response = DeleteSessionProfileResponse {
        deleted_profile_id: session_id.clone(),
        sessions: store.summaries(),
        one_keys: one_key_summaries(&store),
        host_keys: store.host_keys.clone(),
        grants: store.grants.clone(),
    };
    drop(store);

    cleanup_deleted_session_runtime_state(state, &session_id, &transfer_ids);
    if let Some(app_handle) = &state.app_handle {
        let _ = app_handle.emit("portmate-session-profile-deleted", session_id);
    }
    Ok(response)
}

#[tauri::command]
pub(crate) async fn delete_session_profile(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<DeleteSessionProfileResponse, String> {
    delete_session_profile_inner(state.inner(), session_id).await
}

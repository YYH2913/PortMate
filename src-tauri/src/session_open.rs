use super::session_close::{close_session_under_lifecycle_lock, session_has_registered_runtime};
use super::session_commands::{
    describe_endpoint, mark_session_connected_with_events, record_connection_failure,
};
use super::*;

pub(super) const MAX_CONCURRENT_SESSION_OPENS: usize = 64;

pub(super) fn session_lifecycle_lane(
    state: &AppState,
    session_id: &str,
) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let key = (state.store_path.clone(), session_id.to_string());
    let mut lanes = SESSION_LIFECYCLE_LANES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "session lifecycle lane registry poisoned".to_string())?;
    lanes.retain(|_, lane| lane.strong_count() > 0);
    if let Some(lane) = lanes.get(&key).and_then(Weak::upgrade) {
        return Ok(lane);
    }
    let lane = Arc::new(tokio::sync::Mutex::new(()));
    lanes.insert(key, Arc::downgrade(&lane));
    Ok(lane)
}

pub(super) fn register_session_open_cancellation(
    state: &AppState,
    session_id: &str,
) -> Result<Arc<SessionOpenCancellation>, String> {
    let key = (state.store_path.clone(), session_id.to_string());
    let slot = Arc::clone(&state.session_open_slots)
        .try_acquire_owned()
        .map_err(|_| {
            format!("session connection limit reached ({MAX_CONCURRENT_SESSION_OPENS})")
        })?;
    let cancellation = Arc::new(SessionOpenCancellation::new(slot));
    let mut cancellations = SESSION_OPEN_CANCELLATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "session open cancellation registry poisoned".to_string())?;
    cancellations.retain(|_, pending| {
        pending.retain(|cancellation| cancellation.strong_count() > 0);
        !pending.is_empty()
    });
    if cancellations.contains_key(&key) {
        return Err("session connection is already pending".to_string());
    }
    cancellations
        .entry(key)
        .or_default()
        .push(Arc::downgrade(&cancellation));
    Ok(cancellation)
}

pub(super) fn cancel_pending_session_opens(
    state: &AppState,
    session_id: &str,
) -> Result<usize, String> {
    let key = (state.store_path.clone(), session_id.to_string());
    let mut cancellations = SESSION_OPEN_CANCELLATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "session open cancellation registry poisoned".to_string())?;
    let Some(pending) = cancellations.get_mut(&key) else {
        return Ok(0);
    };
    let mut cancelled = 0;
    pending.retain(|cancellation| {
        let Some(cancellation) = cancellation.upgrade() else {
            return false;
        };
        cancellation.cancel();
        cancelled += 1;
        true
    });
    Ok(cancelled)
}

#[derive(Default)]
pub(super) struct SessionOpenCredentials {
    pub(super) username: Option<String>,
    pub(super) password: Option<String>,
    pub(super) passphrase: Option<String>,
    pub(super) identity: Option<IdentityRef>,
    pub(super) isolate_saved_ssh_credentials: bool,
    pub(super) credential_binding: Option<SessionCredentialBinding>,
}

pub(super) fn apply_session_open_profile_credentials(
    profile: &mut SessionProfile,
    username: Option<&str>,
    identity: Option<&IdentityRef>,
    isolate_saved_ssh_credentials: bool,
) -> Result<(), String> {
    if username.is_none() && identity.is_none() && !isolate_saved_ssh_credentials {
        return Ok(());
    }
    let ssh = ssh_connection_mut(profile)?;
    if let Some(username) = username {
        ssh.username = username.to_string();
    }
    if isolate_saved_ssh_credentials {
        ssh.password_secret_ref = None;
        ssh.passphrase_secret_ref = None;
    }
    if let Some(identity) = identity {
        ssh.identity_refs = vec![identity.clone()];
        ssh.identity_policy.identities_only = true;
        if !ssh
            .identity_policy
            .auth_order
            .contains(&AuthMethod::PublicKey)
        {
            ssh.identity_policy
                .auth_order
                .insert(0, AuthMethod::PublicKey);
        }
    }
    Ok(())
}

pub(super) async fn open_session_inner(
    state: AppState,
    session_id: String,
    credentials: SessionOpenCredentials,
) -> Result<SessionSummary, String> {
    open_session_inner_with_validation(state, session_id, credentials, None).await
}

pub(super) async fn open_session_inner_with_validation(
    state: AppState,
    session_id: String,
    credentials: SessionOpenCredentials,
    commit_validation: Option<CommitValidation>,
) -> Result<SessionSummary, String> {
    let cancellation = register_session_open_cancellation(&state, &session_id)?;
    open_reserved_session_inner_with_validation(
        state,
        session_id,
        credentials,
        cancellation,
        commit_validation,
    )
    .await
}

pub(super) async fn open_reserved_session_inner(
    state: AppState,
    session_id: String,
    credentials: SessionOpenCredentials,
    cancellation: Arc<SessionOpenCancellation>,
) -> Result<SessionSummary, String> {
    open_reserved_session_inner_with_validation(
        state,
        session_id,
        credentials,
        cancellation,
        None,
    )
    .await
}

async fn open_reserved_session_inner_with_validation(
    state: AppState,
    session_id: String,
    credentials: SessionOpenCredentials,
    cancellation: Arc<SessionOpenCancellation>,
    commit_validation: Option<CommitValidation>,
) -> Result<SessionSummary, String> {
    let lifecycle_lane = session_lifecycle_lane(&state, &session_id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
    if cancellation.is_cancelled() {
        return Err("session connection was cancelled before it started".to_string());
    }
    if let Some(validate) = commit_validation {
        validate()?;
    }
    open_session_under_lifecycle_lock(state, session_id, credentials, cancellation).await
}

pub(super) async fn open_session_under_lifecycle_lock(
    state: AppState,
    session_id: String,
    credentials: SessionOpenCredentials,
    cancellation: Arc<SessionOpenCancellation>,
) -> Result<SessionSummary, String> {
    ensure_session_can_open(&state, &session_id)?;
    clear_active_command(&state.session_io(), &session_id);
    let SessionOpenCredentials {
        username,
        password,
        passphrase,
        identity,
        isolate_saved_ssh_credentials,
        credential_binding,
    } = credentials;
    let profile = {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        let saved_profile = store
            .profile(&session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        let endpoint = describe_endpoint(&saved_profile);
        let mut profile = normalize_session_profile(saved_profile);
        if let Some(binding) = credential_binding.as_ref() {
            validate_session_credential_binding(&profile, binding)?;
        }
        apply_session_open_profile_credentials(
            &mut profile,
            username.as_deref(),
            identity.as_ref(),
            isolate_saved_ssh_credentials,
        )?;
        commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
            next_store.set_runtime_status(&session_id, SessionStatus::Connecting)?;
            let event_ids = next_store
                .record_system_event_tracked(
                    &session_id,
                    format!("PortMate: connecting to {endpoint} ({:?})", profile.kind),
                )
                .into_iter()
                .collect();
            Ok((profile, event_ids))
        })?
    };

    if cancellation.is_cancelled() {
        return Err(cancel_session_open_under_lifecycle_lock(&state, &session_id).await);
    }

    if matches!(
        profile.connection,
        ConnectionConfig::Ssh(_) | ConnectionConfig::Tmux(_)
    ) {
        let open = open_ssh_session(&state, profile, password, passphrase);
        let result = tokio::select! {
            result = open => result,
            () = cancellation.wait() => {
                return Err(cancel_session_open_under_lifecycle_lock(&state, &session_id).await);
            }
        };
        return match result {
            Ok(summary) => Ok(summary),
            Err(error) => {
                record_connection_failure(&state, &session_id, &error);
                Err(error)
            }
        };
    }

    if matches!(
        profile.connection,
        ConnectionConfig::Tcp(_) | ConnectionConfig::Telnet(_)
    ) {
        let open = open_tcp_session(&state, profile);
        let result = tokio::select! {
            result = open => result,
            () = cancellation.wait() => {
                return Err(cancel_session_open_under_lifecycle_lock(&state, &session_id).await);
            }
        };
        return match result {
            Ok(summary) => Ok(summary),
            Err(error) => {
                record_connection_failure(&state, &session_id, &error);
                Err(error)
            }
        };
    }

    if matches!(profile.connection, ConnectionConfig::Serial(_)) {
        let opening_state = state.clone();
        let result = match tauri::async_runtime::spawn_blocking(move || {
            open_serial_session(&opening_state, profile)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => Err(format!("串口打开任务失败: {error}")),
        };
        if cancellation.is_cancelled() {
            return Err(cancel_session_open_under_lifecycle_lock(&state, &session_id).await);
        }
        return match result {
            Ok(summary) => Ok(summary),
            Err(error) => {
                record_connection_failure(&state, &session_id, &error);
                Err(error)
            }
        };
    }

    if matches!(profile.connection, ConnectionConfig::Shell(_)) {
        let opening_state = state.clone();
        let result = match tauri::async_runtime::spawn_blocking(move || {
            open_shell_session(&opening_state, profile)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => Err(format!("Shell 启动任务失败: {error}")),
        };
        if cancellation.is_cancelled() {
            return Err(cancel_session_open_under_lifecycle_lock(&state, &session_id).await);
        }
        return match result {
            Ok(summary) => Ok(summary),
            Err(error) => {
                record_connection_failure(&state, &session_id, &error);
                Err(error)
            }
        };
    }

    {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
            mark_session_connected_with_events(next_store, &profile, [])
        })
    }
}

pub(super) fn ensure_session_can_open(state: &AppState, session_id: &str) -> Result<(), String> {
    let status = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if !store
            .profiles
            .iter()
            .any(|profile| profile.id == session_id)
        {
            return Err(format!("unknown session: {session_id}"));
        }
        store
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == session_id)
            .map(|runtime| runtime.status)
            .unwrap_or(SessionStatus::Disconnected)
    };
    if matches!(
        status,
        SessionStatus::Connecting | SessionStatus::Connected | SessionStatus::Reconnecting
    ) {
        return Err(format!(
            "session is already active ({status:?}); close it before opening again"
        ));
    }
    if session_has_registered_runtime(state, session_id)? {
        return Err(
            "session transport runtime is still registered; close it before opening again"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) async fn cancel_session_open_under_lifecycle_lock(
    state: &AppState,
    session_id: &str,
) -> String {
    let message = "session connection was cancelled";
    match close_session_under_lifecycle_lock(state, session_id.to_string()).await {
        Ok(_) => message.to_string(),
        Err(error) => format!("{message}; cancellation cleanup failed: {error}"),
    }
}

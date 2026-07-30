use super::*;

pub(super) const MAX_CONCURRENT_SESSION_OPENS: usize = 64;

pub(super) fn mark_session_connected_with_events(
    store: &mut SessionStore,
    profile: &SessionProfile,
    messages: impl IntoIterator<Item = String>,
) -> Result<(SessionSummary, Vec<String>), String> {
    let fallback = store.set_runtime_status(&profile.id, SessionStatus::Connected)?;
    let mut event_ids = Vec::new();
    for message in messages {
        if let Some(event_id) = store.record_system_event_tracked(&profile.id, message) {
            event_ids.push(event_id);
        }
    }
    if let Some(event_id) = store.record_system_event_tracked(
        &profile.id,
        format!(
            "PortMate: connected to {} ({:?})",
            describe_endpoint(profile),
            profile.kind
        ),
    ) {
        event_ids.push(event_id);
    }
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == profile.id)
        .unwrap_or(fallback);
    Ok((summary, event_ids))
}

pub(super) fn profile_requires_runtime(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<bool, String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    Ok(matches!(
        store.profile(session_id).map(|profile| profile.connection),
        Some(
            ConnectionConfig::Ssh(_)
                | ConnectionConfig::Tmux(_)
                | ConnectionConfig::Tcp(_)
                | ConnectionConfig::Telnet(_)
                | ConnectionConfig::Serial(_)
                | ConnectionConfig::Shell(_)
        )
    ))
}

pub(super) fn record_connection_failure(state: &AppState, session_id: &str, error: &str) {
    if let Ok(mut store) = state.store.lock() {
        let _ = store.set_runtime_status_with_reason(
            session_id,
            SessionStatus::Error,
            Some(error.to_string()),
        );
        store.record_system_event(session_id, format!("PortMate: connection failed: {error}"));
        if let Err(error) =
            persist_applied_store(&store, &state.store_path, "connection failure state")
        {
            eprintln!("PortMate: failed to persist connection failure: {error}");
        }
    }
}

pub(super) fn describe_endpoint(profile: &SessionProfile) -> String {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            if ssh.username.is_empty() {
                format!("{}:{}", ssh.endpoint.host, ssh.endpoint.port)
            } else {
                format!(
                    "{}@{}:{}",
                    ssh.username, ssh.endpoint.host, ssh.endpoint.port
                )
            }
        }
        ConnectionConfig::Serial(serial) => serial.port.clone(),
        ConnectionConfig::Shell(shell) => shell.program.clone(),
        ConnectionConfig::Telnet(tcp) | ConnectionConfig::Tcp(tcp) => {
            format!("{}:{}", tcp.host, tcp.port)
        }
    }
}
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
    let cancellation = register_session_open_cancellation(&state, &session_id)?;
    open_reserved_session_inner(state, session_id, credentials, cancellation).await
}

pub(super) async fn open_reserved_session_inner(
    state: AppState,
    session_id: String,
    credentials: SessionOpenCredentials,
    cancellation: Arc<SessionOpenCancellation>,
) -> Result<SessionSummary, String> {
    let lifecycle_lane = session_lifecycle_lane(&state, &session_id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
    if cancellation.is_cancelled() {
        return Err("session connection was cancelled before it started".to_string());
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
    } = credentials;
    let profile = {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        let saved_profile = store
            .profile(&session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        let endpoint = describe_endpoint(&saved_profile);
        let mut profile = normalize_session_profile(saved_profile);
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

pub(super) fn session_has_registered_runtime(
    state: &AppState,
    session_id: &str,
) -> Result<bool, String> {
    if state
        .ssh
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .shell
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .tcp
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .serial
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .active_commands
        .lock()
        .map_err(|error| error.to_string())?
        .contains_key(session_id)
    {
        return Ok(true);
    }
    if state
        .tmux_controls
        .lock()
        .map_err(|error| error.to_string())?
        .iter()
        .any(|((runtime_session_id, _), runtime)| {
            runtime_session_id == session_id && !runtime.cancel.load(Ordering::SeqCst)
        })
    {
        return Ok(true);
    }
    if state
        .tunnels
        .lock()
        .map_err(|error| error.to_string())?
        .values()
        .any(|runtime| runtime.session_id == session_id)
    {
        return Ok(true);
    }
    Ok(false)
}

pub(super) fn cleanup_deleted_session_runtime_state(
    state: &AppState,
    session_id: &str,
    transfer_ids: &[String],
) {
    clear_active_command(&state.session_io(), session_id);
    clear_log_retention_check(&state.store_path, session_id);
    state
        .serial_captures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(session_id);
    state
        .transfer_lanes
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(session_id);
    let transfer_ids = transfer_ids.iter().collect::<HashSet<_>>();
    state
        .transfer_cancellations
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|transfer_id, _| !transfer_ids.contains(transfer_id));
    state
        .one_time_host_keys
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(session_id);
    clear_outbound_lane(&state.store_path, session_id);

    let mut approvals = state
        .pending_mcp_approvals
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let approval_ids = approvals
        .iter()
        .filter_map(|(id, pending)| {
            (pending.request.session_id == session_id).then_some(id.clone())
        })
        .collect::<Vec<_>>();
    for approval_id in approval_ids {
        if let Some(pending) = approvals.remove(&approval_id) {
            let _ = pending.response.send(false);
        }
    }
}

pub(super) async fn close_session_inner(
    state: &AppState,
    session_id: String,
) -> Result<SessionSummary, String> {
    cancel_pending_session_opens(state, &session_id)?;
    let lifecycle_lane = session_lifecycle_lane(state, &session_id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
    close_session_under_lifecycle_lock(state, session_id).await
}

pub(super) async fn close_session_under_lifecycle_lock(
    state: &AppState,
    session_id: String,
) -> Result<SessionSummary, String> {
    clear_active_command(&state.session_io(), &session_id);
    let _ = cancel_tmux_control_runtimes_for_session(state, &session_id);
    let existing = {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing {
        disconnect_registered_ssh_runtime(
            runtime,
            "PortMate close_session",
            "PortMate close jump session",
        )
        .await;
    }
    let existing_shell = {
        let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_shell {
        runtime.closed.store(true, Ordering::SeqCst);
        if let Ok(mut child) = runtime.child.lock() {
            let _ = child.kill();
        }
    }
    let existing_tcp = {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_tcp {
        runtime.closed.store(true, Ordering::SeqCst);
        let mut writer = runtime.writer.lock().await;
        let _ = writer.shutdown().await;
    }
    let existing_serial = {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.remove(&session_id)
    };
    if let Some(runtime) = existing_serial {
        runtime.closed.store(true, Ordering::SeqCst);
    }
    {
        let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        let ids = tunnels
            .iter()
            .filter_map(|(id, runtime)| (runtime.session_id == session_id).then_some(id.clone()))
            .collect::<Vec<_>>();
        for id in ids {
            if let Some(runtime) = tunnels.remove(&id) {
                runtime.closed.store(true, Ordering::SeqCst);
            }
        }
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let summary = store.close_session(&session_id)?;
    persist_applied_store(&store, &state.store_path, "session disconnect state")
        .map_err(|error| format!("会话传输已在本地关闭，但断开状态无法持久化: {error}"))?;
    Ok(summary)
}

#[tauri::command]
pub(crate) async fn open_session(
    state: State<'_, AppState>,
    session_id: String,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<SessionSummary, String> {
    let state = state.inner().clone();
    open_session_inner(
        state,
        session_id,
        SessionOpenCredentials {
            password,
            passphrase,
            ..Default::default()
        },
    )
    .await
}

#[tauri::command]
pub(crate) async fn open_session_with_one_key(
    state: State<'_, AppState>,
    session_id: String,
    one_key_id: String,
) -> Result<SessionSummary, String> {
    let state = state.inner().clone();
    let cancellation = register_session_open_cancellation(&state, &session_id)?;
    let credentials = resolve_one_key_login_credentials(&state, &session_id, &one_key_id)?;
    open_reserved_session_inner(
        state,
        session_id,
        SessionOpenCredentials {
            username: Some(credentials.username),
            password: credentials.password,
            passphrase: credentials.passphrase,
            identity: credentials.identity,
            isolate_saved_ssh_credentials: true,
        },
        cancellation,
    )
    .await
}

#[tauri::command]
pub(crate) async fn close_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionSummary, String> {
    close_session_inner(state.inner(), session_id).await
}

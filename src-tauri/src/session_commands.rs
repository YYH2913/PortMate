use super::*;

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

pub(super) fn apply_proxy_password_update_with_io<WriteSecret>(
    profile: &mut SessionProfile,
    update: Option<ProxyPasswordUpdate>,
    mut write_secret: WriteSecret,
) -> Result<Option<String>, String>
where
    WriteSecret: FnMut(Option<SecretStorage>, &str) -> Result<String, String>,
{
    let Some(update) = update else {
        return Ok(None);
    };
    let proxy =
        profile_proxy_mut(profile).ok_or_else(|| "当前会话协议不支持代理密码".to_string())?;
    match update {
        ProxyPasswordUpdate::Set { password, storage } => {
            let password = Zeroizing::new(password);
            validate_proxy_credentials(proxy.kind, &proxy.username, password.as_str())?;
            let secret_ref = write_secret(storage, password.as_str())?;
            proxy.password_secret_ref = Some(secret_ref.clone());
            Ok(Some(secret_ref))
        }
        ProxyPasswordUpdate::Clear => {
            proxy.password_secret_ref = None;
            Ok(None)
        }
    }
}

pub(super) fn validate_profile_transport_change(
    current_profile: Option<&SessionProfile>,
    next_profile: &SessionProfile,
    runtime_status: Option<SessionStatus>,
) -> Result<(), String> {
    let Some(current_profile) = current_profile else {
        return Ok(());
    };
    let current_kind = current_profile.connection.kind();
    let next_kind = next_profile.connection.kind();
    if current_kind == next_kind {
        return Ok(());
    }
    if matches!(
        runtime_status,
        Some(SessionStatus::Connecting | SessionStatus::Connected | SessionStatus::Reconnecting)
    ) {
        let status = runtime_status.expect("active runtime status was matched above");
        return Err(format!(
            "会话仍在运行（{status:?}，当前协议 {current_kind:?}）；切换到 {next_kind:?} 前请先关闭会话"
        ));
    }
    Ok(())
}

pub(super) fn merge_expected_profile_update(
    current_profile: Option<&SessionProfile>,
    expected_profile: Option<&SessionProfile>,
    incoming_profile: SessionProfile,
) -> Result<SessionProfile, String> {
    match (current_profile, expected_profile) {
        (Some(current), Some(expected)) => {
            if current.id != incoming_profile.id || expected.id != incoming_profile.id {
                return Err("expectedProfile 与保存目标不是同一个 Profile".to_string());
            }
            let expected = serde_json::to_value(expected)
                .map_err(|error| format!("序列化 expectedProfile 失败: {error}"))?;
            let current = serde_json::to_value(current)
                .map_err(|error| format!("序列化当前 Profile 失败: {error}"))?;
            let incoming = serde_json::to_value(&incoming_profile)
                .map_err(|error| format!("序列化待保存 Profile 失败: {error}"))?;
            let merged =
                merge_expected_json_value("Profile", "profile", &expected, &current, &incoming)?;
            serde_json::from_value(merged)
                .map_err(|error| format!("反序列化合并后的 Profile 失败: {error}"))
        }
        (Some(_), None) => Err("保存现有 Profile 必须提供 expectedProfile 版本".to_string()),
        (None, Some(_)) => Err("Profile 已被其他操作删除，请刷新会话列表".to_string()),
        (None, None) => Ok(incoming_profile),
    }
}

pub(super) fn validate_expected_proxy_password(
    current_profile: Option<&SessionProfile>,
    expected_profile: Option<&SessionProfile>,
) -> Result<(), String> {
    let (Some(current), Some(expected)) = (current_profile, expected_profile) else {
        return Ok(());
    };
    let current_ref = profile_proxy(current).and_then(|proxy| proxy.password_secret_ref.as_deref());
    let expected_ref =
        profile_proxy(expected).and_then(|proxy| proxy.password_secret_ref.as_deref());
    if current_ref != expected_ref {
        return Err("代理密码已在其他操作中更新，请重新打开设置后再保存".to_string());
    }
    Ok(())
}

pub(super) fn merge_expected_json_value(
    entity: &str,
    path: &str,
    expected: &serde_json::Value,
    current: &serde_json::Value,
    incoming: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    if incoming == expected {
        return Ok(current.clone());
    }
    if current == expected || incoming == current {
        return Ok(incoming.clone());
    }

    let (
        serde_json::Value::Object(expected),
        serde_json::Value::Object(current),
        serde_json::Value::Object(incoming),
    ) = (expected, current, incoming)
    else {
        return Err(format!(
            "{entity} 字段已被其他操作修改，请刷新后重试: {path}"
        ));
    };
    if expected.len() != current.len()
        || expected.len() != incoming.len()
        || !expected
            .keys()
            .all(|key| current.contains_key(key) && incoming.contains_key(key))
    {
        return Err(format!(
            "{entity} 结构已被其他操作修改，请刷新后重试: {path}"
        ));
    }

    let mut merged = serde_json::Map::with_capacity(expected.len());
    for (key, expected_value) in expected {
        let current_value = current
            .get(key)
            .expect("merged JSON key sets were checked above");
        let incoming_value = incoming
            .get(key)
            .expect("merged JSON key sets were checked above");
        let child_path = format!("{path}.{key}");
        merged.insert(
            key.clone(),
            merge_expected_json_value(
                entity,
                &child_path,
                expected_value,
                current_value,
                incoming_value,
            )?,
        );
    }
    Ok(serde_json::Value::Object(merged))
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

fn terminal_key_sequence(key: &str) -> Result<String, String> {
    let normalized = key.trim().to_ascii_lowercase().replace('_', "-");
    let sequence = match normalized.as_str() {
        "" => return Err("key must not be empty".to_string()),
        "enter" | "return" => "\r".to_string(),
        "linefeed" | "lf" => "\n".to_string(),
        "tab" => "\t".to_string(),
        "backspace" | "bs" => "\u{0008}".to_string(),
        "delete" | "del" => "\x1b[3~".to_string(),
        "escape" | "esc" => "\x1b".to_string(),
        "up" | "arrow-up" => "\x1b[A".to_string(),
        "down" | "arrow-down" => "\x1b[B".to_string(),
        "right" | "arrow-right" => "\x1b[C".to_string(),
        "left" | "arrow-left" => "\x1b[D".to_string(),
        "home" => "\x1b[H".to_string(),
        "end" => "\x1b[F".to_string(),
        "pageup" | "page-up" => "\x1b[5~".to_string(),
        "pagedown" | "page-down" => "\x1b[6~".to_string(),
        "insert" | "ins" => "\x1b[2~".to_string(),
        "f1" => "\x1bOP".to_string(),
        "f2" => "\x1bOQ".to_string(),
        "f3" => "\x1bOR".to_string(),
        "f4" => "\x1bOS".to_string(),
        "f5" => "\x1b[15~".to_string(),
        "f6" => "\x1b[17~".to_string(),
        "f7" => "\x1b[18~".to_string(),
        "f8" => "\x1b[19~".to_string(),
        "f9" => "\x1b[20~".to_string(),
        "f10" => "\x1b[21~".to_string(),
        "f11" => "\x1b[23~".to_string(),
        "f12" => "\x1b[24~".to_string(),
        "space" => " ".to_string(),
        value if value.starts_with("ctrl+") || value.starts_with("ctrl-") => {
            let key = value
                .trim_start_matches("ctrl+")
                .trim_start_matches("ctrl-");
            let byte = match key {
                "space" | "@" => 0,
                "[" | "escape" | "esc" => 27,
                "\\" => 28,
                "]" => 29,
                "^" => 30,
                "_" => 31,
                value if value.len() == 1 => {
                    let ch = value.as_bytes()[0];
                    if ch.is_ascii_alphabetic() {
                        ch.to_ascii_uppercase() - b'@'
                    } else {
                        return Err(format!("unsupported control key: {key}"));
                    }
                }
                _ => return Err(format!("unsupported control key: {key}")),
            };
            String::from_utf8(vec![byte]).map_err(|error| error.to_string())?
        }
        value if value.chars().count() == 1 => value.to_string(),
        _ => return Err(format!("unsupported key sequence: {key}")),
    };
    Ok(sequence)
}

pub(super) fn terminal_key_sequence_for_protocol(
    key: &str,
    is_telnet: bool,
) -> Result<String, String> {
    let sequence = terminal_key_sequence(key)?;
    if is_telnet && sequence == "\r" {
        Ok("\r\n".to_string())
    } else {
        Ok(sequence)
    }
}

pub(super) fn terminate_command_for_protocol(mut command: String, is_telnet: bool) -> String {
    let needs_terminator = !command.ends_with('\n') && !command.ends_with('\r');
    let telnet_bare_cr = is_telnet && command.ends_with('\r') && !command.ends_with("\r\n");
    if needs_terminator || telnet_bare_cr {
        command.push('\n');
    }
    command
}

pub(super) async fn resize_session_inner(
    state: &AppState,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<SessionSummary, String> {
    if cols == 0 || rows == 0 {
        return Err("terminal size must be non-zero".to_string());
    }

    let ssh_writer = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(&session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = ssh_writer {
        let writer = writer.lock().await;
        writer
            .window_change(u32::from(cols), u32::from(rows), 0, 0)
            .await
            .map_err(|error| format!("SSH resize failed: {error}"))?;
    }

    let shell_master = {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections
            .get(&session_id)
            .map(|runtime| Arc::clone(&runtime.master))
    };
    if let Some(master) = shell_master {
        let master = master.lock().map_err(|error| error.to_string())?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Shell PTY resize failed: {error}"))?;
    }

    let telnet_target = {
        let connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.get(&session_id).and_then(|runtime| {
            runtime
                .telnet
                .as_ref()
                .map(|telnet| (Arc::clone(&runtime.writer), Arc::clone(telnet)))
        })
    };
    if let Some((writer, telnet)) = telnet_target {
        let io = state.session_io();
        let lane = outbound_lane(&io.store_path, &session_id)?;
        let _lane_guard = lane.lock().await;
        telnet.cols.store(cols, Ordering::SeqCst);
        telnet.rows.store(rows, Ordering::SeqCst);
        if telnet.naws_negotiated.load(Ordering::SeqCst) {
            let message = telnet_naws_message(cols, rows);
            writer
                .lock()
                .await
                .write_all(&message)
                .await
                .map_err(|error| format!("Telnet NAWS resize failed: {error}"))?;
            record_outbound_control_event(&io, &session_id, &message, "telnet-naws", None, true);
        }
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        resize_session_profile_in_store(next_store, &session_id, cols, rows)
    })
}

pub(super) fn resize_session_profile_in_store(
    store: &mut SessionStore,
    session_id: &str,
    cols: u16,
    rows: u16,
) -> Result<SessionSummary, String> {
    let profile = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    profile.terminal.cols = cols;
    profile.terminal.rows = rows;
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == session_id)
        .ok_or_else(|| format!("session summary is missing: {session_id}"))?;
    Ok(summary)
}

#[tauri::command]
pub(crate) fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.summaries())
}

#[tauri::command]
pub(crate) fn read_screen(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    store
        .screen(&session_id)
        .ok_or_else(|| format!("no screen data for session: {session_id}"))
}

#[tauri::command]
pub(crate) async fn send_text(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<SessionEvent, String> {
    send_text_inner(state.inner().session_io(), session_id, text).await
}

#[tauri::command]
pub(crate) async fn send_bytes(
    state: State<'_, AppState>,
    session_id: String,
    bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    send_bytes_inner(state.inner().session_io(), session_id, bytes).await
}

#[tauri::command]
pub(crate) async fn send_key(
    state: State<'_, AppState>,
    session_id: String,
    key: String,
) -> Result<SessionEvent, String> {
    let io = state.inner().session_io();
    let text =
        terminal_key_sequence_for_protocol(&key, is_telnet_session(&io.store, &session_id)?)?;
    send_text_inner_with_context(io, session_id, text, "desktop-user", Some("send_key")).await
}

#[tauri::command]
pub(crate) async fn run_command(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
) -> Result<SessionEvent, String> {
    let io = state.inner().session_io();
    let text = terminate_command_for_protocol(command, is_telnet_session(&io.store, &session_id)?);
    run_command_inner_with_context(io, session_id, text, "desktop-user", Some("run_command")).await
}

#[tauri::command]
pub(crate) async fn resize_session(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<SessionSummary, String> {
    resize_session_inner(state.inner(), session_id, cols, rows).await
}

#[tauri::command]
pub(crate) async fn delete_session_profile(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<DeleteSessionProfileResponse, String> {
    delete_session_profile_inner(state.inner(), session_id).await
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

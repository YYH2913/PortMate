use super::*;

pub(super) const MAX_ACTIVE_TUNNELS: usize = 256;
pub(super) const MAX_TUNNEL_CONNECTIONS: usize = 256;
pub(super) const TUNNEL_CONNECTION_LIMIT_ERROR_PREFIX: &str = "tunnel connection limit reached:";

#[derive(Clone)]
pub(super) struct TunnelRuntime {
    pub(super) session_id: String,
    pub(super) ssh_runtime_id: String,
    pub(super) spec: TunnelSpec,
    pub(super) metrics: Arc<TunnelMetrics>,
    pub(super) closed: Arc<AtomicBool>,
}

#[derive(Clone, Debug)]
pub(super) struct TunnelForwardTarget {
    pub(super) spec: TunnelSpec,
    pub(super) metrics: Arc<TunnelMetrics>,
    pub(super) connection_slots: Arc<tokio::sync::Semaphore>,
}

#[derive(Debug, Default)]
pub(super) struct TunnelMetrics {
    pub(super) active_connections: AtomicU64,
    pub(super) total_connections: AtomicU64,
    pub(super) tcp_to_ssh_bytes: AtomicU64,
    pub(super) ssh_to_tcp_bytes: AtomicU64,
    pub(super) last_activity: Mutex<Option<String>>,
    pub(super) last_error: Mutex<Option<String>>,
}

impl TunnelMetrics {
    pub(super) fn connection_opened(&self) {
        self.total_connections.fetch_add(1, Ordering::SeqCst);
        self.active_connections.fetch_add(1, Ordering::SeqCst);
        self.touch();
    }

    pub(super) fn connection_closed(&self) {
        let _ = self
            .active_connections
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(value.saturating_sub(1))
            });
        self.touch();
    }

    pub(super) fn add_tcp_to_ssh_bytes(&self, bytes: usize) {
        self.tcp_to_ssh_bytes
            .fetch_add(bytes as u64, Ordering::SeqCst);
        self.touch();
    }

    pub(super) fn add_ssh_to_tcp_bytes(&self, bytes: usize) {
        self.ssh_to_tcp_bytes
            .fetch_add(bytes as u64, Ordering::SeqCst);
        self.touch();
    }

    pub(super) fn record_error(&self, error: &str) {
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = Some(error.to_string());
        }
        self.touch();
    }

    pub(super) fn record_error_if_changed(&self, error: &str) -> bool {
        let changed = if let Ok(mut last_error) = self.last_error.lock() {
            if last_error.as_deref() == Some(error) {
                false
            } else {
                *last_error = Some(error.to_string());
                true
            }
        } else {
            false
        };
        if changed {
            self.touch();
        }
        changed
    }

    pub(super) fn clear_error(&self) {
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = None;
        }
        self.touch();
    }

    pub(super) fn clear_error_with_prefix(&self, prefix: &str) -> bool {
        let cleared = if let Ok(mut last_error) = self.last_error.lock() {
            if last_error
                .as_deref()
                .is_some_and(|error| error.starts_with(prefix))
            {
                *last_error = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        if cleared {
            self.touch();
        }
        cleared
    }

    pub(super) fn touch(&self) {
        if let Ok(mut last_activity) = self.last_activity.lock() {
            *last_activity = Some(Utc::now().to_rfc3339());
        }
    }

    pub(super) fn snapshot(&self, spec: TunnelSpec) -> TunnelStatus {
        TunnelStatus {
            spec,
            active_connections: self.active_connections.load(Ordering::SeqCst),
            total_connections: self.total_connections.load(Ordering::SeqCst),
            tcp_to_ssh_bytes: self.tcp_to_ssh_bytes.load(Ordering::SeqCst),
            ssh_to_tcp_bytes: self.ssh_to_tcp_bytes.load(Ordering::SeqCst),
            last_activity: self
                .last_activity
                .lock()
                .ok()
                .and_then(|value| value.clone()),
            last_error: self.last_error.lock().ok().and_then(|value| value.clone()),
        }
    }
}

pub(super) fn try_acquire_tunnel_connection(
    connection_slots: &Arc<tokio::sync::Semaphore>,
    metrics: &TunnelMetrics,
) -> Option<tokio::sync::OwnedSemaphorePermit> {
    match Arc::clone(connection_slots).try_acquire_owned() {
        Ok(permit) => {
            metrics.clear_error_with_prefix(TUNNEL_CONNECTION_LIMIT_ERROR_PREFIX);
            Some(permit)
        }
        Err(_) => {
            metrics.record_error_if_changed(&format!(
                "{TUNNEL_CONNECTION_LIMIT_ERROR_PREFIX} app limit ({MAX_TUNNEL_CONNECTIONS})"
            ));
            None
        }
    }
}

pub(super) fn forwarded_tcpip_ports(
    connected_port: u32,
    originator_port: u32,
) -> Option<(u16, u16)> {
    Some((
        u16::try_from(connected_port).ok()?,
        u16::try_from(originator_port).ok()?,
    ))
}

pub(super) async fn create_tunnel_inner(
    state: &AppState,
    request: CreateTunnelRequest,
) -> Result<TunnelSpec, String> {
    let request = normalize_tunnel_request(request)?;
    ensure_tunnel_creation_capacity(state, &request.session_id)?;
    let tunnel = TunnelSpec {
        id: Uuid::new_v4().to_string(),
        label: request.label.clone().unwrap_or_else(|| {
            tunnel_label(
                request.mode,
                &request.bind_host,
                request.bind_port,
                &request.target_host,
                request.target_port,
            )
        }),
        mode: request.mode,
        bind_host: request.bind_host.clone(),
        bind_port: request.bind_port,
        target_host: request.target_host.clone(),
        target_port: request.target_port,
        enabled: true,
    };
    validate_tunnels(std::slice::from_ref(&tunnel))?;
    let (tunnel, local_addr, ssh_runtime_id) = start_tunnel_runtime(
        state,
        &request.session_id,
        tunnel,
        request.label.is_none(),
        None,
    )
    .await?;
    commit_started_tunnel(
        state,
        &request.session_id,
        tunnel,
        local_addr,
        &ssh_runtime_id,
    )
    .await
}

pub(super) fn ensure_tunnel_creation_capacity(
    state: &AppState,
    session_id: &str,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profile(session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    let tunnels = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.tunnels,
        _ => return Err("tunnels require an SSH or Tmux session".to_string()),
    };
    if tunnels.iter().filter(|tunnel| tunnel.enabled).count() >= MAX_TUNNELS_PER_PROFILE {
        return Err(format!(
            "enabled tunnel count has reached {MAX_TUNNELS_PER_PROFILE}"
        ));
    }
    Ok(())
}

pub(super) async fn start_tunnel_runtime(
    state: &AppState,
    session_id: &str,
    mut tunnel: TunnelSpec,
    relabel_assigned_port: bool,
    expected_runtime_id: Option<&str>,
) -> Result<(TunnelSpec, Option<std::net::SocketAddr>, String), String> {
    validate_tunnels(std::slice::from_ref(&tunnel))?;
    let (
        handle,
        remote_forwards,
        remote_forward_acceptor_started,
        ssh_runtime_closed,
        ssh_runtime_id,
    ) = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        let runtime = connections
            .get(session_id)
            .filter(|runtime| {
                expected_runtime_id.is_none_or(|expected| runtime.runtime_id == expected)
            })
            .ok_or_else(|| "需要先连接 SSH/Tmux 会话才能创建 tunnel".to_string())?;
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if !store.runtimes.iter().any(|summary| {
            summary.session_id == session_id && summary.status == SessionStatus::Connected
        }) {
            return Err("需要先连接 SSH/Tmux 会话才能创建 tunnel".to_string());
        }
        (
            Arc::clone(&runtime.handle),
            Arc::clone(&runtime.remote_forwards),
            Arc::clone(&runtime.remote_forward_acceptor_started),
            Arc::clone(&runtime.closed),
            runtime.runtime_id.clone(),
        )
    };

    if tunnel.id.trim().is_empty() {
        return Err("tunnel requires an id".to_string());
    }
    {
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        ensure_tunnel_runtime_slot(&tunnels, &tunnel.id)?;
    }

    if tunnel.mode == TunnelMode::Remote {
        if tunnel.target_host.trim().is_empty() || tunnel.target_port == 0 {
            return Err("remote tunnel requires a local target host and port".to_string());
        }
        let (returned_port, libssh_session) = {
            let handle = handle.lock().await;
            let returned_port = handle
                .listen_remote_forward(tunnel.bind_host.clone(), tunnel.bind_port)
                .await
                .map_err(|error| {
                    format!(
                        "remote SSH tunnel request failed {}:{}: {error}",
                        tunnel.bind_host, tunnel.bind_port
                    )
                })?;
            (returned_port, handle.libssh_forward_session())
        };
        if tunnel.bind_port == 0 {
            if returned_port == 0 {
                return Err("remote SSH tunnel request did not return an assigned port".to_string());
            }
            tunnel.bind_port = returned_port;
            if relabel_assigned_port {
                tunnel.label = tunnel_label(
                    tunnel.mode,
                    &tunnel.bind_host,
                    tunnel.bind_port,
                    &tunnel.target_host,
                    tunnel.target_port,
                );
            }
        }
        let metrics = Arc::new(TunnelMetrics::default());
        let closed = Arc::new(AtomicBool::new(false));
        let install_result = (|| {
            let connections = state.ssh.lock().map_err(|error| error.to_string())?;
            if connections
                .get(session_id)
                .is_none_or(|runtime| runtime.runtime_id != ssh_runtime_id)
            {
                return Err("SSH runtime changed while creating tunnel".to_string());
            }
            let store = state.store.lock().map_err(|error| error.to_string())?;
            if !store.runtimes.iter().any(|runtime| {
                runtime.session_id == session_id && runtime.status == SessionStatus::Connected
            }) {
                return Err("SSH session disconnected while creating tunnel".to_string());
            }
            let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
            ensure_tunnel_runtime_slot(&tunnels, &tunnel.id)?;
            let mut forwards = remote_forwards.lock().map_err(|error| error.to_string())?;
            let target = TunnelForwardTarget {
                spec: tunnel.clone(),
                metrics: Arc::clone(&metrics),
                connection_slots: Arc::clone(&state.tunnel_connection_slots),
            };
            forwards.insert(
                remote_forward_key(&tunnel.bind_host, tunnel.bind_port),
                target.clone(),
            );
            forwards.insert(remote_forward_port_key(tunnel.bind_port), target);
            tunnels.insert(
                tunnel.id.clone(),
                TunnelRuntime {
                    session_id: session_id.to_string(),
                    ssh_runtime_id: ssh_runtime_id.clone(),
                    spec: tunnel.clone(),
                    metrics: Arc::clone(&metrics),
                    closed: Arc::clone(&closed),
                },
            );
            Ok::<(), String>(())
        })();
        if let Err(error) = install_result {
            let warnings = cancel_remote_tunnel_forward(
                Arc::clone(&handle),
                Arc::clone(&remote_forwards),
                &tunnel,
            )
            .await;
            return Err(if warnings.is_empty() {
                error
            } else {
                format!(
                    "{error}; remote tunnel cleanup failed: {}",
                    warnings.join("; ")
                )
            });
        }
        if let Some(session) = libssh_session {
            if remote_forward_acceptor_started
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                spawn_libssh_remote_forward_acceptor(
                    session,
                    Arc::clone(&remote_forwards),
                    ssh_runtime_closed,
                );
            }
        }
        spawn_remote_tunnel_health_monitor(state.clone(), tunnel.id.clone(), Arc::clone(&closed));
        return Ok((tunnel, None, ssh_runtime_id));
    }

    let listener = TcpListener::bind((tunnel.bind_host.clone(), tunnel.bind_port))
        .await
        .map_err(|error| {
            format!(
                "SSH tunnel bind failed {}:{}: {error}",
                tunnel.bind_host, tunnel.bind_port
            )
        })?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("SSH tunnel local addr failed: {error}"))?;
    if tunnel.bind_port == 0 {
        tunnel.bind_port = local_addr.port();
        if relabel_assigned_port {
            tunnel.label = tunnel_label(
                tunnel.mode,
                &tunnel.bind_host,
                tunnel.bind_port,
                &tunnel.target_host,
                tunnel.target_port,
            );
        }
    }
    let closed = Arc::new(AtomicBool::new(false));
    let metrics = Arc::new(TunnelMetrics::default());
    let connection_slots = Arc::clone(&state.tunnel_connection_slots);
    {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        if connections
            .get(session_id)
            .is_none_or(|runtime| runtime.runtime_id != ssh_runtime_id)
        {
            return Err("SSH runtime changed while creating tunnel".to_string());
        }
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if !store.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id && runtime.status == SessionStatus::Connected
        }) {
            return Err("SSH session disconnected while creating tunnel".to_string());
        }
        let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        ensure_tunnel_runtime_slot(&tunnels, &tunnel.id)?;
        tunnels.insert(
            tunnel.id.clone(),
            TunnelRuntime {
                session_id: session_id.to_string(),
                ssh_runtime_id: ssh_runtime_id.clone(),
                spec: tunnel.clone(),
                metrics: Arc::clone(&metrics),
                closed: Arc::clone(&closed),
            },
        );
    }

    let session_id = session_id.to_string();
    let store = Arc::clone(&state.store);
    let store_path = state.store_path.clone();
    let tunnel_registry = Arc::clone(&state.tunnels);
    let tunnel_for_task = tunnel.clone();
    let ssh_runtime_id_for_task = ssh_runtime_id.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if closed.load(Ordering::SeqCst) {
                break;
            }
            match tokio::time::timeout(Duration::from_millis(500), listener.accept()).await {
                Ok(Ok((stream, peer))) => {
                    let Some(permit) =
                        try_acquire_tunnel_connection(&connection_slots, metrics.as_ref())
                    else {
                        continue;
                    };
                    let handle = handle.clone();
                    let spec = tunnel_for_task.clone();
                    let metrics = Arc::clone(&metrics);
                    let store = Arc::clone(&store);
                    let store_path = store_path.clone();
                    let session_id = session_id.clone();
                    tauri::async_runtime::spawn(async move {
                        let _permit = permit;
                        metrics.connection_opened();
                        let result = if spec.mode == TunnelMode::Dynamic {
                            handle_dynamic_tunnel_client(handle, stream, peer, Arc::clone(&metrics))
                                .await
                        } else {
                            handle_local_tunnel_client(
                                handle,
                                spec,
                                stream,
                                peer,
                                Arc::clone(&metrics),
                            )
                            .await
                        };
                        match result {
                            Ok(()) => {
                                metrics.clear_error();
                                metrics.connection_closed();
                            }
                            Err(error) => {
                                metrics.record_error(&error);
                                metrics.connection_closed();
                                if let Ok(mut store) = store.lock() {
                                    store.record_system_event(
                                        &session_id,
                                        format!("PortMate: SSH tunnel client failed: {error}"),
                                    );
                                    if let Err(error) = persist_applied_store(
                                        &store,
                                        &store_path,
                                        "tunnel client failure event",
                                    ) {
                                        eprintln!(
                                            "PortMate: failed to persist tunnel client error: {error}"
                                        );
                                    }
                                }
                            }
                        }
                    });
                }
                Ok(Err(error)) => {
                    let message = format!("SSH tunnel accept failed: {error}");
                    let removed = match fail_tunnel_runtime_if_owned(
                        &tunnel_registry,
                        &tunnel_for_task.id,
                        &ssh_runtime_id_for_task,
                        &message,
                    ) {
                        Ok(Some(_)) => true,
                        Ok(None) => false,
                        Err(registry_error) => {
                            closed.store(true, Ordering::SeqCst);
                            metrics.record_error(&format!("{message}; {registry_error}"));
                            false
                        }
                    };
                    if removed {
                        if let Ok(mut store) = store.lock() {
                            let mut stopped = tunnel_for_task.clone();
                            stopped.enabled = false;
                            mark_tunnel_stopped_in_store(&mut store, &session_id, &stopped);
                            store.record_system_event(&session_id, format!("PortMate: {message}"));
                            if let Err(error) = persist_applied_store(
                                &store,
                                &store_path,
                                "failed tunnel listener state",
                            ) {
                                eprintln!(
                                    "PortMate: failed to persist tunnel listener failure: {error}"
                                );
                            }
                        }
                    }
                    break;
                }
                Err(_) => {}
            }
        }
    });

    Ok((tunnel, Some(local_addr), ssh_runtime_id))
}

pub(super) fn ensure_tunnel_runtime_slot(
    tunnels: &HashMap<String, TunnelRuntime>,
    tunnel_id: &str,
) -> Result<(), String> {
    if tunnels.contains_key(tunnel_id) {
        return Err(format!("tunnel already running: {tunnel_id}"));
    }
    if tunnels.len() >= MAX_ACTIVE_TUNNELS {
        return Err(format!(
            "active tunnel count has reached {MAX_ACTIVE_TUNNELS}"
        ));
    }
    Ok(())
}

pub(super) async fn commit_started_tunnel(
    state: &AppState,
    session_id: &str,
    tunnel: TunnelSpec,
    local_addr: Option<std::net::SocketAddr>,
    ssh_runtime_id: &str,
) -> Result<TunnelSpec, String> {
    if let Err(commit_error) =
        persist_tunnel_to_profile_and_log(state, session_id, &tunnel, local_addr)
    {
        let cleanup = stop_tunnel_runtime_effects(state, &tunnel.id, ssh_runtime_id).await;
        return Err(match cleanup {
            Ok((_, warnings)) if warnings.is_empty() => format!(
                "tunnel Store commit failed and the uncommitted runtime was closed: {commit_error}"
            ),
            Ok((_, warnings)) => format!(
                "tunnel Store commit failed: {commit_error}; runtime cleanup warnings: {}",
                warnings.join("; ")
            ),
            Err(cleanup_error) => format!(
                "tunnel Store commit failed: {commit_error}; runtime cleanup failed: {cleanup_error}"
            ),
        });
    }
    Ok(tunnel)
}

pub(super) fn enabled_tunnel_specs(
    state: &AppState,
    session_id: &str,
) -> Result<Vec<TunnelSpec>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profile(session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    let tunnels = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.tunnels,
        _ => Vec::new(),
    };
    Ok(normalize_tunnels(tunnels)
        .into_iter()
        .filter(|tunnel| tunnel.enabled)
        .collect())
}

pub(super) async fn restore_enabled_tunnels(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
) -> (usize, usize) {
    if !ssh_runtime_connected(state, session_id, runtime_id) {
        return (0, 0);
    }
    let tunnels = match enabled_tunnel_specs(state, session_id) {
        Ok(tunnels) => tunnels,
        Err(error) => {
            record_tunnel_restore_failure(state, session_id, None, &error);
            return (0, 1);
        }
    };
    let mut restored = 0;
    let mut failed = 0;
    for tunnel in tunnels {
        if !ssh_runtime_connected(state, session_id, runtime_id) {
            break;
        }
        let tunnel_id = tunnel.id.clone();
        match start_tunnel_runtime(state, session_id, tunnel, false, Some(runtime_id)).await {
            Ok((tunnel, local_addr, _)) => {
                if !ssh_runtime_connected(state, session_id, runtime_id) {
                    let _ = fail_tunnel_runtime_if_owned(
                        &state.tunnels,
                        &tunnel_id,
                        runtime_id,
                        "SSH reconnect superseded while restoring tunnel",
                    );
                    break;
                }
                restored += 1;
                if let Err(error) =
                    persist_tunnel_to_profile_and_log(state, session_id, &tunnel, local_addr)
                {
                    record_tunnel_restore_failure(
                        state,
                        session_id,
                        Some(&tunnel_id),
                        &format!("runtime restored but persistence failed: {error}"),
                    );
                }
            }
            Err(error) => {
                if !ssh_runtime_connected(state, session_id, runtime_id) {
                    break;
                }
                failed += 1;
                record_tunnel_restore_failure(state, session_id, Some(&tunnel_id), &error);
            }
        }
    }
    (restored, failed)
}

pub(super) fn record_tunnel_restore_failure(
    state: &AppState,
    session_id: &str,
    tunnel_id: Option<&str>,
    error: &str,
) {
    let label = tunnel_id.map(|id| format!(" {id}")).unwrap_or_default();
    record_applied_system_event(
        state,
        session_id,
        format!("PortMate: failed to restore SSH tunnel{label}: {error}"),
        "tunnel restore failure event",
    );
}

pub(super) fn normalize_tunnel_request(
    mut request: CreateTunnelRequest,
) -> Result<CreateTunnelRequest, String> {
    for (label, value) in [
        ("session id", request.session_id.as_str()),
        ("bind host", request.bind_host.as_str()),
        ("target host", request.target_host.as_str()),
    ] {
        if value.chars().any(char::is_control) {
            return Err(format!(
                "tunnel {label} must not contain control characters"
            ));
        }
    }
    if request
        .label
        .as_deref()
        .is_some_and(|label| label.chars().any(char::is_control))
    {
        return Err("tunnel label must not contain control characters".to_string());
    }
    request.session_id = request.session_id.trim().to_string();
    request.bind_host = request.bind_host.trim().to_string();
    request.target_host = request.target_host.trim().to_string();
    request.label = request
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned);

    if request.mode == TunnelMode::Dynamic {
        request.target_host.clear();
        request.target_port = 0;
    }
    validate_tunnel_request_text(
        "session id",
        &request.session_id,
        MAX_SESSION_PROFILE_ID_CHARACTERS,
        false,
        false,
    )?;
    validate_tunnel_request_text(
        "bind host",
        &request.bind_host,
        MAX_TUNNEL_HOST_CHARACTERS,
        request.mode == TunnelMode::Remote,
        true,
    )?;
    if let Some(label) = request.label.as_deref() {
        validate_tunnel_request_text("label", label, MAX_TUNNEL_LABEL_CHARACTERS, false, false)?;
    }
    if request.mode != TunnelMode::Dynamic {
        if request.target_host.is_empty() || request.target_port == 0 {
            return Err("local and remote tunnels require a target host and port".to_string());
        }
        validate_tunnel_request_text(
            "target host",
            &request.target_host,
            MAX_TUNNEL_HOST_CHARACTERS,
            false,
            true,
        )?;
    }
    Ok(request)
}

pub(super) fn validate_tunnel_request_text(
    label: &str,
    value: &str,
    max_characters: usize,
    allow_empty: bool,
    reject_whitespace: bool,
) -> Result<(), String> {
    if !allow_empty && value.is_empty() {
        return Err(format!("tunnel {label} must not be empty"));
    }
    let mut count = 0_usize;
    for character in value.chars() {
        count = count.saturating_add(1);
        if count > max_characters {
            return Err(format!(
                "tunnel {label} exceeds {max_characters} Unicode characters"
            ));
        }
        if character.is_control() {
            return Err(format!(
                "tunnel {label} must not contain control characters"
            ));
        }
        if reject_whitespace && character.is_whitespace() {
            return Err(format!("tunnel {label} must not contain whitespace"));
        }
    }
    Ok(())
}

pub(super) fn list_tunnels_inner(
    state: &AppState,
    session_id: Option<&str>,
) -> Result<Vec<TunnelStatus>, String> {
    let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
    let mut statuses = tunnels
        .values()
        .filter(|runtime| {
            !runtime.closed.load(Ordering::SeqCst)
                && match session_id {
                    Some(expected) => runtime.session_id == expected,
                    None => true,
                }
        })
        .map(tunnel_status_from_runtime)
        .collect::<Vec<_>>();
    statuses.sort_by(|left, right| {
        left.spec
            .label
            .cmp(&right.spec.label)
            .then_with(|| left.spec.id.cmp(&right.spec.id))
    });
    Ok(statuses)
}

pub(super) fn fail_tunnel_runtime_if_owned(
    tunnels: &Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    tunnel_id: &str,
    ssh_runtime_id: &str,
    error: &str,
) -> Result<Option<TunnelRuntime>, String> {
    let runtime = remove_tunnel_runtime_if_owned(tunnels, tunnel_id, ssh_runtime_id)?;
    if let Some(runtime) = &runtime {
        runtime.metrics.record_error(error);
        runtime.closed.store(true, Ordering::SeqCst);
    }
    Ok(runtime)
}

pub(super) fn remove_tunnel_runtime_if_owned(
    tunnels: &Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    tunnel_id: &str,
    ssh_runtime_id: &str,
) -> Result<Option<TunnelRuntime>, String> {
    let mut tunnels = tunnels
        .lock()
        .map_err(|lock_error| lock_error.to_string())?;
    if tunnels
        .get(tunnel_id)
        .is_none_or(|runtime| runtime.ssh_runtime_id != ssh_runtime_id)
    {
        return Ok(None);
    }
    Ok(tunnels.remove(tunnel_id))
}

pub(super) fn fail_session_tunnel_runtimes(
    tunnels: &Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    session_id: &str,
    error: &str,
) -> Result<usize, String> {
    let mut tunnels = tunnels
        .lock()
        .map_err(|lock_error| lock_error.to_string())?;
    let ids = tunnels
        .iter()
        .filter_map(|(id, runtime)| (runtime.session_id == session_id).then_some(id.clone()))
        .collect::<Vec<_>>();
    let mut removed = 0;
    for id in ids {
        if let Some(runtime) = tunnels.remove(&id) {
            runtime.metrics.record_error(error);
            runtime.closed.store(true, Ordering::SeqCst);
            removed += 1;
        }
    }
    Ok(removed)
}

pub(super) async fn stop_tunnel_inner(
    state: &AppState,
    tunnel_id: &str,
) -> Result<TunnelStatus, String> {
    let expected_runtime_id = {
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels
            .get(tunnel_id)
            .map(|runtime| runtime.ssh_runtime_id.clone())
            .ok_or_else(|| format!("tunnel not found: {tunnel_id}"))?
    };
    let (runtime, warnings) =
        stop_tunnel_runtime_effects(state, tunnel_id, &expected_runtime_id).await?;

    let mut stopped = runtime.spec.clone();
    stopped.enabled = false;
    let persistence_result =
        persist_stopped_tunnel_to_profile_and_log(state, &runtime.session_id, &stopped);
    if !warnings.is_empty() {
        let warning = warnings.join("; ");
        runtime.metrics.record_error(&warning);
        record_tunnel_health_event(
            state,
            &runtime.session_id,
            tunnel_id,
            &format!("stopped locally with warning: {warning}"),
        );
    }
    if let Err(error) = persistence_result {
        return Err(format!(
            "tunnel stopped locally, but the disabled state could not be persisted: {error}"
        ));
    }
    Ok(runtime.metrics.snapshot(stopped))
}

pub(super) async fn stop_tunnel_runtime_effects(
    state: &AppState,
    tunnel_id: &str,
    ssh_runtime_id: &str,
) -> Result<(TunnelRuntime, Vec<String>), String> {
    let runtime = remove_tunnel_runtime_if_owned(&state.tunnels, tunnel_id, ssh_runtime_id)?
        .ok_or_else(|| format!("tunnel runtime was superseded: {tunnel_id}"))?;
    runtime.closed.store(true, Ordering::SeqCst);
    let mut warnings = Vec::new();
    if runtime.spec.mode != TunnelMode::Remote {
        return Ok((runtime, warnings));
    }

    let remote_forward = match state.ssh.lock() {
        Ok(connections) => connections
            .get(&runtime.session_id)
            .filter(|ssh| ssh.runtime_id == runtime.ssh_runtime_id)
            .map(|ssh| (Arc::clone(&ssh.handle), Arc::clone(&ssh.remote_forwards))),
        Err(error) => {
            warnings.push(format!(
                "SSH runtime lock failed during remote cancel: {error}"
            ));
            None
        }
    };
    if let Some((handle, remote_forwards)) = remote_forward {
        warnings.extend(cancel_remote_tunnel_forward(handle, remote_forwards, &runtime.spec).await);
    } else if warnings.is_empty() {
        warnings.push("SSH runtime unavailable during remote cancel".to_string());
    }
    Ok((runtime, warnings))
}

pub(super) async fn cancel_remote_tunnel_forward(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    tunnel: &TunnelSpec,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let cancel = tokio::time::timeout(REMOTE_TUNNEL_HEALTH_TIMEOUT, async {
        let handle = handle.lock().await;
        handle
            .cancel_remote_forward(tunnel.bind_host.clone(), tunnel.bind_port)
            .await
    })
    .await;
    match cancel {
        Ok(Ok(())) => {}
        Ok(Err(error)) => warnings.push(format!(
            "remote SSH tunnel cancel failed {}:{}: {error}",
            tunnel.bind_host, tunnel.bind_port
        )),
        Err(_) => warnings.push(format!(
            "remote SSH tunnel cancel timed out {}:{}",
            tunnel.bind_host, tunnel.bind_port
        )),
    }
    match remote_forwards.lock() {
        Ok(mut forwards) => {
            forwards.remove(&remote_forward_key(&tunnel.bind_host, tunnel.bind_port));
            forwards.remove(&remote_forward_port_key(tunnel.bind_port));
        }
        Err(error) => warnings.push(format!("remote forward route cleanup failed: {error}")),
    }
    warnings
}

pub(super) fn tunnel_status_from_runtime(runtime: &TunnelRuntime) -> TunnelStatus {
    runtime.metrics.snapshot(runtime.spec.clone())
}

pub(super) fn tunnel_label(
    mode: TunnelMode,
    bind_host: &str,
    bind_port: u16,
    target_host: &str,
    target_port: u16,
) -> String {
    let label = match mode {
        TunnelMode::Local => {
            format!("{bind_host}:{bind_port} -> {target_host}:{target_port}")
        }
        TunnelMode::Dynamic => format!("SOCKS5 {bind_host}:{bind_port}"),
        TunnelMode::Remote => {
            format!("remote {bind_host}:{bind_port} -> {target_host}:{target_port}")
        }
    };
    label.chars().take(MAX_TUNNEL_LABEL_CHARACTERS).collect()
}

pub(super) fn persist_tunnel_to_profile_and_log(
    state: &AppState,
    session_id: &str,
    tunnel: &TunnelSpec,
    local_addr: Option<std::net::SocketAddr>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
        let mut profile = next_store
            .profile(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
                ssh.tunnels = normalize_tunnels(std::mem::take(&mut ssh.tunnels));
                ssh.tunnels.retain(|item| item.id != tunnel.id);
                if ssh.tunnels.len() >= MAX_TUNNELS_PER_PROFILE {
                    if let Some(index) = ssh.tunnels.iter().position(|item| !item.enabled) {
                        ssh.tunnels.remove(index);
                    }
                }
                if ssh.tunnels.len() >= MAX_TUNNELS_PER_PROFILE {
                    return Err(format!(
                        "enabled tunnel count has reached {MAX_TUNNELS_PER_PROFILE}"
                    ));
                }
                ssh.tunnels.push(tunnel.clone());
                validate_tunnels(&ssh.tunnels)?;
                next_store.upsert_profile(profile);
            }
            _ => return Err("tunnels require an SSH or Tmux session".to_string()),
        }
        let listen = local_addr
            .map(|addr| addr.to_string())
            .unwrap_or_else(|| format!("{}:{}", tunnel.bind_host, tunnel.bind_port));
        let event_ids = next_store
            .record_system_event_tracked(
                session_id,
                format!(
                    "PortMate: SSH {:?} tunnel listening on {} -> {}:{}",
                    tunnel.mode, listen, tunnel.target_host, tunnel.target_port
                ),
            )
            .into_iter()
            .collect();
        Ok(((), event_ids))
    })
}

pub(super) fn persist_stopped_tunnel_to_profile_and_log(
    state: &AppState,
    session_id: &str,
    stopped: &TunnelSpec,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    mark_tunnel_stopped_in_store(&mut store, session_id, stopped);
    store.record_system_event(
        session_id,
        format!(
            "PortMate: SSH {:?} tunnel stopped on {}:{}",
            stopped.mode, stopped.bind_host, stopped.bind_port
        ),
    );
    persist_applied_store(&store, &state.store_path, "stopped tunnel state")
}

pub(super) fn mark_tunnel_stopped_in_store(
    store: &mut SessionStore,
    session_id: &str,
    stopped: &TunnelSpec,
) {
    if let Some(mut profile) = store.profile(session_id) {
        match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
                if let Some(saved) = ssh.tunnels.iter_mut().find(|item| item.id == stopped.id) {
                    saved.enabled = false;
                }
                ssh.tunnels = normalize_tunnels(std::mem::take(&mut ssh.tunnels));
                let _ = store.upsert_profile(profile);
            }
            _ => {}
        }
    }
}

pub(super) fn remote_forward_key(host: &str, port: u16) -> String {
    format!("{}:{}", host, port)
}

pub(super) fn remote_forward_port_key(port: u16) -> String {
    format!("*:{}", port)
}

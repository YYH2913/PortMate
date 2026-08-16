use super::*;

type TunnelLifecycleLanes = Mutex<HashMap<(PathBuf, String), Weak<tokio::sync::Mutex<()>>>>;

static TUNNEL_LIFECYCLE_LANES: OnceLock<TunnelLifecycleLanes> = OnceLock::new();

pub(super) fn tunnel_lifecycle_lane(
    state: &AppState,
    tunnel_id: &str,
) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    tunnel_lifecycle_lane_for_path(&state.store_path, tunnel_id)
}

pub(super) fn tunnel_lifecycle_lane_for_path(
    store_path: &Path,
    tunnel_id: &str,
) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let key = (store_path.to_path_buf(), tunnel_id.to_string());
    let mut lanes = TUNNEL_LIFECYCLE_LANES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "tunnel lifecycle lane registry poisoned".to_string())?;
    lanes.retain(|_, lane| lane.strong_count() > 0);
    if let Some(lane) = lanes.get(&key).and_then(Weak::upgrade) {
        return Ok(lane);
    }
    let lane = Arc::new(tokio::sync::Mutex::new(()));
    lanes.insert(key, Arc::downgrade(&lane));
    Ok(lane)
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
    owner: &TunnelRuntimeOwner,
    error: &str,
) -> Result<Option<TunnelRuntime>, String> {
    let runtime = remove_tunnel_runtime_if_owned(tunnels, tunnel_id, owner)?;
    if let Some(runtime) = &runtime {
        runtime.metrics.record_error(error);
        runtime.request_shutdown();
    }
    Ok(runtime)
}

pub(super) fn remove_tunnel_runtime_if_owned(
    tunnels: &Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    tunnel_id: &str,
    owner: &TunnelRuntimeOwner,
) -> Result<Option<TunnelRuntime>, String> {
    let mut tunnels = tunnels
        .lock()
        .map_err(|lock_error| lock_error.to_string())?;
    if tunnels
        .get(tunnel_id)
        .is_none_or(|runtime| !owner.owns(runtime))
    {
        return Ok(None);
    }
    Ok(tunnels.remove(tunnel_id))
}

pub(super) fn fail_session_tunnel_runtimes(
    tunnels: &Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    session_id: &str,
    error: &str,
) -> Result<Vec<TunnelRuntime>, String> {
    let removed = stop_session_tunnel_runtimes(tunnels, session_id)?;
    for runtime in &removed {
        runtime.metrics.record_error(error);
    }
    Ok(removed)
}

pub(super) fn stop_session_tunnel_runtimes(
    tunnels: &Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    session_id: &str,
) -> Result<Vec<TunnelRuntime>, String> {
    let mut tunnels = tunnels
        .lock()
        .map_err(|lock_error| lock_error.to_string())?;
    let ids = tunnels
        .iter()
        .filter_map(|(id, runtime)| (runtime.session_id == session_id).then_some(id.clone()))
        .collect::<Vec<_>>();
    let mut removed = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(runtime) = tunnels.remove(&id) {
            runtime.request_shutdown();
            removed.push(runtime);
        }
    }
    Ok(removed)
}

pub(super) async fn await_tunnel_listener_shutdowns(
    runtimes: &[TunnelRuntime],
) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + TUNNEL_LISTENER_SHUTDOWN_TIMEOUT;
    let mut timed_out = Vec::new();
    for runtime in runtimes {
        if runtime.listener_worker.is_finished() {
            continue;
        }
        if tokio::time::timeout_at(deadline, runtime.listener_worker.wait_finished())
            .await
            .is_err()
            && !runtime.listener_worker.is_finished()
        {
            timed_out.push(runtime.spec.id.clone());
        }
    }
    timed_out
}

pub(super) async fn stop_tunnel_inner(
    state: &AppState,
    tunnel_id: &str,
) -> Result<TunnelStatus, String> {
    stop_tunnel_inner_with_validation(state, tunnel_id, None).await
}

pub(super) async fn stop_tunnel_inner_with_validation(
    state: &AppState,
    tunnel_id: &str,
    commit_validation: Option<CommitValidation>,
) -> Result<TunnelStatus, String> {
    let lifecycle_lane = tunnel_lifecycle_lane(state, tunnel_id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
    if let Some(validate) = commit_validation {
        validate()?;
    }
    let expected_owner = {
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels
            .get(tunnel_id)
            .map(TunnelRuntime::owner)
            .ok_or_else(|| format!("tunnel not found: {tunnel_id}"))?
    };
    let (runtime, warnings) =
        stop_tunnel_runtime_effects(state, tunnel_id, &expected_owner).await?;

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
    owner: &TunnelRuntimeOwner,
) -> Result<(TunnelRuntime, Vec<String>), String> {
    let runtime = remove_tunnel_runtime_if_owned(&state.tunnels, tunnel_id, owner)?
        .ok_or_else(|| format!("tunnel runtime was superseded: {tunnel_id}"))?;
    runtime.request_shutdown();
    let mut warnings = Vec::new();
    if runtime.spec.mode != TunnelMode::Remote {
        if !await_tunnel_listener_shutdowns(std::slice::from_ref(&runtime))
            .await
            .is_empty()
        {
            warnings.push(format!(
                "tunnel listener shutdown timed out after {}ms",
                TUNNEL_LISTENER_SHUTDOWN_TIMEOUT.as_millis()
            ));
        }
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
        warnings.extend(
            cancel_remote_tunnel_forward(
                handle,
                remote_forwards,
                &runtime.spec,
                &runtime.metrics,
            )
            .await,
        );
    } else if warnings.is_empty() {
        warnings.push("SSH runtime unavailable during remote cancel".to_string());
    }
    Ok((runtime, warnings))
}

pub(super) async fn cancel_remote_tunnel_forward(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    tunnel: &TunnelSpec,
    route_owner: &Arc<TunnelMetrics>,
) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Err(error) = cancel_remote_tunnel_forward_with_timeout(
        &handle,
        tunnel.bind_host.clone(),
        tunnel.bind_port,
        REMOTE_TUNNEL_HEALTH_TIMEOUT,
        "remote SSH tunnel cancel",
    )
    .await
    {
        warnings.push(error);
    }
    match remote_forwards.lock() {
        Ok(mut forwards) => {
            remove_remote_forward_routes_if_owned(&mut forwards, tunnel, route_owner);
        }
        Err(error) => warnings.push(format!("remote forward route cleanup failed: {error}")),
    }
    warnings
}

pub(super) async fn cancel_remote_tunnel_forward_with_timeout(
    shared_handle: &Arc<tokio::sync::Mutex<SshBackendSession>>,
    bind_host: String,
    bind_port: u16,
    timeout: Duration,
    label: &str,
) -> Result<(), String> {
    let started = Instant::now();
    let target = format!("{bind_host}:{bind_port}");
    let handle = tokio::time::timeout(timeout, shared_handle.lock())
        .await
        .map_err(|_| {
            format!(
                "{label} handle lock timed out for {target} after {} ms",
                timeout.as_millis()
            )
        })?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            format!(
                "{label} timed out for {target} after {} ms",
                timeout.as_millis()
            )
        })?;
    match bounded_connection_step(
        handle.cancel_remote_forward(bind_host, bind_port, remaining),
        remaining,
    )
    .await
    {
        Ok(()) => Ok(()),
        Err(BoundedConnectionStepError::Failed(error)) => {
            Err(format!("{label} failed for {target}: {error}"))
        }
        Err(BoundedConnectionStepError::TimedOut) => {
            // A timed-out global cancel can still be accepted after its reply waiter is dropped.
            let cleanup_warning = request_backend_disconnect_with_timeout(
                &handle,
                "PortMate remote tunnel cancel request timeout",
            )
            .await
            .map(|warning| format!("; {warning}"))
            .unwrap_or_default();
            Err(format!(
                "{label} timed out for {target} after {} ms{cleanup_warning}",
                timeout.as_millis()
            ))
        }
    }
}

pub(super) fn ensure_remote_forward_route_slot(
    forwards: &HashMap<String, TunnelForwardTarget>,
    tunnel: &TunnelSpec,
    route_owner: &Arc<TunnelMetrics>,
) -> Result<(), String> {
    for key in [
        remote_forward_key(&tunnel.bind_host, tunnel.bind_port),
        remote_forward_port_key(tunnel.bind_port),
    ] {
        if let Some(existing) = forwards.get(&key) {
            let owned = existing.spec.id == tunnel.id
                && Arc::ptr_eq(&existing.metrics, route_owner);
            if !owned {
                return Err(format!(
                    "remote tunnel route already registered for {}:{}",
                    tunnel.bind_host, tunnel.bind_port
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn remove_remote_forward_routes_if_owned(
    forwards: &mut HashMap<String, TunnelForwardTarget>,
    tunnel: &TunnelSpec,
    route_owner: &Arc<TunnelMetrics>,
) {
    for key in [
        remote_forward_key(&tunnel.bind_host, tunnel.bind_port),
        remote_forward_port_key(tunnel.bind_port),
    ] {
        let owned = forwards.get(&key).is_some_and(|target| {
            target.spec.id == tunnel.id && Arc::ptr_eq(&target.metrics, route_owner)
        });
        if owned {
            forwards.remove(&key);
        }
    }
}

pub(super) fn remote_forward_rollback_ports(
    requested_port: u16,
    returned_port: Option<u16>,
) -> Vec<u16> {
    let mut ports = Vec::with_capacity(2);
    if let Some(returned_port) = returned_port.filter(|port| *port != 0) {
        ports.push(returned_port);
    }
    if requested_port != 0 && !ports.contains(&requested_port) {
        ports.push(requested_port);
    }
    if ports.is_empty() {
        ports.push(0);
    }
    ports
}

pub(super) async fn rollback_remote_tunnel_forward_attempt(
    handle: &Arc<tokio::sync::Mutex<SshBackendSession>>,
    remote_forwards: &Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    tunnel: &TunnelSpec,
    route_owner: &Arc<TunnelMetrics>,
    returned_port: Option<u16>,
    error: String,
) -> String {
    let mut warnings = Vec::new();
    for port in remote_forward_rollback_ports(tunnel.bind_port, returned_port) {
        let mut candidate = tunnel.clone();
        candidate.bind_port = port;
        warnings.extend(
            cancel_remote_tunnel_forward(
                Arc::clone(handle),
                Arc::clone(remote_forwards),
                &candidate,
                route_owner,
            )
            .await,
        );
    }
    if warnings.is_empty() {
        error
    } else {
        format!(
            "{error}; remote tunnel cleanup failed: {}",
            warnings.join("; ")
        )
    }
}

pub(super) async fn listen_remote_tunnel_forward_with_timeout(
    shared_handle: &Arc<tokio::sync::Mutex<SshBackendSession>>,
    bind_host: String,
    bind_port: u16,
    timeout: Duration,
    label: &str,
) -> Result<(u16, Option<libssh_rs::Session>), String> {
    let started = Instant::now();
    let handle = tokio::time::timeout(timeout, shared_handle.lock())
        .await
        .map_err(|_| {
            format!(
                "{label} handle lock timed out after {} ms",
                timeout.as_millis()
            )
        })?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{label} timed out after {} ms", timeout.as_millis()))?;
    match bounded_connection_step(
        handle.listen_remote_forward(bind_host, bind_port, remaining),
        remaining,
    )
    .await
    {
        Ok(port) => Ok((port, handle.libssh_forward_session())),
        Err(BoundedConnectionStepError::Failed(error)) => Err(format!("{label} failed: {error}")),
        Err(BoundedConnectionStepError::TimedOut) => {
            // A timed-out global request can have been accepted after its reply waiter is dropped.
            let cleanup_warning = request_backend_disconnect_with_timeout(
                &handle,
                "PortMate remote tunnel forward request timeout",
            )
            .await
            .map(|warning| format!("; {warning}"))
            .unwrap_or_default();
            Err(format!(
                "{label} timed out after {} ms{cleanup_warning}",
                timeout.as_millis()
            ))
        }
    }
}

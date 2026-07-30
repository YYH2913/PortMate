use super::*;

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

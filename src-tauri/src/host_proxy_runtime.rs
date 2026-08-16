use super::*;

pub(super) async fn start_portmate_host_tunnel_runtime_with_validation(
    state: &AppState,
    session_id: &str,
    mut tunnel: TunnelSpec,
    relabel_assigned_port: bool,
    commit_validation: Option<CommitValidation>,
) -> Result<(TunnelSpec, Option<std::net::SocketAddr>, TunnelRuntimeOwner), String> {
    if tunnel.egress != TunnelEgress::PortmateHost {
        return Err("PortMate host proxy runtime requires portmate-host egress".to_string());
    }
    validate_tunnels(std::slice::from_ref(&tunnel))?;
    {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if store.profile(session_id).is_none() {
            return Err(format!("unknown session: {session_id}"));
        }
    }
    {
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        ensure_tunnel_runtime_slot(&tunnels, &tunnel.id)?;
    }

    let listener = TcpListener::bind((tunnel.bind_host.clone(), tunnel.bind_port))
        .await
        .map_err(|error| {
            format!(
                "PortMate host proxy bind failed {}:{}: {error}",
                tunnel.bind_host, tunnel.bind_port
            )
        })?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| format!("PortMate host proxy local addr failed: {error}"))?;
    if tunnel.bind_port == 0 {
        tunnel.bind_port = local_addr.port();
        if relabel_assigned_port {
            tunnel.label = tunnel_label_for_egress(
                tunnel.egress,
                tunnel.mode,
                &tunnel.bind_host,
                tunnel.bind_port,
                &tunnel.target_host,
                tunnel.target_port,
            );
        }
    }
    if let Some(validate) = commit_validation {
        validate()?;
    }

    let closed = Arc::new(AtomicBool::new(false));
    let (listener_worker, listener_completion) = TunnelListenerWorker::running();
    let metrics = Arc::new(TunnelMetrics::default());
    let owner = TunnelRuntimeOwner {
        ssh_runtime_id: format!("portmate-host:{}", Uuid::new_v4()),
        closed: Arc::clone(&closed),
    };
    {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if store.profile(session_id).is_none() {
            return Err(format!("unknown session: {session_id}"));
        }
        let mut tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        ensure_tunnel_runtime_slot(&tunnels, &tunnel.id)?;
        tunnels.insert(
            tunnel.id.clone(),
            TunnelRuntime {
                session_id: session_id.to_string(),
                ssh_runtime_id: owner.ssh_runtime_id.clone(),
                spec: tunnel.clone(),
                metrics: Arc::clone(&metrics),
                closed: Arc::clone(&closed),
                listener_worker: listener_worker.clone(),
            },
        );
    }

    let connection_slots = Arc::clone(&state.tunnel_connection_slots);
    let listener_state = state.clone();
    let session_id = session_id.to_string();
    let tunnel_for_task = tunnel.clone();
    let owner_for_task = owner.clone();
    tauri::async_runtime::spawn(async move {
        let _listener_completion = listener_completion;
        loop {
            if closed.load(Ordering::SeqCst) {
                break;
            }
            let accepted = tokio::select! {
                accepted = listener.accept() => Some(accepted),
                _ = listener_worker.wait_shutdown() => None,
            };
            match accepted {
                Some(Ok((stream, peer))) => {
                    let Some(permit) =
                        try_acquire_tunnel_connection(&connection_slots, metrics.as_ref())
                    else {
                        continue;
                    };
                    let state = listener_state.clone();
                    let spec = tunnel_for_task.clone();
                    let metrics = Arc::clone(&metrics);
                    let session_id = session_id.clone();
                    let owner = owner_for_task.clone();
                    tauri::async_runtime::spawn(async move {
                        let _permit = permit;
                        metrics.connection_opened();
                        let result = handle_portmate_host_tunnel_client(
                            spec.clone(),
                            stream,
                            peer,
                            Arc::clone(&metrics),
                        )
                        .await;
                        match result {
                            Ok(()) => metrics.clear_error(),
                            Err(error) => {
                                metrics.record_error(&error);
                                if let Err(record_error) = record_tunnel_client_failure_if_owned(
                                    &state.tunnels,
                                    &state.store,
                                    &state.store_path,
                                    &spec.id,
                                    &owner,
                                    &session_id,
                                    &error,
                                ) {
                                    eprintln!(
                                        "PortMate: failed to record host proxy client error: {record_error}"
                                    );
                                }
                            }
                        }
                        metrics.connection_closed();
                    });
                }
                Some(Err(error)) => {
                    let message = format!("PortMate host proxy accept failed: {error}");
                    if let Err(registry_error) = fail_tunnel_listener_if_owned(
                        &listener_state,
                        &tunnel_for_task.id,
                        &owner_for_task,
                        &session_id,
                        &tunnel_for_task,
                        &message,
                    )
                    .await
                    {
                        closed.store(true, Ordering::SeqCst);
                        metrics.record_error(&format!("{message}; {registry_error}"));
                    }
                    break;
                }
                None => break,
            }
        }
    });

    Ok((tunnel, Some(local_addr), owner))
}

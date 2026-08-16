use super::*;

pub(super) async fn start_tunnel_runtime(
    state: &AppState,
    session_id: &str,
    tunnel: TunnelSpec,
    relabel_assigned_port: bool,
    expected_runtime_id: Option<&str>,
) -> Result<(TunnelSpec, Option<std::net::SocketAddr>, TunnelRuntimeOwner), String> {
    start_tunnel_runtime_with_validation(
        state,
        session_id,
        tunnel,
        relabel_assigned_port,
        expected_runtime_id,
        None,
    )
    .await
}

pub(super) async fn start_tunnel_runtime_with_validation(
    state: &AppState,
    session_id: &str,
    mut tunnel: TunnelSpec,
    relabel_assigned_port: bool,
    expected_runtime_id: Option<&str>,
    commit_validation: Option<CommitValidation>,
) -> Result<(TunnelSpec, Option<std::net::SocketAddr>, TunnelRuntimeOwner), String> {
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
                return Err(rollback_uninstalled_remote_tunnel(
                    &handle,
                    &remote_forwards,
                    &tunnel,
                    "remote SSH tunnel request did not return an assigned port".to_string(),
                )
                .await);
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
        let owner = TunnelRuntimeOwner {
            ssh_runtime_id: ssh_runtime_id.clone(),
            closed: Arc::clone(&closed),
        };
        let install_result = commit_validation
            .map_or(Ok(()), |validate| validate())
            .and_then(|()| {
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
                    listener_worker: TunnelListenerWorker::completed(),
                },
            );
                Ok::<(), String>(())
            });
        if let Err(error) = install_result {
            return Err(rollback_uninstalled_remote_tunnel(
                &handle,
                &remote_forwards,
                &tunnel,
                error,
            )
            .await);
        }
        ensure_libssh_remote_forward_acceptor(
            libssh_session,
            Arc::clone(&remote_forwards),
            ssh_runtime_closed,
            remote_forward_acceptor_started,
        );
        spawn_remote_tunnel_health_monitor(state.clone(), tunnel.id.clone(), Arc::clone(&closed));
        return Ok((tunnel, None, owner));
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
    if let Some(validate) = commit_validation {
        validate()?;
    }
    let closed = Arc::new(AtomicBool::new(false));
    let (listener_worker, listener_completion) = TunnelListenerWorker::running();
    let metrics = Arc::new(TunnelMetrics::default());
    let owner = TunnelRuntimeOwner {
        ssh_runtime_id: ssh_runtime_id.clone(),
        closed: Arc::clone(&closed),
    };
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
                listener_worker: listener_worker.clone(),
            },
        );
    }

    let session_id = session_id.to_string();
    let store = Arc::clone(&state.store);
    let store_path = state.store_path.clone();
    let tunnel_registry = Arc::clone(&state.tunnels);
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
                            handle_dynamic_tunnel_client(
                                handle,
                                spec,
                                stream,
                                peer,
                                Arc::clone(&metrics),
                            )
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
                Some(Err(error)) => {
                    let message = format!("SSH tunnel accept failed: {error}");
                    let removed = match fail_tunnel_runtime_if_owned(
                        &tunnel_registry,
                        &tunnel_for_task.id,
                        &owner_for_task,
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
                None => break,
            }
        }
    });

    Ok((tunnel, Some(local_addr), owner))
}

async fn rollback_uninstalled_remote_tunnel(
    handle: &Arc<tokio::sync::Mutex<SshBackendSession>>,
    remote_forwards: &Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    tunnel: &TunnelSpec,
    error: String,
) -> String {
    let warnings = cancel_remote_tunnel_forward(
        Arc::clone(handle),
        Arc::clone(remote_forwards),
        tunnel,
    )
    .await;
    if warnings.is_empty() {
        error
    } else {
        format!(
            "{error}; remote tunnel cleanup failed: {}",
            warnings.join("; ")
        )
    }
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

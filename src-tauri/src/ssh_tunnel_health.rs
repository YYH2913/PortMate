use super::*;

pub(super) const REMOTE_TUNNEL_HEALTH_INTERVAL: Duration = Duration::from_secs(15);
pub(super) const REMOTE_TUNNEL_HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const REMOTE_TUNNEL_HEALTH_ERROR_PREFIX: &str = "remote forward health check failed:";
pub(super) const REMOTE_TUNNEL_PROBE_COMMAND: &str = r#"sh -lc 'if [ -r /proc/net/tcp ]; then echo __PORTMATE_PROC__; cat /proc/net/tcp /proc/net/tcp6 2>/dev/null || true; elif command -v ss >/dev/null 2>&1 && probe=$(ss -H -ltn 2>/dev/null); then echo __PORTMATE_SS__; printf "%s\n" "$probe"; elif command -v sockstat >/dev/null 2>&1 && probe=$(sockstat -46ln 2>/dev/null); then echo __PORTMATE_SOCKSTAT__; printf "%s\n" "$probe"; elif command -v lsof >/dev/null 2>&1 && probe=$(lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null); then echo __PORTMATE_LSOF__; printf "%s\n" "$probe"; elif command -v netstat >/dev/null 2>&1 && probe=$(netstat -ltn 2>/dev/null); then echo __PORTMATE_NETSTAT__; printf "%s\n" "$probe"; else echo __PORTMATE_UNSUPPORTED__; fi'"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemoteTunnelHealth {
    Healthy,
    Restored,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemoteListenerProbe {
    Listening,
    Missing,
    Unsupported,
}

pub(super) fn spawn_remote_tunnel_health_monitor(
    state: AppState,
    tunnel_id: String,
    closed: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(REMOTE_TUNNEL_HEALTH_INTERVAL).await;
            if closed.load(Ordering::SeqCst) {
                break;
            }
            match check_remote_tunnel_health(&state, &tunnel_id).await {
                Ok(RemoteTunnelHealth::Unsupported) => break,
                Ok(RemoteTunnelHealth::Healthy | RemoteTunnelHealth::Restored) => {}
                Err(error) => {
                    eprintln!("PortMate: remote tunnel health check failed: {error}");
                }
            }
        }
    });
}

pub(super) async fn check_remote_tunnel_health(
    state: &AppState,
    tunnel_id: &str,
) -> Result<RemoteTunnelHealth, String> {
    let runtime = {
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels
            .get(tunnel_id)
            .cloned()
            .ok_or_else(|| format!("tunnel not found: {tunnel_id}"))?
    };
    if runtime.spec.mode != TunnelMode::Remote {
        return Err(format!("tunnel is not a remote forward: {tunnel_id}"));
    }
    if runtime.closed.load(Ordering::SeqCst) {
        return Err(format!("remote tunnel is closed: {tunnel_id}"));
    }

    let result = probe_remote_tunnel_health(state, &runtime).await;
    match &result {
        Ok(RemoteTunnelHealth::Healthy) => {
            if runtime
                .metrics
                .clear_error_with_prefix(REMOTE_TUNNEL_HEALTH_ERROR_PREFIX)
            {
                record_tunnel_health_event(
                    state,
                    &runtime.session_id,
                    tunnel_id,
                    "remote forward is healthy again",
                );
            }
        }
        Ok(RemoteTunnelHealth::Restored) => {
            runtime
                .metrics
                .clear_error_with_prefix(REMOTE_TUNNEL_HEALTH_ERROR_PREFIX);
            record_tunnel_health_event(
                state,
                &runtime.session_id,
                tunnel_id,
                "remote forward listener was missing and has been restored",
            );
        }
        Ok(RemoteTunnelHealth::Unsupported) => {}
        Err(error) => {
            let message = format!("{REMOTE_TUNNEL_HEALTH_ERROR_PREFIX} {error}");
            if runtime.metrics.record_error_if_changed(&message) {
                record_tunnel_health_event(state, &runtime.session_id, tunnel_id, &message);
            }
        }
    }
    result
}

pub(super) async fn probe_remote_tunnel_health(
    state: &AppState,
    runtime: &TunnelRuntime,
) -> Result<RemoteTunnelHealth, String> {
    let _auxiliary_slot = acquire_ssh_auxiliary_slot(state)?;
    let (handle, remote_forwards) = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(&runtime.session_id)
            .filter(|ssh| ssh.runtime_id == runtime.ssh_runtime_id)
            .map(|ssh| (Arc::clone(&ssh.handle), Arc::clone(&ssh.remote_forwards)))
            .ok_or_else(|| format!("SSH runtime is unavailable for {}", runtime.session_id))?
    };

    let mut routing_restored = false;
    {
        let mut forwards = remote_forwards.lock().map_err(|error| error.to_string())?;
        let exact_key = remote_forward_key(&runtime.spec.bind_host, runtime.spec.bind_port);
        let port_key = remote_forward_port_key(runtime.spec.bind_port);
        if !forwards.contains_key(&exact_key) || !forwards.contains_key(&port_key) {
            let target = TunnelForwardTarget {
                spec: runtime.spec.clone(),
                metrics: Arc::clone(&runtime.metrics),
                connection_slots: Arc::clone(&state.tunnel_connection_slots),
            };
            forwards.insert(exact_key, target.clone());
            forwards.insert(port_key, target);
            routing_restored = true;
        }
    }

    let output = match exec_ssh_command_capture(
        Arc::clone(&handle),
        REMOTE_TUNNEL_PROBE_COMMAND,
        REMOTE_TUNNEL_HEALTH_TIMEOUT,
    )
    .await
    {
        Ok(output) => output,
        Err(error) if error.starts_with("SSH exec 返回非零状态") => {
            return Ok(RemoteTunnelHealth::Unsupported);
        }
        Err(error) => return Err(format!("listener probe failed: {error}")),
    };
    match parse_remote_listener_probe(&output, runtime.spec.bind_port) {
        RemoteListenerProbe::Listening => Ok(if routing_restored {
            RemoteTunnelHealth::Restored
        } else {
            RemoteTunnelHealth::Healthy
        }),
        RemoteListenerProbe::Unsupported => Ok(RemoteTunnelHealth::Unsupported),
        RemoteListenerProbe::Missing => {
            if runtime.closed.load(Ordering::SeqCst) {
                return Err("tunnel closed during listener probe".to_string());
            }
            let returned_port = {
                let handle = handle.lock().await;
                if runtime.closed.load(Ordering::SeqCst) {
                    return Err("tunnel closed before listener restore".to_string());
                }
                handle
                    .listen_remote_forward(runtime.spec.bind_host.clone(), runtime.spec.bind_port)
                    .await
                    .map_err(|error| {
                        format!(
                            "listener restore failed {}:{}: {error}",
                            runtime.spec.bind_host, runtime.spec.bind_port
                        )
                    })?
            };
            if returned_port != runtime.spec.bind_port {
                return Err(format!(
                    "listener restore returned unexpected port {returned_port} for {}",
                    runtime.spec.bind_port
                ));
            }
            Ok(RemoteTunnelHealth::Restored)
        }
    }
}

pub(super) fn parse_remote_listener_probe(output: &str, port: u16) -> RemoteListenerProbe {
    if output.contains("__PORTMATE_UNSUPPORTED__") {
        return RemoteListenerProbe::Unsupported;
    }
    if let Some((_, table)) = output.split_once("__PORTMATE_PROC__") {
        let expected_port = format!("{port:04X}");
        let listening = table.lines().any(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            fields.len() > 3
                && fields[3] == "0A"
                && fields[1]
                    .rsplit_once(':')
                    .is_some_and(|(_, value)| value.eq_ignore_ascii_case(&expected_port))
        });
        return if listening {
            RemoteListenerProbe::Listening
        } else {
            RemoteListenerProbe::Missing
        };
    }
    if output.contains("__PORTMATE_SOCKSTAT__") {
        let listening = output
            .lines()
            .skip_while(|line| !line.contains("__PORTMATE_SOCKSTAT__"))
            .skip(1)
            .any(|line| {
                !line.to_ascii_uppercase().contains("LOCAL ADDRESS")
                    && line
                        .split_whitespace()
                        .any(|field| socket_endpoint_port(field) == Some(port))
            });
        return if listening {
            RemoteListenerProbe::Listening
        } else {
            RemoteListenerProbe::Missing
        };
    }
    if output.contains("__PORTMATE_SS__")
        || output.contains("__PORTMATE_LSOF__")
        || output.contains("__PORTMATE_NETSTAT__")
    {
        let listening = output.lines().any(|line| {
            line.to_ascii_uppercase().contains("LISTEN")
                && line
                    .split_whitespace()
                    .any(|field| socket_endpoint_port(field) == Some(port))
        });
        return if listening {
            RemoteListenerProbe::Listening
        } else {
            RemoteListenerProbe::Missing
        };
    }
    RemoteListenerProbe::Unsupported
}

pub(super) fn socket_endpoint_port(endpoint: &str) -> Option<u16> {
    let endpoint = endpoint.trim_matches(|character: char| {
        character == ',' || character == ';' || character == '(' || character == ')'
    });
    endpoint
        .rsplit_once(':')
        .or_else(|| endpoint.rsplit_once('.'))
        .and_then(|(_, port)| port.parse::<u16>().ok())
}

pub(super) fn record_tunnel_health_event(
    state: &AppState,
    session_id: &str,
    tunnel_id: &str,
    message: &str,
) {
    record_applied_system_event(
        state,
        session_id,
        format!("PortMate: remote tunnel {tunnel_id}: {message}"),
        "remote tunnel health event",
    );
}

use super::*;

pub(super) const REMOTE_TUNNEL_HEALTH_INTERVAL: Duration = Duration::from_secs(15);
pub(super) const REMOTE_TUNNEL_HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const REMOTE_TUNNEL_HEALTH_ERROR_PREFIX: &str = "remote forward health check failed:";
pub(super) const REMOTE_TUNNEL_PROBE_COMMAND: &str = r#"sh -lc 'if [ -r /proc/net/tcp ]; then echo __PORTMATE_PROC__; cat /proc/net/tcp /proc/net/tcp6 2>/dev/null || true; elif command -v ss >/dev/null 2>&1 && probe=$(ss -H -ltn 2>/dev/null); then echo __PORTMATE_SS__; printf "%s\n" "$probe"; elif command -v sockstat >/dev/null 2>&1 && probe=$(sockstat -46ln 2>/dev/null); then echo __PORTMATE_SOCKSTAT__; printf "%s\n" "$probe"; elif command -v lsof >/dev/null 2>&1 && probe=$(lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null); then echo __PORTMATE_LSOF__; printf "%s\n" "$probe"; elif command -v netstat >/dev/null 2>&1 && probe=$(netstat -ltn 2>/dev/null); then echo __PORTMATE_NETSTAT__; printf "%s\n" "$probe"; else echo __PORTMATE_UNSUPPORTED__; fi'"#;
pub(super) const REMOTE_WINDOWS_TUNNEL_PROBE_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$listeners = [System.Net.NetworkInformation.IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners()
[Console]::Out.WriteLine('__PORTMATE_WINDOWS_TCP__')
$listeners | ForEach-Object { [Console]::Out.WriteLine([string]$_.Port) }
"#;

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
        let mut listener_probe_supported = true;
        loop {
            tokio::time::sleep(REMOTE_TUNNEL_HEALTH_INTERVAL).await;
            if closed.load(Ordering::SeqCst) {
                break;
            }
            if !listener_probe_supported {
                match ensure_libssh_remote_forward_acceptor_for_tunnel(&state, &tunnel_id).await {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(error) => {
                        eprintln!("PortMate: remote tunnel acceptor check failed: {error}");
                        continue;
                    }
                }
            }
            match check_remote_tunnel_health(&state, &tunnel_id).await {
                Ok(RemoteTunnelHealth::Unsupported) => {
                    match ensure_libssh_remote_forward_acceptor_for_tunnel(&state, &tunnel_id).await
                    {
                        Ok(true) => listener_probe_supported = false,
                        Ok(false) => break,
                        Err(error) => {
                            eprintln!("PortMate: remote tunnel acceptor check failed: {error}");
                        }
                    }
                }
                Ok(RemoteTunnelHealth::Healthy | RemoteTunnelHealth::Restored) => {}
                Err(error) => {
                    eprintln!("PortMate: remote tunnel health check failed: {error}");
                }
            }
        }
    });
}

async fn ensure_libssh_remote_forward_acceptor_for_tunnel(
    state: &AppState,
    tunnel_id: &str,
) -> Result<bool, String> {
    let runtime = {
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels
            .get(tunnel_id)
            .cloned()
            .ok_or_else(|| format!("tunnel not found: {tunnel_id}"))?
    };
    if runtime.closed.load(Ordering::SeqCst) {
        return Err(format!("remote tunnel is closed: {tunnel_id}"));
    }
    let (handle, remote_forwards, started, ssh_runtime_closed) = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        let ssh = connections
            .get(&runtime.session_id)
            .filter(|ssh| ssh.runtime_id == runtime.ssh_runtime_id)
            .ok_or_else(|| format!("SSH runtime is unavailable for {}", runtime.session_id))?;
        if ssh.backend != SshBackendKind::Libssh {
            return Ok(false);
        }
        (
            Arc::clone(&ssh.handle),
            Arc::clone(&ssh.remote_forwards),
            Arc::clone(&ssh.remote_forward_acceptor_started),
            Arc::clone(&ssh.closed),
        )
    };
    let session = remote_forward_libssh_session(&handle).await?;
    ensure_libssh_remote_forward_acceptor(
        Some(session),
        remote_forwards,
        ssh_runtime_closed,
        started,
    );
    Ok(true)
}

async fn remote_forward_libssh_session(
    handle: &Arc<tokio::sync::Mutex<SshBackendSession>>,
) -> Result<libssh_rs::Session, String> {
    let handle = tokio::time::timeout(REMOTE_TUNNEL_HEALTH_TIMEOUT, handle.lock())
        .await
        .map_err(|_| {
            format!(
                "libssh remote forward handle lock timed out after {}ms",
                REMOTE_TUNNEL_HEALTH_TIMEOUT.as_millis()
            )
        })?;
    handle
        .libssh_forward_session()
        .ok_or_else(|| "libssh runtime does not expose a libssh forward session".to_string())
}

pub(super) async fn check_remote_tunnel_health(
    state: &AppState,
    tunnel_id: &str,
) -> Result<RemoteTunnelHealth, String> {
    let lifecycle_lane = tunnel_lifecycle_lane(state, tunnel_id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
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
    ensure_tunnel_runtime_current(state, tunnel_id, &runtime)?;
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

pub(super) fn ensure_tunnel_runtime_current(
    state: &AppState,
    tunnel_id: &str,
    expected: &TunnelRuntime,
) -> Result<(), String> {
    let owner = expected.owner();
    let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
    if tunnels.get(tunnel_id).is_some_and(|runtime| {
        owner.owns(runtime) && !runtime.closed.load(Ordering::SeqCst)
    }) {
        Ok(())
    } else {
        Err(format!(
            "tunnel runtime changed during health check: {tunnel_id}"
        ))
    }
}

pub(super) async fn probe_remote_tunnel_health(
    state: &AppState,
    runtime: &TunnelRuntime,
) -> Result<RemoteTunnelHealth, String> {
    let _auxiliary_slot = acquire_ssh_auxiliary_slot(state)?;
    if runtime.closed.load(Ordering::SeqCst) {
        return Err("tunnel closed before listener probe".to_string());
    }
    let (
        handle,
        remote_forwards,
        remote_forward_acceptor_started,
        ssh_runtime_closed,
        ssh_backend,
    ) = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(&runtime.session_id)
            .filter(|ssh| ssh.runtime_id == runtime.ssh_runtime_id)
            .map(|ssh| {
                (
                    Arc::clone(&ssh.handle),
                    Arc::clone(&ssh.remote_forwards),
                    Arc::clone(&ssh.remote_forward_acceptor_started),
                    Arc::clone(&ssh.closed),
                    ssh.backend,
                )
            })
            .ok_or_else(|| format!("SSH runtime is unavailable for {}", runtime.session_id))?
    };

    let mut routing_restored = false;
    {
        let mut forwards = remote_forwards.lock().map_err(|error| error.to_string())?;
        let exact_key = remote_forward_key(&runtime.spec.bind_host, runtime.spec.bind_port);
        let port_key = remote_forward_port_key(runtime.spec.bind_port);
        if !forwards.contains_key(&exact_key) || !forwards.contains_key(&port_key) {
            ensure_remote_forward_route_slot(&forwards, &runtime.spec, &runtime.metrics)?;
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
    let libssh_session = if ssh_backend == SshBackendKind::Libssh {
        Some(remote_forward_libssh_session(&handle).await?)
    } else {
        None
    };
    ensure_libssh_remote_forward_acceptor(
        libssh_session,
        Arc::clone(&remote_forwards),
        ssh_runtime_closed,
        remote_forward_acceptor_started,
    );

    let probe_started = Instant::now();
    let probe = match exec_ssh_command_capture(
        Arc::clone(&handle),
        REMOTE_TUNNEL_PROBE_COMMAND,
        REMOTE_TUNNEL_HEALTH_TIMEOUT,
    )
    .await
    {
        Ok(output) => parse_remote_listener_probe(&output, runtime.spec.bind_port),
        Err(error) if error.starts_with("SSH exec 返回非零状态") => {
            RemoteListenerProbe::Unsupported
        }
        Err(error) => return Err(format!("listener probe failed: {error}")),
    };
    let probe = if probe == RemoteListenerProbe::Unsupported {
        let remaining = REMOTE_TUNNEL_HEALTH_TIMEOUT
            .checked_sub(probe_started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| "listener probe timed out before Windows fallback".to_string())?;
        let command = windows_powershell_command(REMOTE_WINDOWS_TUNNEL_PROBE_SCRIPT);
        match exec_ssh_command_capture(handle.clone(), &command, remaining).await {
            Ok(output) => parse_remote_listener_probe(&output, runtime.spec.bind_port),
            Err(error) if error.starts_with("SSH exec 返回非零状态") => {
                RemoteListenerProbe::Unsupported
            }
            Err(error) => return Err(format!("Windows listener probe failed: {error}")),
        }
    } else {
        probe
    };
    if runtime.closed.load(Ordering::SeqCst) {
        return Err("tunnel closed during listener probe".to_string());
    }
    match probe {
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
            if runtime.closed.load(Ordering::SeqCst) {
                return Err("tunnel closed before listener restore".to_string());
            }
            let (returned_port, _) = listen_remote_tunnel_forward_with_timeout(
                &handle,
                runtime.spec.bind_host.clone(),
                runtime.spec.bind_port,
                REMOTE_TUNNEL_HEALTH_TIMEOUT,
                &format!(
                    "remote SSH listener restore {}:{}",
                    runtime.spec.bind_host, runtime.spec.bind_port
                ),
            )
            .await?;
            if returned_port != runtime.spec.bind_port {
                return Err(rollback_remote_tunnel_forward_attempt(
                    &handle,
                    &remote_forwards,
                    &runtime.spec,
                    &runtime.metrics,
                    Some(returned_port),
                    format!(
                        "listener restore returned unexpected port {returned_port} for {}",
                        runtime.spec.bind_port
                    ),
                )
                .await);
            }
            if runtime.closed.load(Ordering::SeqCst) {
                return Err(rollback_remote_tunnel_forward_attempt(
                    &handle,
                    &remote_forwards,
                    &runtime.spec,
                    &runtime.metrics,
                    Some(returned_port),
                    "tunnel closed during listener restore".to_string(),
                )
                .await);
            }
            Ok(RemoteTunnelHealth::Restored)
        }
    }
}

pub(super) fn parse_remote_listener_probe(output: &str, port: u16) -> RemoteListenerProbe {
    if output.contains("__PORTMATE_UNSUPPORTED__") {
        return RemoteListenerProbe::Unsupported;
    }
    if let Some((_, ports)) = output.split_once("__PORTMATE_WINDOWS_TCP__") {
        let listening = ports.lines().any(|line| {
            line.trim()
                .parse::<u16>()
                .is_ok_and(|candidate| candidate == port)
        });
        return if listening {
            RemoteListenerProbe::Listening
        } else {
            RemoteListenerProbe::Missing
        };
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

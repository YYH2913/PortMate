use super::*;

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

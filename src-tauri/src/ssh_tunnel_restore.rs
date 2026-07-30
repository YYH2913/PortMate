use super::*;

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

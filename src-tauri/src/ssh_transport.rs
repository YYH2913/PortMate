use super::*;

pub(super) const SSH_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

pub(super) struct SshConnectRequest<'a> {
    pub(super) config: Arc<client::Config>,
    pub(super) store: Arc<Mutex<SessionStore>>,
    pub(super) store_path: PathBuf,
    pub(super) profile: &'a SessionProfile,
    pub(super) ssh: &'a SshConnection,
    pub(super) host_keys: HostKeyStore,
    pub(super) one_time_host_keys: Vec<TrustedHostKey>,
    pub(super) observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    pub(super) host_key_error: Arc<Mutex<Option<String>>>,
    pub(super) remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    pub(super) password: Option<&'a str>,
    pub(super) passphrase: Option<&'a str>,
    pub(super) enforce_profile_snapshot: bool,
}

#[derive(Clone, Copy)]
pub(super) struct HostKeyPersistenceGuard<'a> {
    pub(super) profile_id: &'a str,
    pub(super) expected_profile: Option<&'a SessionProfile>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SshTargetTransportMode {
    Russh,
    JumpChannel,
}

pub(super) enum ConnectedSshTarget {
    Russh {
        session: client::Handle<PortMateSshHandler>,
        jump_sessions: Vec<client::Handle<PortMateSshHandler>>,
    },
    JumpChannel {
        channel: Channel<client::Msg>,
        jump_sessions: Vec<client::Handle<PortMateSshHandler>>,
    },
}

pub(super) async fn establish_ssh_runtime(
    state: &AppState,
    profile: &SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<EstablishedSshRuntime, String> {
    establish_ssh_runtime_with_timeout_mode(
        state,
        profile,
        password,
        passphrase,
        SSH_CONNECT_TIMEOUT,
        None,
        false,
    )
    .await
}

pub(super) async fn establish_ssh_reconnect_runtime(
    state: &AppState,
    profile: &SessionProfile,
) -> Result<EstablishedSshRuntime, String> {
    establish_ssh_runtime_with_timeout_mode(
        state,
        profile,
        None,
        None,
        SSH_CONNECT_TIMEOUT,
        None,
        true,
    )
    .await
}

#[cfg(test)]
pub(super) async fn establish_ssh_runtime_with_timeout(
    state: &AppState,
    profile: &SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
    connect_timeout: Duration,
    agent_socket_path: Option<PathBuf>,
) -> Result<EstablishedSshRuntime, String> {
    establish_ssh_runtime_with_timeout_mode(
        state,
        profile,
        password,
        passphrase,
        connect_timeout,
        agent_socket_path,
        false,
    )
    .await
}

pub(super) async fn establish_ssh_runtime_with_timeout_mode(
    state: &AppState,
    profile: &SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
    connect_timeout: Duration,
    agent_socket_path: Option<PathBuf>,
    enforce_profile_snapshot: bool,
) -> Result<EstablishedSshRuntime, String> {
    let mut ssh = match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.clone(),
        _ => return Err("profile is not SSH-backed".to_string()),
    };
    ssh.normalize_health_settings();

    let host = ssh.endpoint.host.trim().to_string();
    if host.is_empty() {
        return Err("SSH 主机不能为空".to_string());
    }
    let username = ssh.username.trim().to_string();
    if username.is_empty() {
        return Err("SSH 用户名不能为空；PortMate 不读取系统 ssh_config 的默认用户名".to_string());
    }

    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());
    let one_time_host_keys = one_time_host_keys_snapshot(state, &profile.id)?;
    host_keys.keys.extend(one_time_host_keys.clone());

    let observed_key = Arc::new(Mutex::new(None));
    let host_key_error = Arc::new(Mutex::new(None));
    let remote_forwards = Arc::new(Mutex::new(HashMap::new()));

    if ssh_uses_libssh_gssapi_backend(&ssh) {
        return establish_libssh_gssapi_runtime(
            state,
            profile,
            &ssh,
            password,
            passphrase,
            connect_timeout,
            host_keys,
            one_time_host_keys,
            observed_key,
            host_key_error,
            remote_forwards,
            agent_socket_path,
            enforce_profile_snapshot,
        )
        .await;
    }

    let config = Arc::new(ssh_client_config(&ssh));

    let connected_target = connect_ssh_target(
        SshConnectRequest {
            config: Arc::clone(&config),
            store: Arc::clone(&state.store),
            store_path: state.store_path.clone(),
            profile,
            ssh: &ssh,
            host_keys,
            one_time_host_keys: one_time_host_keys.clone(),
            observed_key: Arc::clone(&observed_key),
            host_key_error: Arc::clone(&host_key_error),
            remote_forwards: Arc::clone(&remote_forwards),
            password: password.as_deref(),
            passphrase: passphrase.as_deref(),
            enforce_profile_snapshot,
        },
        connect_timeout,
        agent_socket_path.as_deref(),
        SshTargetTransportMode::Russh,
    )
    .await?;
    let ConnectedSshTarget::Russh {
        mut session,
        jump_sessions,
    } = connected_target
    else {
        return Err("SSH target returned an unexpected Jump Host transport".to_string());
    };

    let auth_method = authenticate_ssh_with_timeout(
        &mut session,
        SshAuthenticationRequest {
            ssh: ssh.clone(),
            username: username.clone(),
            password,
            passphrase,
            agent_socket_path,
            timeout: connect_timeout,
            disconnect_description: "PortMate target authentication timeout",
        },
    )
    .await;
    let auth_method = match auth_method {
        Ok(method) => method,
        Err(error) => {
            let cleanup_warning = if matches!(error, SshAuthenticationError::Failed(_)) {
                request_ssh_disconnect_with_timeout(
                    &session,
                    "PortMate target authentication failed",
                )
                .await
            } else {
                None
            };
            disconnect_jump_sessions(jump_sessions, "PortMate target authentication failed").await;
            let cleanup_warning = cleanup_warning
                .map(|warning| format!("; {warning}"))
                .unwrap_or_default();
            return Err(format!(
                "SSH 目标认证失败 {host}:{}: {error}{cleanup_warning}",
                ssh.endpoint.port
            ));
        }
    };
    let host_key_persistence = persist_observed_host_key(
        &state.store,
        &state.store_path,
        HostKeyPersistenceGuard {
            profile_id: &profile.id,
            expected_profile: enforce_profile_snapshot.then_some(profile),
        },
        &observed_key,
        &one_time_host_keys,
    );
    if let Err(error) = host_key_persistence {
        let cleanup_warning = request_ssh_disconnect_with_timeout(
            &session,
            "PortMate target host key persistence failed",
        )
        .await;
        disconnect_jump_sessions(jump_sessions, "PortMate target host key persistence failed")
            .await;
        let cleanup_warning = cleanup_warning
            .map(|warning| format!("; {warning}"))
            .unwrap_or_default();
        return Err(format!("{error}{cleanup_warning}"));
    }

    let channel = open_ssh_terminal_channel_with_timeout(
        &session,
        profile,
        &ssh,
        connect_timeout,
        "PortMate terminal channel setup timeout",
    )
    .await;
    let channel = match channel {
        Ok(channel) => channel,
        Err(error) => {
            let cleanup_warning = if matches!(error, SshTerminalSetupError::Failed(_)) {
                request_ssh_disconnect_with_timeout(
                    &session,
                    "PortMate terminal channel setup failed",
                )
                .await
            } else {
                None
            };
            disconnect_jump_sessions(jump_sessions, "PortMate terminal channel setup failed").await;
            let cleanup_warning = cleanup_warning
                .map(|warning| format!("; {warning}"))
                .unwrap_or_default();
            return Err(format!("{error}{cleanup_warning}"));
        }
    };
    let jump_handles = jump_sessions
        .into_iter()
        .map(|session| Arc::new(tokio::sync::Mutex::new(session)))
        .collect();

    let runtime_id = Uuid::new_v4().to_string();
    let (read_half, write_half) = SshBackendChannel::from_russh(channel).split();
    let writer = Arc::new(tokio::sync::Mutex::new(write_half));
    let (tap, _) = broadcast::channel(1024);
    let closed = Arc::new(AtomicBool::new(false));
    let terminal_channel_open = Arc::new(AtomicBool::new(true));
    let (reader_finished_sender, reader_finished) = tokio::sync::oneshot::channel();

    Ok(EstablishedSshRuntime {
        runtime_id: runtime_id.clone(),
        runtime: SshRuntime {
            runtime_id: runtime_id.clone(),
            backend: SshBackendKind::Russh,
            auth_method,
            handle: Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_russh(
                session,
            ))),
            sftp: Arc::new(tokio::sync::Mutex::new(None)),
            jump_handles,
            writer,
            tap: tap.clone(),
            remote_forwards,
            remote_forward_acceptor_started: Arc::new(AtomicBool::new(false)),
            agent_forwarder_finished: None,
            transport_bridge_finished: None,
            closed: Arc::clone(&closed),
            terminal_channel_open: Arc::clone(&terminal_channel_open),
            reader_finished,
        },
        tap,
        read_half,
        auth_method,
        closed,
        terminal_channel_open,
        reader_finished: reader_finished_sender,
    })
}

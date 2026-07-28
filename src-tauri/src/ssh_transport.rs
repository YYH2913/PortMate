use super::*;

#[derive(Debug)]
pub(super) struct PortMateSshHandler {
    pub(super) profile_id: String,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) alias: Option<String>,
    pub(super) policy: portmate_core::HostKeyPolicy,
    pub(super) host_keys: HostKeyStore,
    pub(super) one_time_host_key_ids: Vec<String>,
    pub(super) observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    pub(super) host_key_error: Arc<Mutex<Option<String>>>,
    pub(super) remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
}

pub(super) fn lock_ssh_handler_state<'a, T>(
    state: &'a Mutex<T>,
    label: &str,
) -> Result<MutexGuard<'a, T>, russh::Error> {
    state.lock().map_err(|_| {
        russh::Error::IO(std::io::Error::other(format!(
            "PortMate SSH {label} lock is poisoned"
        )))
    })
}

impl client::Handler for PortMateSshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let observation = HostKeyObservation {
            host: self.host.clone(),
            port: self.port,
            alias: self.alias.clone(),
            algorithm: server_public_key.algorithm().to_string(),
            public_key_base64: server_public_key.public_key_base64(),
        };
        *lock_ssh_handler_state(&self.observed_key, "host key observation")? =
            Some(observation.clone());

        let verification = verify_ssh_host_key_observation(
            &self.profile_id,
            &self.policy,
            &self.host_keys,
            &self.one_time_host_key_ids,
            &observation,
        );
        *lock_ssh_handler_state(&self.host_key_error, "host key error")? =
            verification.as_ref().err().cloned();

        Ok(verification.is_ok())
    }

    fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let forwards = Arc::clone(&self.remote_forwards);
        let connected_address = connected_address.to_string();
        let originator_address = originator_address.to_string();
        async move {
            let Some((connected_port, originator_port)) =
                forwarded_tcpip_ports(connected_port, originator_port)
            else {
                return Ok(());
            };
            let target = {
                let forwards = lock_ssh_handler_state(&forwards, "remote forward targets")?;
                let key = remote_forward_key(&connected_address, connected_port);
                forwards
                    .get(&key)
                    .or_else(|| forwards.get(&remote_forward_port_key(connected_port)))
                    .cloned()
            };
            if let Some(target) = target {
                let Some(permit) = try_acquire_tunnel_connection(
                    &target.connection_slots,
                    target.metrics.as_ref(),
                ) else {
                    return Ok(());
                };
                reply.accept().await;
                tauri::async_runtime::spawn(async move {
                    let _permit = permit;
                    target.metrics.connection_opened();
                    let result = handle_remote_tunnel_client(
                        SshBackendChannel::from_russh(channel),
                        target.spec.clone(),
                        Some((originator_address, originator_port)),
                        Arc::clone(&target.metrics),
                    )
                    .await;
                    match result {
                        Ok(()) => target.metrics.clear_error(),
                        Err(error) => {
                            target.metrics.record_error(&error);
                            eprintln!("PortMate: remote SSH tunnel client failed: {error}");
                        }
                    }
                    target.metrics.connection_closed();
                });
            }
            Ok(())
        }
    }
}

pub(super) fn verify_ssh_host_key_observation(
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    host_keys: &HostKeyStore,
    one_time_host_key_ids: &[String],
    observation: &HostKeyObservation,
) -> Result<(), String> {
    match host_keys.evaluate(profile_id, policy, observation) {
        Ok(HostKeyEvaluation::Trusted { matched_key_id, .. })
            if trusted_host_key_allowed(policy, &matched_key_id, one_time_host_key_ids) =>
        {
            Ok(())
        }
        Ok(HostKeyEvaluation::Trusted {
            fingerprint_sha256, ..
        }) => Err(format!(
            "SSH host key requires confirmation for this connection: {fingerprint_sha256}"
        )),
        Ok(HostKeyEvaluation::Unknown { .. }) if policy.mode == HostKeyMode::TrustOnFirstUse => {
            Ok(())
        }
        Ok(other) => Err(describe_host_key_rejection(&other)),
        Err(error) => Err(format!("host key fingerprint 计算失败: {error}")),
    }
}

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

pub(super) struct SshTransportConnectParams<'a> {
    pub(super) config: Arc<client::Config>,
    pub(super) target_host: &'a str,
    pub(super) target_port: u16,
    pub(super) proxy: &'a ProxyConfig,
    pub(super) tcp_keepalive_enabled: Option<bool>,
    pub(super) timeout: Duration,
    pub(super) label: &'a str,
}

#[derive(Clone, Copy)]
pub(super) struct HostKeyPersistenceGuard<'a> {
    pub(super) profile_id: &'a str,
    pub(super) expected_profile: Option<&'a SessionProfile>,
}

pub(super) struct SshHandlerParams {
    pub(super) profile_id: String,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) alias: Option<String>,
    pub(super) policy: portmate_core::HostKeyPolicy,
    pub(super) host_keys: HostKeyStore,
    pub(super) one_time_host_key_ids: Vec<String>,
    pub(super) observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    pub(super) host_key_error: Arc<Mutex<Option<String>>>,
    pub(super) remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
}

#[derive(Debug)]
pub(super) enum SshTransportConnectError {
    Timeout,
    Transport(String),
    Handshake(String),
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

pub(super) async fn connect_ssh_transport<H>(
    params: SshTransportConnectParams<'_>,
    handler: H,
) -> Result<client::Handle<H>, SshTransportConnectError>
where
    H: client::Handler + Send + 'static,
    H::Error: std::fmt::Display,
{
    let SshTransportConnectParams {
        config,
        target_host,
        target_port,
        proxy,
        tcp_keepalive_enabled,
        timeout,
        label,
    } = params;
    tokio::time::timeout(timeout, async {
        let stream = connect_target_stream(target_host, target_port, proxy, label)
            .await
            .map_err(SshTransportConnectError::Transport)?;
        if config.nodelay {
            stream
                .set_nodelay(true)
                .map_err(|error| SshTransportConnectError::Transport(error.to_string()))?;
        }
        configure_ssh_tcp_keepalive(&stream, label, tcp_keepalive_enabled)
            .map_err(SshTransportConnectError::Transport)?;
        client::connect_stream(config, stream, handler)
            .await
            .map_err(|error| SshTransportConnectError::Handshake(error.to_string()))
    })
    .await
    .map_err(|_| SshTransportConnectError::Timeout)?
}

pub(super) fn configure_ssh_tcp_keepalive(
    stream: &TcpStream,
    label: &str,
    enabled: Option<bool>,
) -> Result<(), String> {
    let Some(enabled) = enabled else {
        return Ok(());
    };
    SockRef::from(stream)
        .set_keepalive(enabled)
        .map_err(|error| format!("{label} 设置 TCP keepalive 失败: {error}"))
}

pub(super) async fn request_ssh_disconnect_with_timeout<H: client::Handler>(
    handle: &client::Handle<H>,
    disconnect_description: &str,
) -> Option<String> {
    match tokio::time::timeout(
        SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT,
        handle.disconnect(Disconnect::ByApplication, disconnect_description, "en"),
    )
    .await
    {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("SSH disconnect request failed: {error}")),
        Err(_) => Some(format!(
            "SSH disconnect request timed out after {} ms",
            SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT.as_millis()
        )),
    }
}

pub(super) async fn request_backend_disconnect_with_timeout<H: client::Handler>(
    handle: &SshBackendSession<H>,
    disconnect_description: &str,
) -> Option<String> {
    match tokio::time::timeout(
        SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT,
        handle.disconnect(disconnect_description),
    )
    .await
    {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("SSH disconnect request failed: {error}")),
        Err(_) => Some(format!(
            "SSH disconnect request timed out after {} ms",
            SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT.as_millis()
        )),
    }
}

pub(super) async fn open_shared_ssh_exec_channel<H: client::Handler>(
    shared_handle: &Arc<tokio::sync::Mutex<SshBackendSession<H>>>,
    command: &str,
    timeout: Duration,
    label: &str,
) -> Result<SshBackendChannel, String> {
    let started = Instant::now();
    let handle = tokio::time::timeout(timeout, shared_handle.lock())
        .await
        .map_err(|_| format!("{label} handle lock 超时（{} ms）", timeout.as_millis()))?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{label} setup 超时（{} ms）", timeout.as_millis()))?;

    let setup = async { handle.open_exec(command, label).await };
    match bounded_connection_step(setup, remaining).await {
        Ok(channel) => Ok(channel),
        Err(BoundedConnectionStepError::Failed(error)) => Err(error),
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning = request_backend_disconnect_with_timeout(
                &handle,
                "PortMate auxiliary SSH exec setup timeout",
            )
            .await
            .map(|warning| format!("; {warning}"))
            .unwrap_or_default();
            Err(format!(
                "{label} setup 超时（{} ms）{cleanup_warning}",
                timeout.as_millis()
            ))
        }
    }
}

pub(super) async fn open_shared_russh_exec_channel<H: client::Handler>(
    shared_handle: &Arc<tokio::sync::Mutex<client::Handle<H>>>,
    command: &str,
    timeout: Duration,
    label: &str,
) -> Result<Channel<client::Msg>, String> {
    let started = Instant::now();
    let handle = tokio::time::timeout(timeout, shared_handle.lock())
        .await
        .map_err(|_| format!("{label} handle lock 超时（{} ms）", timeout.as_millis()))?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{label} setup 超时（{} ms）", timeout.as_millis()))?;

    let setup = async {
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|error| format!("{label} 打开 SSH channel 失败: {error}"))?;
        channel
            .exec(true, command)
            .await
            .map_err(|error| format!("{label} 启动 SSH exec 失败: {error}"))?;
        Ok::<_, String>(channel)
    };
    match bounded_connection_step(setup, remaining).await {
        Ok(channel) => Ok(channel),
        Err(BoundedConnectionStepError::Failed(error)) => Err(error),
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning = request_ssh_disconnect_with_timeout(
                &handle,
                "PortMate auxiliary SSH exec setup timeout",
            )
            .await
            .map(|warning| format!("; {warning}"))
            .unwrap_or_default();
            Err(format!(
                "{label} setup 超时（{} ms）{cleanup_warning}",
                timeout.as_millis()
            ))
        }
    }
}

pub(super) async fn remove_ssh_runtime_after_failed_open(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
) -> Result<(), String> {
    let runtime = remove_runtime_if_owned(&state.ssh, session_id, |runtime| {
        runtime.runtime_id == runtime_id
    })?;
    let Some(runtime) = runtime else {
        return Ok(());
    };
    runtime.closed.store(true, Ordering::SeqCst);
    let handle = runtime.handle.lock().await;
    let _ = handle.disconnect("PortMate connection commit failed").await;
    drop(handle);
    for jump_handle in runtime.jump_handles {
        let handle = jump_handle.lock().await;
        let _ = handle
            .disconnect(
                Disconnect::ByApplication,
                "PortMate connection commit failed",
                "en",
            )
            .await;
    }
    Ok(())
}

pub(super) fn restore_one_time_host_keys(
    state: &AppState,
    profile_id: &str,
    keys: Vec<TrustedHostKey>,
) -> Result<(), String> {
    restore_one_time_host_keys_in(&state.one_time_host_keys, profile_id, keys)
}

pub(super) fn restore_one_time_host_keys_in(
    one_time: &Arc<Mutex<HashMap<String, Vec<TrustedHostKey>>>>,
    profile_id: &str,
    keys: Vec<TrustedHostKey>,
) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    let mut one_time = one_time.lock().map_err(|error| error.to_string())?;
    let retained = one_time.entry(profile_id.to_string()).or_default();
    for key in keys {
        if !retained.iter().any(|existing| existing.id == key.id) {
            retained.push(key);
        }
    }
    Ok(())
}

pub(super) async fn open_ssh_session(
    state: &AppState,
    profile: SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<SessionSummary, String> {
    if let Some(existing) = {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        disconnect_registered_ssh_runtime(
            existing,
            "PortMate reconnect",
            "PortMate reconnect jump",
        )
        .await;
    }

    let established = establish_ssh_runtime(state, &profile, password, passphrase).await?;
    let EstablishedSshRuntime {
        runtime_id,
        runtime,
        tap,
        read_half,
        auth_method,
        closed,
        reader_finished,
    } = established;
    {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.insert(profile.id.clone(), runtime);
    }
    let (consumed_one_time_host_keys, one_time_cleanup_error) =
        match take_one_time_host_keys(state, &profile.id) {
            Ok(keys) => (keys, None),
            Err(error) => (Vec::new(), Some(error)),
        };

    let finalize_result = match state.store.lock() {
        Ok(mut store) => {
            commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
                let _ = next_store.record_auth_success(&profile.id, auth_method);
                let mut messages = vec![format!(
                    "PortMate: SSH authentication succeeded via {auth_method:?}"
                )];
                if let Some(error) = one_time_cleanup_error.as_deref() {
                    messages.push(format!(
                        "PortMate: failed to consume one-time host key trust: {error}"
                    ));
                }
                mark_session_connected_with_events(next_store, &profile, messages)
            })
        }
        Err(error) => Err(error.to_string()),
    };
    let summary = match finalize_result {
        Ok(summary) => summary,
        Err(error) => {
            let mut errors = vec![error];
            if let Err(cleanup_error) =
                remove_ssh_runtime_after_failed_open(state, &profile.id, &runtime_id).await
            {
                errors.push(format!("SSH runtime cleanup failed: {cleanup_error}"));
            }
            if let Err(restore_error) =
                restore_one_time_host_keys(state, &profile.id, consumed_one_time_host_keys)
            {
                errors.push(format!(
                    "one-time host key trust restore failed: {restore_error}"
                ));
            }
            return Err(errors.join("; "));
        }
    };

    tauri::async_runtime::spawn(read_ssh_channel(SshReadTask {
        state: state.clone(),
        profile: profile.clone(),
        runtime_id,
        tap,
        read_half,
        closed,
        reader_finished,
    }));
    Ok(summary)
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
    let (reader_finished_sender, reader_finished) = tokio::sync::oneshot::channel();

    Ok(EstablishedSshRuntime {
        runtime_id: runtime_id.clone(),
        runtime: SshRuntime {
            runtime_id: runtime_id.clone(),
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
            reader_finished,
        },
        tap,
        read_half,
        auth_method,
        closed,
        reader_finished: reader_finished_sender,
    })
}

pub(super) fn ssh_uses_libssh_gssapi_backend(ssh: &SshConnection) -> bool {
    let uses_supported_methods = ssh
        .identity_policy
        .auth_order
        .contains(&AuthMethod::GssapiWithMic)
        && ssh.identity_policy.auth_order.iter().all(|method| {
            matches!(
                method,
                AuthMethod::GssapiWithMic
                    | AuthMethod::PublicKey
                    | AuthMethod::KeyboardInteractive
                    | AuthMethod::Password
                    | AuthMethod::None
            )
        });
    uses_supported_methods && !libssh_auth_order_requires_filtered_agent(ssh)
}

fn libssh_auth_order_requires_filtered_agent(ssh: &SshConnection) -> bool {
    if !ssh
        .identity_policy
        .auth_order
        .contains(&AuthMethod::PublicKey)
        || !ssh.agent_policy.enabled
    {
        return false;
    }
    let has_agent_identity = ssh
        .identity_refs
        .iter()
        .any(|identity| identity.source == IdentitySource::Agent);
    has_agent_identity
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn establish_libssh_gssapi_runtime(
    state: &AppState,
    profile: &SessionProfile,
    ssh: &SshConnection,
    password: Option<String>,
    passphrase: Option<String>,
    connect_timeout: Duration,
    host_keys: HostKeyStore,
    one_time_host_keys: Vec<TrustedHostKey>,
    observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    host_key_error: Arc<Mutex<Option<String>>>,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    agent_socket_path: Option<PathBuf>,
    enforce_profile_snapshot: bool,
) -> Result<EstablishedSshRuntime, String> {
    let agent_socket_path = agent_socket_path
        .or_else(|| std::env::var_os("SSH_AUTH_SOCK").map(std::path::PathBuf::from));
    let host = ssh.endpoint.host.clone();
    let port = ssh.endpoint.port;
    let username = ssh.username.clone();
    let host_key_alias = ssh.host_key_policy.alias.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let (proxy_stream, jump_sessions, transport_bridge_finished, remaining_connect_timeout) =
        if !ssh.jumps.is_empty() {
            let connected_target = connect_ssh_target(
                SshConnectRequest {
                    config: Arc::new(ssh_client_config(ssh)),
                    store: Arc::clone(&state.store),
                    store_path: state.store_path.clone(),
                    profile,
                    ssh,
                    host_keys: host_keys.clone(),
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
                SshTargetTransportMode::JumpChannel,
            )
            .await?;
            let ConnectedSshTarget::JumpChannel {
                channel,
                jump_sessions,
            } = connected_target
            else {
                return Err("libssh Jump Host returned an unexpected target session".to_string());
            };
            let (stream, bridge_finished) =
                start_russh_jump_transport_bridge(channel, Arc::clone(&closed)).await?;
            (
                Some(stream),
                jump_sessions,
                Some(bridge_finished),
                connect_timeout,
            )
        } else if ssh.proxy.enabled {
            let connect_started = Instant::now();
            let stream = tokio::time::timeout(
                connect_timeout,
                connect_target_stream(&host, port, &ssh.proxy, "libssh SSH"),
            )
            .await
            .map_err(|_| {
                format!(
                    "libssh SSH 代理连接超时（{} ms）",
                    connect_timeout.as_millis()
                )
            })??;
            stream
                .set_nodelay(true)
                .map_err(|error| format!("libssh SSH 设置 TCP_NODELAY 失败: {error}"))?;
            configure_ssh_tcp_keepalive(&stream, "libssh SSH", ssh.tcp_keepalive_enabled)?;
            let stream = stream
                .into_std()
                .map_err(|error| format!("libssh SSH 接管代理 socket 失败: {error}"))?;
            (
                Some(stream),
                Vec::new(),
                None,
                connect_timeout.saturating_sub(connect_started.elapsed()),
            )
        } else {
            (None, Vec::new(), None, connect_timeout)
        };
    if remaining_connect_timeout.is_zero() {
        return Err(format!(
            "libssh SSH 连接超时（{} ms）",
            connect_timeout.as_millis()
        ));
    }
    let identity_agent = agent_socket_path
        .clone()
        .map(|path| {
            path.into_os_string()
                .into_string()
                .map_err(|_| "libssh SSH agent socket path is not valid UTF-8".to_string())
        })
        .transpose()?;
    let connected = tokio::time::timeout(
        remaining_connect_timeout,
        tokio::task::spawn_blocking(move || {
            let session = libssh_rs::Session::new()
                .map_err(|error| format!("libssh session 初始化失败: {error}"))?;
            session
                .set_option(libssh_rs::SshOption::ProcessConfig(false))
                .map_err(|error| format!("libssh 禁用系统 ssh_config 失败: {error}"))?;
            #[cfg(test)]
            if std::env::var_os("PORTMATE_COMPAT_LIBSSH_TRACE").is_some() {
                session
                    .set_option(libssh_rs::SshOption::LogLevel(
                        libssh_rs::LogLevel::Protocol,
                    ))
                    .map_err(|error| format!("libssh 设置测试日志级别失败: {error}"))?;
            }
            session
                .set_option(libssh_rs::SshOption::Hostname(host.clone()))
                .map_err(|error| format!("libssh 设置主机失败: {error}"))?;
            session
                .set_option(libssh_rs::SshOption::Port(port))
                .map_err(|error| format!("libssh 设置端口失败: {error}"))?;
            session
                .set_option(libssh_rs::SshOption::User(Some(username)))
                .map_err(|error| format!("libssh 设置用户名失败: {error}"))?;
            if let Some(proxy_stream) = proxy_stream {
                session
                    .set_owned_tcp_stream(proxy_stream)
                    .map_err(|error| format!("libssh 设置代理 socket 失败: {error}"))?;
            }
            if let Some(identity_agent) = identity_agent {
                session
                    .set_option(libssh_rs::SshOption::IdentityAgent(Some(identity_agent)))
                    .map_err(|error| format!("libssh 设置 SSH agent socket 失败: {error}"))?;
            }
            session
                .set_option(libssh_rs::SshOption::Timeout(remaining_connect_timeout))
                .map_err(|error| format!("libssh 设置连接超时失败: {error}"))?;
            session
                .connect()
                .map_err(|error| format!("libssh 连接 {host}:{port} 失败: {error}"))?;
            let key = session
                .get_server_public_key()
                .map_err(|error| format!("libssh 读取服务端 host key 失败: {error}"))?;
            let observation = HostKeyObservation {
                host,
                port,
                alias: host_key_alias,
                algorithm: key
                    .key_type_name()
                    .map_err(|error| format!("libssh 读取 host key 算法失败: {error}"))?,
                public_key_base64: key
                    .export_public_key_base64()
                    .map_err(|error| format!("libssh 导出 host key 失败: {error}"))?,
            };
            Ok::<_, String>((session, observation))
        }),
    )
    .await
    .map_err(|_| format!("libssh 连接超时（{} ms）", connect_timeout.as_millis()))?
    .map_err(|error| format!("libssh 连接 worker 失败: {error}"))??;
    let (session, observation) = connected;

    *observed_key.lock().map_err(|error| error.to_string())? = Some(observation.clone());
    let one_time_host_key_ids = one_time_host_keys
        .iter()
        .map(|key| key.id.clone())
        .collect::<Vec<_>>();
    let verification = verify_ssh_host_key_observation(
        &profile.id,
        &ssh.host_key_policy,
        &host_keys,
        &one_time_host_key_ids,
        &observation,
    );
    *host_key_error.lock().map_err(|error| error.to_string())? =
        verification.as_ref().err().cloned();
    if let Err(error) = verification {
        let session = session.clone();
        let _ = tokio::task::spawn_blocking(move || session.disconnect()).await;
        return Err(error);
    }

    let saved_password = if password
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        match read_optional_secret_ref(ssh.password_secret_ref.as_deref(), "SSH password") {
            Ok(password) => password,
            Err(error) => {
                let cleanup = session.clone();
                let _ = tokio::task::spawn_blocking(move || cleanup.disconnect()).await;
                return Err(error);
            }
        }
    } else {
        None
    };
    let effective_password = password
        .filter(|value| !value.is_empty())
        .or(saved_password);
    let saved_passphrase = if passphrase
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        match read_optional_secret_ref(
            ssh.passphrase_secret_ref.as_deref(),
            "SSH private-key passphrase",
        ) {
            Ok(passphrase) => passphrase,
            Err(error) => {
                let cleanup = session.clone();
                let _ = tokio::task::spawn_blocking(move || cleanup.disconnect()).await;
                return Err(error);
            }
        }
    } else {
        None
    };
    let effective_passphrase = passphrase
        .filter(|value| !value.is_empty())
        .or(saved_passphrase);
    let auth_order = ordered_auth_methods(ssh);
    let identity_refs = ssh.identity_refs.clone();
    let offer_unfiltered_agent = ssh.agent_policy.enabled
        && !ssh.identity_policy.identities_only
        && !ssh
            .identity_refs
            .iter()
            .any(|identity| identity.source == IdentitySource::Agent);
    let offer_agent_before = offer_unfiltered_agent
        && ssh.agent_policy.offer_mode == portmate_core::AgentOfferMode::BeforeProfileKeys;
    let offer_agent_after = offer_unfiltered_agent
        && ssh.agent_policy.offer_mode == portmate_core::AgentOfferMode::AfterProfileKeys;
    let auth_session = session.clone();
    let auth = match tokio::time::timeout(
        connect_timeout,
        tokio::task::spawn_blocking(move || {
            authenticate_libssh_with_order(
                &auth_session,
                &auth_order,
                effective_password.as_deref(),
                &identity_refs,
                effective_passphrase.as_deref(),
                offer_agent_before,
                offer_agent_after,
            )
        }),
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(format!("libssh SSH 认证 worker 失败: {error}")),
        Err(_) => Err(format!(
            "libssh SSH 认证超时（{} ms）",
            connect_timeout.as_millis()
        )),
    };
    let auth_method = match auth {
        Ok(method) => method,
        Err(error) => {
            let cleanup = session.clone();
            let _ = tokio::task::spawn_blocking(move || cleanup.disconnect()).await;
            return Err(error);
        }
    };

    if let Err(error) = persist_observed_host_key(
        &state.store,
        &state.store_path,
        HostKeyPersistenceGuard {
            profile_id: &profile.id,
            expected_profile: enforce_profile_snapshot.then_some(profile),
        },
        &observed_key,
        &one_time_host_keys,
    ) {
        let cleanup = session.clone();
        let _ = tokio::task::spawn_blocking(move || cleanup.disconnect()).await;
        return Err(error);
    }

    let terminal_session = session.clone();
    let agent_forward_socket = if ssh.agent_policy.forwarding {
        Some(
            agent_socket_path
                .clone()
                .ok_or_else(|| "SSH agent forwarding requires an SSH agent socket".to_string())?,
        )
    } else {
        None
    };
    let request_agent_forward = agent_forward_socket.is_some();
    let term = profile.terminal.term.clone();
    let cols = u32::from(profile.terminal.cols);
    let rows = u32::from(profile.terminal.rows);
    let attach_tmux = matches!(profile.connection, ConnectionConfig::Tmux(_));
    let channel = tokio::time::timeout(
        connect_timeout,
        tokio::task::spawn_blocking(move || {
            let channel = terminal_session
                .new_channel()
                .map_err(|error| format!("libssh 创建终端 channel 失败: {error}"))?;
            channel
                .open_session()
                .map_err(|error| format!("libssh 打开终端 channel 失败: {error}"))?;
            channel
                .request_pty(&term, cols, rows)
                .map_err(|error| format!("libssh 请求 PTY 失败: {error}"))?;
            for (name, value) in [
                ("COLORTERM", "truecolor"),
                ("CLICOLOR", "1"),
                ("CLICOLOR_FORCE", "1"),
                ("FORCE_COLOR", "1"),
                ("TERM_PROGRAM", "PortMate"),
            ] {
                let _ = channel.request_env(name, value);
            }
            if request_agent_forward {
                terminal_session.enable_accept_agent_forward(true);
                channel
                    .request_auth_agent()
                    .map_err(|error| format!("libssh 请求 agent forwarding 失败: {error}"))?;
            }
            channel
                .request_shell()
                .map_err(|error| format!("libssh 请求 shell 失败: {error}"))?;
            if attach_tmux {
                let mut stdin = channel.stdin();
                stdin
                    .write_all(b"tmux new-session -A -s portmate\r")
                    .map_err(|error| format!("libssh Tmux attach 写入失败: {error}"))?;
                stdin
                    .flush()
                    .map_err(|error| format!("libssh Tmux attach 刷新失败: {error}"))?;
            }
            Ok::<_, String>(channel)
        }),
    )
    .await
    .map_err(|_| {
        format!(
            "libssh 终端 setup 超时（{} ms）",
            connect_timeout.as_millis()
        )
    })?
    .map_err(|error| format!("libssh 终端 worker 失败: {error}"))??;

    let runtime_id = Uuid::new_v4().to_string();
    let (read_half, write_half) = SshBackendChannel::from_libssh(channel).split();
    let (tap, _) = broadcast::channel(1024);
    let (reader_finished_sender, reader_finished) = tokio::sync::oneshot::channel();
    let agent_forwarder_finished = agent_forward_socket.map(|socket_path| {
        start_libssh_agent_forwarder(session.clone(), socket_path, Arc::clone(&closed))
    });

    Ok(EstablishedSshRuntime {
        runtime_id: runtime_id.clone(),
        runtime: SshRuntime {
            runtime_id,
            handle: Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_libssh(
                session,
            ))),
            sftp: Arc::new(tokio::sync::Mutex::new(None)),
            jump_handles: jump_sessions
                .into_iter()
                .map(|session| Arc::new(tokio::sync::Mutex::new(session)))
                .collect(),
            writer: Arc::new(tokio::sync::Mutex::new(write_half)),
            tap: tap.clone(),
            remote_forwards,
            remote_forward_acceptor_started: Arc::new(AtomicBool::new(false)),
            agent_forwarder_finished,
            transport_bridge_finished,
            closed: Arc::clone(&closed),
            reader_finished,
        },
        tap,
        read_half,
        auth_method,
        closed,
        reader_finished: reader_finished_sender,
    })
}

pub(super) fn authenticate_libssh_with_order(
    session: &libssh_rs::Session,
    auth_order: &[AuthMethod],
    password: Option<&str>,
    identity_refs: &[IdentityRef],
    passphrase: Option<&str>,
    offer_agent_before: bool,
    offer_agent_after: bool,
) -> Result<AuthMethod, String> {
    let none = session
        .userauth_none(None)
        .map_err(|error| format!("libssh authentication capability probe failed: {error}"))?;
    if none == libssh_rs::AuthStatus::Success {
        return auth_order
            .contains(&AuthMethod::None)
            .then_some(AuthMethod::None)
            .ok_or_else(|| {
                "SSH server accepted none authentication, but the profile does not allow it"
                    .to_string()
            });
    }

    let mut methods = session
        .userauth_list(None)
        .map_err(|error| format!("libssh auth method query failed: {error}"))?;
    let mut attempted = Vec::new();
    let mut failures = Vec::new();

    for method in auth_order {
        let status = match method {
            AuthMethod::GssapiWithMic => {
                attempted.push("gssapi-with-mic");
                if !methods.contains(libssh_rs::AuthMethods::GSSAPI_MIC) {
                    failures.push("server did not advertise gssapi-with-mic".to_string());
                    continue;
                }
                session
                    .userauth_gssapi()
                    .map_err(|error| format!("libssh GSSAPI authentication failed: {error}"))?
            }
            AuthMethod::KeyboardInteractive => {
                let Some(password) = password else {
                    continue;
                };
                attempted.push("keyboard-interactive");
                if !methods.contains(libssh_rs::AuthMethods::INTERACTIVE) {
                    failures.push("server did not advertise keyboard-interactive".to_string());
                    continue;
                }
                authenticate_libssh_keyboard_interactive(session, password)?
            }
            AuthMethod::Password => {
                let Some(password) = password else {
                    continue;
                };
                attempted.push("password");
                if !methods.contains(libssh_rs::AuthMethods::PASSWORD) {
                    failures.push("server did not advertise password authentication".to_string());
                    continue;
                }
                session
                    .userauth_password(None, Some(password))
                    .map_err(|error| format!("libssh password authentication failed: {error}"))?
            }
            AuthMethod::None => {
                attempted.push("none");
                failures.push("server rejected none authentication".to_string());
                continue;
            }
            AuthMethod::PublicKey => {
                attempted.push("publickey");
                if !methods.contains(libssh_rs::AuthMethods::PUBLIC_KEY) {
                    failures.push("server did not advertise public-key authentication".to_string());
                    continue;
                }
                if offer_agent_before
                    && authenticate_libssh_agent(session, "before profile keys", &mut failures)?
                {
                    return Ok(AuthMethod::PublicKey);
                }
                let identities = identity_refs.iter().filter(|identity| {
                    matches!(
                        identity.source,
                        IdentitySource::SystemFile | IdentitySource::ProfileVault
                    )
                });
                let mut identity_attempted = false;
                for identity in identities {
                    identity_attempted = true;
                    let key = match load_libssh_private_key(identity, passphrase) {
                        Ok(Some(key)) => key,
                        Ok(None) => continue,
                        Err(error) => {
                            failures.push(format!("{}: {error}", identity.label));
                            continue;
                        }
                    };
                    let status = session.userauth_publickey(None, &key).map_err(|error| {
                        format!(
                            "libssh public-key authentication failed for {}: {error}",
                            identity.label
                        )
                    })?;
                    match status {
                        libssh_rs::AuthStatus::Success => return Ok(AuthMethod::PublicKey),
                        libssh_rs::AuthStatus::Denied => {
                            failures.push(format!("{}: public key was denied", identity.label));
                        }
                        libssh_rs::AuthStatus::Partial => failures.push(format!(
                            "{}: public key was only partially accepted",
                            identity.label
                        )),
                        libssh_rs::AuthStatus::Info | libssh_rs::AuthStatus::Again => {
                            return Err(format!(
                                "libssh public-key authentication for {} returned {status:?}",
                                identity.label
                            ));
                        }
                    }
                }
                if !identity_attempted && !offer_agent_before && !offer_agent_after {
                    failures
                        .push("profile has no usable explicit private-key identity".to_string());
                }
                if offer_agent_after
                    && authenticate_libssh_agent(session, "after profile keys", &mut failures)?
                {
                    return Ok(AuthMethod::PublicKey);
                }
                methods = session
                    .userauth_list(None)
                    .map_err(|error| format!("libssh auth method refresh failed: {error}"))?;
                continue;
            }
        };

        match status {
            libssh_rs::AuthStatus::Success => return Ok(*method),
            libssh_rs::AuthStatus::Denied => {
                failures.push(if *method == AuthMethod::GssapiWithMic {
                    "GSSAPI authentication was denied".to_string()
                } else {
                    format!("{method:?} was denied")
                });
            }
            libssh_rs::AuthStatus::Partial => {
                failures.push(if *method == AuthMethod::GssapiWithMic {
                    "GSSAPI authentication was only partially accepted".to_string()
                } else {
                    format!("{method:?} was only partially accepted")
                });
            }
            libssh_rs::AuthStatus::Info => {
                return Err(format!(
                    "libssh {method:?} authentication returned an unexpected prompt state"
                ));
            }
            libssh_rs::AuthStatus::Again => {
                return Err(format!(
                    "libssh {method:?} authentication unexpectedly requested a retry"
                ));
            }
        }
        methods = session
            .userauth_list(None)
            .map_err(|error| format!("libssh auth method refresh failed: {error}"))?;
    }

    if auth_order == [AuthMethod::GssapiWithMic]
        && failures
            .iter()
            .any(|failure| failure == "server did not advertise gssapi-with-mic")
    {
        return Err("SSH server did not advertise gssapi-with-mic".to_string());
    }
    let attempted = if attempted.is_empty() {
        "none".to_string()
    } else {
        attempted.join(", ")
    };
    let details = if failures.is_empty() {
        String::new()
    } else {
        format!("; {}", failures.join(" | "))
    };
    Err(format!(
        "libssh SSH authentication failed; attempted: {attempted}{details}"
    ))
}

fn authenticate_libssh_agent(
    session: &libssh_rs::Session,
    position: &str,
    failures: &mut Vec<String>,
) -> Result<bool, String> {
    let status = match session.userauth_agent(None) {
        Ok(status) => status,
        Err(error) => {
            failures.push(format!("SSH agent ({position}) failed: {error}"));
            return Ok(false);
        }
    };
    match status {
        libssh_rs::AuthStatus::Success => Ok(true),
        libssh_rs::AuthStatus::Denied => {
            failures.push(format!("SSH agent ({position}) was denied"));
            Ok(false)
        }
        libssh_rs::AuthStatus::Partial => {
            failures.push(format!(
                "SSH agent ({position}) was only partially accepted"
            ));
            Ok(false)
        }
        status @ (libssh_rs::AuthStatus::Info | libssh_rs::AuthStatus::Again) => Err(format!(
            "libssh SSH agent authentication ({position}) returned {status:?}"
        )),
    }
}

fn load_libssh_private_key(
    identity: &IdentityRef,
    passphrase: Option<&str>,
) -> Result<Option<libssh_rs::SshKey>, String> {
    load_libssh_private_key_with(identity, passphrase, read_secret_from_store)
}

pub(super) fn load_libssh_private_key_with<ReadSecret>(
    identity: &IdentityRef,
    passphrase: Option<&str>,
    read_secret: ReadSecret,
) -> Result<Option<libssh_rs::SshKey>, String>
where
    ReadSecret: FnOnce(&str) -> Result<String, String>,
{
    let private_key = match identity.source {
        IdentitySource::SystemFile => {
            let Some(path) = identity
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            else {
                return Ok(None);
            };
            let path = expand_identity_path(path);
            fs::read_to_string(&path)
                .map_err(|error| format!("system-file {}: {error}", path.display()))?
        }
        IdentitySource::ProfileVault => {
            let Some(secret_ref) = identity
                .secret_ref
                .as_deref()
                .map(str::trim)
                .filter(|secret_ref| !secret_ref.is_empty())
            else {
                return Err("profile-vault identity 缺少 secretRef".to_string());
            };
            read_secret(secret_ref)
                .map_err(|error| format!("profile-vault {secret_ref}: {error}"))?
        }
        IdentitySource::Agent | IdentitySource::PublicKeyOnly => return Ok(None),
    };
    libssh_rs::SshKey::from_privkey_base64(&private_key, passphrase)
        .map(Some)
        .map_err(|error| format!("private key 解析失败: {error}"))
}

fn authenticate_libssh_keyboard_interactive(
    session: &libssh_rs::Session,
    password: &str,
) -> Result<libssh_rs::AuthStatus, String> {
    for _ in 0..8 {
        let status = session
            .userauth_keyboard_interactive(None, None)
            .map_err(|error| {
                format!("libssh keyboard-interactive authentication failed: {error}")
            })?;
        if status != libssh_rs::AuthStatus::Info {
            return Ok(status);
        }
        let info = session
            .userauth_keyboard_interactive_info()
            .map_err(|error| format!("libssh keyboard-interactive prompt failed: {error}"))?;
        let answers = info
            .prompts
            .iter()
            .map(|prompt| {
                if prompt.echo {
                    String::new()
                } else {
                    password.to_string()
                }
            })
            .collect::<Vec<_>>();
        session
            .userauth_keyboard_interactive_set_answers(&answers)
            .map_err(|error| format!("libssh keyboard-interactive response failed: {error}"))?;
    }
    Err("libssh keyboard-interactive authentication exceeded 8 rounds".to_string())
}

pub(super) fn ssh_client_config(ssh: &SshConnection) -> client::Config {
    client::Config {
        keepalive_interval: ssh
            .keepalive_enabled
            .then(|| Duration::from_secs(ssh.keepalive_interval_seconds)),
        keepalive_max: ssh.keepalive_max_missed as usize,
        nodelay: true,
        ..Default::default()
    }
}

pub(super) async fn connect_ssh_target(
    request: SshConnectRequest<'_>,
    connect_timeout: Duration,
    agent_socket_path: Option<&Path>,
    mode: SshTargetTransportMode,
) -> Result<ConnectedSshTarget, String> {
    let SshConnectRequest {
        config,
        store,
        store_path,
        profile,
        ssh,
        host_keys,
        one_time_host_keys,
        observed_key,
        host_key_error,
        remote_forwards,
        password,
        passphrase,
        enforce_profile_snapshot,
    } = request;
    let one_time_host_key_ids = one_time_host_keys
        .iter()
        .map(|key| key.id.clone())
        .collect::<Vec<_>>();

    let target_host = ssh.endpoint.host.trim().to_string();
    if target_host.is_empty() {
        return Err("SSH 主机不能为空".to_string());
    }
    if ssh.endpoint.port == 0 {
        return Err("SSH 端口必须在 1-65535 之间".to_string());
    }

    let target_handler = ssh_handler_for_endpoint(SshHandlerParams {
        profile_id: profile.id.clone(),
        host: target_host.clone(),
        port: ssh.endpoint.port,
        alias: ssh
            .host_key_policy
            .alias
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| Some(profile.id.clone())),
        policy: ssh.host_key_policy.clone(),
        host_keys: host_keys.clone(),
        one_time_host_key_ids: one_time_host_key_ids.clone(),
        observed_key: Arc::clone(&observed_key),
        host_key_error: Arc::clone(&host_key_error),
        remote_forwards: Arc::clone(&remote_forwards),
    });

    if ssh.jumps.is_empty() {
        if mode == SshTargetTransportMode::JumpChannel {
            return Err("SSH Jump Host transport requires at least one jump".to_string());
        }
        let session = connect_ssh_transport(
            SshTransportConnectParams {
                config,
                target_host: &target_host,
                target_port: ssh.endpoint.port,
                proxy: &ssh.proxy,
                tcp_keepalive_enabled: ssh.tcp_keepalive_enabled,
                timeout: connect_timeout,
                label: "SSH",
            },
            target_handler,
        )
        .await;
        let session = match session {
            Ok(session) => session,
            Err(SshTransportConnectError::Timeout) => {
                return Err(format!("SSH 连接超时: {target_host}:{}", ssh.endpoint.port));
            }
            Err(SshTransportConnectError::Transport(error)) => return Err(error),
            Err(SshTransportConnectError::Handshake(error)) => {
                return Err(host_key_error
                    .lock()
                    .ok()
                    .and_then(|reason| reason.clone())
                    .unwrap_or_else(|| format!("SSH 握手失败: {error}")));
            }
        };
        return Ok(ConnectedSshTarget::Russh {
            session,
            jump_sessions: Vec::new(),
        });
    }

    let mut jump_sessions: Vec<client::Handle<PortMateSshHandler>> = Vec::new();
    for (index, jump) in ssh.jumps.iter().enumerate() {
        let (jump_host, jump_port, jump_username) = jump_endpoint_details(jump, index)?;
        let jump_policy = jump_host_key_policy(ssh, jump);
        let observed_jump_key = Arc::new(Mutex::new(None));
        let jump_key_error = Arc::new(Mutex::new(None));
        let jump_ssh = jump_ssh_connection(ssh, jump, jump_policy.clone());
        let jump_handler = ssh_handler_for_endpoint(SshHandlerParams {
            profile_id: profile.id.clone(),
            host: jump_host.clone(),
            port: jump_port,
            alias: jump_policy.alias.clone(),
            policy: jump_ssh.host_key_policy.clone(),
            host_keys: host_keys.clone(),
            one_time_host_key_ids: one_time_host_key_ids.clone(),
            observed_key: Arc::clone(&observed_jump_key),
            host_key_error: Arc::clone(&jump_key_error),
            remote_forwards: Arc::new(Mutex::new(HashMap::new())),
        });
        let mut jump_session = if let Some(previous_jump) = jump_sessions.last_mut() {
            let jump_channel = match open_direct_tcpip_with_timeout(
                previous_jump,
                jump_host.clone(),
                jump_port,
                "127.0.0.1".to_string(),
                0,
                connect_timeout,
                "PortMate jump chain channel timeout",
            )
            .await
            {
                Ok(channel) => channel,
                Err(DirectTcpipOpenError::Failed(error)) => {
                    disconnect_jump_sessions(jump_sessions, "PortMate jump chain channel failed")
                        .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳打开 direct-tcpip 到 {jump_host}:{jump_port} 失败: {error}",
                        index + 1
                    ));
                }
                Err(DirectTcpipOpenError::TimedOut {
                    timeout_ms,
                    cleanup_warning,
                }) => {
                    disconnect_jump_sessions(jump_sessions, "PortMate jump chain channel timeout")
                        .await;
                    let cleanup_warning = cleanup_warning
                        .map(|warning| format!("; {warning}"))
                        .unwrap_or_default();
                    return Err(format!(
                        "Jump Host 第 {} 跳打开 direct-tcpip 到 {jump_host}:{jump_port} 超时（{timeout_ms} ms）{cleanup_warning}",
                        index + 1
                    ));
                }
            };
            match tokio::time::timeout(
                connect_timeout,
                client::connect_stream(config.clone(), jump_channel.into_stream(), jump_handler),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Err(_) => {
                    disconnect_jump_sessions(jump_sessions, "PortMate jump chain connect timeout")
                        .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳连接超时: {jump_host}:{jump_port}",
                        index + 1
                    ));
                }
                Ok(Err(error)) => {
                    let reason = jump_key_error
                        .lock()
                        .ok()
                        .and_then(|reason| reason.clone())
                        .unwrap_or_else(|| error.to_string());
                    disconnect_jump_sessions(jump_sessions, "PortMate jump chain handshake failed")
                        .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳 SSH 握手失败 {jump_host}:{jump_port}: {reason}",
                        index + 1
                    ));
                }
            }
        } else {
            let jump_transport_label = format!("Jump Host 第 {} 跳", index + 1);
            match connect_ssh_transport(
                SshTransportConnectParams {
                    config: config.clone(),
                    target_host: &jump_host,
                    target_port: jump_port,
                    proxy: &ssh.proxy,
                    tcp_keepalive_enabled: ssh.tcp_keepalive_enabled,
                    timeout: connect_timeout,
                    label: &jump_transport_label,
                },
                jump_handler,
            )
            .await
            {
                Ok(session) => session,
                Err(SshTransportConnectError::Timeout) => {
                    return Err(format!(
                        "Jump Host 第 {} 跳连接超时: {jump_host}:{jump_port}",
                        index + 1
                    ));
                }
                Err(SshTransportConnectError::Transport(error)) => return Err(error),
                Err(SshTransportConnectError::Handshake(error)) => {
                    let reason = jump_key_error
                        .lock()
                        .ok()
                        .and_then(|reason| reason.clone())
                        .unwrap_or(error);
                    return Err(format!(
                        "Jump Host 第 {} 跳 SSH 握手失败 {jump_host}:{jump_port}: {reason}",
                        index + 1
                    ));
                }
            }
        };

        if let Err(error) = authenticate_ssh_with_timeout(
            &mut jump_session,
            SshAuthenticationRequest {
                ssh: jump_ssh,
                username: jump_username,
                password: jump_runtime_credential(password, jump.password_secret_ref.as_deref()),
                passphrase: jump_runtime_credential(
                    passphrase,
                    jump.passphrase_secret_ref.as_deref(),
                ),
                agent_socket_path: agent_socket_path.map(Path::to_path_buf),
                timeout: connect_timeout,
                disconnect_description: "PortMate jump authentication timeout",
            },
        )
        .await
        {
            disconnect_jump_sessions(jump_sessions, "PortMate jump authentication failed").await;
            let _ = request_ssh_disconnect_with_timeout(
                &jump_session,
                "PortMate jump authentication failed",
            )
            .await;
            return Err(format!(
                "Jump Host 第 {} 跳认证失败 {jump_host}:{jump_port}: {error}",
                index + 1
            ));
        }
        if let Err(error) = persist_observed_host_key_with_policy(
            &store,
            &store_path,
            HostKeyPersistenceGuard {
                profile_id: &profile.id,
                expected_profile: enforce_profile_snapshot.then_some(profile),
            },
            &jump_policy,
            &observed_jump_key,
            &one_time_host_keys,
            &format!("Jump Host #{}", index + 1),
        ) {
            disconnect_jump_sessions(jump_sessions, "PortMate jump host key rejected").await;
            let _ = request_ssh_disconnect_with_timeout(
                &jump_session,
                "PortMate jump host key rejected",
            )
            .await;
            return Err(format!(
                "Jump Host 第 {} 跳 host key 处理失败 {jump_host}:{jump_port}: {error}",
                index + 1
            ));
        }
        jump_sessions.push(jump_session);
    }

    let jump_channel = match open_direct_tcpip_with_timeout(
        jump_sessions
            .last()
            .expect("non-empty jumps should create jump sessions"),
        target_host.clone(),
        ssh.endpoint.port,
        "127.0.0.1".to_string(),
        0,
        connect_timeout,
        "PortMate jump target channel timeout",
    )
    .await
    {
        Ok(channel) => channel,
        Err(DirectTcpipOpenError::Failed(error)) => {
            disconnect_jump_sessions(jump_sessions, "PortMate jump target channel failed").await;
            return Err(format!(
                "Jump Host 打开 direct-tcpip 到 {target_host}:{} 失败: {error}",
                ssh.endpoint.port
            ));
        }
        Err(DirectTcpipOpenError::TimedOut {
            timeout_ms,
            cleanup_warning,
        }) => {
            disconnect_jump_sessions(jump_sessions, "PortMate jump target channel timeout").await;
            let cleanup_warning = cleanup_warning
                .map(|warning| format!("; {warning}"))
                .unwrap_or_default();
            return Err(format!(
                "Jump Host 打开 direct-tcpip 到 {target_host}:{} 超时（{timeout_ms} ms）{cleanup_warning}",
                ssh.endpoint.port
            ));
        }
    };
    if mode == SshTargetTransportMode::JumpChannel {
        return Ok(ConnectedSshTarget::JumpChannel {
            channel: jump_channel,
            jump_sessions,
        });
    }
    let target_session = match tokio::time::timeout(
        connect_timeout,
        client::connect_stream(config, jump_channel.into_stream(), target_handler),
    )
    .await
    {
        Ok(Ok(session)) => session,
        Err(_) => {
            disconnect_jump_sessions(jump_sessions, "PortMate jump target connect timeout").await;
            return Err(format!(
                "SSH 经 Jump Host 连接超时: {target_host}:{}",
                ssh.endpoint.port
            ));
        }
        Ok(Err(error)) => {
            disconnect_jump_sessions(jump_sessions, "PortMate jump target handshake failed").await;
            return Err(host_key_error
                .lock()
                .ok()
                .and_then(|reason| reason.clone())
                .unwrap_or_else(|| format!("SSH 经 Jump Host 握手失败: {error}")));
        }
    };

    Ok(ConnectedSshTarget::Russh {
        session: target_session,
        jump_sessions,
    })
}

pub(super) fn jump_endpoint_details(
    jump: &portmate_core::JumpHop,
    index: usize,
) -> Result<(String, u16, String), String> {
    let label = format!("Jump Host 第 {} 跳", index + 1);
    let host = jump.host.trim().to_string();
    if host.is_empty() {
        return Err(format!("{label} 主机不能为空"));
    }
    if jump.port == 0 {
        return Err(format!("{label} 端口必须在 1-65535 之间"));
    }
    let username = jump.username.trim().to_string();
    if username.is_empty() {
        return Err(format!("{label} 用户名不能为空"));
    }
    Ok((host, jump.port, username))
}

pub(super) async fn disconnect_jump_sessions(
    jump_sessions: Vec<client::Handle<PortMateSshHandler>>,
    reason: &str,
) {
    let disconnect_all = async {
        for session in jump_sessions {
            let _ = session
                .disconnect(Disconnect::ByApplication, reason, "en")
                .await;
        }
    };
    if tokio::time::timeout(SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT, disconnect_all)
        .await
        .is_err()
    {
        eprintln!(
            "PortMate: Jump Host cleanup did not finish within {} ms: {reason}",
            SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT.as_millis()
        );
    }
}

pub(super) fn ssh_handler_for_endpoint(params: SshHandlerParams) -> PortMateSshHandler {
    PortMateSshHandler {
        profile_id: params.profile_id,
        host: params.host,
        port: params.port,
        alias: params.alias,
        policy: params.policy,
        host_keys: params.host_keys,
        one_time_host_key_ids: params.one_time_host_key_ids,
        observed_key: params.observed_key,
        host_key_error: params.host_key_error,
        remote_forwards: params.remote_forwards,
    }
}

pub(super) fn trusted_host_key_allowed(
    policy: &portmate_core::HostKeyPolicy,
    matched_key_id: &str,
    one_time_host_key_ids: &[String],
) -> bool {
    policy.mode != HostKeyMode::AskEveryTime
        || one_time_host_key_ids
            .iter()
            .any(|key_id| key_id == matched_key_id)
}

pub(super) fn jump_host_key_policy(
    ssh: &SshConnection,
    jump: &portmate_core::JumpHop,
) -> portmate_core::HostKeyPolicy {
    let default_alias = format!("jump:{}:{}", jump.host.trim(), jump.port);
    let mut policy = if let Some(custom) = jump.host_key_policy.clone() {
        custom
    } else {
        let mut inherited = ssh.host_key_policy.clone();
        inherited.alias = Some(default_alias.clone());
        inherited.trust_scope = HostKeyScope::Profile;
        inherited
    };
    policy.alias = policy
        .alias
        .as_deref()
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
        .map(str::to_string)
        .or(Some(default_alias));
    policy
}

pub(super) fn jump_ssh_connection(
    ssh: &SshConnection,
    jump: &portmate_core::JumpHop,
    host_key_policy: portmate_core::HostKeyPolicy,
) -> SshConnection {
    let mut identity_refs = ssh.identity_refs.clone();
    if let Some(identity_ref) = jump
        .identity_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        identity_refs.retain(|identity| identity.id == identity_ref);
    }
    SshConnection {
        endpoint: portmate_core::HostEndpoint {
            host: jump.host.trim().to_string(),
            port: jump.port,
        },
        username: jump.username.trim().to_string(),
        reconnect: ssh.reconnect,
        reconnect_delay_ms: ssh.reconnect_delay_ms,
        keepalive_enabled: ssh.keepalive_enabled,
        keepalive_interval_seconds: ssh.keepalive_interval_seconds,
        keepalive_max_missed: ssh.keepalive_max_missed,
        tcp_keepalive_enabled: ssh.tcp_keepalive_enabled,
        proxy: ssh.proxy.clone(),
        password_secret_ref: jump
            .password_secret_ref
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| ssh.password_secret_ref.clone()),
        passphrase_secret_ref: jump
            .passphrase_secret_ref
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| ssh.passphrase_secret_ref.clone()),
        host_key_policy,
        trusted_host_keys: Vec::new(),
        identity_policy: ssh.identity_policy.clone(),
        identity_refs,
        agent_policy: ssh.agent_policy.clone(),
        jumps: Vec::new(),
        tunnels: Vec::new(),
    }
}

pub(super) fn jump_runtime_credential(
    inherited: Option<&str>,
    jump_secret_ref: Option<&str>,
) -> Option<String> {
    if jump_secret_ref.is_some_and(|value| !value.trim().is_empty()) {
        None
    } else {
        inherited
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SshAuthenticationError {
    TimedOut {
        timeout_ms: u128,
        cleanup_warning: Option<String>,
    },
    Failed(String),
}

impl std::fmt::Display for SshAuthenticationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut {
                timeout_ms,
                cleanup_warning,
            } => {
                write!(formatter, "SSH 认证超时（{timeout_ms} ms）")?;
                if let Some(warning) = cleanup_warning {
                    write!(formatter, "; {warning}")?;
                }
                Ok(())
            }
            Self::Failed(error) => formatter.write_str(error),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SshTerminalSetupError {
    TimedOut {
        timeout_ms: u128,
        cleanup_warning: Option<String>,
    },
    Failed(String),
}

impl std::fmt::Display for SshTerminalSetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut {
                timeout_ms,
                cleanup_warning,
            } => {
                write!(formatter, "SSH 终端 channel setup 超时（{timeout_ms} ms）")?;
                if let Some(warning) = cleanup_warning {
                    write!(formatter, "; {warning}")?;
                }
                Ok(())
            }
            Self::Failed(error) => formatter.write_str(error),
        }
    }
}

pub(super) async fn open_ssh_terminal_channel_with_timeout<H: client::Handler>(
    session: &client::Handle<H>,
    profile: &SessionProfile,
    ssh: &SshConnection,
    timeout: Duration,
    disconnect_description: &str,
) -> Result<Channel<client::Msg>, SshTerminalSetupError> {
    let setup = async {
        let channel = session
            .channel_open_session()
            .await
            .map_err(|error| format!("SSH 打开 session channel 失败: {error}"))?;
        channel
            .request_pty(
                true,
                &profile.terminal.term,
                u32::from(profile.terminal.cols),
                u32::from(profile.terminal.rows),
                0,
                0,
                &[],
            )
            .await
            .map_err(|error| format!("SSH 请求 PTY 失败: {error}"))?;
        apply_ssh_terminal_color_env(&channel).await;
        if ssh.agent_policy.forwarding {
            channel
                .agent_forward(false)
                .await
                .map_err(|error| format!("SSH 请求 agent forwarding 失败: {error}"))?;
        }
        channel
            .request_shell(true)
            .await
            .map_err(|error| format!("SSH 请求 shell 失败: {error}"))?;
        if matches!(profile.connection, ConnectionConfig::Tmux(_)) {
            channel
                .data(&b"tmux new-session -A -s portmate\r"[..])
                .await
                .map_err(|_| "Tmux attach 命令发送失败: SSH channel 已关闭".to_string())?;
        }
        Ok::<_, String>(channel)
    };

    match bounded_connection_step(setup, timeout).await {
        Ok(channel) => Ok(channel),
        Err(BoundedConnectionStepError::Failed(error)) => Err(SshTerminalSetupError::Failed(error)),
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning =
                request_ssh_disconnect_with_timeout(session, disconnect_description).await;
            Err(SshTerminalSetupError::TimedOut {
                timeout_ms: timeout.as_millis(),
                cleanup_warning,
            })
        }
    }
}

pub(super) struct SshAuthenticationRequest<'a> {
    pub(super) ssh: SshConnection,
    pub(super) username: String,
    pub(super) password: Option<String>,
    pub(super) passphrase: Option<String>,
    pub(super) agent_socket_path: Option<PathBuf>,
    pub(super) timeout: Duration,
    pub(super) disconnect_description: &'a str,
}

pub(super) async fn authenticate_ssh_with_timeout<H: client::Handler>(
    session: &mut client::Handle<H>,
    request: SshAuthenticationRequest<'_>,
) -> Result<AuthMethod, SshAuthenticationError> {
    let SshAuthenticationRequest {
        ssh,
        username,
        password,
        passphrase,
        agent_socket_path,
        timeout,
        disconnect_description,
    } = request;
    match bounded_connection_step(
        authenticate_ssh_with_agent_socket(
            session,
            ssh,
            username,
            password,
            passphrase,
            agent_socket_path,
        ),
        timeout,
    )
    .await
    {
        Ok(method) => Ok(method),
        Err(BoundedConnectionStepError::Failed(error)) => {
            Err(SshAuthenticationError::Failed(error))
        }
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning =
                request_ssh_disconnect_with_timeout(session, disconnect_description).await;
            Err(SshAuthenticationError::TimedOut {
                timeout_ms: timeout.as_millis(),
                cleanup_warning,
            })
        }
    }
}

pub(super) async fn authenticate_ssh_with_agent_socket<H: client::Handler>(
    session: &mut client::Handle<H>,
    ssh: SshConnection,
    username: String,
    password: Option<String>,
    passphrase: Option<String>,
    agent_socket_path: Option<PathBuf>,
) -> Result<AuthMethod, String> {
    let auth_order = ordered_auth_methods(&ssh);
    let mut attempted = Vec::new();
    let mut key_errors = Vec::new();
    let saved_password = if password
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        read_optional_secret_ref(ssh.password_secret_ref.as_deref(), "SSH password")?
    } else {
        None
    };
    let saved_passphrase = if passphrase
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        read_optional_secret_ref(
            ssh.passphrase_secret_ref.as_deref(),
            "SSH private-key passphrase",
        )?
    } else {
        None
    };
    let effective_password = password
        .filter(|value| !value.is_empty())
        .or(saved_password);
    let effective_passphrase = passphrase
        .filter(|value| !value.is_empty())
        .or(saved_passphrase);
    let mut agent_attempted = false;

    for method in auth_order {
        match method {
            AuthMethod::PublicKey => {
                if ssh.agent_policy.enabled
                    && !agent_attempted
                    && !ssh.identity_policy.identities_only
                    && ssh.agent_policy.offer_mode
                        == portmate_core::AgentOfferMode::BeforeProfileKeys
                {
                    attempted.push("agent(before-profile-keys)");
                    agent_attempted = true;
                    match authenticate_with_agent(
                        session,
                        username.clone(),
                        ssh.identity_policy.identities_only,
                        ssh.agent_policy.offer_mode,
                        ssh.identity_refs.clone(),
                        agent_socket_path.clone(),
                    )
                    .await
                    {
                        Ok(true) => return Ok(AuthMethod::PublicKey),
                        Ok(false) => {}
                        Err(error) => key_errors.push(error),
                    }
                }

                let identities = ssh
                    .identity_refs
                    .iter()
                    .filter(|identity| {
                        matches!(
                            identity.source,
                            IdentitySource::SystemFile | IdentitySource::ProfileVault
                        )
                    })
                    .collect::<Vec<_>>();
                if !identities.is_empty() {
                    attempted.push("publickey");
                    let rsa_hash = session
                        .best_supported_rsa_hash()
                        .await
                        .map_err(|error| {
                            format!("SSH publickey 认证准备失败，无法查询 RSA 签名算法: {error}")
                        })?
                        .flatten();
                    for identity in identities {
                        let label = identity.label.clone();
                        let key = match load_identity_private_key(
                            identity,
                            effective_passphrase.as_deref(),
                        ) {
                            Ok(Some(key)) => key,
                            Ok(None) => continue,
                            Err(error) => {
                                key_errors.push(format!("{label}: {error}"));
                                continue;
                            }
                        };
                        let result = match session
                            .authenticate_publickey(
                                username.clone(),
                                PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
                            )
                            .await
                        {
                            Ok(result) => result,
                            Err(error) => {
                                key_errors.push(format!("{label}: 认证请求失败: {error}"));
                                break;
                            }
                        };
                        if result.success() {
                            return Ok(AuthMethod::PublicKey);
                        }
                        key_errors.push(format!("{label}: 被服务器拒绝"));
                    }
                }

                if ssh.agent_policy.enabled
                    && !agent_attempted
                    && (ssh.agent_policy.offer_mode
                        == portmate_core::AgentOfferMode::AfterProfileKeys
                        || ssh
                            .identity_refs
                            .iter()
                            .any(|identity| identity.source == IdentitySource::Agent))
                    && (!ssh.identity_policy.identities_only
                        || ssh
                            .identity_refs
                            .iter()
                            .any(|identity| identity.source == IdentitySource::Agent))
                {
                    attempted.push("agent(after-profile-keys)");
                    agent_attempted = true;
                    match authenticate_with_agent(
                        session,
                        username.clone(),
                        ssh.identity_policy.identities_only,
                        ssh.agent_policy.offer_mode,
                        ssh.identity_refs.clone(),
                        agent_socket_path.clone(),
                    )
                    .await
                    {
                        Ok(true) => return Ok(AuthMethod::PublicKey),
                        Ok(false) => {}
                        Err(error) => key_errors.push(error),
                    }
                }
            }
            AuthMethod::KeyboardInteractive => {
                let Some(password) = effective_password.clone() else {
                    continue;
                };
                attempted.push("keyboard-interactive");
                if authenticate_keyboard_interactive(session, username.clone(), password).await? {
                    return Ok(AuthMethod::KeyboardInteractive);
                }
            }
            AuthMethod::Password => {
                let Some(password) = effective_password.clone() else {
                    continue;
                };
                attempted.push("password");
                let result = session
                    .authenticate_password(username.clone(), password)
                    .await
                    .map_err(|error| format!("SSH password 认证失败: {error}"))?;
                if result.success() {
                    return Ok(AuthMethod::Password);
                }
            }
            AuthMethod::None => {
                attempted.push("none");
                let result = session
                    .authenticate_none(username.clone())
                    .await
                    .map_err(|error| format!("SSH none 认证失败: {error}"))?;
                if result.success() {
                    return Ok(AuthMethod::None);
                }
            }
            AuthMethod::GssapiWithMic => {
                attempted.push("gssapi-with-mic(unsupported)");
            }
        }
    }

    let mut message = if attempted.is_empty() {
        "SSH 认证失败：没有可尝试的认证方式。请配置 identityRefs 或在连接时输入密码。".to_string()
    } else {
        format!("SSH 认证失败，已尝试: {}", attempted.join(", "))
    };
    if !key_errors.is_empty() {
        message.push_str(&format!("；密钥详情: {}", key_errors.join(" | ")));
    }
    if ssh.agent_policy.enabled && ssh.identity_policy.identities_only {
        message.push_str("；当前按 IdentitiesOnly 处理，不会遍历系统 ssh-agent 的全部密钥");
    }
    Err(message)
}

pub(super) async fn authenticate_keyboard_interactive<H: client::Handler>(
    session: &mut client::Handle<H>,
    username: String,
    password: String,
) -> Result<bool, String> {
    let mut response = session
        .authenticate_keyboard_interactive_start(username, None::<String>)
        .await
        .map_err(|error| format!("SSH keyboard-interactive 启动失败: {error}"))?;

    for _ in 0..8 {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let responses = prompts
                    .iter()
                    .map(|prompt| {
                        if prompt.echo {
                            String::new()
                        } else {
                            password.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                response = session
                    .authenticate_keyboard_interactive_respond(responses)
                    .await
                    .map_err(|error| format!("SSH keyboard-interactive 响应失败: {error}"))?;
            }
        }
    }

    Err("SSH keyboard-interactive 认证轮次过多，已中止".to_string())
}

pub(super) fn ordered_auth_methods(ssh: &SshConnection) -> Vec<AuthMethod> {
    let mut ordered = Vec::new();
    if let Some(last) = ssh.identity_policy.last_successful.filter(|method| {
        ssh.identity_policy.record_success && ssh.identity_policy.auth_order.contains(method)
    }) {
        ordered.push(last);
    }
    for method in &ssh.identity_policy.auth_order {
        if !ordered.contains(method) {
            ordered.push(*method);
        }
    }
    if ordered.is_empty() {
        ordered.extend([
            AuthMethod::PublicKey,
            AuthMethod::KeyboardInteractive,
            AuthMethod::Password,
        ]);
    }
    ordered
}

pub(super) fn load_identity_private_key(
    identity: &IdentityRef,
    passphrase: Option<&str>,
) -> Result<Option<ssh_key::PrivateKey>, String> {
    match identity.source {
        IdentitySource::SystemFile => {
            let Some(path) = identity
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            else {
                return Ok(None);
            };
            load_secret_key(expand_identity_path(path), passphrase)
                .map(Some)
                .map_err(|error| format!("system-file {}: {error}", path))
        }
        IdentitySource::ProfileVault => {
            let Some(secret_ref) = identity
                .secret_ref
                .as_deref()
                .map(str::trim)
                .filter(|secret_ref| !secret_ref.is_empty())
            else {
                return Err("profile-vault identity 缺少 secretRef".to_string());
            };
            let private_key = read_secret_from_store(secret_ref)?;
            decode_secret_key(&private_key, passphrase)
                .map(Some)
                .map_err(|error| format!("profile-vault {secret_ref}: {error}"))
        }
        IdentitySource::Agent | IdentitySource::PublicKeyOnly => Ok(None),
    }
}

pub(super) async fn apply_ssh_terminal_color_env(channel: &Channel<client::Msg>) {
    for (name, value) in [
        ("COLORTERM", "truecolor"),
        ("CLICOLOR", "1"),
        ("CLICOLOR_FORCE", "1"),
        ("FORCE_COLOR", "1"),
        ("TERM_PROGRAM", "PortMate"),
    ] {
        let _ = channel.set_env(false, name, value).await;
    }
}

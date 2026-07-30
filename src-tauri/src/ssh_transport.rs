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

use super::*;

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

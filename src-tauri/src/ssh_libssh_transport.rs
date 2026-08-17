use super::*;

#[derive(Clone, Copy, Debug)]
pub(super) struct LibsshSetupDeadline {
    deadline: Instant,
}

impl LibsshSetupDeadline {
    fn new(timeout: Duration) -> Result<Self, String> {
        Self::from_started_at(Instant::now(), timeout)
    }

    pub(super) fn from_started_at(started_at: Instant, timeout: Duration) -> Result<Self, String> {
        let deadline = started_at
            .checked_add(timeout)
            .ok_or_else(|| "libssh setup deadline is outside the supported range".to_string())?;
        Ok(Self { deadline })
    }

    fn remaining(self) -> Option<Duration> {
        self.remaining_at(Instant::now())
    }

    pub(super) fn remaining_at(self, now: Instant) -> Option<Duration> {
        self.deadline
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
    }
}

fn libssh_terminal_setup_error(
    action: &str,
    error: libssh_rs::Error,
    connect_timeout: Duration,
) -> String {
    if matches!(error, libssh_rs::Error::TryAgain) {
        format!(
            "libssh 终端 setup 超时（{} ms，{action}）",
            connect_timeout.as_millis()
        )
    } else {
        format!("libssh {action}失败: {error}")
    }
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
    let setup_deadline = LibsshSetupDeadline::new(connect_timeout)?;
    let agent_socket_path = agent_socket_path
        .or_else(|| std::env::var_os("SSH_AUTH_SOCK").map(std::path::PathBuf::from));
    #[cfg(unix)]
    let mut filtered_agent_proxy = if libssh_auth_order_requires_filtered_agent(ssh) {
        match agent_socket_path.as_ref() {
            Some(upstream_socket) => Some(
                ssh_agent_filter::start_filtered_agent_proxy(
                    upstream_socket.clone(),
                    &ssh.identity_refs,
                )
                .await?,
            ),
            None => None,
        }
    } else {
        None
    };
    let host = ssh.endpoint.host.clone();
    let port = ssh.endpoint.port;
    let username = ssh.username.clone();
    let host_key_alias = ssh.host_key_policy.alias.clone();
    let closed = Arc::new(AtomicBool::new(false));
    let (proxy_stream, jump_sessions, mut transport_bridge_finished) =
        if !ssh.jumps.is_empty() {
            let remaining_jump_timeout = setup_deadline.remaining().ok_or_else(|| {
                format!(
                    "libssh Jump Host 连接超时（{} ms）",
                    connect_timeout.as_millis()
                )
            })?;
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
                remaining_jump_timeout,
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
                match start_russh_jump_transport_bridge(channel, Arc::clone(&closed)).await {
                    Ok(bridge) => bridge,
                    Err(error) => {
                        disconnect_jump_sessions(
                            jump_sessions,
                            "PortMate libssh jump transport bridge setup failed",
                        )
                        .await;
                        return Err(error);
                    }
                };
            (Some(stream), jump_sessions, Some(bridge_finished))
        } else if ssh.proxy.enabled {
            let remaining_proxy_timeout = setup_deadline.remaining().ok_or_else(|| {
                format!(
                    "libssh SSH 代理连接超时（{} ms）",
                    connect_timeout.as_millis()
                )
            })?;
            let stream = tokio::time::timeout(
                remaining_proxy_timeout,
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
            (Some(stream), Vec::new(), None)
        } else {
            (None, Vec::new(), None)
        };
    let Some(remaining_connect_timeout) = setup_deadline.remaining() else {
        cleanup_failed_libssh_runtime(
            None,
            &jump_sessions,
            &mut transport_bridge_finished,
            closed.as_ref(),
            "connect deadline",
        )
        .await;
        return Err(format!(
            "libssh SSH 连接超时（{} ms）",
            connect_timeout.as_millis()
        ));
    };
    #[cfg(unix)]
    let identity_agent_path = filtered_agent_proxy
        .as_ref()
        .map(|proxy| proxy.socket_path().to_path_buf())
        .or_else(|| agent_socket_path.clone());
    #[cfg(not(unix))]
    let identity_agent_path = agent_socket_path.clone();
    let identity_agent = match identity_agent_path
        .map(|path| {
            path.into_os_string()
                .into_string()
                .map_err(|_| "libssh SSH agent socket path is not valid UTF-8".to_string())
        })
        .transpose()
    {
        Ok(identity_agent) => identity_agent,
        Err(error) => {
            cleanup_failed_libssh_runtime(
                None,
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "agent socket validation",
            )
            .await;
            return Err(error);
        }
    };
    let agent_socket_available = identity_agent.is_some();
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
    .await;
    let (session, observation) = match connected {
        Ok(Ok(Ok(connected))) => connected,
        Ok(Ok(Err(error))) => {
            cleanup_failed_libssh_runtime(
                None,
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "connection",
            )
            .await;
            return Err(error);
        }
        Ok(Err(error)) => {
            cleanup_failed_libssh_runtime(
                None,
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "connection worker",
            )
            .await;
            return Err(format!("libssh 连接 worker 失败: {error}"));
        }
        Err(_) => {
            cleanup_failed_libssh_runtime(
                None,
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "connection timeout",
            )
            .await;
            return Err(format!(
                "libssh 连接超时（{} ms）",
                connect_timeout.as_millis()
            ));
        }
    };

    let observation_error = {
        match observed_key.lock() {
            Ok(mut observed_key) => {
                *observed_key = Some(observation.clone());
                None
            }
            Err(error) => Some(error.to_string()),
        }
    };
    if let Some(error) = observation_error {
        cleanup_failed_libssh_runtime(
            Some(&session),
            &jump_sessions,
            &mut transport_bridge_finished,
            closed.as_ref(),
            "host key observation",
        )
        .await;
        return Err(error);
    }
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
    let verification_state_error = {
        match host_key_error.lock() {
            Ok(mut host_key_error) => {
                *host_key_error = verification.as_ref().err().cloned();
                None
            }
            Err(error) => Some(error.to_string()),
        }
    };
    if let Some(error) = verification_state_error {
        cleanup_failed_libssh_runtime(
            Some(&session),
            &jump_sessions,
            &mut transport_bridge_finished,
            closed.as_ref(),
            "host key verification state",
        )
        .await;
        return Err(error);
    }
    if let Err(error) = verification {
        cleanup_failed_libssh_runtime(
            Some(&session),
            &jump_sessions,
            &mut transport_bridge_finished,
            closed.as_ref(),
            "host key verification",
        )
        .await;
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
                cleanup_failed_libssh_runtime(
                    Some(&session),
                    &jump_sessions,
                    &mut transport_bridge_finished,
                    closed.as_ref(),
                    "password lookup",
                )
                .await;
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
                cleanup_failed_libssh_runtime(
                    Some(&session),
                    &jump_sessions,
                    &mut transport_bridge_finished,
                    closed.as_ref(),
                    "passphrase lookup",
                )
                .await;
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
    let (offer_agent_before, offer_agent_after) =
        libssh_agent_offer_positions(ssh, agent_socket_available);
    let Some(remaining_auth_timeout) = setup_deadline.remaining() else {
        cleanup_failed_libssh_runtime(
            Some(&session),
            &jump_sessions,
            &mut transport_bridge_finished,
            closed.as_ref(),
            "authentication deadline",
        )
        .await;
        return Err(format!(
            "libssh SSH 认证超时（{} ms）",
            connect_timeout.as_millis()
        ));
    };
    let auth_session = session.clone();
    let auth = match tokio::time::timeout(
        remaining_auth_timeout,
        tokio::task::spawn_blocking(move || {
            auth_session
                .set_option(libssh_rs::SshOption::Timeout(remaining_auth_timeout))
                .map_err(|error| format!("libssh 设置认证超时失败: {error}"))?;
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
    #[cfg(unix)]
    if let Some(proxy) = filtered_agent_proxy.take() {
        if let Err(error) = proxy.stop().await {
            cleanup_failed_libssh_runtime(
                Some(&session),
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "agent proxy shutdown",
            )
            .await;
            return Err(error);
        }
    }
    let auth_method = match auth {
        Ok(method) => method,
        Err(error) => {
            cleanup_failed_libssh_runtime(
                Some(&session),
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "authentication",
            )
            .await;
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
        cleanup_failed_libssh_runtime(
            Some(&session),
            &jump_sessions,
            &mut transport_bridge_finished,
            closed.as_ref(),
            "host key persistence",
        )
        .await;
        return Err(error);
    }

    let terminal_session = session.clone();
    let agent_forward_socket = if ssh.agent_policy.forwarding {
        match agent_socket_path.clone() {
            Some(agent_socket_path) => Some(agent_socket_path),
            None => {
                cleanup_failed_libssh_runtime(
                    Some(&session),
                    &jump_sessions,
                    &mut transport_bridge_finished,
                    closed.as_ref(),
                    "agent forwarding validation",
                )
                .await;
                return Err("SSH agent forwarding requires an SSH agent socket".to_string());
            }
        }
    } else {
        None
    };
    let request_agent_forward = agent_forward_socket.is_some();
    let term = profile.terminal.term.clone();
    let cols = u32::from(profile.terminal.cols);
    let rows = u32::from(profile.terminal.rows);
    let attach_tmux = matches!(profile.connection, ConnectionConfig::Tmux(_));
    let Some(remaining_terminal_timeout) = setup_deadline.remaining() else {
        cleanup_failed_libssh_runtime(
            Some(&session),
            &jump_sessions,
            &mut transport_bridge_finished,
            closed.as_ref(),
            "terminal setup deadline",
        )
        .await;
        return Err(format!(
            "libssh 终端 setup 超时（{} ms）",
            connect_timeout.as_millis()
        ));
    };
    let channel = tokio::time::timeout(
        remaining_terminal_timeout,
        tokio::task::spawn_blocking(move || {
            terminal_session
                .set_option(libssh_rs::SshOption::Timeout(remaining_terminal_timeout))
                .map_err(|error| format!("libssh 设置终端 setup 超时失败: {error}"))?;
            let channel = terminal_session
                .new_channel()
                .map_err(|error| {
                    libssh_terminal_setup_error("创建终端 channel", error, connect_timeout)
                })?;
            channel
                .open_session()
                .map_err(|error| {
                    libssh_terminal_setup_error("打开终端 channel", error, connect_timeout)
                })?;
            channel
                .request_pty(&term, cols, rows)
                .map_err(|error| {
                    libssh_terminal_setup_error("请求 PTY", error, connect_timeout)
                })?;
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
                    .map_err(|error| {
                        libssh_terminal_setup_error(
                            "请求 agent forwarding",
                            error,
                            connect_timeout,
                        )
                    })?;
            }
            channel
                .request_shell()
                .map_err(|error| {
                    libssh_terminal_setup_error("请求 shell", error, connect_timeout)
                })?;
            if attach_tmux {
                let mut stdin = channel.stdin();
                stdin
                    .write_all(b"tmux new-session -A -s portmate\r")
                    .map_err(|error| format!("libssh Tmux attach 写入失败: {error}"))?;
                stdin
                    .flush()
                    .map_err(|error| format!("libssh Tmux attach 刷新失败: {error}"))?;
            }
            terminal_session
                .set_option(libssh_rs::SshOption::Timeout(
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                ))
                .map_err(|error| format!("libssh 设置运行期 I/O 超时失败: {error}"))?;
            Ok::<_, String>(channel)
        }),
    )
    .await;
    let channel = match channel {
        Ok(Ok(Ok(channel))) => channel,
        Ok(Ok(Err(error))) => {
            cleanup_failed_libssh_runtime(
                Some(&session),
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "terminal setup",
            )
            .await;
            return Err(error);
        }
        Ok(Err(error)) => {
            cleanup_failed_libssh_runtime(
                Some(&session),
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "terminal worker",
            )
            .await;
            return Err(format!("libssh 终端 worker 失败: {error}"));
        }
        Err(_) => {
            cleanup_failed_libssh_runtime(
                Some(&session),
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "terminal setup timeout",
            )
            .await;
            return Err(format!(
                "libssh 终端 setup 超时（{} ms）",
                connect_timeout.as_millis()
            ));
        }
    };

    let runtime_id = Uuid::new_v4().to_string();
    let profile_snapshot = match ssh_health::ssh_health_profile_snapshot(profile) {
        Ok(profile_snapshot) => profile_snapshot,
        Err(error) => {
            cleanup_failed_libssh_runtime(
                Some(&session),
                &jump_sessions,
                &mut transport_bridge_finished,
                closed.as_ref(),
                "profile snapshot",
            )
            .await;
            return Err(error);
        }
    };
    let (read_half, write_half) = SshBackendChannel::from_libssh(channel).split();
    let (tap, _) = broadcast::channel(1024);
    let (reader_finished_sender, reader_finished) = tokio::sync::oneshot::channel();
    let terminal_channel_open = Arc::new(AtomicBool::new(true));
    let agent_forwarder_finished = agent_forward_socket.map(|socket_path| {
        start_libssh_agent_forwarder(session.clone(), socket_path, Arc::clone(&closed))
    });

    Ok(EstablishedSshRuntime {
        runtime_id: runtime_id.clone(),
        runtime: SshRuntime {
            runtime_id,
            profile_snapshot,
            backend: SshBackendKind::Libssh,
            auth_method,
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

async fn cleanup_failed_libssh_runtime(
    _session: Option<&libssh_rs::Session>,
    jump_sessions: &[client::Handle<PortMateSshHandler>],
    transport_bridge_finished: &mut Option<tokio::sync::oneshot::Receiver<()>>,
    closed: &AtomicBool,
    context: &str,
) {
    closed.store(true, Ordering::SeqCst);
    // A timed-out setup worker may still own a channel. Let the shared SessionHolder
    // close its socket only after that worker and every channel have been destroyed.
    if let Some(finished) = transport_bridge_finished.take() {
        if tokio::time::timeout(SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT, finished)
            .await
            .is_err()
        {
            eprintln!(
                "PortMate: libssh {context} transport bridge cleanup timed out after {} ms",
                SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT.as_millis()
            );
        }
    }
    for jump_session in jump_sessions {
        if let Some(warning) =
            request_ssh_disconnect_with_timeout(jump_session, "PortMate libssh setup failed").await
        {
            eprintln!("PortMate: libssh {context} jump cleanup warning: {warning}");
        }
    }
}

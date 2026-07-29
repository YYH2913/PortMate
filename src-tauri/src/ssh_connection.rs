use super::*;

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

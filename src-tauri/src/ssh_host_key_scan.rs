use super::*;

struct JumpHostKeyScanRequest<'a> {
    state: &'a AppState,
    profile: &'a SessionProfile,
    ssh: &'a SshConnection,
    config: Arc<client::Config>,
    target_handler: HostKeyScanHandler,
    password: Option<&'a str>,
    passphrase: Option<&'a str>,
}

struct HostKeyScanHandler {
    host: String,
    port: u16,
    alias: Option<String>,
    observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
}

impl client::Handler for HostKeyScanHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        *lock_ssh_handler_state(&self.observed_key, "host key scan observation")? =
            Some(HostKeyObservation {
                host: self.host.clone(),
                port: self.port,
                alias: self.alias.clone(),
                algorithm: server_public_key.algorithm().to_string(),
                public_key_base64: server_public_key.public_key_base64(),
            });
        Ok(true)
    }
}

pub(super) async fn scan_ssh_host_key_inner(
    state: &AppState,
    profile: SessionProfile,
    password: Option<&str>,
    passphrase: Option<&str>,
) -> Result<HostKeyScanResult, String> {
    let ssh = match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.clone(),
        _ => return Err("profile is not SSH-backed".to_string()),
    };
    let host = ssh.endpoint.host.trim().to_string();
    if host.is_empty() {
        return Err("SSH 主机不能为空".to_string());
    }
    if ssh.endpoint.port == 0 {
        return Err("SSH 端口必须在 1-65535 之间".to_string());
    }

    let observed_key = Arc::new(Mutex::new(None));
    let alias = ssh
        .host_key_policy
        .alias
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| Some(profile.id.clone()));
    let handler = HostKeyScanHandler {
        host: host.clone(),
        port: ssh.endpoint.port,
        alias,
        observed_key: Arc::clone(&observed_key),
    };
    let config = Arc::new(client::Config {
        keepalive_interval: Some(Duration::from_secs(10)),
        keepalive_max: 1,
        nodelay: true,
        ..Default::default()
    });

    if !ssh.jumps.is_empty() {
        if let Some(result) = scan_ssh_host_key_via_jump(JumpHostKeyScanRequest {
            state,
            profile: &profile,
            ssh: &ssh,
            config,
            target_handler: handler,
            password,
            passphrase,
        })
        .await?
        {
            return Ok(result);
        }
    } else {
        let session = connect_ssh_transport(
            SshTransportConnectParams {
                config,
                target_host: &host,
                target_port: ssh.endpoint.port,
                proxy: &ssh.proxy,
                tcp_keepalive_enabled: ssh.tcp_keepalive_enabled,
                timeout: Duration::from_secs(12),
                label: "SSH host key 扫描",
            },
            handler,
        )
        .await;
        let session = match session {
            Ok(session) => session,
            Err(SshTransportConnectError::Timeout) => {
                return Err(format!(
                    "SSH host key 扫描超时: {host}:{}",
                    ssh.endpoint.port
                ));
            }
            Err(SshTransportConnectError::Transport(error)) => return Err(error),
            Err(SshTransportConnectError::Handshake(error)) => {
                return Err(format!(
                    "SSH host key 扫描失败: {host}:{}: {error}",
                    ssh.endpoint.port
                ));
            }
        };
        if let Some(warning) =
            request_ssh_disconnect_with_timeout(&session, "PortMate host key scan").await
        {
            eprintln!("PortMate: host key scan disconnect warning: {warning}");
        }
    }

    let observation = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "SSH host key 扫描未收到服务器 host key".to_string())?;
    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());
    host_keys
        .keys
        .extend(one_time_host_keys_snapshot(state, &profile.id)?);
    let evaluation = host_keys
        .evaluate(&profile.id, &ssh.host_key_policy, &observation)
        .map_err(|error| error.to_string())?;
    Ok(HostKeyScanResult {
        label: Some("目标 SSH".to_string()),
        observation,
        evaluation,
    })
}

async fn scan_ssh_host_key_via_jump(
    request: JumpHostKeyScanRequest<'_>,
) -> Result<Option<HostKeyScanResult>, String> {
    let JumpHostKeyScanRequest {
        state,
        profile,
        ssh,
        config,
        target_handler,
        password,
        passphrase,
    } = request;

    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());
    let one_time_host_keys = one_time_host_keys_snapshot(state, &profile.id)?;
    let one_time_host_key_ids = one_time_host_keys
        .iter()
        .map(|key| key.id.clone())
        .collect::<Vec<_>>();
    host_keys.keys.extend(one_time_host_keys);

    let mut jump_sessions: Vec<client::Handle<PortMateSshHandler>> = Vec::new();
    for (index, jump) in ssh.jumps.iter().enumerate() {
        let (jump_host, jump_port, jump_username) = jump_endpoint_details(jump, index)?;
        let jump_policy = jump_host_key_policy(ssh, jump);
        let jump_ssh = jump_ssh_connection(ssh, jump, jump_policy.clone());
        let observed_jump_key = Arc::new(Mutex::new(None));
        let jump_key_error = Arc::new(Mutex::new(None));
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
        let jump_label = format!("Jump Host 第 {} 跳", index + 1);
        let mut jump_session = if let Some(previous_jump) = jump_sessions.last_mut() {
            let jump_channel = match open_direct_tcpip_with_timeout(
                previous_jump,
                jump_host.clone(),
                jump_port,
                "127.0.0.1".to_string(),
                0,
                Duration::from_secs(12),
                "PortMate jump host key scan channel timeout",
            )
            .await
            {
                Ok(channel) => channel,
                Err(DirectTcpipOpenError::Failed(error)) => {
                    disconnect_jump_sessions(
                        jump_sessions,
                        "PortMate jump host key scan channel failed",
                    )
                    .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳打开 host key 扫描通道到 {jump_host}:{jump_port} 失败: {error}",
                        index + 1
                    ));
                }
                Err(DirectTcpipOpenError::TimedOut {
                    timeout_ms,
                    cleanup_warning,
                }) => {
                    disconnect_jump_sessions(
                        jump_sessions,
                        "PortMate jump host key scan channel timeout",
                    )
                    .await;
                    let cleanup_warning = cleanup_warning
                        .map(|warning| format!("; {warning}"))
                        .unwrap_or_default();
                    return Err(format!(
                        "Jump Host 第 {} 跳打开 host key 扫描通道到 {jump_host}:{jump_port} 超时（{timeout_ms} ms）{cleanup_warning}",
                        index + 1
                    ));
                }
            };
            match tokio::time::timeout(
                Duration::from_secs(12),
                client::connect_stream(config.clone(), jump_channel.into_stream(), jump_handler),
            )
            .await
            {
                Ok(Ok(session)) => session,
                Err(_) => {
                    disconnect_jump_sessions(jump_sessions, "PortMate jump host key scan timeout")
                        .await;
                    return Err(format!(
                        "Jump Host 第 {} 跳 host key 扫描连接超时: {jump_host}:{jump_port}",
                        index + 1
                    ));
                }
                Ok(Err(error)) => {
                    if let Some(result) = host_key_scan_result_for_policy(
                        profile,
                        ssh,
                        &jump_policy,
                        &observed_jump_key,
                        &jump_label,
                        state,
                    )? {
                        disconnect_jump_sessions(
                            jump_sessions,
                            "PortMate jump host key scan needs confirmation",
                        )
                        .await;
                        return Ok(Some(result));
                    }
                    let message = jump_key_error
                        .lock()
                        .ok()
                        .and_then(|reason| reason.clone())
                        .unwrap_or_else(|| {
                            format!(
                                "Jump Host 第 {} 跳 host key 扫描连接失败: {error}",
                                index + 1
                            )
                        });
                    disconnect_jump_sessions(
                        jump_sessions,
                        "PortMate jump host key scan handshake failed",
                    )
                    .await;
                    return Err(message);
                }
            }
        } else {
            match connect_ssh_transport(
                SshTransportConnectParams {
                    config: config.clone(),
                    target_host: &jump_host,
                    target_port: jump_port,
                    proxy: &ssh.proxy,
                    tcp_keepalive_enabled: ssh.tcp_keepalive_enabled,
                    timeout: Duration::from_secs(12),
                    label: &jump_label,
                },
                jump_handler,
            )
            .await
            {
                Ok(session) => session,
                Err(SshTransportConnectError::Timeout) => {
                    return Err(format!(
                        "Jump Host host key 扫描连接超时: {jump_host}:{jump_port}"
                    ));
                }
                Err(SshTransportConnectError::Transport(error)) => return Err(error),
                Err(SshTransportConnectError::Handshake(error)) => {
                    if let Some(result) = host_key_scan_result_for_policy(
                        profile,
                        ssh,
                        &jump_policy,
                        &observed_jump_key,
                        &jump_label,
                        state,
                    )? {
                        return Ok(Some(result));
                    }
                    return Err(jump_key_error
                        .lock()
                        .ok()
                        .and_then(|reason| reason.clone())
                        .unwrap_or_else(|| format!("Jump Host host key 扫描连接失败: {error}")));
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
                agent_socket_path: None,
                timeout: Duration::from_secs(12),
                disconnect_description: "PortMate jump host key scan authentication timeout",
            },
        )
        .await
        {
            disconnect_jump_sessions(jump_sessions, "PortMate jump host key scan auth failed")
                .await;
            let _ = request_ssh_disconnect_with_timeout(
                &jump_session,
                "PortMate jump host key scan auth failed",
            )
            .await;
            return Err(format!(
                "Jump Host 第 {} 跳 host key 扫描认证失败: {error}",
                index + 1
            ));
        }
        jump_sessions.push(jump_session);
    }

    let target_host = ssh.endpoint.host.trim().to_string();
    let jump_channel = match open_direct_tcpip_with_timeout(
        jump_sessions
            .last()
            .expect("non-empty jumps should create jump sessions"),
        target_host.clone(),
        ssh.endpoint.port,
        "127.0.0.1".to_string(),
        0,
        Duration::from_secs(12),
        "PortMate jump host key scan target channel timeout",
    )
    .await
    {
        Ok(channel) => channel,
        Err(DirectTcpipOpenError::Failed(error)) => {
            disconnect_jump_sessions(
                jump_sessions,
                "PortMate jump host key scan target channel failed",
            )
            .await;
            return Err(format!(
                "Jump Host 打开 host key 扫描通道到 {target_host}:{} 失败: {error}",
                ssh.endpoint.port
            ));
        }
        Err(DirectTcpipOpenError::TimedOut {
            timeout_ms,
            cleanup_warning,
        }) => {
            disconnect_jump_sessions(
                jump_sessions,
                "PortMate jump host key scan target channel timeout",
            )
            .await;
            let cleanup_warning = cleanup_warning
                .map(|warning| format!("; {warning}"))
                .unwrap_or_default();
            return Err(format!(
                "Jump Host 打开 host key 扫描通道到 {target_host}:{} 超时（{timeout_ms} ms）{cleanup_warning}",
                ssh.endpoint.port
            ));
        }
    };
    let target_session = match tokio::time::timeout(
        Duration::from_secs(12),
        client::connect_stream(config, jump_channel.into_stream(), target_handler),
    )
    .await
    {
        Ok(Ok(session)) => session,
        Err(_) => {
            disconnect_jump_sessions(jump_sessions, "PortMate host key scan target timeout").await;
            return Err(format!(
                "SSH 经 Jump Host host key 扫描超时: {target_host}:{}",
                ssh.endpoint.port
            ));
        }
        Ok(Err(error)) => {
            disconnect_jump_sessions(jump_sessions, "PortMate host key scan target failed").await;
            return Err(format!(
                "SSH 经 Jump Host host key 扫描失败: {target_host}:{}: {error}",
                ssh.endpoint.port
            ));
        }
    };
    if let Some(warning) =
        request_ssh_disconnect_with_timeout(&target_session, "PortMate host key scan").await
    {
        eprintln!("PortMate: jump target host key scan disconnect warning: {warning}");
    }
    disconnect_jump_sessions(jump_sessions, "PortMate jump host key scan").await;
    Ok(None)
}

fn host_key_scan_result_for_policy(
    profile: &SessionProfile,
    ssh: &SshConnection,
    policy: &portmate_core::HostKeyPolicy,
    observed_key: &Arc<Mutex<Option<HostKeyObservation>>>,
    label: &str,
    state: &AppState,
) -> Result<Option<HostKeyScanResult>, String> {
    let Some(observation) = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
    else {
        return Ok(None);
    };
    let mut host_keys = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.host_keys.clone()
    };
    host_keys.keys.extend(ssh.trusted_host_keys.clone());
    host_keys
        .keys
        .extend(one_time_host_keys_snapshot(state, &profile.id)?);
    let evaluation = host_keys
        .evaluate(&profile.id, policy, &observation)
        .map_err(|error| error.to_string())?;
    Ok(Some(HostKeyScanResult {
        label: Some(label.to_string()),
        observation,
        evaluation,
    }))
}

use super::*;

pub(super) struct SshTransportConnectParams<'a> {
    pub(super) config: Arc<client::Config>,
    pub(super) target_host: &'a str,
    pub(super) target_port: u16,
    pub(super) proxy: &'a ProxyConfig,
    pub(super) tcp_keepalive_enabled: Option<bool>,
    pub(super) timeout: Duration,
    pub(super) label: &'a str,
}

#[derive(Debug)]
pub(super) enum SshTransportConnectError {
    Timeout,
    Transport(String),
    Handshake(String),
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

pub(super) async fn request_shared_ssh_disconnect_with_timeout<H: client::Handler>(
    handle: &Arc<tokio::sync::Mutex<client::Handle<H>>>,
    disconnect_description: &str,
) -> Option<String> {
    let handle = match tokio::time::timeout(SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT, handle.lock())
        .await
    {
        Ok(handle) => handle,
        Err(_) => {
            return Some(format!(
                "SSH handle lock timed out after {} ms",
                SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT.as_millis()
            ));
        }
    };
    request_ssh_disconnect_with_timeout(&handle, disconnect_description).await
}

pub(super) async fn request_shared_backend_disconnect_with_timeout<H: client::Handler>(
    handle: &Arc<tokio::sync::Mutex<SshBackendSession<H>>>,
    disconnect_description: &str,
) -> Option<String> {
    let handle = match tokio::time::timeout(SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT, handle.lock())
        .await
    {
        Ok(handle) => handle,
        Err(_) => {
            return Some(format!(
                "SSH backend handle lock timed out after {} ms",
                SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT.as_millis()
            ));
        }
    };
    request_backend_disconnect_with_timeout(&handle, disconnect_description).await
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

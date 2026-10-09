#[cfg(all(test, unix))]
pub(super) async fn open_sftp_session_with_timeout<H: client::Handler>(
    handle: Arc<tokio::sync::Mutex<client::Handle<H>>>,
    timeout: Duration,
) -> Result<SftpBackendSession, String> {
    let started = Instant::now();
    let handle = tokio::time::timeout(timeout, handle.lock())
        .await
        .map_err(|_| format!("SFTP handle lock 超时（{} ms）", timeout.as_millis()))?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("SFTP setup 超时（{} ms）", timeout.as_millis()))?;

    let setup = async {
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|error| format!("SFTP 打开 SSH channel 失败: {error}"))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| format!("SFTP subsystem 启动失败: {error}"))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| format!("SFTP 初始化失败: {error}"))?;
        sftp.set_timeout(SFTP_REQUEST_TIMEOUT_SECONDS);
        Ok::<_, String>(SftpBackendSession::from_russh(sftp))
    };
    match bounded_connection_step(setup, remaining).await {
        Ok(sftp) => Ok(sftp),
        Err(BoundedConnectionStepError::Failed(error)) => Err(error),
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning =
                request_ssh_disconnect_with_timeout(&handle, "PortMate SFTP setup timeout")
                    .await
                    .map(|warning| format!("; {warning}"))
                    .unwrap_or_default();
            Err(format!(
                "SFTP setup 超时（{} ms）{cleanup_warning}",
                timeout.as_millis()
            ))
        }
    }
}

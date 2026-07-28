use super::*;

pub(super) const SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub(super) const SSH_EXEC_STATUS_GRACE_TIMEOUT: Duration = Duration::from_secs(1);
pub(super) const MAX_SSH_EXEC_STDOUT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_SSH_EXEC_STDERR_BYTES: usize = 64 * 1024;

pub(super) async fn close_ssh_channel_bounded(channel: &SshBackendChannel) {
    let _ = tokio::time::timeout(SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT, channel.close()).await;
}

pub(super) async fn close_russh_channel_bounded(channel: &Channel<client::Msg>) {
    let _ = tokio::time::timeout(SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT, channel.close()).await;
}

pub(super) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn windows_powershell_command(script: &str) -> String {
    format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
        windows_powershell_encoded_script(script)
    )
}

pub(super) fn windows_powershell_encoded_script(script: &str) -> String {
    let mut utf16le = Vec::with_capacity(script.len().saturating_mul(2));
    for unit in script.encode_utf16() {
        utf16le.extend_from_slice(&unit.to_le_bytes());
    }
    BASE64_STANDARD.encode(utf16le)
}

pub(super) fn ssh_exec_message_completes(
    message: &SshBackendMessage,
    exit_status: &mut Option<u32>,
    eof_received_at: &mut Option<Instant>,
) -> bool {
    match message {
        SshBackendMessage::ExitStatus(code) => {
            *exit_status = Some(*code);
            eof_received_at.is_some()
        }
        SshBackendMessage::Eof => {
            eof_received_at.get_or_insert_with(Instant::now);
            exit_status.is_some()
        }
        SshBackendMessage::Close => true,
        _ => false,
    }
}

pub(super) fn russh_exec_message_completes(
    message: &ChannelMsg,
    exit_status: &mut Option<u32>,
    eof_received_at: &mut Option<Instant>,
) -> bool {
    match message {
        ChannelMsg::ExitStatus { exit_status: code } => {
            *exit_status = Some(*code);
            eof_received_at.is_some()
        }
        ChannelMsg::Eof => {
            eof_received_at.get_or_insert_with(Instant::now);
            exit_status.is_some()
        }
        ChannelMsg::Close => true,
        _ => false,
    }
}

pub(super) fn ssh_exec_status_grace_expired(eof_received_at: Option<Instant>) -> bool {
    eof_received_at
        .is_some_and(|received_at| received_at.elapsed() >= SSH_EXEC_STATUS_GRACE_TIMEOUT)
}

pub(super) async fn exec_ssh_command_capture<H: client::Handler>(
    handle: Arc<tokio::sync::Mutex<SshBackendSession<H>>>,
    command: &str,
    timeout: Duration,
) -> Result<String, String> {
    let started = Instant::now();
    let mut channel = open_shared_ssh_exec_channel(&handle, command, timeout, "SSH exec").await?;
    let result = async {
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| "SSH exec 超时".to_string())?;

        let mut output = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        let mut eof_received_at: Option<Instant> = None;
        tokio::time::timeout(remaining, async {
            loop {
                let message = if let Some(received_at) = eof_received_at {
                    let grace_remaining =
                        SSH_EXEC_STATUS_GRACE_TIMEOUT.saturating_sub(received_at.elapsed());
                    if grace_remaining.is_zero() {
                        break;
                    }
                    match tokio::time::timeout(grace_remaining, channel.wait()).await {
                        Ok(message) => message,
                        Err(_) => break,
                    }
                } else {
                    channel.wait().await
                };
                let Some(message) = message else {
                    break;
                };
                if ssh_exec_message_completes(&message, &mut exit_status, &mut eof_received_at) {
                    break;
                }
                match message {
                    SshBackendMessage::Data(data) => append_bounded_ssh_exec_data(
                        &mut output,
                        &data,
                        MAX_SSH_EXEC_STDOUT_BYTES,
                        "stdout",
                    )?,
                    SshBackendMessage::ExtendedData { data, .. } => append_bounded_ssh_exec_data(
                        &mut stderr,
                        &data,
                        MAX_SSH_EXEC_STDERR_BYTES,
                        "stderr",
                    )?,
                    SshBackendMessage::Error(error) => {
                        return Err(format!("SSH exec channel read failed: {error}"));
                    }
                    _ => {}
                }
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "SSH exec 超时".to_string())??;

        if let Some(code) = exit_status.filter(|code| *code != 0) {
            return Err(format!(
                "SSH exec 返回非零状态 {code}: {}",
                String::from_utf8_lossy(&stderr)
            ));
        }

        Ok(String::from_utf8_lossy(&output).to_string())
    }
    .await;
    close_ssh_channel_bounded(&channel).await;
    result
}

pub(super) fn append_bounded_ssh_exec_data(
    buffer: &mut Vec<u8>,
    data: &[u8],
    max_bytes: usize,
    stream: &str,
) -> Result<(), String> {
    let next_len = buffer
        .len()
        .checked_add(data.len())
        .ok_or_else(|| format!("SSH exec {stream} 长度溢出"))?;
    if next_len > max_bytes {
        return Err(format!("SSH exec {stream} 超过 {} 字节上限", max_bytes));
    }
    buffer.extend_from_slice(data);
    Ok(())
}

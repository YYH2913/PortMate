use super::*;

pub(super) async fn scp_wait_download_completion(
    channel: &mut SshBackendChannel,
    stderr: &mut Vec<u8>,
    progress: &TransferProgressContext,
    idle_timeout: Duration,
) -> Result<(), String> {
    let mut output = Vec::new();
    let mut exit_status = None;
    let mut eof_received_at: Option<Instant> = None;
    let completion_started = Instant::now();
    loop {
        progress.check_cancelled()?;
        if ssh_exec_status_grace_expired(eof_received_at) {
            break;
        }
        let remaining = if let Some(received_at) = eof_received_at {
            SSH_EXEC_STATUS_GRACE_TIMEOUT.saturating_sub(received_at.elapsed())
        } else {
            // The SCP payload has already completed at this point. A remote
            // exit status is control-plane completion, so start a fresh idle
            // window instead of reusing the timestamp of the last data chunk.
            idle_timeout.saturating_sub(completion_started.elapsed())
        };
        if remaining.is_zero() {
            if eof_received_at.is_some() {
                break;
            }
            return Err(format!(
                "SCP 等待远程完成 空闲超时（{} ms）",
                idle_timeout.as_millis()
            ));
        }
        match tokio::time::timeout(remaining.min(TRANSFER_CANCEL_POLL_INTERVAL), channel.wait())
            .await
        {
            Ok(Some(message)) => {
                if ssh_exec_message_completes(&message, &mut exit_status, &mut eof_received_at) {
                    break;
                }
                match message {
                    SshBackendMessage::Data(data) => append_bounded_ssh_exec_data(
                        &mut output,
                        &data,
                        MAX_SSH_EXEC_STDOUT_BYTES,
                        "SCP download stdout",
                    )?,
                    SshBackendMessage::ExtendedData { data, .. } => append_bounded_ssh_exec_data(
                        stderr,
                        &data,
                        MAX_SSH_EXEC_STDERR_BYTES,
                        "SCP download stderr",
                    )?,
                    _ => {}
                }
            }
            Ok(None) => break,
            Err(_) => {}
        }
    }

    if let Some(code) = exit_status.filter(|code| *code != 0) {
        return Err(format!(
            "SCP download remote returned non-zero {code}: {}{}",
            String::from_utf8_lossy(&output),
            String::from_utf8_lossy(stderr)
        ));
    }
    Ok(())
}

pub(super) async fn scp_wait_ack(
    channel: &mut SshBackendChannel,
    pending: &mut VecDeque<u8>,
    stderr: &mut Vec<u8>,
    progress: &TransferProgressContext,
    last_progress: &mut Instant,
    idle_timeout: Duration,
    stage: &str,
) -> Result<(), String> {
    match scp_next_byte(
        channel,
        pending,
        stderr,
        progress,
        last_progress,
        idle_timeout,
        stage,
    )
    .await?
    {
        Some(0) => Ok(()),
        Some(1) | Some(2) => {
            let message = scp_read_line(
                channel,
                pending,
                stderr,
                progress,
                last_progress,
                idle_timeout,
                stage,
            )
            .await?;
            Err(format!("SCP remote error: {message}"))
        }
        Some(byte) => Err(format!("SCP unexpected ack byte: {byte}")),
        None => Err("SCP remote closed while waiting for ack".to_string()),
    }
}

pub(super) async fn scp_read_line(
    channel: &mut SshBackendChannel,
    pending: &mut VecDeque<u8>,
    stderr: &mut Vec<u8>,
    progress: &TransferProgressContext,
    last_progress: &mut Instant,
    idle_timeout: Duration,
    stage: &str,
) -> Result<String, String> {
    let mut bytes = Vec::new();
    loop {
        let byte = scp_next_byte(
            channel,
            pending,
            stderr,
            progress,
            last_progress,
            idle_timeout,
            stage,
        )
        .await?
        .ok_or_else(|| format!("{stage}: remote closed before newline"))?;
        if byte == b'\n' {
            break;
        }
        if bytes.len() >= MAX_SCP_PROTOCOL_LINE_BYTES {
            return Err(format!(
                "{stage} 超过协议行上限（{MAX_SCP_PROTOCOL_LINE_BYTES} bytes）"
            ));
        }
        bytes.push(byte);
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

pub(super) async fn scp_next_byte(
    channel: &mut SshBackendChannel,
    pending: &mut VecDeque<u8>,
    stderr: &mut Vec<u8>,
    progress: &TransferProgressContext,
    last_progress: &mut Instant,
    idle_timeout: Duration,
    stage: &str,
) -> Result<Option<u8>, String> {
    loop {
        progress.check_cancelled()?;
        if let Some(byte) = pending.pop_front() {
            return Ok(Some(byte));
        }
        match scp_wait_channel_message(channel, progress, last_progress, idle_timeout, stage)
            .await?
        {
            Some(SshBackendMessage::Data(data)) => {
                pending.extend(data.iter().copied());
            }
            Some(SshBackendMessage::ExtendedData { data, .. }) => append_bounded_ssh_exec_data(
                stderr,
                &data,
                MAX_SSH_EXEC_STDERR_BYTES,
                "SCP download stderr",
            )?,
            Some(SshBackendMessage::ExitStatus(code)) if code != 0 => {
                return Err(format!(
                    "SCP download remote returned non-zero {code}: {}",
                    String::from_utf8_lossy(stderr)
                ));
            }
            Some(SshBackendMessage::Eof) | Some(SshBackendMessage::Close) | None => {
                return Ok(None)
            }
            _ => {}
        }
    }
}

use super::*;

pub(super) const SCP_IO_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const MAX_SCP_PROTOCOL_LINE_BYTES: usize = 64 * 1024;

pub(super) async fn scp_send_data_with_idle_timeout(
    channel: &SshBackendChannel,
    data: &[u8],
    progress: &TransferProgressContext,
    idle_timeout: Duration,
    stage: &str,
) -> Result<(), String> {
    if let Err(error) = progress.check_cancelled() {
        close_ssh_channel_bounded(channel).await;
        return Err(error);
    }
    let started = Instant::now();
    let outcome = {
        let send = channel.data(data);
        tokio::pin!(send);
        loop {
            let remaining = idle_timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                break Err(format!(
                    "{stage} 空闲超时（{} ms）",
                    idle_timeout.as_millis()
                ));
            }
            tokio::select! {
                result = &mut send => {
                    break result.map_err(|error| format!("{stage}失败: {error}"));
                }
                _ = tokio::time::sleep(remaining.min(TRANSFER_CANCEL_POLL_INTERVAL)) => {
                    if let Err(error) = progress.check_cancelled() {
                        break Err(error);
                    }
                }
            }
        }
    };
    if outcome.is_err() {
        close_ssh_channel_bounded(channel).await;
    }
    outcome
}

pub(super) async fn scp_wait_channel_message(
    channel: &mut SshBackendChannel,
    progress: &TransferProgressContext,
    last_progress: &mut Instant,
    idle_timeout: Duration,
    stage: &str,
) -> Result<Option<SshBackendMessage>, String> {
    progress.check_cancelled()?;
    let outcome = {
        let wait = channel.wait();
        tokio::pin!(wait);
        loop {
            let remaining = idle_timeout.saturating_sub(last_progress.elapsed());
            if remaining.is_zero() {
                break Err(format!(
                    "{stage} 空闲超时（{} ms）",
                    idle_timeout.as_millis()
                ));
            }
            tokio::select! {
                message = &mut wait => break Ok(message),
                _ = tokio::time::sleep(remaining.min(TRANSFER_CANCEL_POLL_INTERVAL)) => {
                    if let Err(error) = progress.check_cancelled() {
                        break Err(error);
                    }
                }
            }
        }
    };
    match outcome {
        Ok(message) => {
            if matches!(
                message.as_ref(),
                Some(SshBackendMessage::Data(_) | SshBackendMessage::ExtendedData { .. })
            ) {
                *last_progress = Instant::now();
            }
            Ok(message)
        }
        Err(error) => Err(error),
    }
}

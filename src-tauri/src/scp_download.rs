use super::*;

pub(super) async fn scp_download(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    remote_source: &str,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    scp_download_with_idle_timeout(
        handle,
        remote_source,
        local_destination,
        progress,
        SCP_IO_IDLE_TIMEOUT,
    )
    .await
}

pub(super) async fn scp_download_with_idle_timeout<H: SshExecChannelOpener>(
    handle: H,
    remote_source: &str,
    local_destination: &str,
    progress: &TransferProgressContext,
    idle_timeout: Duration,
) -> Result<u64, String> {
    let command = scp_download_command(remote_source);
    let mut channel = handle
        .open_exec_channel(&command, SSH_AUXILIARY_SETUP_TIMEOUT, "SCP download")
        .await?;
    let outcome = async {
        let mut pending = VecDeque::new();
        let mut stderr = Vec::new();
        scp_send_data_with_idle_timeout(
            &channel,
            &[0_u8],
            progress,
            idle_timeout,
            "SCP 写入初始确认",
        )
        .await?;
        let mut last_progress = Instant::now();

        let first = scp_next_byte(
            &mut channel,
            &mut pending,
            &mut stderr,
            progress,
            &mut last_progress,
            idle_timeout,
            "SCP 等待文件头",
        )
        .await?
        .ok_or_else(|| "SCP remote closed before header".to_string())?;
        if first == 1 || first == 2 {
            let message = scp_read_line(
                &mut channel,
                &mut pending,
                &mut stderr,
                progress,
                &mut last_progress,
                idle_timeout,
                "SCP 读取远端错误",
            )
            .await?;
            return Err(format!("SCP remote error: {message}"));
        }
        if first != b'C' {
            return Err(format!("SCP unexpected header byte: {first}"));
        }

        let header = scp_read_line(
            &mut channel,
            &mut pending,
            &mut stderr,
            progress,
            &mut last_progress,
            idle_timeout,
            "SCP 读取文件头",
        )
        .await?;
        let parts = header.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err(format!("SCP invalid file header: C{header}"));
        }
        let size = parts[1]
            .parse::<u64>()
            .map_err(|error| format!("SCP invalid file size: {error}"))?;

        let target = Path::new(local_destination);
        prepare_local_transfer_target_path(target, "SCP 本地目标路径")?;
        let temp_target = local_resume_part_path(target);
        local_resume_offset(&temp_target, size)?;
        // Standard scp -f always streams from byte zero and provides no source
        // identity for validating an existing prefix. Truncate instead of
        // combining an unverified old prefix with the new remote suffix.
        progress.set_rate_baseline(0);
        let mut file = open_local_resume_writer(&temp_target, 0)
            .map_err(|error| format!("创建本地目标文件失败: {error}"))?;
        scp_send_data_with_idle_timeout(
            &channel,
            &[0_u8],
            progress,
            idle_timeout,
            "SCP 写入文件头确认",
        )
        .await?;

        let mut received = 0_u64;
        while received < size {
            progress.check_cancelled()?;
            if pending.is_empty() {
                match scp_wait_channel_message(
                    &mut channel,
                    progress,
                    &mut last_progress,
                    idle_timeout,
                    "SCP 接收文件内容",
                )
                .await?
                {
                    Some(SshBackendMessage::Data(data)) => {
                        pending.extend(data.iter().copied());
                    }
                    Some(SshBackendMessage::ExtendedData { data, .. }) => {
                        append_bounded_ssh_exec_data(
                            &mut stderr,
                            &data,
                            MAX_SSH_EXEC_STDERR_BYTES,
                            "SCP download stderr",
                        )?
                    }
                    Some(SshBackendMessage::ExitStatus(code)) if code != 0 => {
                        return Err(format!(
                            "SCP download remote returned non-zero {code}: {}",
                            String::from_utf8_lossy(&stderr)
                        ));
                    }
                    Some(SshBackendMessage::Eof) | Some(SshBackendMessage::Close) | None => {
                        return Err("SCP remote closed during file body".to_string());
                    }
                    _ => {}
                }
                continue;
            }
            let remaining = usize::try_from(size - received).unwrap_or(usize::MAX);
            let take = pending.len().min(remaining);
            let chunk = pending.drain(..take).collect::<Vec<_>>();
            received += take as u64;
            file.write_all(&chunk)
                .map_err(|error| format!("写入本地目标文件失败: {error}"))?;
            progress.update(received, size).await?;
        }
        file.flush()
            .map_err(|error| format!("刷新本地目标文件失败: {error}"))?;
        drop(file);
        scp_wait_ack(
            &mut channel,
            &mut pending,
            &mut stderr,
            progress,
            &mut last_progress,
            idle_timeout,
            "SCP 等待完成确认",
        )
        .await?;
        scp_send_data_with_idle_timeout(
            &channel,
            &[0_u8],
            progress,
            idle_timeout,
            "SCP 写入完成确认",
        )
        .await?;
        scp_wait_download_completion(&mut channel, &mut stderr, progress, idle_timeout).await?;
        finalize_local_resume_file(&temp_target, target)?;
        Ok(size)
    }
    .await;
    close_ssh_channel_bounded(&channel).await;
    outcome
}

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

pub(super) async fn scp_upload<H: SshExecChannelOpener>(
    handle: H,
    local_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut file, size) = open_local_transfer_source(Path::new(local_source), "SCP upload")?;
    let file_name = Path::new(local_source)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-upload.bin");
    let command = scp_upload_command(remote_destination, file_name, size);
    let mut channel = handle
        .open_exec_channel(&command, SSH_AUXILIARY_SETUP_TIMEOUT, "SCP upload")
        .await?;

    let outcome = async {
        let mut output = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        let mut reported_total = None;
        let mut resume_candidate = None;
        let started = Instant::now();
        let mut copied = loop {
            if progress.cancel.load(Ordering::SeqCst) {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            if started.elapsed() > Duration::from_secs(30) {
                return Err("SCP upload 等待远端续传状态超时".to_string());
            }
            match tokio::time::timeout(Duration::from_millis(250), channel.wait()).await {
                Ok(Some(SshBackendMessage::Data(data))) => {
                    append_bounded_ssh_exec_data(
                        &mut output,
                        &data,
                        MAX_SSH_EXEC_STDOUT_BYTES,
                        "SCP upload stdout",
                    )?;
                    let markers = remote_copy_markers(&output);
                    if let Some(total) = markers.total {
                        if total != size {
                            return Err(format!(
                                "SCP upload remote size marker {total} does not match local size {size}"
                            ));
                        }
                        if reported_total != Some(total) {
                            progress.update(0, total).await?;
                            reported_total = Some(total);
                        }
                    }
                    if let Some(candidate) = markers.resume_candidate {
                        if reported_total != Some(size) {
                            return Err(
                                "SCP upload resume candidate arrived before the size marker"
                                    .to_string(),
                            );
                        }
                        if candidate > size {
                            return Err(format!(
                                "SCP upload resume candidate {candidate} exceeds local size {size}"
                            ));
                        }
                        match resume_candidate {
                            Some(previous) if previous != candidate => {
                                return Err(format!(
                                    "SCP upload resume candidate changed from {previous} to {candidate}"
                                ));
                            }
                            Some(_) => {}
                            None => {
                                let digest =
                                    scp_source_prefix_sha256(&mut file, candidate, progress)?;
                                let marker = format!(
                                    "__PORTMATE_PREFIX_SHA256__{digest}\n"
                                );
                                scp_send_data_with_idle_timeout(
                                    &channel,
                                    marker.as_bytes(),
                                    progress,
                                    SCP_IO_IDLE_TIMEOUT,
                                    "SCP 写入续传前缀校验",
                                )
                                .await?;
                                resume_candidate = Some(candidate);
                            }
                        }
                    }
                    if let Some(resume) = markers.resume {
                        if reported_total != Some(size) {
                            return Err(
                                "SCP upload resume marker arrived before the size marker"
                                    .to_string(),
                            );
                        }
                        if resume > size {
                            return Err(format!(
                                "SCP upload resume marker {resume} exceeds local size {size}"
                            ));
                        }
                        match resume_candidate {
                            Some(candidate) if resume == 0 || resume == candidate => {}
                            Some(candidate) => {
                                return Err(format!(
                                    "SCP upload resume marker {resume} did not match verified candidate {candidate}"
                                ));
                            }
                            None if resume == 0 => {}
                            None => {
                                return Err(format!(
                                    "SCP upload accepted unverified resume marker {resume}"
                                ));
                            }
                        }
                        progress.set_rate_baseline(resume);
                        if resume > 0 {
                            progress.update(resume, size).await?;
                        }
                        break resume;
                    }
                }
                Ok(Some(SshBackendMessage::ExtendedData { data, .. })) => append_bounded_ssh_exec_data(
                    &mut stderr,
                    &data,
                    MAX_SSH_EXEC_STDERR_BYTES,
                    "SCP upload stderr",
                )?,
                Ok(Some(SshBackendMessage::ExitStatus(code))) => exit_status = Some(code),
                Ok(Some(SshBackendMessage::Eof | SshBackendMessage::Close)) | Ok(None) => {
                    return Err(format!(
                        "SCP upload remote closed before resume marker: {}{}",
                        String::from_utf8_lossy(&output),
                        String::from_utf8_lossy(&stderr)
                    ));
                }
                Ok(Some(_)) => {}
                Err(_) => {}
            }
            if let Some(code) = exit_status.filter(|code| *code != 0) {
                return Err(format!(
                    "SCP upload remote returned non-zero before upload {code}: {}",
                    String::from_utf8_lossy(&stderr)
                ));
            }
        };

        if copied < size {
            file.seek(std::io::SeekFrom::Start(copied))
                .map_err(|error| format!("SCP 定位本地续传偏移失败: {error}"))?;
        }
        let mut buffer = vec![0_u8; 64 * 1024];
        while copied < size {
            if progress.cancel.load(Ordering::SeqCst) {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            let read = file
                .read(&mut buffer)
                .map_err(|error| format!("读取本地文件失败: {error}"))?;
            if read == 0 {
                break;
            }
            scp_send_data_with_idle_timeout(
                &channel,
                &buffer[..read],
                progress,
                SCP_IO_IDLE_TIMEOUT,
                "SCP 写入文件内容",
            )
            .await?;
            copied += read as u64;
            progress.update(copied, size).await?;
        }
        ensure_exact_transfer_size(copied, size, "SCP upload")?;
        ensure_scp_source_has_not_grown(&mut file)?;
        match tokio::time::timeout(SCP_IO_IDLE_TIMEOUT, channel.eof()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Err(format!("SCP 写入 EOF 失败: {error}"));
            }
            Err(_) => {
                return Err(format!(
                    "SCP 写入 EOF 空闲超时（{} ms）",
                    SCP_IO_IDLE_TIMEOUT.as_millis()
                ));
            }
        }

        let started = Instant::now();
        let mut eof_received_at: Option<Instant> = None;
        loop {
            if progress.cancel.load(Ordering::SeqCst) {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            if ssh_exec_status_grace_expired(eof_received_at) {
                break;
            }
            if started.elapsed() > Duration::from_secs(300) {
                return Err("SCP upload 等待远端完成超时".to_string());
            }
            match tokio::time::timeout(Duration::from_millis(250), channel.wait()).await {
                Ok(Some(SshBackendMessage::Data(data))) => append_bounded_ssh_exec_data(
                    &mut output,
                    &data,
                    MAX_SSH_EXEC_STDOUT_BYTES,
                    "SCP upload stdout",
                )?,
                Ok(Some(SshBackendMessage::ExtendedData { data, .. })) => append_bounded_ssh_exec_data(
                    &mut stderr,
                    &data,
                    MAX_SSH_EXEC_STDERR_BYTES,
                    "SCP upload stderr",
                )?,
                Ok(Some(message)) => {
                    if ssh_exec_message_completes(
                        &message,
                        &mut exit_status,
                        &mut eof_received_at,
                    )
                    {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {}
            }
        }

        if let Some(code) = exit_status.filter(|code| *code != 0) {
            return Err(format!(
                "SCP upload remote returned non-zero {code}: {}",
                String::from_utf8_lossy(&stderr)
            ));
        }
        let markers = remote_copy_markers(&output);
        let done = markers.done.ok_or_else(|| {
            format!(
                "SCP upload completed but done marker was missing: {}",
                String::from_utf8_lossy(&output)
            )
        })?;
        if done != size {
            return Err(format!(
                "SCP upload size mismatch: remote done {done}, expected {size}"
            ));
        }
        progress.update(done, size).await?;
        Ok(done)
    }
    .await;
    close_ssh_channel_bounded(&channel).await;
    outcome
}

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

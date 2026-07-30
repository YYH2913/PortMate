use super::*;

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

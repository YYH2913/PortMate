use super::*;

pub(super) const REMOTE_COPY_IO_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const REMOTE_COPY_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);

pub(super) async fn remote_copy(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    remote_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    remote_copy_with_timeouts(
        handle,
        remote_source,
        remote_destination,
        progress,
        REMOTE_COPY_IO_IDLE_TIMEOUT,
        REMOTE_COPY_TOTAL_TIMEOUT,
    )
    .await
}

pub(super) async fn remote_copy_with_timeouts<H: SshExecChannelOpener>(
    handle: H,
    remote_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
    idle_timeout: Duration,
    total_timeout: Duration,
) -> Result<u64, String> {
    let command = remote_copy_command(remote_source, remote_destination);
    let mut channel = handle
        .open_exec_channel(&command, SSH_AUXILIARY_SETUP_TIMEOUT, "SSH remote copy")
        .await?;

    let mut output = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    let mut eof_received_at: Option<Instant> = None;
    let mut reported = RemoteCopyMarkers::default();
    let started = Instant::now();
    let mut last_progress = Instant::now();

    let outcome = async {
        loop {
            progress.check_cancelled()?;
            if ssh_exec_status_grace_expired(eof_received_at) {
                break;
            }
            let message = {
                let wait = channel.wait();
                tokio::pin!(wait);
                loop {
                    let idle_remaining = idle_timeout.saturating_sub(last_progress.elapsed());
                    if idle_remaining.is_zero() {
                        break Err(format!(
                            "SSH remote copy 空闲超时（{} ms）",
                            idle_timeout.as_millis()
                        ));
                    }
                    let total_remaining = total_timeout.saturating_sub(started.elapsed());
                    if total_remaining.is_zero() {
                        break Err(format!(
                            "SSH remote copy 总超时（{} ms）",
                            total_timeout.as_millis()
                        ));
                    }
                    tokio::select! {
                        message = &mut wait => break Ok(message),
                        _ = tokio::time::sleep(
                            idle_remaining
                                .min(total_remaining)
                                .min(TRANSFER_CANCEL_POLL_INTERVAL)
                        ) => {
                            progress.check_cancelled()?;
                        }
                    }
                }
            }?;

            match message {
                Some(SshBackendMessage::Data(data)) => {
                    append_bounded_ssh_exec_data(
                        &mut output,
                        &data,
                        MAX_SSH_EXEC_STDOUT_BYTES,
                        "remote copy stdout",
                    )?;
                    let markers = remote_copy_markers(&output);
                    validate_remote_copy_markers(&markers, &reported)?;
                    let made_progress = markers != reported;
                    if markers.total.is_some() && markers.total != reported.total {
                        let total = markers.total.unwrap_or_default();
                        progress.update(0, total).await?;
                        reported.total = Some(total);
                    }
                    if markers.resume.is_some() && markers.resume != reported.resume {
                        let resume_bytes = markers.resume.unwrap_or_default();
                        progress.set_rate_baseline(resume_bytes);
                        progress
                            .update(resume_bytes, markers.total.or(reported.total).unwrap_or(0))
                            .await?;
                        reported.resume = Some(resume_bytes);
                    }
                    if markers.progress.is_some() && markers.progress != reported.progress {
                        let progress_bytes = markers.progress.unwrap_or_default();
                        progress
                            .update(
                                progress_bytes,
                                markers.total.or(reported.total).unwrap_or(0),
                            )
                            .await?;
                        reported.progress = Some(progress_bytes);
                    }
                    if markers.done.is_some() && markers.done != reported.done {
                        let done = markers.done.unwrap_or_default();
                        progress
                            .update(done, reported.total.unwrap_or(done))
                            .await?;
                        reported.done = Some(done);
                    }
                    if made_progress {
                        last_progress = Instant::now();
                    }
                }
                Some(SshBackendMessage::ExtendedData { data, .. }) => append_bounded_ssh_exec_data(
                    &mut stderr,
                    &data,
                    MAX_SSH_EXEC_STDERR_BYTES,
                    "remote copy stderr",
                )?,
                Some(message) => {
                    if ssh_exec_message_completes(&message, &mut exit_status, &mut eof_received_at)
                    {
                        break;
                    }
                }
                None => break,
            }
        }

        if let Some(code) = exit_status.filter(|code| *code != 0) {
            return Err(format!(
                "SSH remote copy 返回非零状态 {code}: {}",
                String::from_utf8_lossy(&stderr)
            ));
        }

        let markers = remote_copy_markers(&output);
        let bytes = markers.done.ok_or_else(|| {
            format!(
                "remote copy completed but done marker was missing: {}",
                String::from_utf8_lossy(&output)
            )
        })?;
        progress
            .update(bytes, reported.total.unwrap_or(bytes))
            .await?;
        Ok(bytes)
    }
    .await;
    close_ssh_channel_bounded(&channel).await;
    outcome
}

pub(super) fn remote_copy_command(remote_source: &str, remote_destination: &str) -> String {
    format!(
        concat!(
            "src={}; dst={}; target=; part=; pid=; ",
            "remote_name=${{src##*/}}; if [ -z \"$remote_name\" ]; then remote_name=portmate-file.bin; fi; ",
            "case \"$dst\" in */) target=\"${{dst%/}}/$remote_name\" ;; ",
            "*) if [ -d \"$dst\" ]; then target=\"${{dst%/}}/$remote_name\"; else target=\"$dst\"; fi ;; esac; ",
            "case \"$target\" in */*) part=\"${{target%/*}}/${{target##*/}}.portmate-part\" ;; ",
            "*) part=\"$target.portmate-part\" ;; esac; ",
            "portable_path() {{ case \"$1\" in -*) printf './%s\\n' \"$1\" ;; *) printf '%s\\n' \"$1\" ;; esac; }}; ",
            "src=$(portable_path \"$src\") || exit 1; target=$(portable_path \"$target\") || exit 1; part=$(portable_path \"$part\") || exit 1; ",
            "reject_link() {{ if [ -L \"$1\" ]; then printf 'PortMate refuses symbolic link: %s\\n' \"$1\" >&2; return 1; fi; }}; ",
            "file_size() {{ value=$(wc -c < \"$1\") || return 1; value=$(printf '%s' \"$value\" | tr -d '[:space:]') || return 1; case \"$value\" in ''|*[!0-9]*) return 1 ;; esac; printf '%s\\n' \"$value\"; }}; ",
            "cleanup() {{ if [ -n \"$pid\" ]; then kill \"$pid\" 2>/dev/null || :; fi; }}; ",
            "trap cleanup INT TERM HUP EXIT; ",
            "if ! reject_link \"$src\" || [ ! -f \"$src\" ]; then exit 1; fi; ",
            "if ! reject_link \"$part\" || ! reject_link \"$target\"; then exit 1; fi; ",
            "if ! total=$(file_size \"$src\"); then exit 1; fi; ",
            "printf '__PORTMATE_SIZE__%s\\n' \"$total\"; ",
            "offset=0; ",
            "if [ -e \"$part\" ]; then ",
            "if current=$(file_size \"$part\" 2>/dev/null); then ",
            "if [ \"$current\" -le \"$total\" ]; then ",
            "if [ \"$current\" -eq 0 ] || head -c \"$current\" \"$src\" | cmp -s - \"$part\"; then offset=$current; else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "printf '__PORTMATE_RESUME__%s\\n' \"$offset\"; ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$offset\"; ",
            "if [ \"$offset\" -lt \"$total\" ]; then ",
            "tail -c +$((offset + 1)) \"$src\" >> \"$part\" & pid=$!; ",
            "while kill -0 \"$pid\" 2>/dev/null; do ",
            "if current=$(file_size \"$part\" 2>/dev/null); then ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$current\"; ",
            "fi; sleep 0.25; done; ",
            "wait \"$pid\"; status=$?; pid=; ",
            "if [ \"$status\" -ne 0 ]; then exit \"$status\"; fi; ",
            "fi; ",
            "final=$(file_size \"$part\") || exit 1; ",
            "if [ \"$final\" -ne \"$total\" ]; then ",
            "printf 'PortMate remote copy size mismatch: %s of %s\\n' \"$final\" \"$total\" >&2; exit 1; ",
            "fi; ",
            "if ! reject_link \"$part\" || ! reject_link \"$target\"; then exit 1; fi; ",
            "mv -f \"$part\" \"$target\" || exit 1; ",
            "final_target=$(file_size \"$target\") || exit 1; printf '__PORTMATE_DONE__%s\\n' \"$final_target\""
        ),
        shell_quote(remote_source),
        shell_quote(remote_destination)
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RemoteCopyMarkers {
    pub(super) total: Option<u64>,
    pub(super) resume_candidate: Option<u64>,
    pub(super) resume: Option<u64>,
    pub(super) progress: Option<u64>,
    pub(super) done: Option<u64>,
}

pub(super) fn validate_remote_copy_markers(
    markers: &RemoteCopyMarkers,
    reported: &RemoteCopyMarkers,
) -> Result<(), String> {
    if let (Some(previous), Some(current)) = (reported.total, markers.total) {
        if current != previous {
            return Err(format!(
                "SSH remote copy size marker changed from {previous} to {current}"
            ));
        }
    }
    if let (Some(previous), Some(current)) = (reported.resume, markers.resume) {
        if current != previous {
            return Err(format!(
                "SSH remote copy resume marker changed from {previous} to {current}"
            ));
        }
    }
    if let (Some(previous), Some(current)) = (reported.progress, markers.progress) {
        if current < previous {
            return Err(format!(
                "SSH remote copy progress marker moved backwards from {previous} to {current}"
            ));
        }
    }
    if let (Some(previous), Some(current)) = (reported.done, markers.done) {
        if current != previous {
            return Err(format!(
                "SSH remote copy done marker changed from {previous} to {current}"
            ));
        }
    }

    let total = markers.total.or(reported.total);
    for (label, value) in [
        ("resume", markers.resume),
        ("progress", markers.progress),
        ("done", markers.done),
    ] {
        let Some(value) = value else {
            continue;
        };
        let total = total.ok_or_else(|| {
            format!("SSH remote copy {label} marker arrived before the size marker")
        })?;
        if value > total {
            return Err(format!(
                "SSH remote copy {label} marker {value} exceeds size {total}"
            ));
        }
    }
    if let (Some(total), Some(done)) = (total, markers.done) {
        if done != total {
            return Err(format!(
                "SSH remote copy done marker {done} does not match size {total}"
            ));
        }
    }
    Ok(())
}

pub(super) fn remote_copy_markers(output: &[u8]) -> RemoteCopyMarkers {
    let text = String::from_utf8_lossy(output);
    let mut markers = RemoteCopyMarkers::default();
    for line in text.split_inclusive('\n') {
        if !line.ends_with('\n') {
            continue;
        }
        if let Some(value) = line.trim().strip_prefix("__PORTMATE_SIZE__") {
            markers.total = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_RESUME_CANDIDATE__") {
            markers.resume_candidate = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_RESUME__") {
            markers.resume = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_PROGRESS__") {
            markers.progress = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_DONE__") {
            markers.done = value.trim().parse::<u64>().ok();
        }
    }
    markers
}

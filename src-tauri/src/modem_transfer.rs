use super::*;

pub(super) fn runtime_tap_receiver(
    state: &AppState,
    session_id: &str,
) -> Result<broadcast::Receiver<Vec<u8>>, String> {
    if let Some(tap) = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    Err("需要先连接会话才能执行 X/Y/ZModem 传输".to_string())
}

pub(super) const MODEM_SOH: u8 = 0x01;
pub(super) const MODEM_STX: u8 = 0x02;
pub(super) const MODEM_EOT: u8 = 0x04;
pub(super) const MODEM_ACK: u8 = 0x06;
pub(super) const MODEM_NAK: u8 = 0x15;
pub(super) const MODEM_CAN: u8 = 0x18;
pub(super) const MODEM_CRC_REQUEST: u8 = b'C';
pub(super) const MODEM_EOF: u8 = 0x1a;
pub(super) const MODEM_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const MODEM_ACK_TIMEOUT: Duration = Duration::from_secs(12);
pub(super) const REMOTE_MODEM_READY_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const MODEM_MAX_RETRIES: usize = 10;
pub(super) const XMODEM_BLOCK_SIZE: usize = 128;
pub(super) const YMODEM_BLOCK_SIZE: usize = 1024;

pub(super) async fn transfer_file_via_xmodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let source_size = local_transfer_source_size(&local_source)?;
            let remote_part = remote_resume_part_path(&remote_destination);
            let completion_token = Uuid::new_v4().simple().to_string();
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Xmodem,
                true,
                &remote_part,
            )
            .await?;
            let remote_started = remote_start.is_some();
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let bytes = xmodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                remote_started,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
                let command = xmodem_remote_finalize_command(
                    &remote_part,
                    &remote_destination,
                    source_size,
                    &completion_token,
                );
                let _ = send_text_inner(state.session_io(), request.session_id.clone(), command)
                    .await?;
                wait_for_xmodem_remote_completion(
                    &mut completion_receiver,
                    &completion_token,
                    progress,
                    &request.session_id,
                )
                .await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Xmodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let bytes = xmodem_receive_file(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
            }
            Ok(bytes)
        }
    }
}

pub(super) async fn transfer_file_via_ymodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_part = remote_resume_part_path(&remote_destination);
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Ymodem,
                true,
                &remote_part,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let receiver_destination = if remote_start.is_some() {
                remote_part.as_str()
            } else {
                remote_destination.as_str()
            };
            let bytes = ymodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                Some(receiver_destination),
                remote_start.is_some(),
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
                finalize_remote_modem_upload(
                    state,
                    &request.session_id,
                    &mut completion_receiver,
                    &remote_part,
                    &remote_destination,
                    progress,
                )
                .await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Ymodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let bytes = ymodem_receive_file(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
            }
            Ok(bytes)
        }
    }
}

pub(super) async fn transfer_file_via_zmodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_part = remote_resume_part_path(&remote_destination);
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Zmodem,
                true,
                &remote_part,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let receiver_destination = if remote_start.is_some() {
                remote_part.as_str()
            } else {
                remote_destination.as_str()
            };
            let bytes = zmodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                Some(receiver_destination),
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
                finalize_remote_modem_upload(
                    state,
                    &request.session_id,
                    &mut completion_receiver,
                    &remote_part,
                    &remote_destination,
                    progress,
                )
                .await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Zmodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let bytes = zmodem_receive_files(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
            }
            Ok(bytes)
        }
    }
}

pub(super) enum ModemDirection {
    Upload {
        local_source: String,
        remote_destination: String,
    },
    Download {
        remote_source: String,
        local_destination: String,
    },
}

pub(super) fn modem_direction(request: &StartTransferRequest) -> Result<ModemDirection, String> {
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);

    match (source_remote, destination_remote) {
        (None, Some(remote_destination)) => {
            validate_remote_transfer_path(remote_destination, "Modem 远端目标路径")?;
            if local_transfer_entry(Path::new(&request.source), "本地传输源")?.is_none() {
                return Err("本地传输源不存在".to_string());
            }
            Ok(ModemDirection::Upload {
                local_source: request.source.clone(),
                remote_destination: remote_destination.to_string(),
            })
        }
        (Some(remote_source), None) => {
            validate_remote_transfer_path(remote_source, "Modem 远端源路径")?;
            Ok(ModemDirection::Download {
                remote_source: remote_source.to_string(),
                local_destination: request.destination.clone(),
            })
        }
        (None, None) => {
            if local_transfer_entry(Path::new(&request.source), "本地传输源")?.is_some() {
                validate_remote_transfer_path(&request.destination, "Modem 远端目标路径")?;
                Ok(ModemDirection::Upload {
                    local_source: request.source.clone(),
                    remote_destination: request.destination.clone(),
                })
            } else {
                Err("Modem transfer expects local -> remote:path upload or remote:path -> local download".to_string())
            }
        }
        _ => Err(
            "Modem transfer expects local -> remote:path upload or remote:path -> local download"
                .to_string(),
        ),
    }
}

pub(super) struct RemoteModemStart {
    token: String,
    ready_marker: String,
}

impl RemoteModemStart {
    fn success_marker(&self) -> String {
        format!("__PORTMATE_MODEM_{}_DONE__", self.token)
    }

    fn failure_marker(&self) -> String {
        format!("__PORTMATE_MODEM_{}_FAIL__", self.token)
    }
}

pub(super) async fn maybe_start_remote_modem(
    state: &AppState,
    session_id: &str,
    protocol: TransferProtocol,
    upload: bool,
    remote_path: &str,
) -> Result<Option<RemoteModemStart>, String> {
    if !remote_modem_auto_start_enabled(state, session_id)? {
        return Ok(None);
    }

    let readiness_token = Uuid::new_v4().simple().to_string();
    let command = modem_remote_command(protocol, upload, remote_path, &readiness_token);
    let _ = send_text_inner(state.session_io(), session_id.to_string(), command).await?;
    Ok(Some(RemoteModemStart {
        ready_marker: format!("__PORTMATE_MODEM_{readiness_token}_READY__"),
        token: readiness_token,
    }))
}

pub(super) fn remote_modem_auto_start_enabled(
    state: &AppState,
    session_id: &str,
) -> Result<bool, String> {
    let profile = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store.profile(session_id)
    }
    .ok_or_else(|| format!("unknown session: {session_id}"))?;
    Ok(matches!(
        profile.kind,
        SessionKind::Ssh | SessionKind::Tmux | SessionKind::Shell | SessionKind::Telnet
    ))
}

pub(super) fn xmodem_remote_finalize_command(
    remote_part: &str,
    remote_target: &str,
    source_size: u64,
    completion_token: &str,
) -> String {
    format!(
        concat!(
            "part={}; target={}; portmate_status=0; ",
            "portable_path() {{ case \"$1\" in -*) printf './%s\\n' \"$1\" ;; *) printf '%s\\n' \"$1\" ;; esac; }}; ",
            "part=$(portable_path \"$part\") || exit 1; ",
            "target=$(portable_path \"$target\") || exit 1; ",
            "if command -v truncate >/dev/null 2>&1 && truncate -s {} \"$part\"; then :; ",
            "else trim=\"$part.portmate-trim\"; ",
            "dd if=\"$part\" of=\"$trim\" bs=1 count={} 2>/dev/null ",
            "&& mv -f \"$trim\" \"$part\"; portmate_status=$?; ",
            "if [ \"$portmate_status\" -ne 0 ]; then rm -f \"$trim\"; fi; fi; ",
            "if [ \"$portmate_status\" -eq 0 ]; then mv -f \"$part\" \"$target\"; portmate_status=$?; fi; ",
            "if [ \"$portmate_status\" -eq 0 ]; then ",
            "printf '\\n__PORTMATE_XMODEM_%s_DONE__\\n' {}; ",
            "else printf '\\n__PORTMATE_XMODEM_%s_FAIL__%s\\n' {} \"$portmate_status\"; fi\r"
        ),
        shell_quote(remote_part),
        shell_quote(remote_target),
        source_size,
        source_size,
        shell_quote(completion_token),
        shell_quote(completion_token),
    )
}

pub(super) fn remote_modem_finalize_command(
    remote_part: &str,
    remote_target: &str,
    completion_token: &str,
) -> String {
    format!(
        concat!(
            "part={}; target={}; ",
            "portable_path() {{ case \"$1\" in -*) printf './%s\\n' \"$1\" ;; *) printf '%s\\n' \"$1\" ;; esac; }}; ",
            "part=$(portable_path \"$part\") || exit 1; ",
            "target=$(portable_path \"$target\") || exit 1; ",
            "if mv -f \"$part\" \"$target\"; then ",
            "printf '\\n__PORTMATE_MODEM_FINALIZE_%s_DONE__\\n' {}; ",
            "else portmate_status=$?; ",
            "printf '\\n__PORTMATE_MODEM_FINALIZE_%s_FAIL__%s\\n' {} \"$portmate_status\"; fi\r"
        ),
        shell_quote(remote_part),
        shell_quote(remote_target),
        shell_quote(completion_token),
        shell_quote(completion_token),
    )
}

pub(super) async fn finalize_remote_modem_upload(
    state: &AppState,
    session_id: &str,
    receiver: &mut broadcast::Receiver<Vec<u8>>,
    remote_part: &str,
    remote_target: &str,
    progress: &TransferProgressContext,
) -> Result<(), String> {
    let completion_token = Uuid::new_v4().simple().to_string();
    let command = remote_modem_finalize_command(remote_part, remote_target, &completion_token);
    let _ = send_text_inner(state.session_io(), session_id.to_string(), command).await?;
    wait_for_remote_modem_finalize(receiver, &completion_token, progress, session_id).await
}

pub(super) async fn wait_for_remote_modem_finalize(
    receiver: &mut broadcast::Receiver<Vec<u8>>,
    completion_token: &str,
    progress: &TransferProgressContext,
    session_id: &str,
) -> Result<(), String> {
    let success = format!("__PORTMATE_MODEM_FINALIZE_{completion_token}_DONE__");
    let failure = format!("__PORTMATE_MODEM_FINALIZE_{completion_token}_FAIL__");
    let started = Instant::now();
    let mut output = Vec::new();
    loop {
        progress.check_cancelled()?;
        ensure_modem_session_connected(&progress.state.store, session_id)?;
        let remaining = Duration::from_secs(15).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("remote modem finalize timed out".to_string());
        }
        match tokio::time::timeout(remaining.min(MODEM_CANCEL_POLL_INTERVAL), receiver.recv()).await
        {
            Ok(Ok(bytes)) => {
                output.extend_from_slice(&bytes);
                if output
                    .windows(success.len())
                    .any(|window| window == success.as_bytes())
                {
                    return Ok(());
                }
                if output
                    .windows(failure.len())
                    .any(|window| window == failure.as_bytes())
                {
                    return Err(format!(
                        "remote modem finalize failed: {}",
                        String::from_utf8_lossy(&output)
                    ));
                }
                if output.len() > 64 * 1024 {
                    let keep = success.len().max(failure.len()).saturating_sub(1);
                    output.drain(..output.len().saturating_sub(keep));
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err("remote modem finalize stream closed".to_string())
            }
            Err(_) => {}
        }
    }
}

pub(super) async fn wait_for_xmodem_remote_completion(
    receiver: &mut broadcast::Receiver<Vec<u8>>,
    completion_token: &str,
    progress: &TransferProgressContext,
    session_id: &str,
) -> Result<(), String> {
    let success = format!("__PORTMATE_XMODEM_{completion_token}_DONE__");
    let failure = format!("__PORTMATE_XMODEM_{completion_token}_FAIL__");
    let started = Instant::now();
    let mut output = Vec::new();
    loop {
        progress.check_cancelled()?;
        ensure_modem_session_connected(&progress.state.store, session_id)?;
        let remaining = Duration::from_secs(15).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("XModem remote finalize timed out".to_string());
        }
        match tokio::time::timeout(remaining.min(MODEM_CANCEL_POLL_INTERVAL), receiver.recv()).await
        {
            Ok(Ok(bytes)) => {
                output.extend_from_slice(&bytes);
                if output
                    .windows(success.len())
                    .any(|window| window == success.as_bytes())
                {
                    return Ok(());
                }
                if output
                    .windows(failure.len())
                    .any(|window| window == failure.as_bytes())
                {
                    return Err(format!(
                        "XModem remote finalize failed: {}",
                        String::from_utf8_lossy(&output)
                    ));
                }
                if output.len() > 64 * 1024 {
                    let keep = success.len().max(failure.len()).saturating_sub(1);
                    output.drain(..output.len().saturating_sub(keep));
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err("XModem remote finalize stream closed".to_string())
            }
            Err(_) => {}
        }
    }
}

pub(super) async fn wait_for_remote_modem_completion(
    receiver: &mut broadcast::Receiver<Vec<u8>>,
    remote_start: &RemoteModemStart,
    progress: &TransferProgressContext,
    session_id: &str,
) -> Result<(), String> {
    let success = remote_start.success_marker();
    let failure = remote_start.failure_marker();
    let started = Instant::now();
    let mut output = Vec::new();
    loop {
        progress.check_cancelled()?;
        ensure_modem_session_connected(&progress.state.store, session_id)?;
        let remaining = Duration::from_secs(15).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("remote modem command completion timed out".to_string());
        }
        match tokio::time::timeout(remaining.min(MODEM_CANCEL_POLL_INTERVAL), receiver.recv()).await
        {
            Ok(Ok(bytes)) => {
                output.extend_from_slice(&bytes);
                if output
                    .windows(success.len())
                    .any(|window| window == success.as_bytes())
                {
                    return Ok(());
                }
                if output
                    .windows(failure.len())
                    .any(|window| window == failure.as_bytes())
                {
                    return Err(format!(
                        "remote modem command failed: {}",
                        String::from_utf8_lossy(&output)
                    ));
                }
                if output.len() > 64 * 1024 {
                    let keep = success.len().max(failure.len()).saturating_sub(1);
                    output.drain(..output.len().saturating_sub(keep));
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err("remote modem command completion stream closed".to_string())
            }
            Err(_) => {}
        }
    }
}

pub(super) fn modem_remote_command(
    protocol: TransferProtocol,
    upload: bool,
    remote_path: &str,
    readiness_token: &str,
) -> String {
    match (protocol, upload) {
        (TransferProtocol::Xmodem, true) => {
            let target = modem_shell_path(remote_path);
            format!(
                "{}\r",
                modem_raw_tty_shell_command(
                    &format!("rm -f {target} && rx {target}"),
                    readiness_token,
                )
            )
        }
        (TransferProtocol::Xmodem, false) => format!(
            "{}\r",
            modem_raw_tty_shell_command(
                &format!("sx {}", modem_shell_path(remote_path)),
                readiness_token,
            )
        ),
        (TransferProtocol::Ymodem, true) => {
            let (parent, _) = remote_parent_and_file_name(remote_path);
            if parent.is_empty() {
                format!(
                    "{}\r",
                    modem_raw_tty_shell_command("rb -y", readiness_token)
                )
            } else {
                format!(
                    "mkdir -p {} && cd {} && {}\r",
                    shell_quote(&parent),
                    shell_quote(&parent),
                    modem_raw_tty_shell_command("rb -y", readiness_token)
                )
            }
        }
        (TransferProtocol::Ymodem, false) => format!(
            "{}\r",
            modem_raw_tty_shell_command(
                &format!("sb {}", modem_shell_path(remote_path)),
                readiness_token,
            )
        ),
        (TransferProtocol::Zmodem, true) => {
            let (parent, _) = remote_parent_and_file_name(remote_path);
            if parent.is_empty() {
                format!(
                    "{}\r",
                    modem_raw_tty_shell_command("rz -y", readiness_token)
                )
            } else {
                format!(
                    "mkdir -p {} && cd {} && {}\r",
                    shell_quote(&parent),
                    shell_quote(&parent),
                    modem_raw_tty_shell_command("rz -y", readiness_token)
                )
            }
        }
        (TransferProtocol::Zmodem, false) => format!(
            "{}\r",
            modem_raw_tty_shell_command(
                &format!("sz {}", modem_shell_path(remote_path)),
                readiness_token,
            )
        ),
        _ => String::new(),
    }
}

fn modem_shell_path(path: &str) -> String {
    if path.starts_with('-') {
        shell_quote(&format!("./{path}"))
    } else {
        shell_quote(path)
    }
}

pub(super) fn modem_raw_tty_shell_command(command: &str, readiness_token: &str) -> String {
    format!(
        concat!(
            "{{ portmate_stty=0; ",
            "if command -v stty >/dev/null 2>&1; then ",
            "stty raw -echo; portmate_stty=1; fi; ",
            "printf '__PORTMATE_MODEM_%s_READY__' {}; ",
            "{}; portmate_modem_status=$?; ",
            "if [ \"$portmate_stty\" -eq 1 ]; then stty sane; fi; ",
            "if [ \"$portmate_modem_status\" -eq 0 ]; then ",
            "printf '\\n__PORTMATE_MODEM_%s_DONE__\\n' {}; ",
            "else printf '\\n__PORTMATE_MODEM_%s_FAIL__%s\\n' {} ",
            "\"$portmate_modem_status\"; fi; ",
            "(exit \"$portmate_modem_status\"); }}"
        ),
        shell_quote(readiness_token),
        command,
        shell_quote(readiness_token),
        shell_quote(readiness_token),
    )
}
pub(super) async fn check_modem_cancelled(
    state: &AppState,
    session_id: &str,
    progress: &TransferProgressContext,
) -> Result<(), String> {
    if progress.cancel.load(Ordering::SeqCst) {
        let _ = write_runtime_bytes(state, session_id, &[MODEM_CAN, MODEM_CAN, MODEM_CAN]).await;
        Err(TRANSFER_CANCELLED_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

pub(super) struct ModemByteReader {
    receiver: broadcast::Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
    cancel: Arc<AtomicBool>,
    connection: Option<(Arc<Mutex<SessionStore>>, String)>,
}

impl ModemByteReader {
    pub(super) fn new(receiver: broadcast::Receiver<Vec<u8>>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            receiver,
            pending: VecDeque::new(),
            cancel,
            connection: None,
        }
    }

    pub(super) fn watch_connection(
        mut self,
        store: Arc<Mutex<SessionStore>>,
        session_id: String,
    ) -> Self {
        self.connection = Some((store, session_id));
        self
    }

    pub(super) fn check_interrupted(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
        }
        if let Some((store, session_id)) = &self.connection {
            ensure_modem_session_connected(store, session_id)?;
        }
        Ok(())
    }

    pub(super) async fn after_marker(
        mut receiver: broadcast::Receiver<Vec<u8>>,
        marker: &str,
        cancel: Arc<AtomicBool>,
        connection: Option<(Arc<Mutex<SessionStore>>, String)>,
    ) -> Result<Self, String> {
        let started = Instant::now();
        let marker = marker.as_bytes();
        let mut buffered = Vec::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            if let Some((store, session_id)) = &connection {
                ensure_modem_session_connected(store, session_id)?;
            }
            let remaining = REMOTE_MODEM_READY_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("remote modem readiness marker timed out".to_string());
            }
            match tokio::time::timeout(remaining.min(MODEM_CANCEL_POLL_INTERVAL), receiver.recv())
                .await
            {
                Ok(Ok(bytes)) => {
                    buffered.extend_from_slice(&bytes);
                    if let Some(offset) = buffered
                        .windows(marker.len())
                        .position(|window| window == marker)
                    {
                        return Ok(Self {
                            receiver,
                            pending: buffered[offset + marker.len()..].iter().copied().collect(),
                            cancel,
                            connection,
                        });
                    }
                    if buffered.len() > 64 * 1024 {
                        let keep = marker.len().saturating_sub(1);
                        buffered.drain(..buffered.len().saturating_sub(keep));
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("remote modem readiness stream closed".to_string())
                }
                Err(_) => {}
            }
        }
    }

    pub(super) async fn next_byte(&mut self, timeout: Duration) -> Result<u8, String> {
        let started = Instant::now();
        loop {
            self.check_interrupted()?;
            if let Some(byte) = self.pending.pop_front() {
                return Ok(byte);
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("modem byte timeout".to_string());
            }
            match tokio::time::timeout(
                remaining.min(MODEM_CANCEL_POLL_INTERVAL),
                self.receiver.recv(),
            )
            .await
            {
                Ok(Ok(bytes)) => self.pending.extend(bytes),
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("modem byte stream closed".to_string())
                }
                Err(_) => {}
            }
        }
    }

    pub(super) async fn read_exact(
        &mut self,
        len: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(len);
        while bytes.len() < len {
            bytes.push(self.next_byte(timeout).await?);
        }
        Ok(bytes)
    }

    pub(super) async fn next_chunk(
        &mut self,
        timeout: Duration,
        max_len: usize,
    ) -> Result<Vec<u8>, String> {
        self.check_interrupted()?;
        if !self.pending.is_empty() {
            let take = self.pending.len().min(max_len);
            return Ok(self.pending.drain(..take).collect());
        }

        let started = Instant::now();
        loop {
            self.check_interrupted()?;
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("modem byte timeout".to_string());
            }
            match tokio::time::timeout(
                remaining.min(MODEM_CANCEL_POLL_INTERVAL),
                self.receiver.recv(),
            )
            .await
            {
                Ok(Ok(bytes)) => {
                    if bytes.len() <= max_len {
                        return Ok(bytes);
                    }
                    self.pending.extend(bytes[max_len..].iter().copied());
                    return Ok(bytes[..max_len].to_vec());
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("modem byte stream closed".to_string())
                }
                Err(_) => {}
            }
        }
    }
}

pub(super) async fn modem_reader_after_start(
    receiver: broadcast::Receiver<Vec<u8>>,
    remote_start: Option<&RemoteModemStart>,
    progress: &TransferProgressContext,
    session_id: &str,
) -> Result<ModemByteReader, String> {
    let connection = Some((Arc::clone(&progress.state.store), session_id.to_string()));
    match remote_start {
        Some(start) => {
            ModemByteReader::after_marker(
                receiver,
                &start.ready_marker,
                Arc::clone(&progress.cancel),
                connection,
            )
            .await
        }
        None => Ok(ModemByteReader::new(receiver, Arc::clone(&progress.cancel))
            .watch_connection(Arc::clone(&progress.state.store), session_id.to_string())),
    }
}

pub(super) fn ensure_modem_session_connected(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<(), String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    let status = store
        .runtimes
        .iter()
        .find(|runtime| runtime.session_id == session_id)
        .map(|runtime| runtime.status)
        .ok_or_else(|| format!("modem session runtime missing: {session_id}"))?;
    if status == SessionStatus::Connected {
        Ok(())
    } else {
        Err(format!("modem session disconnected ({status:?})"))
    }
}

pub(super) async fn zmodem_send_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    remote_destination: Option<&str>,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut file, total) = open_local_transfer_source(Path::new(local_source), "ZModem")?;
    let size = u32::try_from(total)
        .map_err(|_| "ZModem 当前状态机只支持 4 GiB 以内的单文件".to_string())?;
    let (_, remote_name) = remote_destination
        .map(remote_parent_and_file_name)
        .unwrap_or_else(|| ("".to_string(), local_file_name(local_source)));
    let file_name = if remote_name.is_empty() {
        local_file_name(local_source)
    } else {
        remote_name
    };

    let mut sender =
        zmodem2::Sender::new().map_err(|error| format!("ZModem sender 初始化失败: {error}"))?;
    sender
        .start_file(file_name.as_bytes(), size)
        .map_err(|error| format!("ZModem 文件发送启动失败: {error}"))?;

    let mut input_buf = Vec::<u8>::new();
    let mut input_offset = 0_usize;
    let mut file_buf = vec![0_u8; 1024];
    let mut session_done = false;
    let mut last_progress = Instant::now();
    let mut bytes_done = 0_u64;

    while !session_done || !sender.drain_outgoing().is_empty() {
        check_modem_cancelled(state, session_id, progress).await?;
        let mut progressed = false;

        let outgoing = sender.drain_outgoing().to_vec();
        if !outgoing.is_empty() {
            write_runtime_bytes(state, session_id, &outgoing).await?;
            sender.advance_outgoing(outgoing.len());
            progressed = true;
        }

        if let Some(request) = sender.poll_file() {
            file.seek(std::io::SeekFrom::Start(u64::from(request.offset)))
                .map_err(|error| format!("ZModem 本地文件 seek 失败: {error}"))?;
            let read_len = request.len.min(file_buf.len());
            let read = file
                .read(&mut file_buf[..read_len])
                .map_err(|error| format!("ZModem 读取本地文件失败: {error}"))?;
            if read == 0 && request.len > 0 {
                return Err("ZModem 本地文件提前结束".to_string());
            }
            sender
                .feed_file(&file_buf[..read])
                .map_err(|error| format!("ZModem 发送文件块失败: {error}"))?;
            bytes_done = bytes_done.max(u64::from(request.offset) + read as u64);
            progress
                .update(bytes_done.min(u64::from(size)), u64::from(size))
                .await?;
            progressed = true;
        }

        match reader.next_chunk(Duration::from_millis(30), 4096).await {
            Ok(bytes) if !bytes.is_empty() => {
                input_buf.extend_from_slice(&bytes);
                progressed = true;
            }
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }

        if sender.drain_outgoing().is_empty() && input_offset < input_buf.len() {
            let consumed = sender
                .feed_incoming(&input_buf[input_offset..])
                .map_err(|error| format!("ZModem 接收远端响应失败: {error}"))?;
            if consumed > 0 {
                input_offset += consumed;
                progressed = true;
                if input_offset == input_buf.len() {
                    input_buf.clear();
                    input_offset = 0;
                } else if input_offset > 4096 {
                    input_buf.drain(..input_offset);
                    input_offset = 0;
                }
            }
        }

        if let Some(event) = sender.poll_event() {
            match event {
                zmodem2::SenderEvent::FileComplete => {
                    sender
                        .finish_session()
                        .map_err(|error| format!("ZModem 结束会话失败: {error}"))?;
                }
                zmodem2::SenderEvent::SessionComplete => {
                    session_done = true;
                }
            }
            progressed = true;
        }

        if progressed {
            last_progress = Instant::now();
        } else if last_progress.elapsed() > Duration::from_secs(90) {
            return Err("ZModem upload idle timeout".to_string());
        } else {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(u64::from(size))
}

pub(super) async fn zmodem_receive_files(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut modem_receiver =
        zmodem2::Receiver::new().map_err(|error| format!("ZModem receiver 初始化失败: {error}"))?;
    let mut input_buf = Vec::<u8>::new();
    let mut input_offset = 0_usize;
    let mut current_file: Option<(fs::File, PathBuf, PathBuf)> = None;
    let mut received_files = 0_usize;
    let mut bytes_done = 0_u64;
    let mut session_done = false;
    let mut last_progress = Instant::now();

    while !session_done || !modem_receiver.drain_outgoing().is_empty() {
        check_modem_cancelled(state, session_id, progress).await?;
        let mut progressed = false;

        let outgoing = modem_receiver.drain_outgoing().to_vec();
        if !outgoing.is_empty() {
            write_runtime_bytes(state, session_id, &outgoing).await?;
            modem_receiver.advance_outgoing(outgoing.len());
            progressed = true;
        }

        while let Some(event) = modem_receiver.poll_event() {
            match event {
                zmodem2::ReceiverEvent::FileStart => {
                    let incoming = String::from_utf8_lossy(modem_receiver.file_name()).to_string();
                    let target =
                        zmodem_local_target_path(local_destination, &incoming, received_files)?;
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(|error| {
                            format!("创建 ZModem 本地目录失败 {}: {error}", parent.display())
                        })?;
                    }
                    let (file, temp) = open_new_local_transfer_file(&target)?;
                    current_file = Some((file, target, temp));
                }
                zmodem2::ReceiverEvent::FileComplete => {
                    if let Some((mut file, target, temp)) = current_file.take() {
                        file.flush()
                            .map_err(|error| format!("刷新 ZModem 本地文件失败: {error}"))?;
                        drop(file);
                        finalize_local_resume_file(&temp, &target)?;
                    }
                    received_files += 1;
                }
                zmodem2::ReceiverEvent::SessionComplete => {
                    session_done = true;
                }
            }
            progressed = true;
        }

        let file_data = modem_receiver.drain_file().to_vec();
        if !file_data.is_empty() {
            let Some((file, path, _)) = current_file.as_mut() else {
                return Err("ZModem 收到文件数据但还没有文件头".to_string());
            };
            file.write_all(&file_data)
                .map_err(|error| format!("写入 ZModem 本地文件失败 {}: {error}", path.display()))?;
            modem_receiver
                .advance_file(file_data.len())
                .map_err(|error| format!("ZModem 文件写入确认失败: {error}"))?;
            bytes_done += file_data.len() as u64;
            progress.update(bytes_done, 0).await?;
            progressed = true;
        }

        match reader.next_chunk(Duration::from_millis(30), 4096).await {
            Ok(bytes) if !bytes.is_empty() => {
                input_buf.extend_from_slice(&bytes);
                progressed = true;
            }
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }

        if modem_receiver.drain_outgoing().is_empty()
            && modem_receiver.drain_file().is_empty()
            && input_offset < input_buf.len()
        {
            let consumed = modem_receiver
                .feed_incoming(&input_buf[input_offset..])
                .map_err(|error| format!("ZModem 接收远端数据失败: {error}"))?;
            if consumed > 0 {
                input_offset += consumed;
                progressed = true;
                if input_offset == input_buf.len() {
                    input_buf.clear();
                    input_offset = 0;
                } else if input_offset > 4096 {
                    input_buf.drain(..input_offset);
                    input_offset = 0;
                }
            }
        }

        if progressed {
            last_progress = Instant::now();
        } else if last_progress.elapsed() > Duration::from_secs(90) {
            return Err("ZModem download idle timeout".to_string());
        } else {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    if let Some((mut file, target, temp)) = current_file.take() {
        file.flush()
            .map_err(|error| format!("刷新 ZModem 本地文件失败: {error}"))?;
        drop(file);
        finalize_local_resume_file(&temp, &target)?;
    }

    Ok(bytes_done)
}

pub(super) fn zmodem_local_target_path(
    local_destination: &str,
    incoming_name: &str,
    received_files: usize,
) -> Result<PathBuf, String> {
    let destination = local_destination.trim();
    if destination.is_empty() {
        return Err("ZModem 本地目标路径不能为空".to_string());
    }
    let incoming =
        portable_file_name(incoming_name).unwrap_or_else(|| "zmodem-file.bin".to_string());
    let base = expand_identity_path(destination);
    let ends_with_separator = destination.ends_with('/') || destination.ends_with('\\');

    if base.is_dir() || ends_with_separator {
        return Ok(base.join(incoming));
    }
    if received_files == 0 {
        return Ok(base);
    }
    Ok(base
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(incoming))
}

pub(super) async fn xmodem_send_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    auto_remote_receiver: bool,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut file, total) = open_local_transfer_source(Path::new(local_source), "XModem 本地源")?;
    let crc = modem_wait_for_receiver(&mut reader).await?;
    let mut block_no = 1_u8;
    let mut bytes_done = 0_u64;
    let mut buffer = [0_u8; XMODEM_BLOCK_SIZE];

    while bytes_done < total {
        check_modem_cancelled(state, session_id, progress).await?;
        let limit = (total - bytes_done).min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..limit])
            .map_err(|error| format!("读取 XModem 本地文件失败: {error}"))?;
        if read == 0 {
            return Err("XModem 本地文件在传输期间提前结束".to_string());
        }
        modem_send_packet_with_retries(
            state,
            session_id,
            &mut reader,
            MODEM_SOH,
            block_no,
            &buffer[..read],
            crc,
        )
        .await
        .map_err(|error| format!("XModem data block {block_no} failed: {error}"))?;
        bytes_done += read as u64;
        progress.update(bytes_done, total).await?;
        block_no = block_no.wrapping_add(1);
    }
    ensure_local_transfer_source_size(&file, total, "XModem 本地源")?;
    if auto_remote_receiver {
        modem_finish_auto_remote_xmodem(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("XModem EOT handshake failed: {error}"))?;
    } else {
        modem_finish_eot(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("XModem EOT handshake failed: {error}"))?;
    }
    Ok(bytes_done)
}

pub(super) async fn modem_finish_auto_remote_xmodem(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    for _ in 0..3 {
        write_runtime_bytes(state, session_id, &[MODEM_EOT]).await?;
        match modem_wait_for_ack(reader, Duration::from_secs(2)).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Err("remote did not ACK modem EOT".to_string())
}

pub(super) async fn xmodem_receive_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut expected = 1_u8;
    let mut output =
        PendingLocalTransferOutput::create(Path::new(local_destination), "XModem 本地目标文件")?;
    let mut trailing_padding = 0_u64;
    let mut bytes_received = 0_u64;
    let mut bytes_written = 0_u64;
    let mut first_packet = true;

    loop {
        check_modem_cancelled(state, session_id, progress).await?;
        let marker = if first_packet {
            first_packet = false;
            modem_wait_for_packet_marker(state, session_id, &mut reader).await?
        } else {
            modem_wait_for_next_marker(&mut reader, Duration::from_secs(15)).await?
        };
        if marker == MODEM_EOT {
            write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
            break;
        }
        let packet = match modem_read_packet(&mut reader, marker).await {
            Ok(packet) => packet,
            Err(error) => {
                write_runtime_bytes(state, session_id, &[MODEM_NAK]).await?;
                return Err(error);
            }
        };
        if packet.block_no == expected {
            append_modem_data_without_trailing_padding(
                &mut output,
                &packet.data,
                &mut trailing_padding,
                &mut bytes_written,
            )?;
            bytes_received = bytes_received
                .checked_add(packet.data.len() as u64)
                .ok_or_else(|| "XModem 接收字节数溢出".to_string())?;
            progress.update(bytes_received, 0).await?;
            expected = expected.wrapping_add(1);
        }
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
    }

    output.finish()?;
    Ok(bytes_written)
}

pub(super) async fn ymodem_send_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    remote_destination: Option<&str>,
    auto_remote_receiver: bool,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut file, total) = open_local_transfer_source(Path::new(local_source), "YModem 本地源")?;
    if !modem_wait_for_receiver(&mut reader).await? {
        return Err("YModem receiver did not request CRC mode".to_string());
    }

    let (_, remote_name) = remote_destination
        .map(remote_parent_and_file_name)
        .unwrap_or_else(|| ("".to_string(), local_file_name(local_source)));
    let name = if remote_name.is_empty() {
        local_file_name(local_source)
    } else {
        remote_name
    };
    let mut metadata = vec![0_u8; XMODEM_BLOCK_SIZE];
    let metadata_text = format!("{}\0{} ", name, total);
    let metadata_bytes = metadata_text.as_bytes();
    let metadata_len = metadata_bytes.len().min(metadata.len());
    metadata[..metadata_len].copy_from_slice(&metadata_bytes[..metadata_len]);
    modem_send_packet_with_retries(
        state,
        session_id,
        &mut reader,
        MODEM_SOH,
        0,
        &metadata,
        true,
    )
    .await
    .map_err(|error| format!("YModem metadata block failed: {error}"))?;
    let _ = modem_wait_for_crc_request(&mut reader, Duration::from_secs(10)).await;

    let mut block_no = 1_u8;
    let mut bytes_done = 0_u64;
    let mut buffer = [0_u8; YMODEM_BLOCK_SIZE];
    while bytes_done < total {
        check_modem_cancelled(state, session_id, progress).await?;
        let limit = (total - bytes_done).min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..limit])
            .map_err(|error| format!("读取 YModem 本地文件失败: {error}"))?;
        if read == 0 {
            return Err("YModem 本地文件在传输期间提前结束".to_string());
        }
        modem_send_packet_with_retries(
            state,
            session_id,
            &mut reader,
            MODEM_STX,
            block_no,
            &buffer[..read],
            true,
        )
        .await
        .map_err(|error| format!("YModem data block {block_no} failed: {error}"))?;
        bytes_done += read as u64;
        progress.update(bytes_done, total).await?;
        block_no = block_no.wrapping_add(1);
    }
    ensure_local_transfer_source_size(&file, total, "YModem 本地源")?;
    modem_finish_eot(state, session_id, &mut reader)
        .await
        .map_err(|error| format!("YModem EOT handshake failed: {error}"))?;
    let _ = modem_wait_for_crc_request(&mut reader, Duration::from_secs(10)).await;
    if auto_remote_receiver {
        modem_finish_auto_remote_ymodem_batch(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("YModem final empty block failed: {error}"))?;
    } else {
        let empty = vec![0_u8; XMODEM_BLOCK_SIZE];
        modem_send_packet_with_retries(state, session_id, &mut reader, MODEM_SOH, 0, &empty, true)
            .await
            .map_err(|error| format!("YModem final empty block failed: {error}"))?;
    }
    Ok(bytes_done)
}

pub(super) async fn modem_finish_auto_remote_ymodem_batch(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    let empty = vec![0_u8; XMODEM_BLOCK_SIZE];
    let packet = modem_packet_bytes(MODEM_SOH, 0, &empty, XMODEM_BLOCK_SIZE, true);
    for _ in 0..3 {
        write_runtime_bytes(state, session_id, &packet).await?;
        match modem_wait_for_ack(reader, Duration::from_secs(2)).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Err("remote rejected final YModem empty block".to_string())
}

pub(super) async fn ymodem_receive_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let marker = modem_wait_for_packet_marker(state, session_id, &mut reader).await?;
    if marker == MODEM_EOT {
        return Err("YModem sender ended before metadata block".to_string());
    }
    let metadata = modem_read_packet(&mut reader, marker).await?;
    if metadata.block_no != 0 {
        return Err("YModem metadata block missing".to_string());
    }
    let (name, expected_size) = parse_ymodem_metadata(&metadata.data);
    if name.is_empty() {
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
        return Err("YModem sender sent empty batch".to_string());
    }
    write_runtime_bytes(state, session_id, &[MODEM_ACK, MODEM_CRC_REQUEST]).await?;

    let destination = if Path::new(local_destination).is_dir() {
        let safe_name = portable_file_name(&name).unwrap_or_else(|| "ymodem-file.bin".to_string());
        Path::new(local_destination).join(safe_name)
    } else {
        PathBuf::from(local_destination)
    };
    let mut expected = 1_u8;
    let mut output = PendingLocalTransferOutput::create(&destination, "YModem 本地目标文件")?;
    let mut trailing_padding = 0_u64;
    let mut bytes_received = 0_u64;
    let mut bytes_written = 0_u64;
    let total = expected_size.unwrap_or(0) as u64;
    loop {
        check_modem_cancelled(state, session_id, progress).await?;
        let marker = modem_wait_for_next_marker(&mut reader, Duration::from_secs(15)).await?;
        if marker == MODEM_EOT {
            write_runtime_bytes(state, session_id, &[MODEM_ACK, MODEM_CRC_REQUEST]).await?;
            if let Ok(final_marker) =
                modem_wait_for_next_marker(&mut reader, Duration::from_secs(5)).await
            {
                if final_marker != MODEM_EOT {
                    let final_packet = modem_read_packet(&mut reader, final_marker).await?;
                    if final_packet.block_no == 0 {
                        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
                    }
                }
            }
            break;
        }
        let packet = modem_read_packet(&mut reader, marker).await?;
        if packet.block_no == expected {
            if let Some(expected_size) = expected_size {
                append_modem_data_with_size_limit(
                    &mut output,
                    &packet.data,
                    expected_size as u64,
                    &mut bytes_written,
                )?;
            } else {
                append_modem_data_without_trailing_padding(
                    &mut output,
                    &packet.data,
                    &mut trailing_padding,
                    &mut bytes_written,
                )?;
            }
            bytes_received = bytes_received
                .checked_add(packet.data.len() as u64)
                .ok_or_else(|| "YModem 接收字节数溢出".to_string())?;
            progress.update(bytes_received, total).await?;
            expected = expected.wrapping_add(1);
        }
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
    }

    output.finish()?;
    Ok(bytes_written)
}

pub(super) struct ModemPacket {
    block_no: u8,
    data: Vec<u8>,
}

pub(super) async fn modem_wait_for_receiver(reader: &mut ModemByteReader) -> Result<bool, String> {
    let started = Instant::now();
    loop {
        let remaining = Duration::from_secs(45).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("modem receiver did not send NAK/C within 45s".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(3)))
            .await
        {
            Ok(MODEM_CRC_REQUEST) => return Ok(true),
            Ok(MODEM_NAK) => return Ok(false),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn modem_wait_for_crc_request(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("timed out waiting for YModem CRC request".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(2)))
            .await
        {
            Ok(MODEM_CRC_REQUEST) => return Ok(()),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn modem_send_packet_with_retries(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
    marker: u8,
    block_no: u8,
    payload: &[u8],
    crc: bool,
) -> Result<(), String> {
    let size = if marker == MODEM_STX {
        YMODEM_BLOCK_SIZE
    } else {
        XMODEM_BLOCK_SIZE
    };
    let packet = modem_packet_bytes(marker, block_no, payload, size, crc);
    modem_send_packet_bytes_with_retries(
        state,
        session_id,
        reader,
        block_no,
        &packet,
        MODEM_ACK_TIMEOUT,
    )
    .await
}

pub(super) async fn modem_send_packet_bytes_with_retries(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
    block_no: u8,
    packet: &[u8],
    ack_timeout: Duration,
) -> Result<(), String> {
    for _ in 0..MODEM_MAX_RETRIES {
        write_runtime_bytes(state, session_id, packet).await?;
        match modem_wait_for_ack(reader, ack_timeout).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Err(format!(
        "modem block {block_no} was not acknowledged after {MODEM_MAX_RETRIES} attempts"
    ))
}

pub(super) fn modem_packet_bytes(
    marker: u8,
    block_no: u8,
    payload: &[u8],
    size: usize,
    crc: bool,
) -> Vec<u8> {
    let mut data = vec![MODEM_EOF; size];
    data[..payload.len().min(size)].copy_from_slice(&payload[..payload.len().min(size)]);
    let mut packet = Vec::with_capacity(3 + size + if crc { 2 } else { 1 });
    packet.push(marker);
    packet.push(block_no);
    packet.push(255_u8.wrapping_sub(block_no));
    packet.extend_from_slice(&data);
    if crc {
        let crc = crc16_xmodem(&data);
        packet.extend_from_slice(&crc.to_be_bytes());
    } else {
        packet.push(data.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte)));
    }
    packet
}

pub(super) enum ModemAck {
    Ack,
    Nak,
}

pub(super) async fn modem_wait_for_ack(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<ModemAck, String> {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("timed out waiting for modem ACK".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(2)))
            .await
        {
            Ok(MODEM_ACK) => return Ok(ModemAck::Ack),
            Ok(MODEM_NAK) => return Ok(ModemAck::Nak),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn modem_finish_eot(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    modem_finish_eot_with_timeout(state, session_id, reader, MODEM_ACK_TIMEOUT).await
}

pub(super) async fn modem_finish_eot_with_timeout(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
    ack_timeout: Duration,
) -> Result<(), String> {
    for _ in 0..MODEM_MAX_RETRIES {
        write_runtime_bytes(state, session_id, &[MODEM_EOT]).await?;
        match modem_wait_for_ack(reader, ack_timeout).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Err(format!(
        "remote did not ACK modem EOT after {MODEM_MAX_RETRIES} attempts"
    ))
}

pub(super) async fn modem_wait_for_packet_marker(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<u8, String> {
    for _ in 0..24 {
        write_runtime_bytes(state, session_id, &[MODEM_CRC_REQUEST]).await?;
        match modem_wait_for_next_marker(reader, Duration::from_secs(3)).await {
            Ok(marker) => return Ok(marker),
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Err("timed out waiting for modem packet".to_string())
}

pub(super) async fn modem_wait_for_next_marker(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<u8, String> {
    loop {
        match reader.next_byte(timeout).await? {
            MODEM_SOH => return Ok(MODEM_SOH),
            MODEM_STX => return Ok(MODEM_STX),
            MODEM_EOT => return Ok(MODEM_EOT),
            MODEM_CAN => return Err("modem transfer cancelled by remote".to_string()),
            _ => {}
        }
    }
}

pub(super) async fn modem_read_packet(
    reader: &mut ModemByteReader,
    marker: u8,
) -> Result<ModemPacket, String> {
    let size = match marker {
        MODEM_SOH => XMODEM_BLOCK_SIZE,
        MODEM_STX => YMODEM_BLOCK_SIZE,
        _ => return Err(format!("unexpected modem packet marker: {marker}")),
    };
    let header = reader.read_exact(2, Duration::from_secs(5)).await?;
    let block_no = header[0];
    let inverse = header[1];
    if block_no != 255_u8.wrapping_sub(inverse) {
        return Err("modem packet block number check failed".to_string());
    }
    let mut data = reader.read_exact(size + 2, Duration::from_secs(8)).await?;
    let received_crc = u16::from_be_bytes([data[size], data[size + 1]]);
    data.truncate(size);
    let actual_crc = crc16_xmodem(&data);
    if received_crc != actual_crc {
        return Err(format!(
            "modem packet CRC mismatch: received={received_crc:04x} actual={actual_crc:04x}"
        ));
    }
    Ok(ModemPacket { block_no, data })
}

pub(super) fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
pub(super) fn write_local_transfer_file(path: &str, data: &[u8]) -> Result<(), String> {
    let mut output = PendingLocalTransferOutput::create(Path::new(path), "本地传输目标路径")?;
    output
        .file_mut()?
        .write_all(data)
        .map_err(|error| format!("写入本地文件失败: {error}"))?;
    output.finish()
}

pub(super) struct PendingLocalTransferOutput {
    target: PathBuf,
    pub(super) temp: PathBuf,
    file: Option<fs::File>,
    finished: bool,
}

impl PendingLocalTransferOutput {
    pub(super) fn create(target: &Path, label: &str) -> Result<Self, String> {
        prepare_local_transfer_target_path(target, label)?;
        let (file, temp) = open_new_local_transfer_file(target)?;
        Ok(Self {
            target: target.to_path_buf(),
            temp,
            file: Some(file),
            finished: false,
        })
    }

    pub(super) fn file_mut(&mut self) -> Result<&mut fs::File, String> {
        self.file
            .as_mut()
            .ok_or_else(|| "本地传输临时文件已关闭".to_string())
    }

    pub(super) fn finish(mut self) -> Result<(), String> {
        let file = self
            .file
            .take()
            .ok_or_else(|| "本地传输临时文件已关闭".to_string())?;
        file.sync_all()
            .map_err(|error| format!("写入本地文件失败: {error}"))?;
        drop(file);
        finalize_local_resume_file(&self.temp, &self.target)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for PendingLocalTransferOutput {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.file.take();
        let _ = fs::remove_file(&self.temp);
    }
}

pub(super) fn append_modem_data_without_trailing_padding(
    output: &mut PendingLocalTransferOutput,
    data: &[u8],
    trailing_padding: &mut u64,
    bytes_written: &mut u64,
) -> Result<(), String> {
    let Some(last_content) = data.iter().rposition(|byte| *byte != MODEM_EOF) else {
        *trailing_padding = trailing_padding
            .checked_add(data.len() as u64)
            .ok_or_else(|| "Modem 填充字节数溢出".to_string())?;
        return Ok(());
    };

    if *trailing_padding > 0 {
        write_modem_padding(output.file_mut()?, *trailing_padding)
            .map_err(|error| format!("写入本地 Modem 文件失败: {error}"))?;
        *bytes_written = bytes_written
            .checked_add(*trailing_padding)
            .ok_or_else(|| "Modem 写入字节数溢出".to_string())?;
        *trailing_padding = 0;
    }

    let trailing_count = data.len().saturating_sub(last_content.saturating_add(1)) as u64;
    let data = &data[..=last_content];
    output
        .file_mut()?
        .write_all(data)
        .map_err(|error| format!("写入本地 Modem 文件失败: {error}"))?;
    *bytes_written = bytes_written
        .checked_add(data.len() as u64)
        .ok_or_else(|| "Modem 写入字节数溢出".to_string())?;
    *trailing_padding = trailing_count;
    Ok(())
}

pub(super) fn append_modem_data_with_size_limit(
    output: &mut PendingLocalTransferOutput,
    data: &[u8],
    limit: u64,
    bytes_written: &mut u64,
) -> Result<(), String> {
    let remaining = limit.saturating_sub(*bytes_written);
    let count = remaining.min(data.len() as u64) as usize;
    if count == 0 {
        return Ok(());
    }
    output
        .file_mut()?
        .write_all(&data[..count])
        .map_err(|error| format!("写入本地 Modem 文件失败: {error}"))?;
    *bytes_written = bytes_written
        .checked_add(count as u64)
        .ok_or_else(|| "Modem 写入字节数溢出".to_string())?;
    Ok(())
}

pub(super) fn write_modem_padding(file: &mut fs::File, count: u64) -> std::io::Result<()> {
    let padding = [MODEM_EOF; 1024];
    let mut remaining = count;
    while remaining > 0 {
        let count = remaining.min(padding.len() as u64) as usize;
        file.write_all(&padding[..count])?;
        remaining -= count as u64;
    }
    Ok(())
}
pub(super) fn parse_ymodem_metadata(data: &[u8]) -> (String, Option<usize>) {
    let name_end = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let name = String::from_utf8_lossy(&data[..name_end])
        .trim()
        .to_string();
    let rest = if name_end < data.len() {
        &data[name_end + 1..]
    } else {
        &[]
    };
    let rest = String::from_utf8_lossy(rest);
    let size = rest
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<usize>().ok());
    (name, size)
}

pub(super) fn local_file_name(path: &str) -> String {
    portable_file_name(path).unwrap_or_else(|| "portmate-transfer.bin".to_string())
}

pub(super) fn remote_parent_and_file_name(path: &str) -> (String, String) {
    let normalized = path.trim().trim_end_matches(['/', '\\']);
    let Some((index, separator)) = normalized
        .char_indices()
        .rev()
        .find(|(_, character)| matches!(character, '/' | '\\'))
    else {
        return (
            String::new(),
            portable_file_name(normalized).unwrap_or_default(),
        );
    };
    let parent = if index == 0 {
        "/".to_string()
    } else {
        normalized[..index].to_string()
    };
    let name_start = index + separator.len_utf8();
    let name = portable_file_name(&normalized[name_start..]).unwrap_or_default();
    (parent, name)
}
pub(super) fn is_modem_timeout(error: &str) -> bool {
    error.contains("timeout") || error.contains("timed out")
}

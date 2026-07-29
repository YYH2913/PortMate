use super::*;

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

use super::*;

fn terminal_key_sequence(key: &str) -> Result<String, String> {
    let normalized = key.trim().to_ascii_lowercase().replace('_', "-");
    let sequence = match normalized.as_str() {
        "" => return Err("key must not be empty".to_string()),
        "enter" | "return" => "\r".to_string(),
        "linefeed" | "lf" => "\n".to_string(),
        "tab" => "\t".to_string(),
        "backspace" | "bs" => "\u{0008}".to_string(),
        "delete" | "del" => "\x1b[3~".to_string(),
        "escape" | "esc" => "\x1b".to_string(),
        "up" | "arrow-up" => "\x1b[A".to_string(),
        "down" | "arrow-down" => "\x1b[B".to_string(),
        "right" | "arrow-right" => "\x1b[C".to_string(),
        "left" | "arrow-left" => "\x1b[D".to_string(),
        "home" => "\x1b[H".to_string(),
        "end" => "\x1b[F".to_string(),
        "pageup" | "page-up" => "\x1b[5~".to_string(),
        "pagedown" | "page-down" => "\x1b[6~".to_string(),
        "insert" | "ins" => "\x1b[2~".to_string(),
        "f1" => "\x1bOP".to_string(),
        "f2" => "\x1bOQ".to_string(),
        "f3" => "\x1bOR".to_string(),
        "f4" => "\x1bOS".to_string(),
        "f5" => "\x1b[15~".to_string(),
        "f6" => "\x1b[17~".to_string(),
        "f7" => "\x1b[18~".to_string(),
        "f8" => "\x1b[19~".to_string(),
        "f9" => "\x1b[20~".to_string(),
        "f10" => "\x1b[21~".to_string(),
        "f11" => "\x1b[23~".to_string(),
        "f12" => "\x1b[24~".to_string(),
        "space" => " ".to_string(),
        value if value.starts_with("ctrl+") || value.starts_with("ctrl-") => {
            let key = value
                .trim_start_matches("ctrl+")
                .trim_start_matches("ctrl-");
            let byte = match key {
                "space" | "@" => 0,
                "[" | "escape" | "esc" => 27,
                "\\" => 28,
                "]" => 29,
                "^" => 30,
                "_" => 31,
                value if value.len() == 1 => {
                    let ch = value.as_bytes()[0];
                    if ch.is_ascii_alphabetic() {
                        ch.to_ascii_uppercase() - b'@'
                    } else {
                        return Err(format!("unsupported control key: {key}"));
                    }
                }
                _ => return Err(format!("unsupported control key: {key}")),
            };
            String::from_utf8(vec![byte]).map_err(|error| error.to_string())?
        }
        value if value.chars().count() == 1 => value.to_string(),
        _ => return Err(format!("unsupported key sequence: {key}")),
    };
    Ok(sequence)
}

pub(super) fn terminal_key_sequence_for_protocol(
    key: &str,
    is_telnet: bool,
) -> Result<String, String> {
    let sequence = terminal_key_sequence(key)?;
    if is_telnet && sequence == "\r" {
        Ok("\r\n".to_string())
    } else {
        Ok(sequence)
    }
}

pub(super) fn terminate_command_for_protocol(mut command: String, is_telnet: bool) -> String {
    let needs_terminator = !command.ends_with('\n') && !command.ends_with('\r');
    let telnet_bare_cr = is_telnet && command.ends_with('\r') && !command.ends_with("\r\n");
    if needs_terminator || telnet_bare_cr {
        command.push('\n');
    }
    command
}

pub(super) async fn resize_session_inner(
    state: &AppState,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<SessionSummary, String> {
    if cols == 0 || rows == 0 {
        return Err("terminal size must be non-zero".to_string());
    }

    let ssh_writer = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(&session_id)
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = ssh_writer {
        resize_ssh_channel_with_timeout(
            &writer,
            u32::from(cols),
            u32::from(rows),
            SSH_TERMINAL_WRITE_TIMEOUT,
            "SSH resize",
        )
        .await?;
    }

    let shell_master = {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections
            .get(&session_id)
            .map(|runtime| Arc::clone(&runtime.master))
    };
    if let Some(master) = shell_master {
        let master = master.lock().map_err(|error| error.to_string())?;
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("Shell PTY resize failed: {error}"))?;
    }

    let telnet_target = {
        let connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.get(&session_id).and_then(|runtime| {
            runtime
                .telnet
                .as_ref()
                .map(|telnet| (Arc::clone(&runtime.writer), Arc::clone(telnet)))
        })
    };
    if let Some((writer, telnet)) = telnet_target {
        let io = state.session_io();
        let _lane_guard = acquire_outbound_lane(&io.store_path, &session_id).await?;
        telnet.cols.store(cols, Ordering::SeqCst);
        telnet.rows.store(rows, Ordering::SeqCst);
        if telnet.naws_negotiated.load(Ordering::SeqCst) {
            let message = telnet_naws_message(cols, rows);
            write_tcp_bytes(&writer, &message, "Telnet NAWS resize 写入").await?;
            record_outbound_control_event(&io, &session_id, &message, "telnet-naws", None, true);
        }
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        resize_session_profile_in_store(next_store, &session_id, cols, rows)
    })
}

pub(super) fn resize_session_profile_in_store(
    store: &mut SessionStore,
    session_id: &str,
    cols: u16,
    rows: u16,
) -> Result<SessionSummary, String> {
    let profile = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    profile.terminal.cols = cols;
    profile.terminal.rows = rows;
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == session_id)
        .ok_or_else(|| format!("session summary is missing: {session_id}"))?;
    Ok(summary)
}

#[tauri::command]
pub(crate) fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.summaries())
}

#[tauri::command]
pub(crate) fn read_screen(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    store
        .screen(&session_id)
        .ok_or_else(|| format!("no screen data for session: {session_id}"))
}

#[tauri::command]
pub(crate) async fn send_text(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
    interactive: Option<bool>,
    queued: Option<bool>,
) -> Result<Option<SessionEvent>, String> {
    let interactive = interactive.unwrap_or(false);
    if queued.unwrap_or(false) {
        enqueue_interactive_text(state.inner().session_io(), session_id, text, interactive)?;
        return Ok(None);
    }
    if interactive {
        return send_text_interactive_inner(state.inner().session_io(), session_id, text)
            .await
            .map(Some);
    }
    send_text_inner(state.inner().session_io(), session_id, text)
        .await
        .map(Some)
}

#[tauri::command]
pub(crate) async fn send_bytes(
    state: State<'_, AppState>,
    session_id: String,
    bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    send_bytes_inner(state.inner().session_io(), session_id, bytes).await
}

#[tauri::command]
pub(crate) async fn send_key(
    state: State<'_, AppState>,
    session_id: String,
    key: String,
) -> Result<SessionEvent, String> {
    let io = state.inner().session_io();
    let text =
        terminal_key_sequence_for_protocol(&key, is_telnet_session(&io.store, &session_id)?)?;
    send_text_inner_with_context(io, session_id, text, "desktop-user", Some("send_key")).await
}

#[tauri::command]
pub(crate) async fn run_command(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
) -> Result<SessionEvent, String> {
    let io = state.inner().session_io();
    let text = terminate_command_for_protocol(command, is_telnet_session(&io.store, &session_id)?);
    run_command_inner_with_context(io, session_id, text, "desktop-user", Some("run_command")).await
}

#[tauri::command]
pub(crate) async fn resize_session(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<SessionSummary, String> {
    resize_session_inner(state.inner(), session_id, cols, rows).await
}

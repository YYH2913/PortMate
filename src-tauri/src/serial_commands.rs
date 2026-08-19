use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SerialControlLine {
    Dtr,
    Rts,
}

impl SerialControlLine {
    fn label(self) -> &'static str {
        match self {
            Self::Dtr => "DTR",
            Self::Rts => "RTS",
        }
    }
}

pub(super) fn apply_serial_line_updates_with<WriteLine>(
    old_dtr: bool,
    old_rts: bool,
    dtr: Option<bool>,
    rts: Option<bool>,
    mut write_line: WriteLine,
) -> Result<(), String>
where
    WriteLine: FnMut(SerialControlLine, bool) -> Result<(), String>,
{
    let requested = [
        (SerialControlLine::Dtr, dtr, old_dtr),
        (SerialControlLine::Rts, rts, old_rts),
    ];
    let mut applied = Vec::new();
    for (line, value, previous) in requested {
        let Some(value) = value else {
            continue;
        };
        if let Err(error) = write_line(line, value) {
            let mut rollback_errors = Vec::new();
            if let Err(rollback_error) = write_line(line, previous) {
                rollback_errors.push(format!("恢复 {} 失败: {rollback_error}", line.label()));
            }
            for (applied_line, applied_previous) in applied.into_iter().rev() {
                if let Err(rollback_error) = write_line(applied_line, applied_previous) {
                    rollback_errors.push(format!(
                        "恢复 {} 失败: {rollback_error}",
                        applied_line.label()
                    ));
                }
            }
            let error = format!("设置 {} 失败: {error}", line.label());
            return Err(if rollback_errors.is_empty() {
                error
            } else {
                format!("{error}; {}", rollback_errors.join("; "))
            });
        }
        applied.push((line, previous));
    }
    Ok(())
}

pub(super) fn record_applied_serial_line_state(
    store: &mut SessionStore,
    store_path: &Path,
    request: &SerialLineRequest,
) -> Result<SessionSummary, String> {
    if request.dtr.is_none() && request.rts.is_none() {
        return Err("串口线路请求必须包含 DTR 或 RTS".to_string());
    }
    let profile = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == request.session_id)
        .ok_or_else(|| format!("unknown session: {}", request.session_id))?;
    let ConnectionConfig::Serial(serial) = &mut profile.connection else {
        return Err(format!(
            "session is not serial-backed: {}",
            request.session_id
        ));
    };
    if let Some(dtr) = request.dtr {
        serial.dtr = dtr;
    }
    if let Some(rts) = request.rts {
        serial.rts = rts;
    }
    store.record_system_event(&request.session_id, "PortMate: serial line state updated");
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == request.session_id)
        .ok_or_else(|| format!("session summary is missing: {}", request.session_id))?;
    persist_applied_store(store, store_path, "serial line state")
        .map_err(|error| format!("串口线路已在设备上更新，但 Profile 状态无法持久化: {error}"))?;
    Ok(summary)
}

pub(super) fn pulse_serial_break_with<SetBreak, ClearBreak, Wait>(
    mut set_break: SetBreak,
    mut clear_break: ClearBreak,
    wait: Wait,
) -> Result<bool, String>
where
    SetBreak: FnMut() -> Result<(), String>,
    ClearBreak: FnMut() -> Result<(), String>,
    Wait: FnOnce(),
{
    if let Err(error) = set_break() {
        return Err(match clear_break() {
            Ok(()) => format!("发送 Break 失败并已尝试清除线路: {error}"),
            Err(clear_error) => {
                format!("发送 Break 失败: {error}; 清除 Break 也失败: {clear_error}")
            }
        });
    }
    wait();
    let first_error = match clear_break() {
        Ok(()) => return Ok(false),
        Err(error) => error,
    };
    match clear_break() {
        Ok(()) => Ok(true),
        Err(retry_error) => Err(format!(
            "清除 Break 失败: {first_error}; 重试清除 Break 仍失败: {retry_error}"
        )),
    }
}

#[tauri::command]
pub(crate) fn list_serial_ports() -> Result<Vec<String>, String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|port| port.port_name).collect())
        .map_err(|error| format!("串口枚举失败: {error}"))
}

#[tauri::command]
pub(crate) fn list_serial_capture(
    state: State<'_, AppState>,
    session_id: String,
    after_id: Option<String>,
) -> Result<SerialCaptureSnapshot, String> {
    serial_capture_snapshot_inner(state.inner(), &session_id, after_id.as_deref())
}

#[tauri::command]
pub(crate) fn list_serial_capture_history(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SerialCaptureHistorySnapshot, String> {
    serial_capture_history_inner(&state.store_path, &state.store, &session_id)
}

#[tauri::command]
pub(crate) fn clear_serial_capture(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SerialCaptureSnapshot, String> {
    ensure_serial_profile(&state.store, &session_id)?;
    let capture = serial_capture_for_session(&state.serial_captures, &session_id)?;
    let mut capture = capture.lock().map_err(|error| error.to_string())?;
    capture.clear();
    Ok(capture.snapshot_since(None))
}

#[tauri::command]
pub(crate) fn export_serial_capture(
    state: State<'_, AppState>,
    request: ExportSerialCaptureRequest,
) -> Result<ExportSerialCaptureResult, String> {
    ensure_serial_profile(&state.store, &request.session_id)?;
    export_serial_capture_inner(&state.store_path, &state.serial_captures, request)
}

#[tauri::command]
pub(crate) fn export_serial_capture_history(
    state: State<'_, AppState>,
    request: ExportSerialCaptureRequest,
) -> Result<ExportSerialCaptureResult, String> {
    ensure_serial_profile(&state.store, &request.session_id)?;
    export_serial_capture_history_inner(&state.store_path, &state.store, request)
}

#[tauri::command]
pub(crate) fn serial_set_lines(
    state: State<'_, AppState>,
    request: SerialLineRequest,
) -> Result<SessionSummary, String> {
    if request.dtr.is_none() && request.rts.is_none() {
        return Err("串口线路请求必须包含 DTR 或 RTS".to_string());
    }
    let connections = state.serial.lock().map_err(|error| error.to_string())?;
    let writer = connections
        .get(&request.session_id)
        .ok_or_else(|| "串口会话尚未连接".to_string())?
        .writer
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| "串口正在重连".to_string())?;
    let mut port = writer.lock().map_err(|error| error.to_string())?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let (old_dtr, old_rts) = match store.profile(&request.session_id) {
        Some(SessionProfile {
            connection: ConnectionConfig::Serial(serial),
            ..
        }) => (serial.dtr, serial.rts),
        Some(_) => {
            return Err(format!(
                "session is not serial-backed: {}",
                request.session_id
            ))
        }
        None => return Err(format!("unknown session: {}", request.session_id)),
    };
    apply_serial_line_updates_with(old_dtr, old_rts, request.dtr, request.rts, |line, value| {
        match line {
            SerialControlLine::Dtr => port
                .write_data_terminal_ready(value)
                .map_err(|error| error.to_string()),
            SerialControlLine::Rts => port
                .write_request_to_send(value)
                .map_err(|error| error.to_string()),
        }
    })?;
    record_applied_serial_line_state(&mut store, &state.store_path, &request)
}

#[tauri::command]
pub(crate) fn serial_send_break(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), String> {
    serial_send_break_inner_with_validation(state.inner(), &session_id, None)
}

pub(super) fn serial_send_break_inner_with_validation(
    state: &AppState,
    session_id: &str,
    commit_validation: Option<CommitValidation>,
) -> Result<(), String> {
    ensure_serial_profile(&state.store, session_id)?;
    let connections = state.serial.lock().map_err(|error| error.to_string())?;
    let runtime = connections
        .get(session_id)
        .filter(|runtime| !runtime.closed.load(Ordering::SeqCst))
        .ok_or_else(|| "串口会话尚未连接".to_string())?;
    let writer = runtime
        .writer
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| "串口正在重连".to_string())?;
    let port = writer.lock().map_err(|error| error.to_string())?;
    if let Some(validate) = commit_validation {
        validate()?;
    }
    let clear_retried = pulse_serial_break_with(
        || port.set_break().map_err(|error| error.to_string()),
        || port.clear_break().map_err(|error| error.to_string()),
        || std::thread::sleep(Duration::from_millis(250)),
    )?;
    if clear_retried {
        eprintln!("PortMate: serial Break clear succeeded on retry");
    }
    drop(port);
    drop(connections);

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.record_system_event(session_id, "PortMate: serial Break sent");
    persist_applied_store(&store, &state.store_path, "serial Break event")
        .map_err(|error| format!("Break 已发送并清除，但系统事件无法持久化: {error}"))?;
    Ok(())
}

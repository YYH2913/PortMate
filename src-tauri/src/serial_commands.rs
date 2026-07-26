use super::*;

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
    let connections = state.serial.lock().map_err(|error| error.to_string())?;
    let writer = connections
        .get(&session_id)
        .ok_or_else(|| "串口会话尚未连接".to_string())?
        .writer
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| "串口正在重连".to_string())?;
    let port = writer.lock().map_err(|error| error.to_string())?;
    let clear_retried = pulse_serial_break_with(
        || port.set_break().map_err(|error| error.to_string()),
        || port.clear_break().map_err(|error| error.to_string()),
        || std::thread::sleep(Duration::from_millis(250)),
    )?;
    if clear_retried {
        eprintln!("PortMate: serial Break clear succeeded on retry");
    }

    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    store.record_system_event(&session_id, "PortMate: serial Break sent");
    persist_applied_store(&store, &state.store_path, "serial Break event")
        .map_err(|error| format!("Break 已发送并清除，但系统事件无法持久化: {error}"))?;
    Ok(())
}

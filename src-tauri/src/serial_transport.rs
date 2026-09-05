use super::transport_timing::SERIAL_RUNTIME_SHUTDOWN_TIMEOUT;
use super::*;

pub(super) type SerialPortHandle = Box<dyn serialport::SerialPort>;
pub(super) struct SerialPortPair {
    pub(super) writer: SerialPortHandle,
    pub(super) reader: SerialPortHandle,
    // A dedicated clone lets close_session abort a pending driver write
    // without waiting for the writer mutex held by that write.
    pub(super) abort: Option<SerialPortHandle>,
}

pub(super) struct PreparedSerialSession {
    profile: SessionProfile,
    serial: portmate_core::SerialConnection,
    port_name: String,
    port: SerialPortHandle,
    reader: SerialPortHandle,
    abort: Option<SerialPortHandle>,
}

pub(super) struct SerialRuntime {
    pub(super) runtime_id: String,
    pub(super) writer: Option<Arc<Mutex<SerialPortHandle>>>,
    pub(super) abort: Option<Arc<Mutex<SerialPortHandle>>>,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) capture: Arc<Mutex<SerialCaptureBuffer>>,
}

/// Interrupts driver-level serial I/O before the runtime is dropped. The
/// abort clone is deliberately separate from the writer mutex: a blocked
/// WriteFile/Write can therefore be purged without waiting for that mutex.
pub(super) fn stop_serial_runtime(runtime: &SerialRuntime) {
    runtime.closed.store(true, Ordering::SeqCst);
    if let Some(abort) = runtime.abort.as_ref() {
        if let Ok(abort) = abort.lock() {
            let _ = abort.clear(serialport::ClearBuffer::All);
        }
    }
    // Older or unusual serial implementations may not support cloning an
    // abort handle. Best-effort clearing of the writer still helps those
    // implementations when the writer is not currently locked.
    if let Some(writer) = runtime.writer.as_ref() {
        if let Ok(writer) = writer.try_lock() {
            let _ = writer.clear(serialport::ClearBuffer::All);
        }
    }
}

pub(super) fn serial_connection_details(
    profile: &SessionProfile,
) -> Result<(portmate_core::SerialConnection, String), String> {
    let mut serial = match &profile.connection {
        ConnectionConfig::Serial(serial) => serial.clone(),
        _ => return Err("profile is not serial-backed".to_string()),
    };
    serial.normalize_health_settings();
    let port_name = serial.port.clone();
    if port_name.trim().is_empty() {
        return Err("串口不能为空".to_string());
    }
    Ok((serial, port_name))
}

pub(super) fn open_configured_serial_port(
    serial: &portmate_core::SerialConnection,
    port_name: &str,
) -> Result<SerialPortPair, String> {
    let mut port = serialport::new(port_name, serial.baud_rate)
        .data_bits(serial_data_bits(serial.data_bits))
        .stop_bits(serial_stop_bits(serial.stop_bits))
        .parity(serial_parity(&serial.parity))
        .flow_control(serial_flow_control(&serial.flow_control))
        .timeout(Duration::from_millis(100))
        .open()
        .map_err(|error| format!("串口打开失败 {port_name}: {error}"))?;
    if let Err(error) = port.write_data_terminal_ready(serial.dtr) {
        if serial.dtr {
            return Err(format!("设置 DTR 失败: {error}"));
        }
        eprintln!("PortMate: serial device does not support clearing DTR: {error}");
    }
    if let Err(error) = port.write_request_to_send(serial.rts) {
        if serial.rts {
            return Err(format!("设置 RTS 失败: {error}"));
        }
        eprintln!("PortMate: serial device does not support clearing RTS: {error}");
    }

    let reader = port
        .try_clone()
        .map_err(|error| format!("串口 reader 克隆失败: {error}"))?;
    let abort = match port.try_clone() {
        Ok(abort) => Some(abort),
        Err(error) => {
            eprintln!("PortMate: serial abort handle clone unavailable for {port_name}: {error}");
            None
        }
    };
    Ok(SerialPortPair {
        writer: port,
        reader,
        abort,
    })
}

#[cfg(test)]
pub(super) fn open_serial_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    install_serial_session(state, prepare_serial_session(state, profile)?)
}

pub(super) fn prepare_serial_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<PreparedSerialSession, String> {
    let _worker = state.serial_workers.register_for_session(&profile.id)?;
    let (serial, port_name) = serial_connection_details(&profile)?;
    let SerialPortPair {
        writer: port,
        reader,
        abort,
    } = open_configured_serial_port(&serial, &port_name)?;
    if state.serial_workers.is_shutting_down() {
        return Err("PortMate is shutting down; serial port was released".to_string());
    }
    Ok(PreparedSerialSession {
        profile,
        serial,
        port_name,
        port,
        reader,
        abort,
    })
}

pub(super) fn install_serial_session(
    state: &AppState,
    prepared: PreparedSerialSession,
) -> Result<SessionSummary, String> {
    let _worker = state.serial_workers.register_for_session(&prepared.profile.id)?;
    let PreparedSerialSession {
        profile,
        serial,
        port_name,
        port,
        reader,
        abort,
    } = prepared;

    let runtime_id = Uuid::new_v4().to_string();
    let closed = Arc::new(AtomicBool::new(false));
    let reader_start_gate = Arc::new(ReaderStartGate::default());
    let (tap, _) = broadcast::channel(1024);
    let writer = Arc::new(Mutex::new(port));
    let capture = serial_capture_for_session(&state.serial_captures, &profile.id)?;
    let existing = {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            SerialRuntime {
                runtime_id: runtime_id.clone(),
                writer: Some(writer),
                abort: abort.map(|abort| Arc::new(Mutex::new(abort))),
                tap: tap.clone(),
                closed: Arc::clone(&closed),
                capture: Arc::clone(&capture),
            },
        )
    };
    if let Some(existing) = existing {
        stop_serial_runtime(&existing);
    }

    let reader_handle = match spawn_serial_reader(SerialReadTask {
        io: state.session_io(),
        profile: profile.clone(),
        runtime_id: runtime_id.clone(),
        port_name: port_name.clone(),
        tap,
        closed: Arc::clone(&closed),
        start_gate: Arc::clone(&reader_start_gate),
        reader,
        capture,
        receive_idle_timeout: serial
            .receive_idle_timeout_enabled
            .then(|| Duration::from_secs(serial.receive_idle_timeout_seconds)),
    }) {
        Ok(handle) => handle,
        Err(error) => {
            closed.store(true, Ordering::SeqCst);
            reader_start_gate.cancel();
            remove_runtime_if_owned(&state.serial, &profile.id, |runtime| {
                runtime.runtime_id == runtime_id
            })?;
            return Err(format!("串口读取线程启动失败: {error}"));
        }
    };

    let finalize_result = match state.store.lock() {
        Ok(mut store) => {
            commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
                mark_session_connected_with_events(
                    next_store,
                    &profile,
                    [format!(
                        "PortMate: serial port connected ({port_name}, {} baud)",
                        serial.baud_rate
                    )],
                )
            })
        }
        Err(error) => Err(error.to_string()),
    };
    match finalize_result {
        Ok(summary) => {
            reader_start_gate.start();
            Ok(summary)
        }
        Err(error) => {
            closed.store(true, Ordering::SeqCst);
            reader_start_gate.cancel();
            let _ = reader_handle.join();
            let cleanup_error = remove_runtime_if_owned(&state.serial, &profile.id, |runtime| {
                runtime.runtime_id == runtime_id
            })
            .err();
            if let Some(cleanup_error) = cleanup_error {
                Err(format!(
                    "{error}; serial runtime cleanup failed: {cleanup_error}"
                ))
            } else {
                Err(error)
            }
        }
    }
}

pub(super) struct SerialReadTask {
    pub(super) io: SessionIo,
    pub(super) profile: SessionProfile,
    pub(super) runtime_id: String,
    pub(super) port_name: String,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) start_gate: Arc<ReaderStartGate>,
    pub(super) reader: SerialPortHandle,
    pub(super) capture: Arc<Mutex<SerialCaptureBuffer>>,
    pub(super) receive_idle_timeout: Option<Duration>,
}

enum SerialReaderTransition {
    Disconnect,
    Reconnect,
}

pub(super) fn spawn_serial_reader(
    task: SerialReadTask,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    let worker = task
        .io
        .serial_workers
        .register_for_session(&task.profile.id)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::Interrupted, error))?;
    let name = format!("portmate-serial-{}", task.profile.id);
    std::thread::Builder::new()
        .name(name)
        .spawn(move || {
            let _worker = worker;
            read_serial_port(task)();
        })
}

pub(super) fn shutdown_serial_runtimes(state: &AppState) {
    state.serial_workers.begin_shutdown();
    let runtimes = {
        let mut connections = state
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        connections.drain().map(|(_, runtime)| runtime).collect::<Vec<_>>()
    };
    for runtime in &runtimes {
        runtime.closed.store(true, Ordering::SeqCst);
    }
    drop(runtimes);

    let remaining = state
        .serial_workers
        .wait_for_idle(SERIAL_RUNTIME_SHUTDOWN_TIMEOUT);
    if remaining > 0 {
        eprintln!(
            "PortMate: {remaining} serial worker(s) did not release before the shutdown deadline"
        );
    }
}

fn read_serial_port(task: SerialReadTask) -> impl FnOnce() + Send + 'static {
    move || {
        let SerialReadTask {
            io,
            profile,
            runtime_id,
            port_name,
            tap,
            closed,
            start_gate,
            mut reader,
            capture,
            receive_idle_timeout,
        } = task;
        let session_id = profile.id.clone();
        if !start_gate.wait() {
            return;
        }
        let mut buffer = vec![0_u8; 8192];
        let mut last_received_at = Instant::now();
        let mut disconnect_reason = None;

        while !closed.load(Ordering::SeqCst) {
            match reader.read(&mut buffer) {
                Ok(0) => {}
                Ok(size) => {
                    last_received_at = Instant::now();
                    let bytes = buffer[..size].to_vec();
                    record_channel_bytes_with_accepted_side_effect(
                        &io,
                        &session_id,
                        Some(&runtime_id),
                        EventStream::Stdout,
                        ChannelByteViews::same(&bytes),
                        String::from_utf8_lossy(&bytes).to_string(),
                        || {
                            let _ = tap.send(bytes.clone());
                            record_serial_capture(&capture, EventDirection::Inbound, &bytes);
                        },
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                    if receive_idle_timeout
                        .is_some_and(|timeout| last_received_at.elapsed() >= timeout)
                    {
                        let seconds = receive_idle_timeout
                            .expect("checked receive idle timeout")
                            .as_secs();
                        disconnect_reason = Some(format!(
                            "serial receive idle timeout on {port_name} after {seconds}s"
                        ));
                        break;
                    }
                }
                Err(error) => {
                    disconnect_reason = Some(format!("serial read failed on {port_name}: {error}"));
                    break;
                }
            }
        }

        // Abort a driver-level write before releasing the reader clone. This
        // is especially important on Windows, where a pending COM write keeps
        // the exclusive device handle alive and makes an immediate reopen fail.
        let _ = reader.clear(serialport::ClearBuffer::All);

        let disconnect_reason =
            disconnect_reason.unwrap_or_else(|| format!("serial port closed ({port_name})"));
        let transition =
            match with_current_session_runtime_store(&io, &session_id, &runtime_id, |store| {
                clear_active_command(&io, &session_id);
                let reconnect_profile = (!closed.load(Ordering::SeqCst))
                    .then(|| store.profile(&session_id).map(normalize_session_profile))
                    .flatten()
                    .filter(serial_reconnect_enabled);
                if let Some(reconnect_profile) = reconnect_profile {
                    let reconnect_delay = serial_reconnect_delay(&reconnect_profile);
                    let _ = store.set_runtime_status_with_reason(
                        &session_id,
                        SessionStatus::Reconnecting,
                        Some(disconnect_reason.clone()),
                    );
                    store.record_system_event(
                        &session_id,
                        format!(
                            "PortMate: {disconnect_reason}; reconnecting in {}ms",
                            reconnect_delay.as_millis()
                        ),
                    );
                    if let Err(error) =
                        persist_applied_store(store, &io.store_path, "serial reconnect transition")
                    {
                        eprintln!("PortMate: failed to persist serial reconnect event: {error}");
                    }
                    SerialReaderTransition::Reconnect
                } else {
                    let _ = store.set_runtime_status_with_reason(
                        &session_id,
                        SessionStatus::Disconnected,
                        Some(disconnect_reason.clone()),
                    );
                    store
                        .record_system_event(&session_id, format!("PortMate: {disconnect_reason}"));
                    if let Err(error) =
                        persist_applied_store(store, &io.store_path, "serial disconnect transition")
                    {
                        eprintln!("PortMate: failed to persist serial close event: {error}");
                    }
                    SerialReaderTransition::Disconnect
                }
            }) {
                Ok(Some(transition)) => transition,
                Ok(None) => return,
                Err(error) => {
                    eprintln!("PortMate: failed to commit serial reader transition: {error}");
                    if let Ok(Some(runtime)) =
                        remove_runtime_if_owned(&io.runtimes.serial, &session_id, |runtime| {
                            runtime.runtime_id == runtime_id
                        })
                    {
                        stop_serial_runtime(&runtime);
                    }
                    return;
                }
            };

        match transition {
            SerialReaderTransition::Reconnect => {
                // The reader owns a clone of the serial handle. Drop it before
                // starting a replacement open; Windows serial drivers keep
                // the device exclusively locked until every clone is gone.
                drop(reader);
                let still_current = match io.runtimes.serial.lock() {
                    Ok(mut connections) => connections
                        .get_mut(&session_id)
                        .filter(|runtime| runtime.runtime_id == runtime_id)
                        .map(|runtime| {
                            runtime.writer = None;
                            if let Some(abort) = runtime.abort.take() {
                                if let Ok(abort) = abort.lock() {
                                    let _ = abort.clear(serialport::ClearBuffer::All);
                                }
                            }
                        })
                        .is_some(),
                    Err(_) => false,
                };
                if still_current {
                    spawn_serial_reconnect(io, session_id, runtime_id, closed);
                }
            }
            SerialReaderTransition::Disconnect => {
                drop(reader);
                if let Ok(Some(runtime)) =
                    remove_runtime_if_owned(&io.runtimes.serial, &session_id, |runtime| {
                        runtime.runtime_id == runtime_id
                    })
                {
                    stop_serial_runtime(&runtime);
                }
            }
        }
    }
}

pub(super) fn serial_data_bits(value: u8) -> serialport::DataBits {
    match value {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        _ => serialport::DataBits::Eight,
    }
}

pub(super) fn serial_stop_bits(value: u8) -> serialport::StopBits {
    match value {
        2 => serialport::StopBits::Two,
        _ => serialport::StopBits::One,
    }
}

pub(super) fn serial_parity(value: &str) -> serialport::Parity {
    match value {
        "odd" => serialport::Parity::Odd,
        "even" => serialport::Parity::Even,
        _ => serialport::Parity::None,
    }
}

pub(super) fn serial_flow_control(value: &str) -> serialport::FlowControl {
    match value {
        "software" => serialport::FlowControl::Software,
        "hardware" => serialport::FlowControl::Hardware,
        _ => serialport::FlowControl::None,
    }
}

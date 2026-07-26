use super::*;

pub(super) type SerialPortHandle = Box<dyn serialport::SerialPort>;
type SerialPortPair = (SerialPortHandle, SerialPortHandle);

pub(super) struct SerialRuntime {
    pub(super) runtime_id: String,
    pub(super) writer: Option<Arc<Mutex<SerialPortHandle>>>,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) capture: Arc<Mutex<SerialCaptureBuffer>>,
}

pub(super) fn serial_connection_details(
    profile: &SessionProfile,
) -> Result<(portmate_core::SerialConnection, String), String> {
    let mut serial = match &profile.connection {
        ConnectionConfig::Serial(serial) => serial.clone(),
        _ => return Err("profile is not serial-backed".to_string()),
    };
    serial.normalize_health_settings();
    let port_name = serial.port.trim().to_string();
    if port_name.is_empty() {
        return Err("串口不能为空".to_string());
    }
    Ok((serial, port_name))
}

pub(super) fn serial_reconnect_delay(profile: &SessionProfile) -> Duration {
    match &profile.connection {
        ConnectionConfig::Serial(serial) => Duration::from_millis(serial.reconnect_delay_ms.clamp(
            portmate_core::MIN_SERIAL_RECONNECT_DELAY_MS,
            portmate_core::MAX_SERIAL_RECONNECT_DELAY_MS,
        )),
        _ => Duration::from_millis(portmate_core::DEFAULT_SERIAL_RECONNECT_DELAY_MS),
    }
}

pub(super) fn serial_reconnect_enabled(profile: &SessionProfile) -> bool {
    match &profile.connection {
        ConnectionConfig::Serial(serial) => serial.reconnect,
        _ => false,
    }
}

pub(super) fn serial_reconnect_attempt_matches_profile(
    attempt: &SessionProfile,
    latest: &SessionProfile,
) -> bool {
    let attempt = normalize_session_profile(attempt.clone());
    let latest = normalize_session_profile(latest.clone());
    serial_reconnect_enabled(&latest) && attempt.connection == latest.connection
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SerialReconnectProfileState {
    Current,
    Changed,
    Disabled,
}

pub(super) fn serial_reconnect_profile_state(
    store: &SessionStore,
    session_id: &str,
    attempt: &SessionProfile,
) -> SerialReconnectProfileState {
    let Some(latest) = store.profile(session_id).map(normalize_session_profile) else {
        return SerialReconnectProfileState::Disabled;
    };
    if !serial_reconnect_enabled(&latest) {
        return SerialReconnectProfileState::Disabled;
    }
    if !serial_reconnect_attempt_matches_profile(attempt, &latest) {
        return SerialReconnectProfileState::Changed;
    }
    SerialReconnectProfileState::Current
}

pub(super) fn latest_serial_reconnect_profile(
    io: &SessionIo,
    session_id: &str,
) -> Result<Option<SessionProfile>, String> {
    let store = io.store.lock().map_err(|error| error.to_string())?;
    let Some(profile) = store.profile(session_id) else {
        return Ok(None);
    };
    let profile = normalize_session_profile(profile);
    Ok(serial_reconnect_enabled(&profile).then_some(profile))
}

fn open_configured_serial_port(
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
    Ok((port, reader))
}

pub(super) fn open_serial_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    let (serial, port_name) = serial_connection_details(&profile)?;

    if let Some(existing) = {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        existing.closed.store(true, Ordering::SeqCst);
    }

    let (port, reader) = open_configured_serial_port(&serial, &port_name)?;
    let runtime_id = Uuid::new_v4().to_string();
    let closed = Arc::new(AtomicBool::new(false));
    let reader_start_gate = Arc::new(ReaderStartGate::default());
    let (tap, _) = broadcast::channel(1024);
    let writer = Arc::new(Mutex::new(port));
    let capture = serial_capture_for_session(&state.serial_captures, &profile.id)?;
    {
        let mut connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            SerialRuntime {
                runtime_id: runtime_id.clone(),
                writer: Some(writer),
                tap: tap.clone(),
                closed: Arc::clone(&closed),
                capture: Arc::clone(&capture),
            },
        );
    }

    if let Err(error) = spawn_serial_reader(SerialReadTask {
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
        closed.store(true, Ordering::SeqCst);
        reader_start_gate.cancel();
        remove_runtime_if_owned(&state.serial, &profile.id, |runtime| {
            runtime.runtime_id == runtime_id
        })?;
        return Err(format!("串口读取线程启动失败: {error}"));
    }

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

struct SerialReadTask {
    io: SessionIo,
    profile: SessionProfile,
    runtime_id: String,
    port_name: String,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
    start_gate: Arc<ReaderStartGate>,
    reader: SerialPortHandle,
    capture: Arc<Mutex<SerialCaptureBuffer>>,
    receive_idle_timeout: Option<Duration>,
}

fn spawn_serial_reader(task: SerialReadTask) -> std::io::Result<std::thread::JoinHandle<()>> {
    let name = format!("portmate-serial-{}", task.profile.id);
    std::thread::Builder::new()
        .name(name)
        .spawn(read_serial_port(task))
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
        let mut last_persist = Instant::now();
        let mut has_unpersisted_stream = false;
        let mut last_received_at = Instant::now();
        let mut disconnect_reason = None;

        while !closed.load(Ordering::SeqCst) {
            match reader.read(&mut buffer) {
                Ok(0) => {}
                Ok(size) => {
                    last_received_at = Instant::now();
                    let bytes = buffer[..size].to_vec();
                    record_serial_capture(&capture, EventDirection::Inbound, &bytes);
                    let _ = tap.send(bytes.clone());
                    record_channel_bytes(
                        &io,
                        &session_id,
                        Some(&runtime_id),
                        EventStream::Stdout,
                        &bytes,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
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

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                    eprintln!("PortMate: failed to persist serial stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                eprintln!("PortMate: failed to persist final serial stream data: {error}");
            }
        }

        let reconnect_profile = (!closed.load(Ordering::SeqCst))
            .then(|| {
                io.store
                    .lock()
                    .ok()
                    .and_then(|store| store.profile(&session_id))
                    .map(normalize_session_profile)
                    .filter(serial_reconnect_enabled)
            })
            .flatten();
        let reconnect_delay = reconnect_profile.as_ref().map(serial_reconnect_delay);
        let reconnect_enabled = reconnect_profile.is_some();
        let disconnect_reason =
            disconnect_reason.unwrap_or_else(|| format!("serial port closed ({port_name})"));
        let mut should_reconnect = false;
        let removed_current = {
            let mut connections = match io.runtimes.serial.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                if reconnect_enabled {
                    if let Some(runtime) = connections.get_mut(&session_id) {
                        runtime.writer = None;
                    }
                    should_reconnect = true;
                    false
                } else {
                    connections.remove(&session_id);
                    true
                }
            } else {
                false
            }
        };
        if should_reconnect || removed_current {
            clear_active_command(&io, &session_id);
        }

        if should_reconnect {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Reconnecting,
                    Some(disconnect_reason.clone()),
                );
                store.record_system_event(
                    &session_id,
                    format!(
                        "PortMate: {disconnect_reason}; reconnecting in {}ms",
                        reconnect_delay
                            .unwrap_or_else(|| Duration::from_millis(
                                portmate_core::DEFAULT_SERIAL_RECONNECT_DELAY_MS
                            ))
                            .as_millis()
                    ),
                );
                if let Err(error) =
                    persist_applied_store(&store, &io.store_path, "serial reconnect transition")
                {
                    eprintln!("PortMate: failed to persist serial reconnect event: {error}");
                }
            }
            spawn_serial_reconnect(io, session_id, runtime_id, closed);
            return;
        }

        if removed_current {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some(disconnect_reason.clone()),
                );
                store.record_system_event(&session_id, format!("PortMate: {disconnect_reason}"));
                if let Err(error) =
                    persist_applied_store(&store, &io.store_path, "serial disconnect transition")
                {
                    eprintln!("PortMate: failed to persist serial close event: {error}");
                }
            }
        }
    }
}

fn serial_reconnect_pending(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    if closed.load(Ordering::SeqCst) {
        return false;
    }
    let connections = match io.runtimes.serial.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    io.store.lock().ok().is_some_and(|store| {
        store.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SerialReconnectFailureDisposition {
    Recorded,
    RetryLatestProfile,
    StopDisabled,
    Superseded,
}

fn record_serial_reconnect_failure_if_pending(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    attempt: &SessionProfile,
    port_name: &str,
    error: &str,
) -> SerialReconnectFailureDisposition {
    if closed.load(Ordering::SeqCst) {
        return SerialReconnectFailureDisposition::Superseded;
    }
    let connections = match io.runtimes.serial.lock() {
        Ok(connections) => connections,
        Err(_) => return SerialReconnectFailureDisposition::Superseded,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return SerialReconnectFailureDisposition::Superseded;
    }
    let mut store = match io.store.lock() {
        Ok(store) => store,
        Err(_) => return SerialReconnectFailureDisposition::Superseded,
    };
    if !store.runtimes.iter().any(|runtime| {
        runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
    }) {
        return SerialReconnectFailureDisposition::Superseded;
    }
    match serial_reconnect_profile_state(&store, session_id, attempt) {
        SerialReconnectProfileState::Current => {}
        SerialReconnectProfileState::Changed => {
            return SerialReconnectFailureDisposition::RetryLatestProfile;
        }
        SerialReconnectProfileState::Disabled => {
            return SerialReconnectFailureDisposition::StopDisabled;
        }
    }
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Reconnecting,
        Some(format!("serial reconnect failed on {port_name}: {error}")),
    );
    store.record_system_event(
        session_id,
        format!(
            "PortMate: serial reconnect failed on {port_name}: {error}; retrying in {}ms",
            serial_reconnect_delay(attempt).as_millis()
        ),
    );
    if let Err(save_error) =
        persist_applied_store(&store, &io.store_path, "serial reconnect failure state")
    {
        eprintln!("PortMate: failed to persist serial reconnect failure: {save_error}");
    }
    SerialReconnectFailureDisposition::Recorded
}

pub(super) fn stop_pending_serial_reconnect_if_disabled(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    reason: &str,
) -> bool {
    let mut connections = match io.runtimes.serial.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    let mut store = match io.store.lock() {
        Ok(store) => store,
        Err(_) => return false,
    };
    let reconnect_disabled = store
        .profile(session_id)
        .map(normalize_session_profile)
        .is_none_or(|profile| !serial_reconnect_enabled(&profile));
    if !reconnect_disabled {
        return false;
    }
    if let Some(runtime) = connections.remove(session_id) {
        runtime.closed.store(true, Ordering::SeqCst);
    }
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Disconnected,
        Some(reason.to_string()),
    );
    store.record_system_event(
        session_id,
        format!("PortMate: serial reconnect stopped: {reason}"),
    );
    if let Err(error) =
        persist_applied_store(&store, &io.store_path, "stopped serial reconnect state")
    {
        eprintln!("PortMate: failed to persist serial reconnect stop: {error}");
    }
    true
}

fn spawn_serial_reconnect(
    io: SessionIo,
    session_id: String,
    previous_runtime_id: String,
    closed: Arc<AtomicBool>,
) {
    let thread_name = format!("portmate-serial-reconnect-{session_id}");
    if let Err(error) = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || reconnect_serial_session(io, session_id, previous_runtime_id, closed))
    {
        eprintln!("PortMate: failed to start serial reconnect thread: {error}");
    }
}

enum SerialReconnectInstallDecision {
    Installed,
    Retry,
    Stop,
    Superseded,
    Failed(String),
}

fn wait_for_serial_reconnect_attempt(
    io: &SessionIo,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    let started = Instant::now();
    loop {
        if !serial_reconnect_pending(io, session_id, runtime_id, closed) {
            return false;
        }
        let profile = match latest_serial_reconnect_profile(io, session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_serial_reconnect_if_disabled(
                    io,
                    session_id,
                    runtime_id,
                    "automatic reconnect disabled while waiting for the next attempt",
                ) {
                    return false;
                }
                std::thread::sleep(RECONNECT_DELAY_POLL_INTERVAL);
                continue;
            }
            Err(error) => {
                eprintln!(
                    "PortMate: failed to load serial reconnect delay from latest profile: {error}"
                );
                std::thread::sleep(RECONNECT_DELAY_POLL_INTERVAL);
                continue;
            }
        };
        let remaining = serial_reconnect_delay(&profile).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        std::thread::sleep(remaining.min(RECONNECT_DELAY_POLL_INTERVAL));
    }
}

fn reconnect_serial_session(
    io: SessionIo,
    session_id: String,
    previous_runtime_id: String,
    closed: Arc<AtomicBool>,
) {
    loop {
        if !wait_for_serial_reconnect_attempt(
            &io,
            &session_id,
            &previous_runtime_id,
            closed.as_ref(),
        ) {
            return;
        }

        let profile = match latest_serial_reconnect_profile(&io, &session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_serial_reconnect_if_disabled(
                    &io,
                    &session_id,
                    &previous_runtime_id,
                    "automatic reconnect disabled by latest profile",
                ) {
                    return;
                }
                continue;
            }
            Err(error) => {
                eprintln!("PortMate: failed to load latest serial reconnect profile: {error}");
                continue;
            }
        };
        let (serial, port_name) = match serial_connection_details(&profile) {
            Ok(details) => details,
            Err(error) => {
                let attempted_port = match &profile.connection {
                    ConnectionConfig::Serial(serial) => serial.port.trim(),
                    _ => "<non-serial>",
                };
                match record_serial_reconnect_failure_if_pending(
                    &io,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    &profile,
                    attempted_port,
                    &error,
                ) {
                    SerialReconnectFailureDisposition::Recorded
                    | SerialReconnectFailureDisposition::RetryLatestProfile => continue,
                    SerialReconnectFailureDisposition::StopDisabled => {
                        if stop_pending_serial_reconnect_if_disabled(
                            &io,
                            &session_id,
                            &previous_runtime_id,
                            "automatic reconnect disabled while validating the latest profile",
                        ) {
                            return;
                        }
                        continue;
                    }
                    SerialReconnectFailureDisposition::Superseded => return,
                }
            }
        };
        let (port, reader) = match open_configured_serial_port(&serial, &port_name) {
            Ok(port) => port,
            Err(error) => {
                match record_serial_reconnect_failure_if_pending(
                    &io,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    &profile,
                    &port_name,
                    &error,
                ) {
                    SerialReconnectFailureDisposition::Recorded
                    | SerialReconnectFailureDisposition::RetryLatestProfile => continue,
                    SerialReconnectFailureDisposition::StopDisabled => {
                        if stop_pending_serial_reconnect_if_disabled(
                            &io,
                            &session_id,
                            &previous_runtime_id,
                            "automatic reconnect disabled while the previous attempt was running",
                        ) {
                            return;
                        }
                        continue;
                    }
                    SerialReconnectFailureDisposition::Superseded => return,
                }
            }
        };

        let runtime_id = Uuid::new_v4().to_string();
        let writer = Arc::new(Mutex::new(port));
        let (tap, _) = broadcast::channel(1024);
        let next_closed = Arc::new(AtomicBool::new(false));
        let capture = match serial_capture_for_session(&io.serial_captures, &session_id) {
            Ok(capture) => capture,
            Err(error) => {
                eprintln!("PortMate: failed to load serial capture buffer: {error}");
                return;
            }
        };
        let install = match io.runtimes.serial.lock() {
            Err(error) => SerialReconnectInstallDecision::Failed(error.to_string()),
            Ok(mut connections) => {
                if connections
                    .get(&session_id)
                    .is_none_or(|runtime| runtime.runtime_id != previous_runtime_id)
                    || closed.load(Ordering::SeqCst)
                {
                    SerialReconnectInstallDecision::Superseded
                } else {
                    match io.store.lock() {
                        Err(error) => SerialReconnectInstallDecision::Failed(error.to_string()),
                        Ok(mut store) => {
                            match serial_reconnect_profile_state(&store, &session_id, &profile) {
                                SerialReconnectProfileState::Changed => {
                                    SerialReconnectInstallDecision::Retry
                                }
                                SerialReconnectProfileState::Disabled => {
                                    if let Some(runtime) = connections.remove(&session_id) {
                                        runtime.closed.store(true, Ordering::SeqCst);
                                    }
                                    let reason = "automatic reconnect disabled while the previous attempt was running";
                                    let _ = store.set_runtime_status_with_reason(
                                        &session_id,
                                        SessionStatus::Disconnected,
                                        Some(reason.to_string()),
                                    );
                                    store.record_system_event(
                                        &session_id,
                                        format!("PortMate: serial reconnect stopped: {reason}"),
                                    );
                                    if let Err(error) = persist_applied_store(
                                        &store,
                                        &io.store_path,
                                        "stopped serial reconnect state",
                                    ) {
                                        eprintln!(
                                            "PortMate: failed to persist serial reconnect stop: {error}"
                                        );
                                    }
                                    SerialReconnectInstallDecision::Stop
                                }
                                SerialReconnectProfileState::Current => {
                                    connections.insert(
                                        session_id.clone(),
                                        SerialRuntime {
                                            runtime_id: runtime_id.clone(),
                                            writer: Some(Arc::clone(&writer)),
                                            tap: tap.clone(),
                                            closed: Arc::clone(&next_closed),
                                            capture: Arc::clone(&capture),
                                        },
                                    );
                                    SerialReconnectInstallDecision::Installed
                                }
                            }
                        }
                    }
                }
            }
        };
        if !matches!(install, SerialReconnectInstallDecision::Installed) {
            match install {
                SerialReconnectInstallDecision::Retry => continue,
                SerialReconnectInstallDecision::Stop
                | SerialReconnectInstallDecision::Superseded => return,
                SerialReconnectInstallDecision::Failed(error) => {
                    eprintln!("PortMate: failed to install serial reconnect runtime: {error}");
                    return;
                }
                SerialReconnectInstallDecision::Installed => unreachable!(),
            }
        }

        let reader_start_gate = Arc::new(ReaderStartGate::default());
        if let Err(error) = spawn_serial_reader(SerialReadTask {
            io: io.clone(),
            profile: profile.clone(),
            runtime_id: runtime_id.clone(),
            port_name: port_name.clone(),
            tap,
            closed: Arc::clone(&next_closed),
            start_gate: Arc::clone(&reader_start_gate),
            reader,
            capture,
            receive_idle_timeout: serial
                .receive_idle_timeout_enabled
                .then(|| Duration::from_secs(serial.receive_idle_timeout_seconds)),
        }) {
            next_closed.store(true, Ordering::SeqCst);
            reader_start_gate.cancel();
            if let Ok(mut connections) = io.runtimes.serial.lock() {
                if connections
                    .get(&session_id)
                    .is_some_and(|runtime| runtime.runtime_id == runtime_id)
                {
                    connections.remove(&session_id);
                }
            }
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Error,
                    Some(format!("serial read thread restart failed: {error}")),
                );
                store.record_system_event(
                    &session_id,
                    format!("PortMate: serial read thread restart failed: {error}"),
                );
                if let Err(save_error) = persist_applied_store(
                    &store,
                    &io.store_path,
                    "failed serial reader restart state",
                ) {
                    eprintln!(
                        "PortMate: failed to persist serial reader restart failure: {save_error}"
                    );
                }
            }
            return;
        }

        let finalize_result = match io.store.lock() {
            Ok(mut store) => {
                commit_tracked_store_mutation(&mut store, &io.store_path, |next_store| {
                    mark_session_connected_with_events(
                        next_store,
                        &profile,
                        [format!(
                            "PortMate: serial port reconnected ({port_name}, {} baud)",
                            serial.baud_rate
                        )],
                    )
                })
            }
            Err(error) => Err(error.to_string()),
        };
        match finalize_result {
            Ok(_) => {
                reader_start_gate.start();
                return;
            }
            Err(error) => {
                next_closed.store(true, Ordering::SeqCst);
                reader_start_gate.cancel();
                if let Ok(mut connections) = io.runtimes.serial.lock() {
                    if connections
                        .get(&session_id)
                        .is_some_and(|runtime| runtime.runtime_id == runtime_id)
                    {
                        connections.remove(&session_id);
                    }
                }
                if let Ok(mut store) = io.store.lock() {
                    let reason = format!("serial reconnect completion failed: {error}");
                    let _ = store.set_runtime_status_with_reason(
                        &session_id,
                        SessionStatus::Error,
                        Some(reason.clone()),
                    );
                    store.record_system_event(&session_id, format!("PortMate: {reason}"));
                }
                eprintln!("PortMate: failed to complete serial reconnect: {error}");
                return;
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

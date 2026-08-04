use super::*;

pub(super) fn spawn_serial_reconnect(
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
                    ConnectionConfig::Serial(serial) => serial.port.as_str(),
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

use super::*;

pub(super) fn spawn_serial_reconnect(
    io: SessionIo,
    session_id: String,
    previous_runtime_id: String,
    closed: Arc<AtomicBool>,
) {
    let thread_name = format!("portmate-serial-reconnect-{session_id}");
    let worker_io = io.clone();
    let worker_session_id = session_id.clone();
    let worker_runtime_id = previous_runtime_id.clone();
    let worker_closed = Arc::clone(&closed);
    if let Err(error) = std::thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            reconnect_serial_session(
                worker_io,
                worker_session_id,
                worker_runtime_id,
                worker_closed,
            )
        })
    {
        fail_pending_serial_reconnect_install(
            &io,
            &session_id,
            &previous_runtime_id,
            closed.as_ref(),
            &format!("reconnect thread start failed: {error}"),
        );
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
                fail_pending_serial_reconnect_install(
                    &io,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    &format!("capture registry unavailable: {error}"),
                );
                eprintln!("PortMate: failed to load serial capture buffer: {error}");
                return;
            }
        };
        let reader_start_gate = Arc::new(ReaderStartGate::default());
        if let Err(error) = spawn_serial_reader(SerialReadTask {
            io: io.clone(),
            profile: profile.clone(),
            runtime_id: runtime_id.clone(),
            port_name: port_name.clone(),
            tap: tap.clone(),
            closed: Arc::clone(&next_closed),
            start_gate: Arc::clone(&reader_start_gate),
            reader,
            capture: Arc::clone(&capture),
            receive_idle_timeout: serial
                .receive_idle_timeout_enabled
                .then(|| Duration::from_secs(serial.receive_idle_timeout_seconds)),
        }) {
            next_closed.store(true, Ordering::SeqCst);
            reader_start_gate.cancel();
            let error = format!("serial read thread restart failed: {error}");
            fail_pending_serial_reconnect_install(
                &io,
                &session_id,
                &previous_runtime_id,
                closed.as_ref(),
                &error,
            );
            eprintln!("PortMate: {error}");
            return;
        }

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
                                    let committed = commit_tracked_store_mutation(
                                        &mut store,
                                        &io.store_path,
                                        |next_store| {
                                            mark_session_connected_with_events(
                                                next_store,
                                                &profile,
                                                [format!(
                                                    "PortMate: serial port reconnected ({port_name}, {} baud)",
                                                    serial.baud_rate
                                                )],
                                            )
                                        },
                                    );
                                    match committed {
                                        Ok(_) => {
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
                                        Err(error) => {
                                            SerialReconnectInstallDecision::Failed(error)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };
        if !matches!(install, SerialReconnectInstallDecision::Installed) {
            next_closed.store(true, Ordering::SeqCst);
            reader_start_gate.cancel();
            match install {
                SerialReconnectInstallDecision::Retry => continue,
                SerialReconnectInstallDecision::Stop
                | SerialReconnectInstallDecision::Superseded => return,
                SerialReconnectInstallDecision::Failed(error) => {
                    fail_pending_serial_reconnect_install(
                        &io,
                        &session_id,
                        &previous_runtime_id,
                        closed.as_ref(),
                        &error,
                    );
                    eprintln!("PortMate: failed to install serial reconnect runtime: {error}");
                    return;
                }
                SerialReconnectInstallDecision::Installed => unreachable!(),
            }
        }
        reader_start_gate.start();
        return;
    }
}

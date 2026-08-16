use super::transport_timing::STREAM_PERSIST_INTERVAL;
use super::*;

pub(super) struct TcpRuntime {
    pub(super) runtime_id: String,
    pub(super) writer: Arc<tokio::sync::Mutex<TcpWriteHalf>>,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) telnet: Option<Arc<TelnetRuntimeState>>,
}

pub(super) async fn open_tcp_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    let (tcp, label) = tcp_connection_details(&profile)?;
    if let Some(existing) = {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        existing.closed.store(true, Ordering::SeqCst);
        let mut writer = existing.writer.lock().await;
        let _ = writer.shutdown().await;
    }

    let stream = connect_tcp_transport(&tcp, label).await?;

    let runtime_id = Uuid::new_v4().to_string();
    let (read_half, write_half) = stream.split();
    let writer = Arc::new(tokio::sync::Mutex::new(write_half));
    let (tap, _) = broadcast::channel(1024);
    let closed = Arc::new(AtomicBool::new(false));
    let telnet = TelnetRuntimeState::from_profile(&profile);
    {
        let mut connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: runtime_id.clone(),
                writer: Arc::clone(&writer),
                tap: tap.clone(),
                closed: Arc::clone(&closed),
                telnet: telnet.as_ref().map(Arc::clone),
            },
        );
    }

    let finalize_result = match state.store.lock() {
        Ok(mut store) => {
            commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
                mark_session_connected_with_events(
                    next_store,
                    &profile,
                    [format!("PortMate: {label} socket connected")],
                )
            })
        }
        Err(error) => Err(error.to_string()),
    };
    let summary = match finalize_result {
        Ok(summary) => summary,
        Err(error) => {
            closed.store(true, Ordering::SeqCst);
            let cleanup_error = remove_runtime_if_owned(&state.tcp, &profile.id, |runtime| {
                runtime.runtime_id == runtime_id
            })
            .err();
            let shutdown_error = writer.lock().await.shutdown().await.err();
            let mut errors = vec![error];
            if let Some(cleanup_error) = cleanup_error {
                errors.push(format!("{label} runtime cleanup failed: {cleanup_error}"));
            }
            if let Some(shutdown_error) = shutdown_error {
                errors.push(format!("{label} socket shutdown failed: {shutdown_error}"));
            }
            return Err(errors.join("; "));
        }
    };

    tauri::async_runtime::spawn(read_tcp_stream(TcpReadTask {
        state: state.clone(),
        profile,
        runtime_id,
        label: label.to_string(),
        tap,
        writer,
        read_half,
        closed,
        telnet,
    }));
    Ok(summary)
}

pub(super) struct TcpReadTask {
    pub(super) state: AppState,
    pub(super) profile: SessionProfile,
    pub(super) runtime_id: String,
    pub(super) label: String,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) writer: Arc<tokio::sync::Mutex<TcpWriteHalf>>,
    pub(super) read_half: TcpReadHalf,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) telnet: Option<Arc<TelnetRuntimeState>>,
}

enum TcpReaderTransition {
    Disconnect,
    Reconnect,
}

pub(super) fn read_tcp_stream(
    task: TcpReadTask,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        let TcpReadTask {
            state,
            profile,
            runtime_id,
            label,
            tap,
            writer,
            mut read_half,
            closed,
            telnet,
        } = task;
        let io = state.session_io();
        let session_id = profile.id.clone();
        let mut buffer = vec![0_u8; 8192];
        let mut last_persist = Instant::now();
        let mut has_unpersisted_stream = false;
        let mut telnet = telnet.map(TelnetNegotiator::new);
        let mut disconnect_reason = None;
        let mut disconnect_reason_recorded = false;

        'read_loop: loop {
            match read_half.read(&mut buffer).await {
                Ok(0) => break,
                Ok(size) => {
                    let telnet_lane = if telnet.is_some() {
                        match outbound_lane(&io.store_path, &session_id) {
                            Ok(lane) => Some(lane),
                            Err(error) => {
                                disconnect_reason = Some(format!(
                                    "{label} Telnet negotiation outbound lane failed: {error}"
                                ));
                                eprintln!(
                                    "PortMate: Telnet negotiation outbound lane failed: {error}"
                                );
                                break 'read_loop;
                            }
                        }
                    } else {
                        None
                    };
                    let _telnet_lane_guard = if let Some(lane) = telnet_lane.as_ref() {
                        Some(lane.lock().await)
                    } else {
                        None
                    };
                    let (bytes, replies) = if let Some(negotiator) = telnet.as_mut() {
                        negotiator.filter(&buffer[..size])
                    } else {
                        (buffer[..size].to_vec(), Vec::new())
                    };
                    let has_protocol_bytes = !bytes.is_empty();
                    let accepted = record_channel_bytes_with_accepted_side_effect(
                        &io,
                        &session_id,
                        Some(&runtime_id),
                        EventStream::Stdout,
                        &buffer[..size],
                        String::from_utf8_lossy(&bytes).to_string(),
                        || {
                            if has_protocol_bytes {
                                let _ = tap.send(bytes.clone());
                            }
                        },
                    );
                    if accepted && has_protocol_bytes {
                        has_unpersisted_stream = true;
                    }
                    for reply in replies {
                        let write_result = {
                            let mut writer = writer.lock().await;
                            writer.write_all(&reply).await
                        };
                        if let Err(error) = write_result {
                            let reason =
                                format!("{label} Telnet negotiation reply failed: {error}");
                            disconnect_reason = Some(reason.clone());
                            disconnect_reason_recorded = record_runtime_system_event(
                                &io,
                                &session_id,
                                &runtime_id,
                                format!("PortMate: {reason}"),
                                "Telnet negotiation failure event",
                            );
                            break 'read_loop;
                        }
                        record_outbound_control_event_for_runtime(
                            &io,
                            &session_id,
                            &runtime_id,
                            &reply,
                            "telnet-negotiation",
                            true,
                        );
                    }
                    if bytes.is_empty() {
                        continue;
                    }
                }
                Err(error) => {
                    let reason = format!("{label} read failed: {error}");
                    disconnect_reason = Some(reason.clone());
                    disconnect_reason_recorded = record_runtime_system_event(
                        &io,
                        &session_id,
                        &runtime_id,
                        format!("PortMate: {reason}"),
                        "TCP/Telnet read failure event",
                    );
                    break;
                }
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                    eprintln!("PortMate: failed to persist {label} stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if let Some(negotiator) = telnet.as_mut() {
            let bytes = negotiator.finish();
            if !bytes.is_empty() {
                let accepted = record_channel_bytes_with_accepted_side_effect(
                    &io,
                    &session_id,
                    Some(&runtime_id),
                    EventStream::Stdout,
                    &[],
                    String::from_utf8_lossy(&bytes).to_string(),
                    || {
                        let _ = tap.send(bytes.clone());
                    },
                );
                has_unpersisted_stream |= accepted;
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                eprintln!("PortMate: failed to persist final {label} stream data: {error}");
            }
        }

        let disconnect_reason = portmate_core::normalize_session_disconnect_reason(
            &disconnect_reason.unwrap_or_else(|| format!("{label} socket closed")),
        )
        .unwrap_or_else(|| format!("{label} socket closed"));

        let transition =
            match with_current_session_runtime_store(&io, &session_id, &runtime_id, |store| {
                clear_active_command(&io, &session_id);
                let reconnect_profile = (!closed.load(Ordering::SeqCst))
                    .then(|| store.profile(&session_id).map(normalize_session_profile))
                    .flatten()
                    .filter(tcp_reconnect_enabled);
                if let Some(reconnect_profile) = reconnect_profile {
                    let reconnect_delay = tcp_reconnect_delay(&reconnect_profile);
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
                    if let Err(error) = persist_applied_store(
                        store,
                        &io.store_path,
                        "TCP/Telnet reconnect transition",
                    ) {
                        eprintln!("PortMate: failed to persist {label} reconnect event: {error}");
                    }
                    TcpReaderTransition::Reconnect
                } else {
                    let _ = store.set_runtime_status_with_reason(
                        &session_id,
                        SessionStatus::Disconnected,
                        Some(disconnect_reason.clone()),
                    );
                    if !disconnect_reason_recorded {
                        store.record_system_event(
                            &session_id,
                            format!("PortMate: {disconnect_reason}"),
                        );
                    }
                    if let Err(error) = persist_applied_store(
                        store,
                        &io.store_path,
                        "TCP/Telnet disconnect transition",
                    ) {
                        eprintln!("PortMate: failed to persist {label} close event: {error}");
                    }
                    TcpReaderTransition::Disconnect
                }
            }) {
                Ok(Some(transition)) => transition,
                Ok(None) => return,
                Err(error) => {
                    eprintln!("PortMate: failed to commit {label} reader transition: {error}");
                    if let Ok(Some(runtime)) =
                        remove_runtime_if_owned(&io.runtimes.tcp, &session_id, |runtime| {
                            runtime.runtime_id == runtime_id
                        })
                    {
                        runtime.closed.store(true, Ordering::SeqCst);
                    }
                    return;
                }
            };

        match transition {
            TcpReaderTransition::Reconnect => {
                let still_current = io.runtimes.tcp.lock().ok().is_some_and(|connections| {
                    connections
                        .get(&session_id)
                        .is_some_and(|runtime| runtime.runtime_id == runtime_id)
                });
                if still_current {
                    tauri::async_runtime::spawn(reconnect_tcp_session(
                        state, session_id, runtime_id, closed,
                    ));
                }
            }
            TcpReaderTransition::Disconnect => {
                if let Ok(Some(runtime)) =
                    remove_runtime_if_owned(&io.runtimes.tcp, &session_id, |runtime| {
                        runtime.runtime_id == runtime_id
                    })
                {
                    runtime.closed.store(true, Ordering::SeqCst);
                }
            }
        }
    })
}

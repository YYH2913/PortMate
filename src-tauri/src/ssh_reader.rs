use super::transport_timing::STREAM_PERSIST_INTERVAL;
use super::*;

pub(super) struct SshReadTask {
    pub(super) state: AppState,
    pub(super) profile: SessionProfile,
    pub(super) runtime_id: String,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) read_half: SshBackendChannelReader,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) terminal_channel_open: Arc<AtomicBool>,
    pub(super) reader_finished: tokio::sync::oneshot::Sender<()>,
}

struct SshReaderCompletionGuard {
    terminal_channel_open: Arc<AtomicBool>,
    reader_finished: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for SshReaderCompletionGuard {
    fn drop(&mut self) {
        self.terminal_channel_open.store(false, Ordering::SeqCst);
        if let Some(sender) = self.reader_finished.take() {
            let _ = sender.send(());
        }
    }
}

pub(super) fn read_ssh_channel(
    task: SshReadTask,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
    Box::pin(async move {
        let SshReadTask {
            state,
            profile,
            runtime_id,
            tap,
            mut read_half,
            closed,
            terminal_channel_open,
            reader_finished,
        } = task;
        let _reader_completion = SshReaderCompletionGuard {
            terminal_channel_open,
            reader_finished: Some(reader_finished),
        };
        let io = state.session_io();
        let session_id = profile.id.clone();
        let mut last_persist = Instant::now();
        let mut has_unpersisted_stream = false;
        let mut disconnect_reason = None;

        loop {
            if closed.load(Ordering::SeqCst) {
                break;
            }
            let Some(message) = read_half.wait_until_closed(&closed).await else {
                break;
            };
            if let Some(reason) = ssh_channel_disconnect_reason(&message) {
                disconnect_reason = Some(reason);
            }
            match message {
                SshBackendMessage::Data(bytes) => {
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
                SshBackendMessage::ExtendedData { data: bytes, ext } => {
                    let _ = tap.send(bytes.clone());
                    let stream = if ext == 1 {
                        EventStream::Stderr
                    } else {
                        EventStream::Stdout
                    };
                    record_channel_bytes(
                        &io,
                        &session_id,
                        Some(&runtime_id),
                        stream,
                        &bytes,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                SshBackendMessage::ExitStatus(exit_status) => {
                    record_runtime_system_event(
                        &io,
                        &session_id,
                        &runtime_id,
                        format!("PortMate: SSH remote process exited with status {exit_status}"),
                        "SSH exit status event",
                    );
                }
                SshBackendMessage::ExitSignal {
                    signal_name,
                    error_message,
                    ..
                } => {
                    record_runtime_system_event(
                        &io,
                        &session_id,
                        &runtime_id,
                        format!(
                            "PortMate: SSH remote process exited by signal {signal_name} {error_message}"
                        ),
                        "SSH exit signal event",
                    );
                }
                SshBackendMessage::Error(_) | SshBackendMessage::Eof | SshBackendMessage::Close => {
                    break
                }
                _ => {}
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                    eprintln!("PortMate: failed to persist SSH stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                eprintln!("PortMate: failed to persist final SSH stream data: {error}");
            }
        }

        let disconnect_reason = portmate_core::normalize_session_disconnect_reason(
            &disconnect_reason.unwrap_or_else(|| "SSH channel closed".to_string()),
        )
        .unwrap_or_else(|| "SSH channel closed".to_string());

        let (should_reconnect, stopped_tunnel_runtimes) = {
            let mut connections = match io.runtimes.ssh.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_none_or(|runtime| runtime.runtime_id != runtime_id)
            {
                return;
            }
            clear_active_command(&io, &session_id);

            let mut store = match io.store.lock() {
                Ok(store) => store,
                Err(_) => {
                    connections.remove(&session_id);
                    return;
                }
            };
            let stopped_tunnel_runtimes =
                match fail_session_tunnel_runtimes(&state.tunnels, &session_id, &disconnect_reason)
                {
                    Ok(runtimes) => runtimes,
                    Err(error) => {
                        eprintln!("PortMate: failed to clean up SSH tunnel runtimes: {error}");
                        Vec::new()
                    }
                };
            let stopped_tunnels = stopped_tunnel_runtimes.len();
            let reconnect_profile = (!closed.load(Ordering::SeqCst))
                .then(|| store.profile(&session_id).map(normalize_session_profile))
                .flatten()
                .filter(ssh_reconnect_enabled);
            if let Some(reconnect_profile) = reconnect_profile {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Reconnecting,
                    Some(disconnect_reason.clone()),
                );
                store.record_system_event(
                    &session_id,
                    format!(
                        "PortMate: {disconnect_reason}; stopped {stopped_tunnels} tunnel runtime(s); reconnecting in {}ms",
                        ssh_reconnect_delay(&reconnect_profile).as_millis()
                    ),
                );
                if let Err(error) =
                    persist_applied_store(&store, &io.store_path, "SSH reconnect transition")
                {
                    eprintln!("PortMate: failed to persist SSH reconnect event: {error}");
                }
                (true, stopped_tunnel_runtimes)
            } else {
                if let Some(runtime) = connections.remove(&session_id) {
                    runtime.closed.store(true, Ordering::SeqCst);
                }
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some(disconnect_reason.clone()),
                );
                store.record_system_event(
                    &session_id,
                    format!(
                        "PortMate: {disconnect_reason}; stopped {stopped_tunnels} tunnel runtime(s)"
                    ),
                );
                if let Err(error) =
                    persist_applied_store(&store, &io.store_path, "SSH disconnect transition")
                {
                    eprintln!("PortMate: failed to persist SSH close event: {error}");
                }
                (false, stopped_tunnel_runtimes)
            }
        };

        let timed_out_tunnels = await_tunnel_listener_shutdowns(&stopped_tunnel_runtimes).await;
        if !timed_out_tunnels.is_empty() {
            let message = format!(
                "PortMate: timed out waiting for SSH tunnel listener shutdown: {}",
                timed_out_tunnels.join(", ")
            );
            eprintln!("{message}");
            if let Ok(mut store) = io.store.lock() {
                store.record_system_event(&session_id, message);
                if let Err(error) = persist_applied_store(
                    &store,
                    &io.store_path,
                    "SSH tunnel listener shutdown timeout",
                ) {
                    eprintln!(
                        "PortMate: failed to persist tunnel listener shutdown timeout: {error}"
                    );
                }
            }
        }

        if should_reconnect {
            tauri::async_runtime::spawn(reconnect_ssh_session(
                state, session_id, runtime_id, closed,
            ));
        }
    })
}

pub(super) fn ssh_channel_disconnect_reason(message: &SshBackendMessage) -> Option<String> {
    match message {
        SshBackendMessage::ExitStatus(exit_status) => Some(format!(
            "SSH remote process exited with status {exit_status}"
        )),
        SshBackendMessage::ExitSignal {
            signal_name,
            error_message,
            ..
        } => {
            let detail = error_message.trim();
            let suffix = (!detail.is_empty()).then(|| format!(": {detail}"));
            Some(format!(
                "SSH remote process exited by signal {signal_name}{}",
                suffix.unwrap_or_default()
            ))
        }
        SshBackendMessage::Error(error) => Some(format!("SSH channel read failed: {error}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_reader_completion_closes_the_terminal_health_flag() {
        let terminal_channel_open = Arc::new(AtomicBool::new(true));
        let (sender, mut receiver) = tokio::sync::oneshot::channel();
        {
            let _guard = SshReaderCompletionGuard {
                terminal_channel_open: Arc::clone(&terminal_channel_open),
                reader_finished: Some(sender),
            };
        }
        assert!(!terminal_channel_open.load(Ordering::SeqCst));
        assert_eq!(receiver.try_recv(), Ok(()));
    }
}

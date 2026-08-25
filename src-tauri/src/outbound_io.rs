use super::*;

type OutboundLanes = Mutex<HashMap<(PathBuf, String), Weak<tokio::sync::Mutex<()>>>>;

static OUTBOUND_LANES: OnceLock<OutboundLanes> = OnceLock::new();
const OUTBOUND_LANE_WAIT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) fn outbound_lane(
    store_path: &Path,
    session_id: &str,
) -> Result<Arc<tokio::sync::Mutex<()>>, String> {
    let key = (store_path.to_path_buf(), session_id.to_string());
    let mut lanes = OUTBOUND_LANES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "outbound lane registry poisoned".to_string())?;
    lanes.retain(|_, lane| lane.strong_count() > 0);
    if let Some(lane) = lanes.get(&key).and_then(Weak::upgrade) {
        return Ok(lane);
    }
    let lane = Arc::new(tokio::sync::Mutex::new(()));
    lanes.insert(key, Arc::downgrade(&lane));
    Ok(lane)
}

pub(super) async fn acquire_outbound_lane_with_timeout(
    store_path: &Path,
    session_id: &str,
    timeout: Duration,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    let lane = outbound_lane(store_path, session_id)?;
    tokio::time::timeout(timeout, lane.lock_owned())
        .await
        .map_err(|_| format!("出站队列等待超时（{} ms）", timeout.as_millis()))
}

pub(super) async fn acquire_outbound_lane(
    store_path: &Path,
    session_id: &str,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    acquire_outbound_lane_with_timeout(store_path, session_id, OUTBOUND_LANE_WAIT_TIMEOUT).await
}

pub(super) fn clear_outbound_lane(store_path: &Path, session_id: &str) {
    if let Some(lanes) = OUTBOUND_LANES.get() {
        lanes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(store_path.to_path_buf(), session_id.to_string()));
    }
}

pub(super) fn current_session_runtime_id(
    runtimes: &RuntimeRegistry,
    session_id: &str,
) -> Result<Option<String>, String> {
    fn merge_runtime_id(
        current: &mut Option<String>,
        candidate: Option<String>,
    ) -> Result<(), String> {
        let Some(candidate) = candidate else {
            return Ok(());
        };
        if current.is_some() {
            return Err("会话存在多个活动连接，拒绝发送输入".to_string());
        }
        *current = Some(candidate);
        Ok(())
    }

    let mut current = None;
    merge_runtime_id(
        &mut current,
        runtimes
            .ssh
            .lock()
            .map_err(|error| error.to_string())?
            .get(session_id)
            .filter(|runtime| !runtime.closed.load(Ordering::SeqCst))
            .map(|runtime| runtime.runtime_id.clone()),
    )?;
    merge_runtime_id(
        &mut current,
        runtimes
            .shell
            .lock()
            .map_err(|error| error.to_string())?
            .get(session_id)
            .filter(|runtime| !runtime.closed.load(Ordering::SeqCst))
            .map(|runtime| runtime.runtime_id.clone()),
    )?;
    merge_runtime_id(
        &mut current,
        runtimes
            .tcp
            .lock()
            .map_err(|error| error.to_string())?
            .get(session_id)
            .filter(|runtime| !runtime.closed.load(Ordering::SeqCst))
            .map(|runtime| runtime.runtime_id.clone()),
    )?;
    merge_runtime_id(
        &mut current,
        runtimes
            .serial
            .lock()
            .map_err(|error| error.to_string())?
            .get(session_id)
            .filter(|runtime| !runtime.closed.load(Ordering::SeqCst))
            .map(|runtime| runtime.runtime_id.clone()),
    )?;
    Ok(current)
}

pub(super) fn write_serial_payload<Writer: Write + ?Sized>(
    writer: &mut Writer,
    bytes: &[u8],
) -> std::io::Result<()> {
    // SerialPort::flush drains the physical device queue (tcdrain on Unix and
    // FlushFileBuffers on Windows). Flow control or a stalled USB driver can
    // make that wait indefinitely, so interactive writes must only enqueue.
    writer.write_all(bytes)
}

async fn write_serial_port_bytes(
    worker: SerialWorkerGuard,
    writer: Arc<Mutex<SerialPortHandle>>,
    closed: Arc<AtomicBool>,
    bytes: &[u8],
    label: &'static str,
) -> Result<(), String> {
    let bytes = bytes.to_vec();
    tauri::async_runtime::spawn_blocking(move || {
        let _worker = worker;
        if closed.load(Ordering::SeqCst) {
            return Err(format!("{label}失败: 串口会话已关闭"));
        }
        let mut writer = writer
            .lock()
            .map_err(|error| format!("{label}失败: {error}"))?;
        if closed.load(Ordering::SeqCst) {
            return Err(format!("{label}失败: 串口会话已关闭"));
        }
        write_serial_payload(&mut **writer, &bytes)
            .map_err(|error| format!("{label}失败: {error}"))?;
        if closed.load(Ordering::SeqCst) {
            return Err(format!("{label}已中止: 串口会话已关闭"));
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("{label}任务异常终止: {error}"))?
}

pub(super) async fn write_session_bytes_for_runtime(
    store: &Arc<Mutex<SessionStore>>,
    runtimes: &RuntimeRegistry,
    serial_workers: &Arc<SerialWorkerRegistry>,
    session_id: &str,
    bytes: &[u8],
    expected_runtime_id: Option<&str>,
) -> Result<(), String> {
    let writer = {
        let connections = runtimes.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .filter(|runtime| {
                expected_runtime_id.is_none_or(|expected| runtime.runtime_id == expected)
            })
            .map(|runtime| Arc::clone(&runtime.writer))
    };

    if let Some(writer) = writer {
        write_ssh_channel_bytes_with_timeout(
            &writer,
            bytes,
            SSH_TERMINAL_WRITE_TIMEOUT,
            "SSH 写入",
        )
        .await?;
    } else {
        let writer = {
            let connections = runtimes.shell.lock().map_err(|error| error.to_string())?;
            connections
                .get(session_id)
                .filter(|runtime| {
                    expected_runtime_id.is_none_or(|expected| runtime.runtime_id == expected)
                })
                .map(|runtime| Arc::clone(&runtime.writer))
        };
        if let Some(writer) = writer {
            let mut writer = writer.lock().map_err(|error| error.to_string())?;
            writer
                .write_all(bytes)
                .map_err(|error| format!("Shell PTY 写入失败: {error}"))?;
            writer
                .flush()
                .map_err(|error| format!("Shell PTY 刷新失败: {error}"))?;
        } else {
            let writer = {
                let connections = runtimes.tcp.lock().map_err(|error| error.to_string())?;
                connections
                    .get(session_id)
                    .filter(|runtime| {
                        expected_runtime_id.is_none_or(|expected| runtime.runtime_id == expected)
                    })
                    .map(|runtime| Arc::clone(&runtime.writer))
            };
            if let Some(writer) = writer {
                write_tcp_bytes(&writer, bytes, "TCP/Telnet 写入").await?;
            } else {
                // Register before reading the runtime entry. Once close begins
                // waiting for this session, no captured serial handle may
                // appear outside the tracked worker set.
                let serial_worker = serial_workers
                    .register_for_session(session_id)
                    .map_err(|error| format!("串口写入任务被拒绝: {error}"))?;
                let serial_writer = {
                    let connections = runtimes.serial.lock().map_err(|error| error.to_string())?;
                    connections
                        .get(session_id)
                        .filter(|runtime| {
                            expected_runtime_id
                                .is_none_or(|expected| runtime.runtime_id == expected)
                        })
                        .map(|runtime| {
                            (
                                runtime.writer.as_ref().map(Arc::clone),
                                Arc::clone(&runtime.capture),
                                Arc::clone(&runtime.closed),
                            )
                        })
                };
                match serial_writer {
                    Some((Some(writer), capture, closed)) => {
                        write_serial_port_bytes(
                            serial_worker,
                            writer,
                            closed,
                            bytes,
                            "串口写入",
                        )
                        .await?;
                        record_serial_capture(&capture, EventDirection::Outbound, bytes);
                    }
                    Some((None, _, _)) => return Err("串口正在重连，无法发送输入".to_string()),
                    None if expected_runtime_id.is_some() => {
                        return Err("触发动作来源连接已关闭或被新连接替换".to_string());
                    }
                    None if profile_requires_runtime(store, session_id)? => {
                        return Err("会话尚未连接，无法发送输入".to_string());
                    }
                    None => {}
                }
            }
        }
    }
    Ok(())
}

pub(super) fn outbound_text_for_session(
    store: &Arc<Mutex<SessionStore>>,
    tcp_runtimes: &Arc<Mutex<HashMap<String, TcpRuntime>>>,
    session_id: &str,
    text: &str,
) -> Result<String, String> {
    if is_telnet_session(store, session_id)? {
        let local_binary = {
            let runtimes = tcp_runtimes.lock().map_err(|error| error.to_string())?;
            runtimes
                .get(session_id)
                .and_then(|runtime| runtime.telnet.as_ref())
                .is_some_and(|telnet| telnet.local_binary.load(Ordering::SeqCst))
        };
        Ok(encode_telnet_outbound_text(text, local_binary))
    } else {
        Ok(text.to_string())
    }
}

/// Encodes queued desktop input from the installed runtime only. The caller
/// already captured and later revalidates the runtime generation, so the hot
/// path does not need to wait for the persisted Store lock.
pub(super) fn outbound_text_for_active_runtime(
    runtimes: &RuntimeRegistry,
    session_id: &str,
    text: &str,
) -> Result<String, String> {
    let tcp = runtimes.tcp.lock().map_err(|error| error.to_string())?;
    let Some(runtime) = tcp.get(session_id) else {
        return Ok(text.to_string());
    };
    let Some(telnet) = runtime.telnet.as_ref() else {
        return Ok(text.to_string());
    };
    Ok(encode_telnet_outbound_text(
        text,
        telnet.local_binary.load(Ordering::SeqCst),
    ))
}

pub(super) fn outbound_bytes_for_session(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
    bytes: &[u8],
) -> Result<Vec<u8>, String> {
    Ok(if is_telnet_session(store, session_id)? {
        encode_telnet_outbound_bytes(bytes)
    } else {
        bytes.to_vec()
    })
}

pub(super) fn is_telnet_session(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<bool, String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    Ok(store
        .profile(session_id)
        .is_some_and(|profile| matches!(profile.connection, ConnectionConfig::Telnet(_))))
}

#[cfg(test)]
pub(super) async fn write_runtime_bytes(
    state: &AppState,
    session_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    write_runtime_bytes_for_runtime(state, session_id, bytes, None).await
}

pub(super) async fn write_runtime_bytes_for_runtime(
    state: &AppState,
    session_id: &str,
    bytes: &[u8],
    expected_runtime_id: Option<&str>,
) -> Result<(), String> {
    let io = state.session_io();
    let lane_guard = acquire_outbound_lane(&io.store_path, session_id).await?;
    write_runtime_bytes_for_runtime_with_lane(
        state,
        session_id,
        bytes,
        expected_runtime_id,
        &lane_guard,
    )
    .await
}

/// Write bytes while the caller owns the session outbound lane. Long-running
/// modem transfers use this form to keep interactive input from interleaving
/// with their command setup and cleanup bytes.
pub(super) async fn write_runtime_bytes_for_runtime_with_lane(
    state: &AppState,
    session_id: &str,
    bytes: &[u8],
    expected_runtime_id: Option<&str>,
    _lane_guard: &tokio::sync::OwnedMutexGuard<()>,
) -> Result<(), String> {
    let io = state.session_io();
    let wire_bytes = outbound_bytes_for_session(&io.store, session_id, bytes)?;
    clear_active_command(&io, session_id);
    let ssh_writer = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .filter(|runtime| {
                expected_runtime_id.is_none_or(|expected| runtime.runtime_id == expected)
            })
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = ssh_writer {
        let writer = writer.lock().await;
        writer
            .data(wire_bytes.as_slice())
            .await
            .map_err(|error| format!("SSH modem 写入失败: {error}"))?;
        record_outbound_control_event_for_optional_runtime_with_accepted_side_effect(
            &io,
            session_id,
            expected_runtime_id,
            &wire_bytes,
            "modem",
            false,
            || {},
        );
        return Ok(());
    }

    let shell_writer = {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .filter(|runtime| {
                expected_runtime_id.is_none_or(|expected| runtime.runtime_id == expected)
            })
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = shell_writer {
        let mut writer = writer.lock().map_err(|error| error.to_string())?;
        writer
            .write_all(&wire_bytes)
            .map_err(|error| format!("Shell modem 写入失败: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("Shell modem 刷新失败: {error}"))?;
        record_outbound_control_event_for_optional_runtime_with_accepted_side_effect(
            &io,
            session_id,
            expected_runtime_id,
            &wire_bytes,
            "modem",
            false,
            || {},
        );
        return Ok(());
    }

    let tcp_writer = {
        let connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .filter(|runtime| {
                expected_runtime_id.is_none_or(|expected| runtime.runtime_id == expected)
            })
            .map(|runtime| Arc::clone(&runtime.writer))
    };
    if let Some(writer) = tcp_writer {
        write_tcp_bytes(&writer, &wire_bytes, "TCP/Telnet modem 写入").await?;
        record_outbound_control_event_for_optional_runtime_with_accepted_side_effect(
            &io,
            session_id,
            expected_runtime_id,
            &wire_bytes,
            "modem",
            false,
            || {},
        );
        return Ok(());
    }

    let serial_writer = {
        // See the interactive path above: worker registration must precede
        // runtime handle capture so close/reopen cannot miss this write.
        let serial_worker = state
            .serial_workers
            .register_for_session(session_id)
            .map_err(|error| format!("串口 modem 写入任务被拒绝: {error}"))?;
        let connections = state.serial.lock().map_err(|error| error.to_string())?;
        let runtime = connections
            .get(session_id)
            .filter(|runtime| {
                expected_runtime_id.is_none_or(|expected| runtime.runtime_id == expected)
            })
            .map(|runtime| {
                (
                    runtime.writer.as_ref().map(Arc::clone),
                    Arc::clone(&runtime.capture),
                    Arc::clone(&runtime.closed),
                )
            });
        (serial_worker, runtime)
    };
    let (serial_worker, serial_writer) = serial_writer;
    match serial_writer {
        Some((Some(writer), capture, closed)) => {
            write_serial_port_bytes(
                serial_worker,
                writer,
                closed,
                &wire_bytes,
                "串口 modem 写入",
            )
            .await?;
            record_outbound_control_event_for_optional_runtime_with_accepted_side_effect(
                &io,
                session_id,
                expected_runtime_id,
                &wire_bytes,
                "modem",
                false,
                || record_serial_capture(&capture, EventDirection::Outbound, &wire_bytes),
            );
            return Ok(());
        }
        Some((None, _, _)) => return Err("串口正在重连，无法执行 modem 写入".to_string()),
        None => {}
    }

    Err(if expected_runtime_id.is_some() {
        "Modem 来源连接已关闭或被新连接替换".to_string()
    } else {
        "会话尚未连接，无法执行 modem 写入".to_string()
    })
}

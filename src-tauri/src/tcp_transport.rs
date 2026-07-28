use super::*;
use native_tls::TlsConnector as NativeTlsConnector;
use socket2::TcpKeepalive;
#[cfg(test)]
use tokio::net::tcp::OwnedWriteHalf;
use tokio_native_tls::{TlsConnector as TokioTlsConnector, TlsStream};

const TCP_CONNECTION_SETUP_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) type TcpReadHalf = Box<dyn AsyncRead + Send + Unpin>;
pub(super) type TcpWriteHalf = Box<dyn AsyncWrite + Send + Unpin>;

enum TcpConnectedStream {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl TcpConnectedStream {
    fn split(self) -> (TcpReadHalf, TcpWriteHalf) {
        match self {
            Self::Plain(stream) => {
                let (reader, writer) = tokio::io::split(stream);
                (Box::new(reader), Box::new(writer))
            }
            Self::Tls(stream) => {
                let (reader, writer) = tokio::io::split(stream);
                (Box::new(reader), Box::new(writer))
            }
        }
    }
}

#[cfg(test)]
pub(super) fn box_tcp_write_half(writer: OwnedWriteHalf) -> TcpWriteHalf {
    Box::new(writer)
}

pub(super) struct TcpRuntime {
    pub(super) runtime_id: String,
    pub(super) writer: Arc<tokio::sync::Mutex<TcpWriteHalf>>,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) telnet: Option<Arc<TelnetRuntimeState>>,
}

pub(super) fn tcp_connection_details(
    profile: &SessionProfile,
) -> Result<(TcpConnection, &'static str), String> {
    let (mut tcp, label) = match &profile.connection {
        ConnectionConfig::Tcp(tcp) => (tcp.clone(), "TCP"),
        ConnectionConfig::Telnet(tcp) => (tcp.clone(), "Telnet"),
        _ => return Err("profile is not TCP/Telnet-backed".to_string()),
    };
    tcp.host = tcp.host.trim().to_string();
    tcp.normalize_health_settings();
    if tcp.host.is_empty() {
        return Err(format!("{label} 主机不能为空"));
    }
    if tcp.port == 0 {
        return Err(format!("{label} 端口不能为空"));
    }
    Ok((tcp, label))
}

pub(super) fn tcp_reconnect_enabled(profile: &SessionProfile) -> bool {
    match &profile.connection {
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => tcp.reconnect,
        _ => false,
    }
}

pub(super) fn tcp_reconnect_attempt_matches_profile(
    attempt: &SessionProfile,
    latest: &SessionProfile,
) -> bool {
    let attempt = normalize_session_profile(attempt.clone());
    let latest = normalize_session_profile(latest.clone());
    tcp_reconnect_enabled(&latest)
        && attempt.connection == latest.connection
        && attempt.terminal == latest.terminal
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TcpReconnectProfileState {
    Current,
    Changed,
    Disabled,
}

pub(super) fn tcp_reconnect_profile_state(
    store: &SessionStore,
    session_id: &str,
    attempt: &SessionProfile,
) -> TcpReconnectProfileState {
    let Some(latest) = store.profile(session_id).map(normalize_session_profile) else {
        return TcpReconnectProfileState::Disabled;
    };
    if !tcp_reconnect_enabled(&latest) {
        return TcpReconnectProfileState::Disabled;
    }
    if !tcp_reconnect_attempt_matches_profile(attempt, &latest) {
        return TcpReconnectProfileState::Changed;
    }
    TcpReconnectProfileState::Current
}

pub(super) fn latest_tcp_reconnect_profile(
    state: &AppState,
    session_id: &str,
) -> Result<Option<SessionProfile>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let Some(profile) = store.profile(session_id) else {
        return Ok(None);
    };
    let profile = normalize_session_profile(profile);
    Ok(tcp_reconnect_enabled(&profile).then_some(profile))
}

pub(super) async fn connect_tcp_socket(
    tcp: &TcpConnection,
    label: &str,
) -> Result<TcpStream, String> {
    let stream = tokio::time::timeout(
        TCP_CONNECTION_SETUP_TIMEOUT,
        connect_target_stream(&tcp.host, tcp.port, &tcp.proxy, label),
    )
    .await
    .map_err(|_| format!("{label} 连接超时: {}:{}", tcp.host, tcp.port))??;
    configure_tcp_socket(&stream, label, tcp)?;
    Ok(stream)
}

async fn connect_tcp_transport(
    tcp: &TcpConnection,
    label: &str,
) -> Result<TcpConnectedStream, String> {
    let started = Instant::now();
    let stream = connect_tcp_socket(tcp, label).await?;
    if !tcp.tls_enabled {
        return Ok(TcpConnectedStream::Plain(stream));
    }

    let server_name = tcp.tls_server_name.as_deref().unwrap_or(tcp.host.as_str());
    if server_name.is_empty()
        || server_name
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(format!("{label} TLS Server Name 无效"));
    }
    let mut builder = NativeTlsConnector::builder();
    builder.danger_accept_invalid_certs(tcp.tls_accept_invalid_cert);
    let connector = builder
        .build()
        .map_err(|error| format!("{label} TLS connector 初始化失败: {error}"))?;
    let remaining = TCP_CONNECTION_SETUP_TIMEOUT.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        return Err(format!("{label} TLS 握手超时: {server_name}"));
    }
    tokio::time::timeout(
        remaining,
        TokioTlsConnector::from(connector).connect(server_name, stream),
    )
    .await
    .map_err(|_| format!("{label} TLS 握手超时: {server_name}"))?
    .map(TcpConnectedStream::Tls)
    .map_err(|error| format!("{label} TLS 握手失败: {error}"))
}

pub(super) fn configure_tcp_socket(
    stream: &TcpStream,
    label: &str,
    tcp: &TcpConnection,
) -> Result<(), String> {
    stream
        .set_nodelay(true)
        .map_err(|error| format!("{label} 设置 TCP_NODELAY 失败: {error}"))?;
    let socket = SockRef::from(stream);
    if !tcp.keepalive_enabled {
        return socket
            .set_keepalive(false)
            .map_err(|error| format!("{label} 关闭 TCP keepalive 失败: {error}"));
    }
    socket
        .set_tcp_keepalive(&tcp_keepalive_config(tcp))
        .map_err(|error| format!("{label} 设置 TCP keepalive 失败: {error}"))
}

fn tcp_keepalive_config(tcp: &TcpConnection) -> TcpKeepalive {
    let keepalive = TcpKeepalive::new().with_time(Duration::from_secs(tcp.keepalive_idle_seconds));
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "ios",
        target_os = "visionos",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "windows",
        target_os = "cygwin",
        all(target_os = "wasi", not(target_env = "p1")),
    ))]
    let keepalive = keepalive.with_interval(Duration::from_secs(tcp.keepalive_interval_seconds));
    #[cfg(any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "illumos",
        target_os = "ios",
        target_os = "visionos",
        target_os = "linux",
        target_os = "macos",
        target_os = "netbsd",
        target_os = "tvos",
        target_os = "watchos",
        target_os = "windows",
        target_os = "cygwin",
        all(target_os = "wasi", not(target_env = "p1")),
    ))]
    let keepalive = keepalive.with_retries(tcp.keepalive_retries);
    keepalive
}

pub(super) fn tcp_reconnect_delay(profile: &SessionProfile) -> Duration {
    match &profile.connection {
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => {
            Duration::from_millis(tcp.reconnect_delay_ms)
        }
        _ => Duration::from_millis(portmate_core::DEFAULT_TCP_RECONNECT_DELAY_MS),
    }
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
                    record_channel_bytes(
                        &io,
                        &session_id,
                        Some(&runtime_id),
                        EventStream::Stdout,
                        &buffer[..size],
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    if !bytes.is_empty() {
                        let _ = tap.send(bytes.clone());
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
                            if let Ok(mut store) = io.store.lock() {
                                store.record_system_event(
                                    &session_id,
                                    format!("PortMate: {reason}"),
                                );
                                disconnect_reason_recorded = true;
                                if let Err(error) = persist_applied_store(
                                    &store,
                                    &io.store_path,
                                    "Telnet negotiation failure event",
                                ) {
                                    eprintln!(
                                        "PortMate: failed to persist Telnet negotiation error: {error}"
                                    );
                                }
                            }
                            break 'read_loop;
                        }
                        record_outbound_control_event(
                            &io,
                            &session_id,
                            &reply,
                            "telnet-negotiation",
                            None,
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
                    if let Ok(mut store) = io.store.lock() {
                        store.record_system_event(&session_id, format!("PortMate: {reason}"));
                        disconnect_reason_recorded = true;
                        if let Err(error) = persist_applied_store(
                            &store,
                            &io.store_path,
                            "TCP/Telnet read failure event",
                        ) {
                            eprintln!("PortMate: failed to persist {label} read error: {error}");
                        }
                    }
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
                let _ = tap.send(bytes.clone());
                record_channel_bytes(
                    &io,
                    &session_id,
                    Some(&runtime_id),
                    EventStream::Stdout,
                    &[],
                    String::from_utf8_lossy(&bytes).to_string(),
                );
                has_unpersisted_stream = true;
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

        let reconnect_profile = (!closed.load(Ordering::SeqCst))
            .then(|| {
                io.store
                    .lock()
                    .ok()
                    .and_then(|store| store.profile(&session_id))
                    .map(normalize_session_profile)
                    .filter(tcp_reconnect_enabled)
            })
            .flatten();
        let reconnect_delay_ms = reconnect_profile
            .as_ref()
            .map(tcp_reconnect_delay)
            .unwrap_or_default()
            .as_millis();
        let reconnect_enabled = reconnect_profile.is_some();
        let mut should_reconnect = false;
        let removed_current = {
            let mut connections = match io.runtimes.tcp.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                if reconnect_enabled {
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
                        "PortMate: {disconnect_reason}; reconnecting in {reconnect_delay_ms}ms"
                    ),
                );
                if let Err(error) =
                    persist_applied_store(&store, &io.store_path, "TCP/Telnet reconnect transition")
                {
                    eprintln!("PortMate: failed to persist {label} reconnect event: {error}");
                }
            }
            tauri::async_runtime::spawn(reconnect_tcp_session(
                state, session_id, runtime_id, closed,
            ));
            return;
        }

        if removed_current {
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some(disconnect_reason.clone()),
                );
                if !disconnect_reason_recorded {
                    store
                        .record_system_event(&session_id, format!("PortMate: {disconnect_reason}"));
                }
                if let Err(error) = persist_applied_store(
                    &store,
                    &io.store_path,
                    "TCP/Telnet disconnect transition",
                ) {
                    eprintln!("PortMate: failed to persist {label} close event: {error}");
                }
            }
        }
    })
}

fn tcp_reconnect_pending(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    if closed.load(Ordering::SeqCst) {
        return false;
    }
    let connections = match state.tcp.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    state.store.lock().ok().is_some_and(|store| {
        store.runtimes.iter().any(|runtime| {
            runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TcpReconnectFailureDisposition {
    Recorded,
    RetryLatestProfile,
    StopDisabled,
    Superseded,
}

fn record_tcp_reconnect_failure_if_pending(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    attempt: &SessionProfile,
    label: &str,
    error: &str,
) -> TcpReconnectFailureDisposition {
    if closed.load(Ordering::SeqCst) {
        return TcpReconnectFailureDisposition::Superseded;
    }
    let connections = match state.tcp.lock() {
        Ok(connections) => connections,
        Err(_) => return TcpReconnectFailureDisposition::Superseded,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return TcpReconnectFailureDisposition::Superseded;
    }
    let mut store = match state.store.lock() {
        Ok(store) => store,
        Err(_) => return TcpReconnectFailureDisposition::Superseded,
    };
    if !store.runtimes.iter().any(|runtime| {
        runtime.session_id == session_id && runtime.status == SessionStatus::Reconnecting
    }) {
        return TcpReconnectFailureDisposition::Superseded;
    }
    match tcp_reconnect_profile_state(&store, session_id, attempt) {
        TcpReconnectProfileState::Current => {}
        TcpReconnectProfileState::Changed => {
            return TcpReconnectFailureDisposition::RetryLatestProfile;
        }
        TcpReconnectProfileState::Disabled => {
            return TcpReconnectFailureDisposition::StopDisabled;
        }
    }
    let _ = store.set_runtime_status_with_reason(
        session_id,
        SessionStatus::Reconnecting,
        Some(format!("{label} reconnect failed: {error}")),
    );
    store.record_system_event(
        session_id,
        format!(
            "PortMate: {label} reconnect failed: {error}; retrying in {}ms",
            tcp_reconnect_delay(attempt).as_millis()
        ),
    );
    if let Err(save_error) = persist_applied_store(
        &store,
        &state.store_path,
        "TCP/Telnet reconnect failure state",
    ) {
        eprintln!("PortMate: failed to persist {label} reconnect failure: {save_error}");
    }
    TcpReconnectFailureDisposition::Recorded
}

fn stop_pending_tcp_reconnect_if_disabled(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    reason: &str,
) -> bool {
    let mut connections = match state.tcp.lock() {
        Ok(connections) => connections,
        Err(_) => return false,
    };
    if connections
        .get(session_id)
        .is_none_or(|runtime| runtime.runtime_id != runtime_id)
    {
        return false;
    }
    let mut store = match state.store.lock() {
        Ok(store) => store,
        Err(_) => return false,
    };
    let reconnect_disabled = store
        .profile(session_id)
        .map(normalize_session_profile)
        .is_none_or(|profile| !tcp_reconnect_enabled(&profile));
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
        format!("PortMate: TCP/Telnet reconnect stopped: {reason}"),
    );
    if let Err(error) = persist_applied_store(
        &store,
        &state.store_path,
        "stopped TCP/Telnet reconnect state",
    ) {
        eprintln!("PortMate: failed to persist TCP/Telnet reconnect stop: {error}");
    }
    true
}

fn fail_pending_tcp_reconnect_install(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
    label: &str,
    error: &str,
) {
    closed.store(true, Ordering::SeqCst);
    let removed_current = match state.tcp.lock() {
        Ok(mut connections) => {
            if connections
                .get(session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                if let Some(runtime) = connections.remove(session_id) {
                    runtime.closed.store(true, Ordering::SeqCst);
                }
                true
            } else {
                false
            }
        }
        Err(lock_error) => {
            eprintln!(
                "PortMate: failed to clean up {label} reconnect runtime after Store failure: {lock_error}"
            );
            false
        }
    };
    if !removed_current {
        return;
    }

    clear_active_command(&state.session_io(), session_id);
    let reason = portmate_core::normalize_session_disconnect_reason(&format!(
        "{label} reconnect install failed: {error}"
    ))
    .unwrap_or_else(|| format!("{label} reconnect install failed"));
    if let Ok(mut store) = state.store.lock() {
        let _ = store.set_runtime_status_with_reason(
            session_id,
            SessionStatus::Error,
            Some(reason.clone()),
        );
        store.record_system_event(session_id, format!("PortMate: {reason}"));
    }
}

enum TcpReconnectInstallDecision {
    Installed,
    Retry,
    Stop,
    Superseded,
    Failed(String),
}

async fn wait_for_tcp_reconnect_attempt(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
    closed: &AtomicBool,
) -> bool {
    let started = Instant::now();
    loop {
        if !tcp_reconnect_pending(state, session_id, runtime_id, closed) {
            return false;
        }
        let profile = match latest_tcp_reconnect_profile(state, session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_tcp_reconnect_if_disabled(
                    state,
                    session_id,
                    runtime_id,
                    "automatic reconnect disabled while waiting for the next attempt",
                ) {
                    return false;
                }
                tokio::time::sleep(RECONNECT_DELAY_POLL_INTERVAL).await;
                continue;
            }
            Err(error) => {
                eprintln!(
                    "PortMate: failed to load TCP/Telnet reconnect delay from latest profile: {error}"
                );
                tokio::time::sleep(RECONNECT_DELAY_POLL_INTERVAL).await;
                continue;
            }
        };
        let remaining = tcp_reconnect_delay(&profile).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return true;
        }
        tokio::time::sleep(remaining.min(RECONNECT_DELAY_POLL_INTERVAL)).await;
    }
}

async fn reconnect_tcp_session(
    state: AppState,
    session_id: String,
    previous_runtime_id: String,
    closed: Arc<AtomicBool>,
) {
    loop {
        if !wait_for_tcp_reconnect_attempt(
            &state,
            &session_id,
            &previous_runtime_id,
            closed.as_ref(),
        )
        .await
        {
            return;
        }

        let profile = match latest_tcp_reconnect_profile(&state, &session_id) {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                if stop_pending_tcp_reconnect_if_disabled(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    "automatic reconnect disabled by latest profile",
                ) {
                    return;
                }
                continue;
            }
            Err(error) => {
                eprintln!("PortMate: failed to load latest TCP/Telnet reconnect profile: {error}");
                continue;
            }
        };
        let (tcp, label) = match tcp_connection_details(&profile) {
            Ok(details) => details,
            Err(error) => {
                match record_tcp_reconnect_failure_if_pending(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    &profile,
                    "TCP/Telnet",
                    &error,
                ) {
                    TcpReconnectFailureDisposition::Recorded
                    | TcpReconnectFailureDisposition::RetryLatestProfile => continue,
                    TcpReconnectFailureDisposition::StopDisabled => {
                        if stop_pending_tcp_reconnect_if_disabled(
                            &state,
                            &session_id,
                            &previous_runtime_id,
                            "automatic reconnect disabled while validating the latest profile",
                        ) {
                            return;
                        }
                        continue;
                    }
                    TcpReconnectFailureDisposition::Superseded => return,
                }
            }
        };

        let stream = match connect_tcp_transport(&tcp, label).await {
            Ok(stream) => stream,
            Err(error) => {
                match record_tcp_reconnect_failure_if_pending(
                    &state,
                    &session_id,
                    &previous_runtime_id,
                    closed.as_ref(),
                    &profile,
                    label,
                    &error,
                ) {
                    TcpReconnectFailureDisposition::Recorded
                    | TcpReconnectFailureDisposition::RetryLatestProfile => continue,
                    TcpReconnectFailureDisposition::StopDisabled => {
                        if stop_pending_tcp_reconnect_if_disabled(
                            &state,
                            &session_id,
                            &previous_runtime_id,
                            "automatic reconnect disabled while the previous attempt was running",
                        ) {
                            return;
                        }
                        continue;
                    }
                    TcpReconnectFailureDisposition::Superseded => return,
                }
            }
        };

        let runtime_id = Uuid::new_v4().to_string();
        let (read_half, write_half) = stream.split();
        let writer = Arc::new(tokio::sync::Mutex::new(write_half));
        let (tap, _) = broadcast::channel(1024);
        let next_closed = Arc::new(AtomicBool::new(false));
        let telnet = TelnetRuntimeState::from_profile(&profile);
        let install = match state.tcp.lock() {
            Err(error) => TcpReconnectInstallDecision::Failed(error.to_string()),
            Ok(mut connections) => {
                if connections
                    .get(&session_id)
                    .is_none_or(|runtime| runtime.runtime_id != previous_runtime_id)
                    || closed.load(Ordering::SeqCst)
                {
                    TcpReconnectInstallDecision::Superseded
                } else {
                    match state.store.lock() {
                        Err(error) => TcpReconnectInstallDecision::Failed(error.to_string()),
                        Ok(mut store) => {
                            match tcp_reconnect_profile_state(&store, &session_id, &profile) {
                                TcpReconnectProfileState::Changed => {
                                    TcpReconnectInstallDecision::Retry
                                }
                                TcpReconnectProfileState::Disabled => {
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
                                        format!("PortMate: TCP/Telnet reconnect stopped: {reason}"),
                                    );
                                    if let Err(error) = persist_applied_store(
                                        &store,
                                        &state.store_path,
                                        "stopped TCP/Telnet reconnect state",
                                    ) {
                                        eprintln!(
                                            "PortMate: failed to persist TCP/Telnet reconnect stop: {error}"
                                        );
                                    }
                                    TcpReconnectInstallDecision::Stop
                                }
                                TcpReconnectProfileState::Current => {
                                    let committed = commit_tracked_store_mutation(
                                        &mut store,
                                        &state.store_path,
                                        |next_store| {
                                            mark_session_connected_with_events(
                                                next_store,
                                                &profile,
                                                [format!("PortMate: {label} socket reconnected")],
                                            )
                                        },
                                    );
                                    match committed {
                                        Ok(_) => {
                                            connections.insert(
                                                session_id.clone(),
                                                TcpRuntime {
                                                    runtime_id: runtime_id.clone(),
                                                    writer: Arc::clone(&writer),
                                                    tap: tap.clone(),
                                                    closed: Arc::clone(&next_closed),
                                                    telnet: telnet.as_ref().map(Arc::clone),
                                                },
                                            );
                                            TcpReconnectInstallDecision::Installed
                                        }
                                        Err(error) => TcpReconnectInstallDecision::Failed(error),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        };

        if !matches!(install, TcpReconnectInstallDecision::Installed) {
            let mut writer = writer.lock().await;
            let _ = writer.shutdown().await;
            drop(writer);
            match install {
                TcpReconnectInstallDecision::Retry => continue,
                TcpReconnectInstallDecision::Stop | TcpReconnectInstallDecision::Superseded => {
                    return
                }
                TcpReconnectInstallDecision::Failed(error) => {
                    fail_pending_tcp_reconnect_install(
                        &state,
                        &session_id,
                        &previous_runtime_id,
                        closed.as_ref(),
                        label,
                        &error,
                    );
                    eprintln!("PortMate: failed to install TCP/Telnet reconnect runtime: {error}");
                    return;
                }
                TcpReconnectInstallDecision::Installed => unreachable!(),
            }
        }

        tauri::async_runtime::spawn(read_tcp_stream(TcpReadTask {
            state: state.clone(),
            profile,
            runtime_id,
            label: label.to_string(),
            tap,
            writer,
            read_half,
            closed: next_closed,
            telnet,
        }));
        return;
    }
}

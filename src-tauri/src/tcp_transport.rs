use super::*;
use socket2::TcpKeepalive;

pub(super) struct TcpRuntime {
    pub(super) runtime_id: String,
    pub(super) writer: Arc<tokio::sync::Mutex<OwnedWriteHalf>>,
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
        Duration::from_secs(15),
        connect_target_stream(&tcp.host, tcp.port, &tcp.proxy, label),
    )
    .await
    .map_err(|_| format!("{label} 连接超时: {}:{}", tcp.host, tcp.port))??;
    configure_tcp_socket(&stream, label, tcp)?;
    Ok(stream)
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

    let stream = connect_tcp_socket(&tcp, label).await?;

    let runtime_id = Uuid::new_v4().to_string();
    let (read_half, write_half) = stream.into_split();
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

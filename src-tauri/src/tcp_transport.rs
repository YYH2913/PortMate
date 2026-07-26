use super::*;
use socket2::TcpKeepalive;

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

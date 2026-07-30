use super::*;
use native_tls::TlsConnector as NativeTlsConnector;
use socket2::TcpKeepalive;
#[cfg(test)]
use tokio::net::tcp::OwnedWriteHalf;
use tokio_native_tls::{TlsConnector as TokioTlsConnector, TlsStream};

const TCP_CONNECTION_SETUP_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) type TcpReadHalf = Box<dyn AsyncRead + Send + Unpin>;
pub(super) type TcpWriteHalf = Box<dyn AsyncWrite + Send + Unpin>;

pub(super) enum TcpConnectedStream {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl TcpConnectedStream {
    pub(super) fn split(self) -> (TcpReadHalf, TcpWriteHalf) {
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

pub(super) async fn connect_tcp_transport(
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

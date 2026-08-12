use super::*;

pub(super) const TUNNEL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn open_tunnel_direct_tcpip(
    shared_handle: &Arc<tokio::sync::Mutex<SshBackendSession>>,
    target_host: String,
    target_port: u16,
    peer: std::net::SocketAddr,
    label: &str,
) -> Result<SshBackendChannel, String> {
    let handle = tokio::time::timeout(TUNNEL_CONNECT_TIMEOUT, shared_handle.lock())
        .await
        .map_err(|_| {
            format!(
                "{label} handle lock timed out after {} ms",
                TUNNEL_CONNECT_TIMEOUT.as_millis()
            )
        })?;
    match bounded_connection_step(
        handle.open_direct_tcpip(target_host, target_port, peer.ip().to_string(), peer.port()),
        TUNNEL_CONNECT_TIMEOUT,
    )
    .await
    {
        Ok(channel) => Ok(channel),
        Err(BoundedConnectionStepError::Failed(error)) => Err(format!("{label} failed: {error}")),
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning = request_backend_disconnect_with_timeout(
                &handle,
                "PortMate tunnel channel open timeout",
            )
            .await
            .map(|warning| format!("; {warning}"))
            .unwrap_or_default();
            Err(format!(
                "{label} timed out after {} ms{cleanup_warning}",
                TUNNEL_CONNECT_TIMEOUT.as_millis()
            ))
        }
    }
}

pub(super) async fn handle_local_tunnel_client(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    tunnel: TunnelSpec,
    local_stream: TcpStream,
    peer: std::net::SocketAddr,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let channel = open_tunnel_direct_tcpip(
        &handle,
        tunnel.target_host.clone(),
        tunnel.target_port,
        peer,
        "direct-tcpip open",
    )
    .await?;
    pipe_ssh_channel_to_tcp(channel, local_stream, tunnel, metrics).await
}

pub(super) async fn handle_remote_tunnel_client(
    channel: SshBackendChannel,
    tunnel: TunnelSpec,
    originator: Option<(String, u16)>,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let target_connect = bounded_connection_step(
        TcpStream::connect((tunnel.target_host.clone(), tunnel.target_port)),
        TUNNEL_CONNECT_TIMEOUT,
    )
    .await;
    let local_stream = match target_connect {
        Ok(stream) => stream,
        Err(error) => {
            close_ssh_channel_bounded(&channel).await;
            let originator = originator
                .map(|(address, port)| format!("{address}:{port}"))
                .unwrap_or_else(|| format!("remote port {}", tunnel.bind_port));
            return Err(match error {
                BoundedConnectionStepError::Failed(error) => format!(
                    "remote tunnel target connect failed {}:{} for {originator}: {error}",
                    tunnel.target_host, tunnel.target_port
                ),
                BoundedConnectionStepError::TimedOut => format!(
                    "remote tunnel target connect timed out after {} ms {}:{} for {originator}",
                    TUNNEL_CONNECT_TIMEOUT.as_millis(),
                    tunnel.target_host,
                    tunnel.target_port
                ),
            });
        }
    };
    pipe_ssh_channel_to_tcp(channel, local_stream, tunnel, metrics).await
}

pub(super) fn spawn_libssh_remote_forward_acceptor(
    session: libssh_rs::Session,
    remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    runtime_closed: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        while !runtime_closed.load(Ordering::SeqCst) {
            let has_routes = match remote_forwards.lock() {
                Ok(forwards) => !forwards.is_empty(),
                Err(error) => {
                    eprintln!("PortMate: libssh remote forward route lock failed: {error}");
                    break;
                }
            };
            if !has_routes {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }

            let accept_session = session.clone();
            let accepted = tokio::task::spawn_blocking(move || {
                accept_session.accept_forward(Duration::from_millis(50))
            })
            .await;
            let (destination_port, channel) = match accepted {
                Ok(Ok(accepted)) => accepted,
                Ok(Err(libssh_rs::Error::TryAgain)) => continue,
                Ok(Err(error)) => {
                    if !runtime_closed.load(Ordering::SeqCst) {
                        eprintln!("PortMate: libssh remote forward accept failed: {error}");
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                    continue;
                }
                Err(error) => {
                    if !runtime_closed.load(Ordering::SeqCst) {
                        eprintln!("PortMate: libssh remote forward worker failed: {error}");
                    }
                    break;
                }
            };
            let channel = SshBackendChannel::from_libssh_forward(channel);
            let target = remote_forwards.lock().ok().and_then(|forwards| {
                forwards
                    .get(&remote_forward_port_key(destination_port))
                    .cloned()
            });
            let Some(target) = target else {
                close_ssh_channel_bounded(&channel).await;
                continue;
            };
            let Some(permit) =
                try_acquire_tunnel_connection(&target.connection_slots, target.metrics.as_ref())
            else {
                close_ssh_channel_bounded(&channel).await;
                continue;
            };
            tauri::async_runtime::spawn(async move {
                let _permit = permit;
                target.metrics.connection_opened();
                let result = handle_remote_tunnel_client(
                    channel,
                    target.spec.clone(),
                    None,
                    Arc::clone(&target.metrics),
                )
                .await;
                match result {
                    Ok(()) => target.metrics.clear_error(),
                    Err(error) => {
                        target.metrics.record_error(&error);
                        eprintln!("PortMate: remote SSH tunnel client failed: {error}");
                    }
                }
                target.metrics.connection_closed();
            });
        }
    });
}

pub(super) async fn handle_dynamic_tunnel_client(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    tunnel: TunnelSpec,
    mut local_stream: TcpStream,
    peer: std::net::SocketAddr,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let (target_host, target_port) = read_socks5_connect_request(&mut local_stream).await?;
    if !tunnel_route_allowed(&tunnel.route_rules, &target_host, target_port) {
        let _ = local_stream.write_all(&socks5_reply(2)).await;
        return Err(format!(
            "dynamic route denied by target rules: {target_host}:{target_port}"
        ));
    }

    let channel = match open_tunnel_direct_tcpip(
        &handle,
        target_host.clone(),
        target_port,
        peer,
        "dynamic direct-tcpip open",
    )
    .await
    {
        Ok(channel) => channel,
        Err(error) => {
            let _ = local_stream.write_all(&socks5_reply(5)).await;
            return Err(error);
        }
    };

    local_stream
        .write_all(&socks5_reply(0))
        .await
        .map_err(|error| format!("SOCKS5 success response failed: {error}"))?;

    let spec = TunnelSpec {
        id: "dynamic-client".to_string(),
        label: format!("SOCKS5 -> {target_host}:{target_port}"),
        mode: TunnelMode::Dynamic,
        bind_host: String::new(),
        bind_port: 0,
        target_host,
        target_port,
        route_rules: Vec::new(),
        enabled: true,
    };
    pipe_ssh_channel_to_tcp(channel, local_stream, spec, metrics).await
}

pub(super) async fn pipe_ssh_channel_to_tcp(
    channel: SshBackendChannel,
    local_stream: TcpStream,
    tunnel: TunnelSpec,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let (mut remote_read, remote_write) = channel.split();
    let (mut local_read, mut local_write) = local_stream.into_split();

    let upload_metrics = Arc::clone(&metrics);
    let local_to_remote = async move {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            let size = local_read
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if size == 0 {
                remote_write
                    .eof()
                    .await
                    .map_err(|error| error.to_string())?;
                break;
            }
            upload_metrics.add_tcp_to_ssh_bytes(size);
            remote_write
                .data(&buffer[..size])
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok::<(), String>(())
    };

    let download_metrics = Arc::clone(&metrics);
    let remote_to_local = async move {
        while let Some(message) = remote_read.wait().await {
            match message {
                SshBackendMessage::Data(data) | SshBackendMessage::ExtendedData { data, .. } => {
                    download_metrics.add_ssh_to_tcp_bytes(data.len());
                    local_write
                        .write_all(&data)
                        .await
                        .map_err(|error| error.to_string())?;
                }
                SshBackendMessage::Eof | SshBackendMessage::Close => break,
                SshBackendMessage::Failure => {
                    return Err("SSH tunnel channel reported failure".to_string());
                }
                SshBackendMessage::Error(error) => {
                    return Err(format!("SSH tunnel channel read failed: {error}"));
                }
                _ => {}
            }
        }
        Ok::<(), String>(())
    };

    let pipe_kind = if tunnel.mode == TunnelMode::Local {
        "local tunnel"
    } else {
        "tunnel"
    };
    tokio::try_join!(local_to_remote, remote_to_local)
        .map(|_| ())
        .map_err(|error| format!("{pipe_kind} pipe failed ({}): {error}", tunnel.label))
}

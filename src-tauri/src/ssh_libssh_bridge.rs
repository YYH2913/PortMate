use super::*;

const MAX_LIBSSH_AGENT_FORWARD_CHANNELS: usize = 16;
const LIBSSH_AGENT_CHANNEL_IO_TIMEOUT: Duration = Duration::from_millis(250);

fn run_libssh_agent_channel_operation<T>(
    channel: &libssh_rs::Channel,
    timeout: Duration,
    label: &str,
    operation: impl FnOnce(&libssh_rs::Channel, Instant) -> Result<T, String>,
) -> Result<T, String> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| format!("{label} deadline is outside the supported range"))?;
    channel
        .with_session_operation_until(deadline, || {
            channel
                .set_session_timeout_until(deadline)
                .map_err(|error| format!("{label} deadline setup failed: {error}"))?;
            let result = operation(channel, deadline);
            let restored = channel
                .set_session_timeout(SSH_RUNTIME_OPERATION_TIMEOUT)
                .map_err(|error| format!("{label} runtime timeout restore failed: {error}"));
            match (result, restored) {
                (Ok(value), Ok(())) => Ok(value),
                (Err(error), Ok(())) => Err(error),
                (Ok(_), Err(error)) => Err(error),
                (Err(error), Err(restore_error)) => Err(format!("{error}; {restore_error}")),
            }
        })
        .map_err(|error| format!("{label} operation gate failed: {error}"))?
}

#[cfg(unix)]
fn write_libssh_agent_channel(
    channel: &libssh_rs::Channel,
    mut data: &[u8],
) -> Result<(), String> {
    run_libssh_agent_channel_operation(
        channel,
        LIBSSH_AGENT_CHANNEL_IO_TIMEOUT,
        "SSH agent channel write",
        |channel, deadline| {
            while !data.is_empty() {
                channel
                    .set_session_timeout_until(deadline)
                    .map_err(|error| error.to_string())?;
                let mut stdin = channel.stdin();
                match std::io::Write::write(&mut stdin, data) {
                    Ok(0) => return Err("SSH agent channel write returned zero bytes".to_string()),
                    Ok(written) => data = &data[written..],
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(format!("write SSH agent response failed: {error}")),
                }
            }
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|remaining| !remaining.is_zero())
                .ok_or_else(|| "SSH agent channel write timed out".to_string())?;
            channel
                .flush(Some(remaining))
                .map_err(|error| format!("flush SSH agent response failed: {error}"))
        },
    )
}

fn close_libssh_agent_channel(channel: &libssh_rs::Channel) -> Result<(), String> {
    run_libssh_agent_channel_operation(
        channel,
        LIBSSH_AGENT_CHANNEL_IO_TIMEOUT,
        "SSH agent channel close",
        |channel, _| channel.close().map_err(|error| error.to_string()),
    )
}

#[cfg(unix)]
fn eof_libssh_agent_channel(channel: &libssh_rs::Channel) -> Result<(), String> {
    run_libssh_agent_channel_operation(
        channel,
        LIBSSH_AGENT_CHANNEL_IO_TIMEOUT,
        "SSH agent channel EOF",
        |channel, _| channel.send_eof().map_err(|error| error.to_string()),
    )
}

pub(super) async fn start_russh_jump_transport_bridge(
    channel: Channel<client::Msg>,
    closed: Arc<AtomicBool>,
) -> Result<(std::net::TcpStream, tokio::sync::oneshot::Receiver<()>), String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|error| format!("bind libssh Jump Host bridge failed: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("read libssh Jump Host bridge address failed: {error}"))?;
    let client = tokio::net::TcpStream::connect(address)
        .await
        .map_err(|error| format!("connect libssh Jump Host bridge failed: {error}"))?;
    let expected_peer = client
        .local_addr()
        .map_err(|error| format!("read libssh Jump Host bridge peer failed: {error}"))?;
    let (mut bridge, peer) = listener
        .accept()
        .await
        .map_err(|error| format!("accept libssh Jump Host bridge failed: {error}"))?;
    if peer != expected_peer {
        return Err("libssh Jump Host bridge accepted an unexpected local peer".to_string());
    }
    client
        .set_nodelay(true)
        .map_err(|error| format!("set libssh Jump Host bridge TCP_NODELAY failed: {error}"))?;
    bridge
        .set_nodelay(true)
        .map_err(|error| format!("set libssh Jump Host bridge peer TCP_NODELAY failed: {error}"))?;
    let client = client
        .into_std()
        .map_err(|error| format!("transfer libssh Jump Host bridge socket failed: {error}"))?;
    let (finished_sender, finished_receiver) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let mut jump_stream = channel.into_stream();
        let mut copy = Box::pin(tokio::io::copy_bidirectional(&mut bridge, &mut jump_stream));
        loop {
            tokio::select! {
                result = &mut copy => {
                    if let Err(error) = result {
                        eprintln!("PortMate: libssh Jump Host bridge failed: {error}");
                    }
                    break;
                }
                _ = tokio::time::sleep(Duration::from_millis(20)) => {
                    if closed.load(Ordering::SeqCst) {
                        break;
                    }
                }
            }
        }
        drop(copy);
        let _ = tokio::io::AsyncWriteExt::shutdown(&mut bridge).await;
        let _ = tokio::io::AsyncWriteExt::shutdown(&mut jump_stream).await;
        let _ = finished_sender.send(());
    });
    Ok((client, finished_receiver))
}

pub(super) fn start_libssh_agent_forwarder(
    session: libssh_rs::Session,
    agent_socket_path: std::path::PathBuf,
    closed: Arc<AtomicBool>,
) -> tokio::sync::oneshot::Receiver<()> {
    let (finished_sender, finished_receiver) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let mut bridges = tokio::task::JoinSet::new();
        while !closed.load(Ordering::SeqCst) {
            while let Some(result) = bridges.try_join_next() {
                report_libssh_agent_bridge_result(result);
            }
            let accept_session = session.clone();
            let accepted =
                tokio::task::spawn_blocking(move || accept_session.accept_agent_forward()).await;
            match accepted {
                Ok(Some(channel)) if bridges.len() >= MAX_LIBSSH_AGENT_FORWARD_CHANNELS => {
                    eprintln!(
                        "PortMate: rejected libssh agent forward channel at the {} channel limit",
                        MAX_LIBSSH_AGENT_FORWARD_CHANNELS
                    );
                    let closed = tokio::task::spawn_blocking(move || {
                        close_libssh_agent_channel(&channel)
                    })
                    .await;
                    report_libssh_agent_bridge_result(closed);
                }
                Ok(Some(channel)) => {
                    let socket_path = agent_socket_path.clone();
                    let channel_closed = Arc::clone(&closed);
                    bridges.spawn_blocking(move || {
                        bridge_libssh_agent_channel(channel, socket_path, channel_closed)
                    });
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(20)).await,
                Err(error) => {
                    eprintln!("PortMate: libssh agent forward accept worker failed: {error}");
                    break;
                }
            }
        }
        let disable_session = session.clone();
        if let Err(error) = tokio::task::spawn_blocking(move || {
            disable_session.enable_accept_agent_forward(false)
        })
        .await
        {
            eprintln!("PortMate: libssh agent forward disable worker failed: {error}");
        }
        while let Some(result) = bridges.join_next().await {
            report_libssh_agent_bridge_result(result);
        }
        let _ = finished_sender.send(());
    });
    finished_receiver
}

fn report_libssh_agent_bridge_result(result: Result<Result<(), String>, tokio::task::JoinError>) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("PortMate: libssh agent forward bridge failed: {error}"),
        Err(error) => eprintln!("PortMate: libssh agent forward worker failed: {error}"),
    }
}

#[cfg(unix)]
fn bridge_libssh_agent_channel(
    channel: libssh_rs::Channel,
    agent_socket_path: std::path::PathBuf,
    closed: Arc<AtomicBool>,
) -> Result<(), String> {
    use std::io::{ErrorKind, Read, Write};
    use std::os::unix::net::UnixStream;

    let mut socket = UnixStream::connect(&agent_socket_path).map_err(|error| {
        format!(
            "connect SSH agent socket {} failed: {error}",
            agent_socket_path.display()
        )
    })?;
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|error| format!("set SSH agent read timeout failed: {error}"))?;
    socket
        .set_write_timeout(Some(Duration::from_millis(250)))
        .map_err(|error| format!("set SSH agent write timeout failed: {error}"))?;
    let mut socket_buffer = [0_u8; 64 * 1024];
    let mut channel_buffer = vec![0_u8; 64 * 1024];

    loop {
        if closed.load(Ordering::SeqCst) {
            break;
        }
        let mut channel_finished = false;
        match channel
            .poll_timeout(false, Some(Duration::ZERO))
            .map_err(|error| error.to_string())?
        {
            libssh_rs::PollStatus::AvailableBytes(available) if available > 0 => {
                let read_limit = (available as usize).min(channel_buffer.len());
                let read = channel
                    .read_timeout(
                        &mut channel_buffer[..read_limit],
                        false,
                        Some(Duration::from_millis(50)),
                    )
                    .map_err(|error| error.to_string())?;
                if read > 0 {
                    socket
                        .write_all(&channel_buffer[..read])
                        .map_err(|error| format!("write SSH agent request failed: {error}"))?;
                }
            }
            libssh_rs::PollStatus::AvailableBytes(_) => {}
            libssh_rs::PollStatus::EndOfFile => channel_finished = true,
        }
        if channel.is_closed() || channel.is_eof() || channel_finished {
            break;
        }

        match socket.read(&mut socket_buffer) {
            Ok(0) => {
                let _ = eof_libssh_agent_channel(&channel);
                break;
            }
            Ok(read) => {
                write_libssh_agent_channel(&channel, &socket_buffer[..read])?;
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(format!("read SSH agent response failed: {error}")),
        }
    }
    let _ = close_libssh_agent_channel(&channel);
    Ok(())
}

#[cfg(not(unix))]
fn bridge_libssh_agent_channel(
    _channel: libssh_rs::Channel,
    _agent_socket_path: std::path::PathBuf,
    _closed: Arc<AtomicBool>,
) -> Result<(), String> {
    Err("libssh SSH agent forwarding requires a Unix-domain SSH agent socket".to_string())
}

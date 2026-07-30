use super::*;

const MAX_LIBSSH_AGENT_FORWARD_CHANNELS: usize = 16;

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
                    let _ = tokio::task::spawn_blocking(move || channel.close()).await;
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
        session.enable_accept_agent_forward(false);
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
                let _ = channel.send_eof();
                break;
            }
            Ok(read) => {
                let mut stdin = channel.stdin();
                stdin
                    .write_all(&socket_buffer[..read])
                    .map_err(|error| format!("write SSH agent response failed: {error}"))?;
                drop(stdin);
                channel
                    .flush(Some(Duration::from_millis(250)))
                    .map_err(|error| format!("flush SSH agent response failed: {error}"))?;
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(format!("read SSH agent response failed: {error}")),
        }
    }
    let _ = channel.close();
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

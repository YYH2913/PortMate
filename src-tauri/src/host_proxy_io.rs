use super::*;

pub(super) async fn handle_portmate_host_tunnel_client(
    tunnel: TunnelSpec,
    mut client_stream: TcpStream,
    peer: std::net::SocketAddr,
    metrics: Arc<TunnelMetrics>,
) -> Result<(), String> {
    let (target_host, target_port, dynamic) = if tunnel.mode == TunnelMode::Dynamic {
        let (target_host, target_port) = read_socks5_connect_request(&mut client_stream).await?;
        if !tunnel_route_allowed(&tunnel.route_rules, &target_host, target_port) {
            let _ = client_stream.write_all(&socks5_reply(2)).await;
            return Err(format!(
                "PortMate host route denied by target rules: {target_host}:{target_port}"
            ));
        }
        (target_host, target_port, true)
    } else {
        (tunnel.target_host.clone(), tunnel.target_port, false)
    };

    let target = format!("{target_host}:{target_port}");
    let mut target_stream = match tokio::time::timeout(
        TUNNEL_CONNECT_TIMEOUT,
        TcpStream::connect((target_host.as_str(), target_port)),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            if dynamic {
                let _ = client_stream.write_all(&socks5_reply(5)).await;
            }
            return Err(format!(
                "PortMate host target connect failed {target} for {peer}: {error}"
            ));
        }
        Err(_) => {
            if dynamic {
                let _ = client_stream.write_all(&socks5_reply(4)).await;
            }
            return Err(format!(
                "PortMate host target connect timed out after {} ms {target} for {peer}",
                TUNNEL_CONNECT_TIMEOUT.as_millis()
            ));
        }
    };

    if dynamic {
        client_stream
            .write_all(&socks5_reply(0))
            .await
            .map_err(|error| format!("SOCKS5 success response failed: {error}"))?;
    }

    let (client_to_target, target_to_client) =
        tokio::io::copy_bidirectional(&mut client_stream, &mut target_stream)
            .await
            .map_err(|error| {
                format!(
                    "PortMate host proxy pipe failed ({} -> {target}): {error}",
                    tunnel.label
                )
            })?;
    metrics.add_tcp_to_ssh_bytes_u64(client_to_target);
    metrics.add_ssh_to_tcp_bytes_u64(target_to_client);
    Ok(())
}

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

const DEFAULT_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS: u64 = 10_000;

pub(super) async fn execute_tunnel_request_inner(
    state: &AppState,
    client_id: &str,
    request: McpTunnelExchangeRequest,
    commit_validation: Option<CommitValidation>,
) -> Result<McpTunnelExchangeResult, String> {
    if let Some(validate) = commit_validation {
        validate()?;
    }
    validate_mcp_tunnel_exchange_request(&request)?;
    let owner_id = mcp_host_route_owner_id(client_id)?;
    let runtime = {
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        tunnels
            .get(&request.tunnel_id)
            .filter(|runtime| {
                runtime.session_id == owner_id
                    && runtime.spec.egress == TunnelEgress::PortmateHost
                    && !runtime.closed.load(Ordering::SeqCst)
            })
            .cloned()
            .ok_or_else(|| "host route not found or owned by another MCP client".to_string())?
    };
    let (target_host, target_port) = match runtime.spec.mode {
        TunnelMode::Local => {
            if request.target_host.is_some() || request.target_port.is_some() {
                return Err(
                    "fixed MCP tunnel requests must not override targetHost or targetPort"
                        .to_string(),
                );
            }
            (runtime.spec.target_host.clone(), runtime.spec.target_port)
        }
        TunnelMode::Dynamic => {
            let host = request.target_host.clone().ok_or_else(|| {
                "dynamic MCP tunnel requests require targetHost and targetPort".to_string()
            })?;
            let port = request.target_port.ok_or_else(|| {
                "dynamic MCP tunnel requests require targetHost and targetPort".to_string()
            })?;
            if !tunnel_route_allowed(&runtime.spec.route_rules, &host, port) {
                return Err(format!("MCP tunnel target denied by route rules: {host}:{port}"));
            }
            (host, port)
        }
        TunnelMode::Remote => {
            return Err("remote forwarding does not support MCP tunnel requests".to_string())
        }
    };
    let payload = decode_mcp_tunnel_exchange_payload(&request.encoding, &request.data)?;
    let timeout_ms = request
        .timeout_ms
        .unwrap_or(DEFAULT_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS);
    // One deadline covers connect, write, and response reads so a peer cannot
    // multiply the caller-selected budget across independent I/O phases.
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let max_response_bytes = request.max_response_bytes.unwrap_or(MAX_MCP_TUNNEL_EXCHANGE_BYTES);
    let metrics = Arc::clone(&runtime.metrics);
    let Some(permit) =
        try_acquire_tunnel_connection(&state.tunnel_connection_slots, metrics.as_ref())
    else {
        return Err(format!(
            "{TUNNEL_CONNECTION_LIMIT_ERROR_PREFIX} app limit ({MAX_TUNNEL_CONNECTIONS})"
        ));
    };
    let _permit = permit;
    metrics.connection_opened();
    let exchange = exchange_host_tcp_request(
        &target_host,
        target_port,
        &payload,
        deadline,
        timeout_ms,
        max_response_bytes,
        request.close_write,
    )
    .await;
    let (response, truncated, timed_out) = match exchange {
        Ok(result) => result,
        Err(error) => {
            metrics.record_error(&error);
            metrics.connection_closed();
            return Err(error);
        }
    };
    metrics.add_tcp_to_ssh_bytes_u64(payload.len() as u64);
    metrics.add_ssh_to_tcp_bytes_u64(response.len() as u64);
    metrics.clear_error();
    metrics.connection_closed();
    Ok(McpTunnelExchangeResult {
        tunnel_id: runtime.spec.id,
        target_host,
        target_port,
        sent_bytes: payload.len(),
        received_bytes: response.len(),
        response_base64: BASE64_STANDARD.encode(&response),
        truncated,
        timed_out,
    })
}

async fn exchange_host_tcp_request(
    target_host: &str,
    target_port: u16,
    payload: &[u8],
    deadline: tokio::time::Instant,
    timeout_ms: u64,
    max_response_bytes: usize,
    close_write: bool,
) -> Result<(Vec<u8>, bool, bool), String> {
    let target = format!("{target_host}:{target_port}");
    let mut stream = match tokio::time::timeout_at(
        deadline,
        TcpStream::connect((target_host, target_port)),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            return Err(format!(
                "MCP tunnel target connect failed {target}: {error}"
            ));
        }
        Err(_) => {
            return Err(format!(
                "MCP tunnel target connect timed out after {} ms {target}",
                timeout_ms
            ));
        }
    };
    let write = tokio::time::timeout_at(deadline, async {
        stream.write_all(payload).await?;
        if close_write {
            stream.shutdown().await?;
        }
        Ok::<(), std::io::Error>(())
    })
    .await;
    match write {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return Err(format!(
                "MCP tunnel request write failed {target}: {error}"
            ));
        }
        Err(_) => {
            return Err(format!(
                "MCP tunnel request write timed out after {} ms {target}",
                timeout_ms
            ));
        }
    }
    read_bounded_tunnel_response(&mut stream, deadline, max_response_bytes).await
}

async fn read_bounded_tunnel_response(
    stream: &mut TcpStream,
    deadline: tokio::time::Instant,
    max_response_bytes: usize,
) -> Result<(Vec<u8>, bool, bool), String> {
    let mut response = Vec::new();
    let mut truncated = false;
    let mut buffer = vec![0_u8; 16 * 1024];
    let read = tokio::time::timeout_at(deadline, async {
        loop {
            let size = stream
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if size == 0 {
                break;
            }
            let remaining = max_response_bytes.saturating_sub(response.len());
            if remaining == 0 {
                truncated = true;
                break;
            }
            if size > remaining {
                response.extend_from_slice(&buffer[..remaining]);
                truncated = true;
                break;
            }
            response.extend_from_slice(&buffer[..size]);
            if response.len() == max_response_bytes {
                // At the limit, do not wait for an EOF merely to distinguish
                // an exact-size response from a longer one. The result is
                // conservatively marked as truncated and the slot is released.
                truncated = true;
                break;
            }
        }
        Ok::<(), String>(())
    })
    .await;
    let timed_out = read.is_err();
    match read {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return Err(format!("MCP tunnel response read failed: {error}"));
        }
        Err(_) => {}
    }
    Ok((response, truncated, timed_out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_limit_releases_without_waiting_for_peer_eof() {
        tauri::async_runtime::block_on(async {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                stream.write_all(b"pong").await.unwrap();
                tokio::time::sleep(Duration::from_secs(2)).await;
            });

            let started = Instant::now();
            let result = exchange_host_tcp_request(
                "127.0.0.1",
                address.port(),
                b"ping",
                tokio::time::Instant::now() + Duration::from_secs(1),
                1_000,
                4,
                false,
            )
            .await
            .unwrap();
            assert_eq!(result.0, b"pong");
            assert!(result.1);
            assert!(!result.2);
            assert!(started.elapsed() < Duration::from_millis(500));
            server.abort();
        });
    }
}

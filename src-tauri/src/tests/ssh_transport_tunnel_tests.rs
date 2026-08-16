#[cfg(unix)]
async fn assert_libssh_local_and_dynamic_tunnels(state: &AppState, session_id: &str) {
    let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let echo_address = echo_listener.local_addr().unwrap();
    drop(echo_listener);
    let tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: session_id.to_string(),
            egress: TunnelEgress::Ssh,
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: "127.0.0.1".to_string(),
            target_port: echo_address.port(),
            route_rules: Vec::new(),
            allow_remote_bind: false,
            label: None,
        },
    )
    .await
    .unwrap();

    let mut failed_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
        .await
        .unwrap();
    failed_client.write_all(b"ping").await.unwrap();
    let mut closed_byte = [0_u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(3), failed_client.read(&mut closed_byte))
        .await
        .expect("failed libssh local tunnel client did not close");
    assert_tunnel_client_closed(read, "failed libssh local tunnel client");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 1 {
                assert!(status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("direct-tcpip open failed")));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh local tunnel failure metrics did not settle");

    let echo_listener = TcpListener::bind(echo_address).await.unwrap();
    let echo = tokio::spawn(async move {
        let (mut socket, _) = echo_listener.accept().await.unwrap();
        let mut request = [0_u8; 4];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut tunnel_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
        .await
        .unwrap();
    tunnel_client.write_all(b"ping").await.unwrap();
    let mut response = [0_u8; 4];
    tunnel_client.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"pong");
    drop(tunnel_client);
    echo.await.unwrap();

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 2 {
                assert_eq!(status.tcp_to_ssh_bytes, 4);
                assert_eq!(status.ssh_to_tcp_bytes, 4);
                assert!(status.last_error.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh local tunnel metrics did not settle");
    let stopped = stop_tunnel_inner(state, &tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);

    let dynamic_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let dynamic_echo_address = dynamic_echo_listener.local_addr().unwrap();
    drop(dynamic_echo_listener);
    let denied_port = if dynamic_echo_address.port() == u16::MAX {
        u16::MAX - 1
    } else {
        dynamic_echo_address.port() + 1
    };
    let dynamic_tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: session_id.to_string(),
            egress: TunnelEgress::Ssh,
            mode: TunnelMode::Dynamic,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: String::new(),
            target_port: 0,
            route_rules: vec![portmate_core::TunnelRouteRule {
                host: "127.0.0.1".to_string(),
                port: Some(dynamic_echo_address.port()),
            }],
            allow_remote_bind: false,
            label: None,
        },
    )
    .await
    .unwrap();
    let [port_high, port_low] = dynamic_echo_address.port().to_be_bytes();
    let [denied_port_high, denied_port_low] = denied_port.to_be_bytes();

    for (request, denied_target) in [
        (
            [5, 1, 0, 1, 127, 0, 0, 2, port_high, port_low],
            format!("127.0.0.2:{}", dynamic_echo_address.port()),
        ),
        (
            [
                5,
                1,
                0,
                1,
                127,
                0,
                0,
                1,
                denied_port_high,
                denied_port_low,
            ],
            format!("127.0.0.1:{denied_port}"),
        ),
    ] {
        let mut denied_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
            .await
            .unwrap();
        denied_client.write_all(&[5, 1, 0]).await.unwrap();
        let mut denied_method = [0_u8; 2];
        denied_client.read_exact(&mut denied_method).await.unwrap();
        assert_eq!(denied_method, [5, 0]);
        denied_client.write_all(&request).await.unwrap();
        let mut denied_reply = [0_u8; 10];
        denied_client.read_exact(&mut denied_reply).await.unwrap();
        assert_eq!(denied_reply, super::socks5_reply(2));

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let status = list_tunnels_inner(state, Some(session_id))
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == dynamic_tunnel.id)
                    .unwrap();
                if status.active_connections == 0
                    && status.last_error.as_deref().is_some_and(|error| {
                        error.contains("dynamic route denied by target rules")
                            && error.contains(&denied_target)
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("libssh dynamic tunnel denial metrics did not settle");
    }

    let mut failed_socks_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
        .await
        .unwrap();
    failed_socks_client.write_all(&[5, 1, 0]).await.unwrap();
    let mut failed_method = [0_u8; 2];
    failed_socks_client
        .read_exact(&mut failed_method)
        .await
        .unwrap();
    assert_eq!(failed_method, [5, 0]);
    failed_socks_client
        .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
        .await
        .unwrap();
    let mut failed_socks_reply = [0_u8; 10];
    failed_socks_client
        .read_exact(&mut failed_socks_reply)
        .await
        .unwrap();
    assert_eq!(failed_socks_reply, super::socks5_reply(5));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == dynamic_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 3 {
                assert!(status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("dynamic direct-tcpip open failed")));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh dynamic tunnel failure metrics did not settle");

    let dynamic_echo_listener = TcpListener::bind(dynamic_echo_address).await.unwrap();
    let dynamic_echo = tokio::spawn(async move {
        let (mut socket, _) = dynamic_echo_listener.accept().await.unwrap();
        let mut request = [0_u8; 4];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut socks_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
        .await
        .unwrap();
    socks_client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0_u8; 2];
    socks_client.read_exact(&mut method).await.unwrap();
    assert_eq!(method, [5, 0]);
    socks_client
        .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
        .await
        .unwrap();
    let mut socks_reply = [0_u8; 10];
    socks_client.read_exact(&mut socks_reply).await.unwrap();
    assert_eq!(socks_reply, super::socks5_reply(0));
    socks_client.write_all(b"ping").await.unwrap();
    let mut socks_response = [0_u8; 4];
    socks_client.read_exact(&mut socks_response).await.unwrap();
    assert_eq!(&socks_response, b"pong");
    drop(socks_client);
    dynamic_echo.await.unwrap();

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == dynamic_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 4 {
                assert_eq!(status.tcp_to_ssh_bytes, 4);
                assert_eq!(status.ssh_to_tcp_bytes, 4);
                assert!(status.last_error.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh dynamic tunnel metrics did not settle");
    let stopped = stop_tunnel_inner(state, &dynamic_tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);

    let remote_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let remote_echo_address = remote_echo_listener.local_addr().unwrap();
    drop(remote_echo_listener);
    let remote_tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: session_id.to_string(),
            egress: TunnelEgress::Ssh,
            mode: TunnelMode::Remote,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: "127.0.0.1".to_string(),
            target_port: remote_echo_address.port(),
            route_rules: Vec::new(),
            allow_remote_bind: false,
            label: None,
        },
    )
    .await
    .unwrap();
    assert_ne!(remote_tunnel.bind_port, 0);

    let mut failed_remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
        .await
        .unwrap();
    failed_remote_client.write_all(b"ping").await.unwrap();
    let mut closed_byte = [0_u8; 1];
    let read = tokio::time::timeout(
        Duration::from_secs(3),
        failed_remote_client.read(&mut closed_byte),
    )
    .await
    .expect("failed libssh remote tunnel client did not close");
    assert_tunnel_client_closed(read, "failed libssh remote tunnel client");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == remote_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 1 {
                assert!(status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("target connect failed")));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh remote tunnel failure metrics did not settle");

    let remote_echo_listener = TcpListener::bind(remote_echo_address).await.unwrap();
    let remote_echo = tokio::spawn(async move {
        let (mut socket, _) = remote_echo_listener.accept().await.unwrap();
        let mut request = [0_u8; 4];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
        .await
        .unwrap();
    remote_client.write_all(b"ping").await.unwrap();
    let mut remote_response = [0_u8; 4];
    remote_client
        .read_exact(&mut remote_response)
        .await
        .unwrap();
    assert_eq!(&remote_response, b"pong");
    drop(remote_client);
    remote_echo.await.unwrap();

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == remote_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 2 {
                assert_eq!(status.tcp_to_ssh_bytes, 4);
                assert_eq!(status.ssh_to_tcp_bytes, 4);
                assert!(status.last_error.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh remote tunnel metrics did not settle");
    let stopped = stop_tunnel_inner(state, &remote_tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Ok(stream) = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port)).await {
            drop(stream);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh remote tunnel listener remained reachable after stop");
}

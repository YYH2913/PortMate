use super::*;

#[test]
fn tcp_connection_details_validate_endpoint_and_reconnect_flag() {
    let mut profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
        host: " 127.0.0.1 ".to_string(),
        port: 2323,
        reconnect: true,
        ..Default::default()
    }));
    let (tcp, label) = tcp_connection_details(&profile).unwrap();
    assert_eq!(
        (tcp.host, tcp.port, label),
        ("127.0.0.1".to_string(), 2323, "TCP")
    );
    assert!(tcp_reconnect_enabled(&profile));

    profile.connection = ConnectionConfig::Telnet(portmate_core::TcpConnection {
        host: "console.lab".to_string(),
        port: 23,
        reconnect: false,
        ..Default::default()
    });
    let (tcp, label) = tcp_connection_details(&profile).unwrap();
    assert_eq!(
        (tcp.host, tcp.port, label),
        ("console.lab".to_string(), 23, "Telnet")
    );
    assert!(!tcp_reconnect_enabled(&profile));

    profile.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
        host: " ".to_string(),
        port: 23,
        reconnect: true,
        ..Default::default()
    });
    assert!(tcp_connection_details(&profile)
        .unwrap_err()
        .contains("主机不能为空"));

    profile.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
        host: "127.0.0.1".to_string(),
        port: 0,
        reconnect: true,
        ..Default::default()
    });
    assert!(tcp_connection_details(&profile)
        .unwrap_err()
        .contains("端口不能为空"));
}

#[test]
fn tcp_socket_enables_bounded_kernel_keepalive() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = release_rx.await;
            drop(socket);
        });

        let mut tcp = TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            keepalive_idle_seconds: 45,
            keepalive_interval_seconds: 7,
            keepalive_retries: 5,
            ..Default::default()
        };
        let stream = connect_tcp_socket(&tcp, "TCP").await.unwrap();
        let socket = SockRef::from(&stream);
        assert!(socket.keepalive().unwrap());
        #[cfg(target_os = "linux")]
        {
            assert_eq!(
                socket.tcp_keepalive_time().unwrap(),
                Duration::from_secs(tcp.keepalive_idle_seconds)
            );
            assert_eq!(
                socket.tcp_keepalive_interval().unwrap(),
                Duration::from_secs(tcp.keepalive_interval_seconds)
            );
            assert_eq!(
                socket.tcp_keepalive_retries().unwrap(),
                tcp.keepalive_retries
            );
        }
        tcp.keepalive_enabled = false;
        configure_tcp_socket(&stream, "TCP", &tcp).unwrap();
        assert!(!socket.keepalive().unwrap());

        drop(stream);
        let _ = release_tx.send(());
        server.await.unwrap();
    });
}

#[test]
fn ssh_socket_applies_explicit_tcp_keepalive_only() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = release_rx.await;
            drop(socket);
        });

        let stream = TcpStream::connect(address).await.unwrap();
        configure_ssh_tcp_keepalive(&stream, "SSH", None).unwrap();
        configure_ssh_tcp_keepalive(&stream, "SSH", Some(true)).unwrap();
        let socket = SockRef::from(&stream);
        assert!(socket.keepalive().unwrap());
        configure_ssh_tcp_keepalive(&stream, "SSH", Some(false)).unwrap();
        assert!(!socket.keepalive().unwrap());

        drop(stream);
        let _ = release_tx.send(());
        server.await.unwrap();
    });
}

#[test]
fn authenticated_proxy_handshakes_accept_valid_credentials_and_reject_invalid_ones() {
    tauri::async_runtime::block_on(async {
        let expected_http = format!(
            "Proxy-Authorization: Basic {}",
            BASE64_STANDARD.encode("proxy-user:proxy-password")
        );
        let (http_port, http_task) = spawn_test_http_auth_endpoint(expected_http.clone()).await;
        let mut stream = TcpStream::connect(("127.0.0.1", http_port)).await.unwrap();
        let credentials = ProxyCredentials {
            username: "proxy-user".to_string(),
            password: Zeroizing::new("proxy-password".to_string()),
        };
        perform_http_connect(&mut stream, "target.example:443", Some(&credentials), "TCP")
            .await
            .unwrap();
        http_task.await.unwrap();

        let (http_port, http_task) = spawn_test_http_auth_endpoint(expected_http).await;
        let mut stream = TcpStream::connect(("127.0.0.1", http_port)).await.unwrap();
        let wrong_credentials = ProxyCredentials {
            username: "proxy-user".to_string(),
            password: Zeroizing::new("wrong-password".to_string()),
        };
        let error = perform_http_connect(
            &mut stream,
            "target.example:443",
            Some(&wrong_credentials),
            "TCP",
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("407 Proxy Authentication Required"),
            "{error}"
        );
        assert!(!error.contains("wrong-password"));
        http_task.await.unwrap();

        let (socks_port, socks_task) =
            spawn_test_socks5_auth_endpoint("proxy-user".to_string(), "proxy-password".to_string())
                .await;
        let mut stream = TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap();
        perform_socks5_connect(
            &mut stream,
            "target.example",
            443,
            Some(&credentials),
            "TCP",
        )
        .await
        .unwrap();
        socks_task.await.unwrap();

        let (socks_port, socks_task) =
            spawn_test_socks5_auth_endpoint("proxy-user".to_string(), "proxy-password".to_string())
                .await;
        let mut stream = TcpStream::connect(("127.0.0.1", socks_port)).await.unwrap();
        let error = perform_socks5_connect(
            &mut stream,
            "target.example",
            443,
            Some(&wrong_credentials),
            "TCP",
        )
        .await
        .unwrap_err();
        assert!(error.contains("用户名/密码认证失败"), "{error}");
        assert!(!error.contains("wrong-password"));
        socks_task.await.unwrap();
    });
}

#[test]
fn tcp_proxy_transports_forward_and_report_rejections() {
    tauri::async_runtime::block_on(async {
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_port = target.local_addr().unwrap().port();
        let target_task = tokio::spawn(async move {
            for payload in [
                b"http-ok".as_slice(),
                b"socks-ok".as_slice(),
                b"direct-ok".as_slice(),
            ] {
                let (mut socket, _) = target.accept().await.unwrap();
                socket.write_all(payload).await.unwrap();
            }
        });

        let (http_port, http_connections, http_task) = spawn_test_http_connect_proxy(200).await;
        let http = TcpConnection {
            host: "127.0.0.1".to_string(),
            port: target_port,
            proxy: ProxyConfig {
                enabled: true,
                kind: ProxyKind::HttpConnect,
                host: "127.0.0.1".to_string(),
                port: http_port,
                ..ProxyConfig::default()
            },
            ..Default::default()
        };
        let mut stream = connect_tcp_socket(&http, "TCP").await.unwrap();
        let mut payload = [0_u8; 7];
        stream.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"http-ok");
        drop(stream);

        let (socks_port, socks_connections, socks_task) = spawn_test_socks5_proxy(0).await;
        let socks = TcpConnection {
            proxy: ProxyConfig {
                enabled: true,
                kind: ProxyKind::Socks5,
                host: "127.0.0.1".to_string(),
                port: socks_port,
                ..ProxyConfig::default()
            },
            ..http.clone()
        };
        let mut stream = connect_tcp_socket(&socks, "Telnet").await.unwrap();
        let mut payload = [0_u8; 8];
        stream.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"socks-ok");
        drop(stream);

        let disabled = TcpConnection {
            proxy: ProxyConfig {
                enabled: false,
                host: "invalid\r\nproxy".to_string(),
                port: 0,
                ..socks.proxy.clone()
            },
            ..socks.clone()
        };
        let mut stream = connect_tcp_socket(&disabled, "TCP").await.unwrap();
        let mut payload = [0_u8; 9];
        stream.read_exact(&mut payload).await.unwrap();
        assert_eq!(&payload, b"direct-ok");
        drop(stream);
        target_task.await.unwrap();
        assert_eq!(http_connections.load(Ordering::SeqCst), 1);
        assert_eq!(socks_connections.load(Ordering::SeqCst), 1);

        let (rejected_http_port, _, rejected_http_task) = spawn_test_http_connect_proxy(407).await;
        let rejected_http = TcpConnection {
            proxy: ProxyConfig {
                port: rejected_http_port,
                ..http.proxy.clone()
            },
            ..http.clone()
        };
        let error = connect_tcp_socket(&rejected_http, "TCP").await.unwrap_err();
        assert!(error.contains("407 Rejected"), "{error}");

        let (rejected_socks_port, _, rejected_socks_task) = spawn_test_socks5_proxy(0x05).await;
        let rejected_socks = TcpConnection {
            proxy: ProxyConfig {
                enabled: true,
                kind: ProxyKind::Socks5,
                host: "127.0.0.1".to_string(),
                port: rejected_socks_port,
                ..ProxyConfig::default()
            },
            ..http
        };
        let error = connect_tcp_socket(&rejected_socks, "TCP")
            .await
            .unwrap_err();
        assert!(error.contains("connection refused (0x05)"), "{error}");

        let invalid_proxy = TcpConnection {
            proxy: ProxyConfig {
                enabled: true,
                host: "   ".to_string(),
                port: 0,
                ..ProxyConfig::default()
            },
            ..rejected_socks.clone()
        };
        let error = connect_tcp_socket(&invalid_proxy, "TCP").await.unwrap_err();
        assert!(error.contains("代理主机不能为空"), "{error}");

        let injected_target = TcpConnection {
            host: "target.example\r\nX-Injected: true".to_string(),
            ..rejected_socks
        };
        let error = connect_tcp_socket(&injected_target, "TCP")
            .await
            .unwrap_err();
        assert!(error.contains("代理目标主机不能包含换行符"), "{error}");

        for task in [
            http_task,
            socks_task,
            rejected_http_task,
            rejected_socks_task,
        ] {
            task.abort();
            let _ = task.await;
        }
    });
}

#[test]
fn tcp_reconnect_profile_reloads_latest_endpoint_and_disable_state() {
    let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
        host: "old.example".to_string(),
        port: 2323,
        reconnect: true,
        ..Default::default()
    }));
    let state = test_app_state(profile.clone(), PathBuf::from("tcp-reconnect-test.sqlite3"));
    assert!(tcp_reconnect_attempt_matches_profile(&profile, &profile));
    assert_eq!(
        tcp_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
        TcpReconnectProfileState::Current
    );

    let mut renamed = profile.clone();
    renamed.name = "Renamed TCP".to_string();
    assert!(tcp_reconnect_attempt_matches_profile(&profile, &renamed));

    let mut terminal_updated = profile.clone();
    terminal_updated.terminal.term = "vt100".to_string();
    assert!(!tcp_reconnect_attempt_matches_profile(
        &profile,
        &terminal_updated
    ));

    let mut updated = profile.clone();
    updated.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
        host: "new.example".to_string(),
        port: 4242,
        reconnect: true,
        proxy: ProxyConfig {
            enabled: true,
            kind: ProxyKind::HttpConnect,
            host: "proxy.example".to_string(),
            port: 3128,
            ..ProxyConfig::default()
        },
        reconnect_delay_ms: 2_500,
        keepalive_enabled: false,
        keepalive_idle_seconds: 90,
        keepalive_interval_seconds: 15,
        keepalive_retries: 6,
        telnet_binary: false,
        telnet_naws: false,
        tls_enabled: false,
        tls_server_name: None,
        tls_accept_invalid_cert: false,
    });
    state.store.lock().unwrap().upsert_profile(updated);
    assert_eq!(
        tcp_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
        TcpReconnectProfileState::Changed
    );
    let latest = latest_tcp_reconnect_profile(&state, &profile.id)
        .unwrap()
        .unwrap();
    let (tcp, label) = tcp_connection_details(&latest).unwrap();
    assert_eq!(
        (tcp.host, tcp.port, label),
        ("new.example".to_string(), 4242, "TCP")
    );
    assert_eq!(tcp.reconnect_delay_ms, 2_500);
    assert!(tcp.proxy.enabled);
    assert_eq!(tcp.proxy.kind, ProxyKind::HttpConnect);
    assert_eq!(tcp.proxy.host, "proxy.example");
    assert_eq!(tcp.proxy.port, 3128);
    assert!(!tcp.keepalive_enabled);
    assert_eq!(tcp.keepalive_idle_seconds, 90);
    assert_eq!(tcp.keepalive_interval_seconds, 15);
    assert_eq!(tcp.keepalive_retries, 6);
    assert!(!tcp.telnet_binary);
    assert!(!tcp.telnet_naws);

    let mut disabled = latest;
    if let ConnectionConfig::Tcp(tcp) = &mut disabled.connection {
        tcp.reconnect = false;
    }
    state.store.lock().unwrap().upsert_profile(disabled);
    assert_eq!(
        tcp_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
        TcpReconnectProfileState::Disabled
    );
    assert!(latest_tcp_reconnect_profile(&state, &profile.id)
        .unwrap()
        .is_none());
}

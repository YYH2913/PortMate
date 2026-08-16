use super::*;

#[cfg(unix)]
pub(super) async fn exercise_openssh_local_and_dynamic_tunnels(
    state: &AppState,
    profile: &SessionProfile,
) {
    let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let echo_address = echo_listener.local_addr().unwrap();
    drop(echo_listener);
    let tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: profile.id.clone(),
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
    assert_ne!(tunnel.bind_port, 0);

    let mut failed_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
        .await
        .unwrap();
    failed_client.write_all(b"ping").await.unwrap();
    let mut closed_byte = [0_u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(2), failed_client.read(&mut closed_byte))
        .await
        .expect("failed local tunnel client did not close");
    assert_tunnel_client_closed(read, "failed local tunnel client");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = list_tunnels_inner(state, Some(&profile.id))
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
    .expect("local tunnel failure metrics did not settle");

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

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = list_tunnels_inner(state, Some(&profile.id))
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
    .expect("local tunnel metrics did not settle");
    let stopped = stop_tunnel_inner(state, &tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);

    let dynamic_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let dynamic_echo_address = dynamic_echo_listener.local_addr().unwrap();
    drop(dynamic_echo_listener);
    let dynamic_tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: profile.id.clone(),
            egress: TunnelEgress::Ssh,
            mode: TunnelMode::Dynamic,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: String::new(),
            target_port: 0,
            route_rules: Vec::new(),
            allow_remote_bind: false,
            label: None,
        },
    )
    .await
    .unwrap();
    assert_ne!(dynamic_tunnel.bind_port, 0);

    let [port_high, port_low] = dynamic_echo_address.port().to_be_bytes();
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
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = list_tunnels_inner(state, Some(&profile.id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == dynamic_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 1 {
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
    .expect("dynamic tunnel failure metrics did not settle");

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

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = list_tunnels_inner(state, Some(&profile.id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == dynamic_tunnel.id)
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
    .expect("dynamic tunnel metrics did not settle");
    let stopped = stop_tunnel_inner(state, &dynamic_tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);
}

#[cfg(unix)]
pub(super) async fn exercise_openssh_remote_tunnel(state: &AppState, profile: &SessionProfile) {
    let remote_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let remote_echo_address = remote_echo_listener.local_addr().unwrap();
    drop(remote_echo_listener);
    let remote_tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: profile.id.clone(),
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
    assert!(remote_tunnel
        .label
        .contains(&remote_tunnel.bind_port.to_string()));

    let mut failed_remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
        .await
        .unwrap();
    failed_remote_client.write_all(b"ping").await.unwrap();
    let mut closed_byte = [0_u8; 1];
    let read = tokio::time::timeout(
        Duration::from_secs(2),
        failed_remote_client.read(&mut closed_byte),
    )
    .await
    .expect("failed remote tunnel client did not close");
    assert_tunnel_client_closed(read, "failed remote tunnel client");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = list_tunnels_inner(state, Some(&profile.id))
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
    .expect("remote tunnel failure metrics did not settle");

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

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let status = list_tunnels_inner(state, Some(&profile.id))
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
    .expect("remote tunnel metrics did not settle");

    let (remote_health_handle, remote_forward_routes) = {
        let connections = state.ssh.lock().unwrap();
        let runtime = connections.get(&profile.id).unwrap();
        (
            Arc::clone(&runtime.handle),
            Arc::clone(&runtime.remote_forwards),
        )
    };
    {
        let handle = remote_health_handle.lock().await;
        handle
            .russh_compat()
            .unwrap()
            .cancel_tcpip_forward(
                remote_tunnel.bind_host.clone(),
                u32::from(remote_tunnel.bind_port),
            )
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port)).await {
                Err(_) => break,
                Ok(stream) => drop(stream),
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("server-side remote forward cancellation did not close the listener");

    assert_eq!(
        check_remote_tunnel_health(state, &remote_tunnel.id)
            .await
            .unwrap(),
        RemoteTunnelHealth::Restored
    );
    assert!(state
        .store
        .lock()
        .unwrap()
        .tail_log(&profile.id, 100)
        .iter()
        .any(|event| event.text.as_deref().is_some_and(|text| {
            text.contains(&remote_tunnel.id)
                && text.contains("listener was missing and has been restored")
        })));

    let restored_remote_echo_listener = TcpListener::bind(remote_echo_address).await.unwrap();
    let restored_remote_echo = tokio::spawn(async move {
        let (mut socket, _) = restored_remote_echo_listener.accept().await.unwrap();
        let mut request = [0_u8; 4];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut restored_remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
        .await
        .unwrap();
    restored_remote_client.write_all(b"ping").await.unwrap();
    let mut restored_remote_response = [0_u8; 4];
    restored_remote_client
        .read_exact(&mut restored_remote_response)
        .await
        .unwrap();
    assert_eq!(&restored_remote_response, b"pong");
    drop(restored_remote_client);
    restored_remote_echo.await.unwrap();

    {
        let handle = remote_health_handle.lock().await;
        handle
            .russh_compat()
            .unwrap()
            .cancel_tcpip_forward(
                remote_tunnel.bind_host.clone(),
                u32::from(remote_tunnel.bind_port),
            )
            .await
            .unwrap();
    }
    let stopped = stop_tunnel_inner(state, &remote_tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);
    assert!(stopped
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("remote SSH tunnel cancel failed")));
    assert!(list_tunnels_inner(state, Some(&profile.id))
        .unwrap()
        .iter()
        .all(|status| status.spec.id != remote_tunnel.id));
    {
        let routes = remote_forward_routes.lock().unwrap();
        assert!(!routes.contains_key(&remote_forward_key(
            &remote_tunnel.bind_host,
            remote_tunnel.bind_port,
        )));
        assert!(!routes.contains_key(&remote_forward_port_key(remote_tunnel.bind_port)));
    }
    let saved_profile = state.store.lock().unwrap().profile(&profile.id).unwrap();
    let saved_remote_tunnel = match saved_profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh
            .tunnels
            .into_iter()
            .find(|tunnel| tunnel.id == remote_tunnel.id)
            .unwrap(),
        _ => panic!("expected SSH profile"),
    };
    assert!(!saved_remote_tunnel.enabled);
}

#[cfg(unix)]
pub(super) async fn exercise_openssh_tunnel_reconnect(
    state: &AppState,
    profile: &SessionProfile,
    port: u16,
) {
    let reconnect_tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: profile.id.clone(),
            egress: TunnelEgress::Ssh,
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: "127.0.0.1".to_string(),
            target_port: port,
            route_rules: Vec::new(),
            allow_remote_bind: false,
            label: Some("reconnect tunnel".to_string()),
        },
    )
    .await
    .unwrap();
    let reconnect_remote_tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: profile.id.clone(),
            egress: TunnelEgress::Ssh,
            mode: TunnelMode::Remote,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: "127.0.0.1".to_string(),
            target_port: port,
            route_rules: Vec::new(),
            allow_remote_bind: false,
            label: Some("reconnect remote tunnel".to_string()),
        },
    )
    .await
    .unwrap();
    let conflict_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let conflict_port = conflict_listener.local_addr().unwrap().port();
    let conflict_tunnel = TunnelSpec {
        id: "reconnect-conflict".to_string(),
        label: "occupied reconnect tunnel".to_string(),
        egress: TunnelEgress::Ssh,
        mode: TunnelMode::Local,
        bind_host: "127.0.0.1".to_string(),
        bind_port: conflict_port,
        target_host: "127.0.0.1".to_string(),
        target_port: port,
        route_rules: Vec::new(),
        enabled: true,
    };
    {
        let mut store = state.store.lock().unwrap();
        let mut saved_profile = store.profile(&profile.id).unwrap();
        match &mut saved_profile.connection {
            ConnectionConfig::Ssh(ssh) => {
                ssh.tunnels.push(conflict_tunnel.clone());
                ssh.reconnect_delay_ms = 5_000;
            }
            _ => panic!("expected SSH profile"),
        }
        store.upsert_profile(saved_profile);
        save_store(&state.store_path, &store).unwrap();
    }
    let (previous_runtime_id, reconnect_handle) = {
        let connections = state.ssh.lock().unwrap();
        let runtime = connections.get(&profile.id).unwrap();
        (runtime.runtime_id.clone(), Arc::clone(&runtime.handle))
    };
    {
        let handle = reconnect_handle.lock().await;
        handle
            .disconnect("PortMate tunnel reconnect integration test")
            .await
            .unwrap();
    }

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let reconnecting = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                runtime.session_id == profile.id && runtime.status == SessionStatus::Reconnecting
            });
            if reconnecting {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("SSH runtime did not enter reconnecting state");
    {
        let mut store = state.store.lock().unwrap();
        let mut updated = store.profile(&profile.id).unwrap();
        match &mut updated.connection {
            ConnectionConfig::Ssh(ssh) => ssh.reconnect_delay_ms = 100,
            _ => panic!("expected SSH profile"),
        }
        store.upsert_profile(updated);
        save_store(&state.store_path, &store).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let runtime_replaced = state
                .ssh
                .lock()
                .unwrap()
                .get(&profile.id)
                .is_some_and(|runtime| runtime.runtime_id != previous_runtime_id);
            if runtime_replaced {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("SSH reconnect did not adopt the shortened profile delay");

    let restored = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let runtime_replaced = state
                .ssh
                .lock()
                .unwrap()
                .get(&profile.id)
                .is_some_and(|runtime| runtime.runtime_id != previous_runtime_id);
            let restored = list_tunnels_inner(state, Some(&profile.id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == reconnect_tunnel.id);
            let remote_restored = list_tunnels_inner(state, Some(&profile.id))
                .unwrap()
                .iter()
                .any(|status| status.spec.id == reconnect_remote_tunnel.id);
            let conflict_reported = state
                .store
                .lock()
                .unwrap()
                .tail_log(&profile.id, 200)
                .iter()
                .any(|event| {
                    event.text.as_deref().is_some_and(|text| {
                        text.contains("failed to restore SSH tunnel reconnect-conflict")
                            && text.contains("SSH tunnel bind failed")
                    })
                });
            if runtime_replaced && remote_restored && conflict_reported {
                if let Some(restored) = restored {
                    break restored;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        let statuses = list_tunnels_inner(state, Some(&profile.id)).unwrap();
        let events = state.store.lock().unwrap().tail_log(&profile.id, 20);
        panic!(
            "SSH reconnect did not restore the tunnel runtime; statuses={statuses:?}; recent events={events:?}"
        )
    });
    assert_eq!(restored.spec.id, reconnect_tunnel.id);
    assert_eq!(restored.spec.label, reconnect_tunnel.label);
    assert_eq!(restored.spec.bind_port, reconnect_tunnel.bind_port);
    let restored_tunnels = list_tunnels_inner(state, Some(&profile.id)).unwrap();
    let restored_remote = restored_tunnels
        .iter()
        .find(|status| status.spec.id == reconnect_remote_tunnel.id)
        .unwrap();
    assert_eq!(restored_remote.spec.label, reconnect_remote_tunnel.label);
    assert_eq!(
        restored_remote.spec.bind_port,
        reconnect_remote_tunnel.bind_port
    );
    assert!(restored_tunnels
        .iter()
        .all(|status| status.spec.id != conflict_tunnel.id));

    let saved_profile = state.store.lock().unwrap().profile(&profile.id).unwrap();
    let saved_tunnels = match saved_profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh.tunnels,
        _ => panic!("expected SSH profile"),
    };
    assert!(saved_tunnels
        .iter()
        .any(|tunnel| tunnel.id == conflict_tunnel.id && tunnel.enabled));
    assert!(state
        .store
        .lock()
        .unwrap()
        .tail_log(&profile.id, 200)
        .iter()
        .any(|event| event.text.as_deref().is_some_and(|text| {
            text.contains("failed to restore SSH tunnel reconnect-conflict")
                && text.contains("SSH tunnel bind failed")
        })));
    let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
    assert!(screen.contains("reconnecting in 5000ms"), "{screen}");

    let mut restored_client = TcpStream::connect(("127.0.0.1", reconnect_tunnel.bind_port))
        .await
        .unwrap();
    let mut ssh_banner = [0_u8; 4];
    tokio::time::timeout(
        Duration::from_secs(2),
        restored_client.read_exact(&mut ssh_banner),
    )
    .await
    .expect("restored tunnel did not receive an SSH banner")
    .unwrap();
    assert_eq!(&ssh_banner, b"SSH-");
    drop(restored_client);

    let mut restored_remote_client =
        TcpStream::connect(("127.0.0.1", reconnect_remote_tunnel.bind_port))
            .await
            .unwrap();
    let mut remote_ssh_banner = [0_u8; 4];
    tokio::time::timeout(
        Duration::from_secs(2),
        restored_remote_client.read_exact(&mut remote_ssh_banner),
    )
    .await
    .expect("restored remote tunnel did not receive an SSH banner")
    .unwrap();
    assert_eq!(&remote_ssh_banner, b"SSH-");
    drop(restored_remote_client);
    drop(conflict_listener);
    let stopped = stop_tunnel_inner(state, &reconnect_tunnel.id)
        .await
        .unwrap();
    assert!(!stopped.spec.enabled);
    let stopped = stop_tunnel_inner(state, &reconnect_remote_tunnel.id)
        .await
        .unwrap();
    assert!(!stopped.spec.enabled);
}

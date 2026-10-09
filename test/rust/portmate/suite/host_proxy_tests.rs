use super::*;

#[test]
fn portmate_host_proxy_approval_names_routes_and_remote_listener_exposure() {
    let root = tempfile::tempdir().unwrap();
    let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
    let request = IpcRequest {
        token: "authenticated-token".to_string(),
        client_id: "host-proxy-client".to_string(),
        trusted_write: false,
        command: "create_tunnel".to_string(),
        args: serde_json::json!({
            "egress": "portmate-host",
            "mode": "dynamic",
            "bindHost": "0.0.0.0",
            "bindPort": 1080,
            "routeRules": [
                { "host": "*.internal.example", "port": 443 },
                { "host": "10.20.0.0/16", "port": null },
                { "host": "2001:db8:42::/48", "port": 22 }
            ],
            "allowRemoteBind": true
        }),
    };
    validate_ipc_write_args(&state, &request).unwrap();
    let target = capture_mcp_write_execution_context(&state, &request)
        .unwrap()
        .approval_target()
        .unwrap();
    assert_eq!(target.kind, "portmate-host-proxy");
    assert_eq!(target.id, "0.0.0.0:1080");
    assert!(target.label.contains("SOCKS5"));
    assert!(target.label.contains("*.internal.example:443"));
    assert!(target.label.contains("10.20.0.0/16"));
    assert!(target.label.contains("+1 more"));
    assert!(target.label.contains("remote listener allowed"));
}

#[test]
fn unified_host_tunnel_approval_keeps_the_host_proxy_target() {
    let root = tempfile::tempdir().unwrap();
    let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
    let request = IpcRequest {
        token: "authenticated-token".to_string(),
        client_id: "unified-host-proxy-client".to_string(),
        trusted_write: false,
        command: "create_tunnel".to_string(),
        args: serde_json::json!({
            "egress": "portmate-host",
            "mode": "local",
            "bindHost": "127.0.0.1",
            "bindPort": 0,
            "targetHost": "192.168.33.143",
            "targetPort": 80
        }),
    };
    validate_ipc_write_args(&state, &request).unwrap();
    let target = capture_mcp_write_execution_context(&state, &request)
        .unwrap()
        .approval_target()
        .unwrap();
    assert_eq!(target.kind, "portmate-host-proxy");
    assert_eq!(target.id, "127.0.0.1:0");
    assert!(target.label.contains("TCP"));
    assert!(target.label.contains("192.168.33.143:80"));
}

#[test]
fn mcp_host_route_forwards_tcp_without_any_session_and_is_client_isolated() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
        {
            let mut store = state.store.lock().unwrap();
            store.profiles.clear();
            store
                .grants
                .extend(
                    ["host-proxy-client", "other-host-client"].map(|client_id| McpGrant {
                        client_id: client_id.to_string(),
                        name: client_id.to_string(),
                        scopes: vec![McpScope::Tunnel],
                        allowed_sessions: vec!["serial-session-only".to_string()],
                        confirm_writes: false,
                        expires_at: None,
                        revoked_at: None,
                    }),
                );
        }

        let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_address = echo_listener.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let response = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "host-proxy-client".to_string(),
                trusted_write: false,
                command: "create_tunnel".to_string(),
                args: serde_json::json!({
                    "mode": "local",
                    "egress": "portmate-host",
                    "bindHost": "127.0.0.1",
                    "bindPort": 0,
                    "targetHost": "127.0.0.1",
                    "targetPort": echo_address.port(),
                    "routeRules": [],
                    "allowRemoteBind": false
                }),
            },
        )
        .await
        .unwrap();
        let tunnel = serde_json::from_value::<TunnelSpec>(response).unwrap();
        assert_eq!(tunnel.egress, TunnelEgress::PortmateHost);
        assert_ne!(tunnel.bind_port, 0);
        assert!(
            stop_session_tunnel_runtimes(&state.tunnels, "serial-session-only")
                .unwrap()
                .is_empty()
        );

        let mut client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
            .await
            .unwrap();
        client.write_all(b"ping").await.unwrap();
        let mut response = [0_u8; 4];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        drop(client);
        echo.await.unwrap();

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let status = list_host_routes_inner(&state, "host-proxy-client")
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == tunnel.id)
                    .unwrap();
                if status.active_connections == 0 && status.total_connections == 1 {
                    assert_eq!(status.spec.egress, TunnelEgress::PortmateHost);
                    assert_eq!(status.tcp_to_ssh_bytes, 4);
                    assert_eq!(status.ssh_to_tcp_bytes, 4);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("PortMate host TCP proxy metrics did not settle");

        let listed = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "host-proxy-client".to_string(),
                trusted_write: false,
                command: "list_tunnels".to_string(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        assert_eq!(listed.as_array().unwrap().len(), 1);

        let other_list = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "other-host-client".to_string(),
                trusted_write: false,
                command: "list_tunnels".to_string(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        assert_eq!(other_list, serde_json::json!([]));
        let other_stop_error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "other-host-client".to_string(),
                trusted_write: false,
                command: "stop_tunnel".to_string(),
                args: serde_json::json!({ "tunnelId": tunnel.id }),
            },
        )
        .await
        .unwrap_err();
        assert!(other_stop_error.contains("owned by another MCP client"));
        assert_eq!(
            list_host_routes_inner(&state, "host-proxy-client")
                .unwrap()
                .len(),
            1
        );

        let stopped = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "host-proxy-client".to_string(),
                trusted_write: false,
                command: "stop_tunnel".to_string(),
                args: serde_json::json!({ "tunnelId": tunnel.id }),
            },
        )
        .await
        .unwrap();
        assert_eq!(stopped["spec"]["egress"], "portmate-host");
        assert!(state.tunnels.lock().unwrap().is_empty());
        assert!(state.store.lock().unwrap().events.is_empty());
        let create_audit = state
            .store
            .lock()
            .unwrap()
            .audit
            .iter()
            .find(|record| record.action == "create_tunnel")
            .cloned()
            .unwrap();
        assert_eq!(create_audit.session_id, None);
        assert_eq!(
            create_audit.details.get("mode").map(String::as_str),
            Some("local")
        );
        assert_eq!(
            create_audit
                .details
                .get("allowRemoteBind")
                .map(String::as_str),
            Some("false")
        );
        assert_eq!(
            create_audit
                .details
                .get("routeRuleCount")
                .map(String::as_str),
            Some("0")
        );
    });
}

#[test]
fn mcp_rejects_session_id_on_host_egress() {
    let root = tempfile::tempdir().unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    let error = validate_ipc_write_args(
        &state,
        &IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "host-proxy-client".to_string(),
            trusted_write: false,
            command: "create_tunnel".to_string(),
            args: serde_json::json!({
                "sessionId": profile.id,
                "egress": "portmate-host",
                "mode": "local",
                "bindHost": "127.0.0.1",
                "bindPort": 0,
                "targetHost": "127.0.0.1",
                "targetPort": 80
            }),
        },
    )
    .unwrap_err();
    assert!(error.contains("must not include sessionId"), "{error}");
}

#[test]
fn portmate_host_route_capacity_is_isolated_per_owner_at_registration() {
    let owner = mcp_host_route_owner_id("capacity-client").unwrap();
    let mut tunnels = HashMap::new();
    for index in 0..MAX_TUNNELS_PER_PROFILE {
        let id = format!("host-route-{index}");
        tunnels.insert(
            id.clone(),
            TunnelRuntime {
                session_id: owner.clone(),
                ssh_runtime_id: format!("host-runtime-{index}"),
                spec: TunnelSpec {
                    id,
                    label: format!("Host route {index}"),
                    egress: TunnelEgress::PortmateHost,
                    mode: TunnelMode::Local,
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 10_000 + index as u16,
                    target_host: "127.0.0.1".to_string(),
                    target_port: 80,
                    route_rules: Vec::new(),
                    enabled: true,
                },
                metrics: Arc::new(TunnelMetrics::default()),
                closed: Arc::new(AtomicBool::new(false)),
                listener_worker: TunnelListenerWorker::completed(),
            },
        );
    }
    assert!(
        ensure_portmate_host_runtime_slot(&tunnels, &owner, "overflow")
            .unwrap_err()
            .contains("for this owner")
    );
    assert!(ensure_portmate_host_runtime_slot(
        &tunnels,
        &mcp_host_route_owner_id("other-capacity-client").unwrap(),
        "other-owner-route",
    )
    .is_ok());
}

#[test]
fn portmate_host_socks5_enforces_route_rules_and_relays_allowed_targets() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "host-socks-client".to_string(),
            name: "Host SOCKS client".to_string(),
            scopes: vec![McpScope::Tunnel],
            allowed_sessions: Vec::new(),
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_address = echo_listener.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let tunnel = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "host-socks-client".to_string(),
                trusted_write: false,
                command: "create_tunnel".to_string(),
                args: serde_json::json!({
                    "egress": "portmate-host",
                    "mode": "dynamic",
                    "bindHost": "127.0.0.1",
                    "bindPort": 0,
                    "routeRules": [{
                        "host": "127.0.0.1",
                        "port": echo_address.port()
                    }]
                }),
            },
        )
        .await
        .unwrap();
        let tunnel = serde_json::from_value::<TunnelSpec>(tunnel).unwrap();

        let denied_port = echo_address.port().saturating_add(1).max(1);
        let [denied_high, denied_low] = denied_port.to_be_bytes();
        let mut denied = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
            .await
            .unwrap();
        denied.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0_u8; 2];
        denied.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);
        denied
            .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, denied_high, denied_low])
            .await
            .unwrap();
        let mut denied_reply = [0_u8; 10];
        denied.read_exact(&mut denied_reply).await.unwrap();
        assert_eq!(denied_reply, socks5_reply(2));

        let [port_high, port_low] = echo_address.port().to_be_bytes();
        let mut client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
            .await
            .unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);
        client
            .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
            .await
            .unwrap();
        let mut accepted_reply = [0_u8; 10];
        client.read_exact(&mut accepted_reply).await.unwrap();
        assert_eq!(accepted_reply, socks5_reply(0));
        client.write_all(b"ping").await.unwrap();
        let mut response = [0_u8; 4];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        drop(client);
        echo.await.unwrap();

        let stopped = handle_ipc_request(
            state,
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "host-socks-client".to_string(),
                trusted_write: false,
                command: "stop_tunnel".to_string(),
                args: serde_json::json!({ "tunnelId": tunnel.id }),
            },
        )
        .await
        .unwrap();
        let stopped = serde_json::from_value::<TunnelStatus>(stopped).unwrap();
        assert!(!stopped.spec.enabled);
    });
}

#[test]
fn mcp_tunnel_request_relays_request_response_through_client_host_route() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "tunnel-request-client".to_string(),
            name: "Tunnel request client".to_string(),
            scopes: vec![McpScope::Tunnel],
            allowed_sessions: Vec::new(),
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_address = echo_listener.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let created = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "tunnel-request-client".to_string(),
                trusted_write: false,
                command: "create_tunnel".to_string(),
                args: serde_json::json!({
                    "egress": "portmate-host",
                    "mode": "local",
                    "bindHost": "127.0.0.1",
                    "bindPort": 0,
                    "targetHost": "127.0.0.1",
                    "targetPort": echo_address.port()
                }),
            },
        )
        .await
        .unwrap();
        let tunnel = serde_json::from_value::<TunnelSpec>(created).unwrap();

        let exchange = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "tunnel-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "base64",
                    "data": BASE64_STANDARD.encode(b"ping")
                }),
            },
        )
        .await
        .unwrap();
        let result = serde_json::from_value::<McpTunnelExchangeResult>(exchange).unwrap();
        assert_eq!(result.tunnel_id, tunnel.id);
        assert_eq!(result.target_host, "127.0.0.1");
        assert_eq!(result.target_port, echo_address.port());
        assert_eq!(result.sent_bytes, 4);
        assert_eq!(result.received_bytes, 4);
        assert!(!result.truncated);
        assert!(!result.timed_out);
        assert_eq!(
            BASE64_STANDARD.decode(&result.response_base64).unwrap(),
            b"pong"
        );
        echo.await.unwrap();

        let store = state.store.lock().unwrap();
        let audit = store
            .audit
            .iter()
            .find(|record| record.action == "tunnel_request")
            .cloned()
            .expect("tunnel_request audit record");
        assert_eq!(audit.session_id, None);
        assert_eq!(
            audit.details.get("tunnelId").map(String::as_str),
            Some(tunnel.id.as_str())
        );
        assert_eq!(
            audit.details.get("encoding").map(String::as_str),
            Some("base64")
        );
        assert_eq!(audit.decision, "succeeded");
        assert!(!serde_json::to_string(&store.audit)
            .unwrap()
            .contains("ping"));
    });
}

#[test]
fn mcp_udp_request_relays_one_datagram_through_owned_host_route() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "udp-request-client".to_string(),
            name: "UDP request client".to_string(),
            scopes: vec![McpScope::Tunnel],
            allowed_sessions: Vec::new(),
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let echo_socket = UdpSocket::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_address = echo_socket.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let mut request = [0_u8; 4];
            let (size, peer) = echo_socket.recv_from(&mut request).await.unwrap();
            assert_eq!(&request[..size], b"ping");
            echo_socket.send_to(b"pong", peer).await.unwrap();
        });

        let created = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "udp-request-client".to_string(),
                trusted_write: false,
                command: "create_tunnel".to_string(),
                args: serde_json::json!({
                    "egress": "portmate-host",
                    "mode": "local",
                    "bindHost": "127.0.0.1",
                    "bindPort": 0,
                    "targetHost": "127.0.0.1",
                    "targetPort": echo_address.port()
                }),
            },
        )
        .await
        .unwrap();
        let tunnel = serde_json::from_value::<TunnelSpec>(created).unwrap();

        let exchange = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "udp-request-client".to_string(),
                trusted_write: false,
                command: "udp_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "base64",
                    "data": BASE64_STANDARD.encode(b"ping")
                }),
            },
        )
        .await
        .unwrap();
        let result = serde_json::from_value::<McpUdpExchangeResult>(exchange).unwrap();
        assert_eq!(result.sent_bytes, 4);
        assert_eq!(result.received_bytes, 4);
        assert!(!result.timed_out);
        assert_eq!(
            BASE64_STANDARD.decode(result.response_base64).unwrap(),
            b"pong"
        );
        echo.await.unwrap();
    });
}

#[test]
fn mcp_tunnel_request_enforces_dynamic_routes_ownership_and_target_rules() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
        state.store.lock().unwrap().grants.extend(
            ["dynamic-request-client", "foreign-request-client"].map(|client_id| McpGrant {
                client_id: client_id.to_string(),
                name: client_id.to_string(),
                scopes: vec![McpScope::Tunnel],
                allowed_sessions: Vec::new(),
                confirm_writes: false,
                expires_at: None,
                revoked_at: None,
            }),
        );

        let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_address = echo_listener.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let created = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "dynamic-request-client".to_string(),
                trusted_write: false,
                command: "create_tunnel".to_string(),
                args: serde_json::json!({
                    "egress": "portmate-host",
                    "mode": "dynamic",
                    "bindHost": "127.0.0.1",
                    "bindPort": 0,
                    "routeRules": [{ "host": "127.0.0.1", "port": echo_address.port() }]
                }),
            },
        )
        .await
        .unwrap();
        let tunnel = serde_json::from_value::<TunnelSpec>(created).unwrap();

        let allowed = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "dynamic-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "hex",
                    "data": "70696e67",
                    "targetHost": "127.0.0.1",
                    "targetPort": echo_address.port()
                }),
            },
        )
        .await
        .unwrap();
        let result = serde_json::from_value::<McpTunnelExchangeResult>(allowed).unwrap();
        assert_eq!(
            BASE64_STANDARD.decode(&result.response_base64).unwrap(),
            b"pong"
        );
        echo.await.unwrap();

        let denied = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "dynamic-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "base64",
                    "data": BASE64_STANDARD.encode(b"ping"),
                    "targetHost": "127.0.0.1",
                    "targetPort": echo_address.port().saturating_add(1).max(1)
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(denied.contains("denied by route rules"), "{denied}");

        let missing_target = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "dynamic-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "base64",
                    "data": BASE64_STANDARD.encode(b"ping")
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(
            missing_target.contains("require targetHost and targetPort"),
            "{missing_target}"
        );

        let foreign = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "foreign-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "base64",
                    "data": BASE64_STANDARD.encode(b"ping"),
                    "targetHost": "127.0.0.1",
                    "targetPort": echo_address.port()
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(foreign.contains("owned by another MCP client"), "{foreign}");
    });
}

#[test]
fn mcp_tunnel_request_rejects_invalid_shapes_and_non_host_egress() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "shape-request-client".to_string(),
            name: "Shape request client".to_string(),
            scopes: vec![McpScope::Tunnel],
            allowed_sessions: Vec::new(),
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let created = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "shape-request-client".to_string(),
                trusted_write: false,
                command: "create_tunnel".to_string(),
                args: serde_json::json!({
                    "egress": "portmate-host",
                    "mode": "local",
                    "bindHost": "127.0.0.1",
                    "bindPort": 0,
                    "targetHost": "127.0.0.1",
                    "targetPort": 443
                }),
            },
        )
        .await
        .unwrap();
        let tunnel = serde_json::from_value::<TunnelSpec>(created).unwrap();

        let invalid_encoding = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "shape-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "utf-8",
                    "data": "cGluZw=="
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(
            invalid_encoding.contains("encoding must be"),
            "{invalid_encoding}"
        );

        let invalid_base64 = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "shape-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "base64",
                    "data": "%%%not-base64%%%"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(
            invalid_base64.contains("not valid standard Base64"),
            "{invalid_base64}"
        );

        let target_override = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "shape-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": tunnel.id,
                    "encoding": "base64",
                    "data": BASE64_STANDARD.encode(b"ping"),
                    "targetHost": "127.0.0.1",
                    "targetPort": 80
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(
            target_override.contains("must not override"),
            "{target_override}"
        );

        let unknown = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "shape-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": "missing-route",
                    "encoding": "base64",
                    "data": BASE64_STANDARD.encode(b"ping")
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(unknown.contains("owned by another MCP client"), "{unknown}");

        let owner = mcp_host_route_owner_id("shape-request-client").unwrap();
        state.tunnels.lock().unwrap().insert(
            "ssh-egress-route".to_string(),
            TunnelRuntime {
                session_id: owner,
                ssh_runtime_id: "ssh-runtime-1".to_string(),
                spec: TunnelSpec {
                    id: "ssh-egress-route".to_string(),
                    label: "SSH route".to_string(),
                    egress: TunnelEgress::Ssh,
                    mode: TunnelMode::Local,
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 10_001,
                    target_host: "127.0.0.1".to_string(),
                    target_port: 80,
                    route_rules: Vec::new(),
                    enabled: true,
                },
                metrics: Arc::new(TunnelMetrics::default()),
                closed: Arc::new(AtomicBool::new(false)),
                listener_worker: TunnelListenerWorker::completed(),
            },
        );
        let ssh_egress = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "shape-request-client".to_string(),
                trusted_write: false,
                command: "tunnel_request".to_string(),
                args: serde_json::json!({
                    "tunnelId": "ssh-egress-route",
                    "encoding": "base64",
                    "data": BASE64_STANDARD.encode(b"ping")
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(
            ssh_egress.contains("owned by another MCP client"),
            "{ssh_egress}"
        );
    });
}

#[test]
fn mcp_tunnel_request_rejects_zero_dynamic_target_port_before_connecting() {
    let error = validate_mcp_tunnel_exchange_request(&McpTunnelExchangeRequest {
        tunnel_id: "dynamic-route".to_string(),
        encoding: "base64".to_string(),
        data: BASE64_STANDARD.encode(b"ping"),
        target_host: Some("127.0.0.1".to_string()),
        target_port: Some(0),
        timeout_ms: None,
        max_response_bytes: None,
        close_write: true,
    })
    .unwrap_err();
    assert!(
        error.contains("targetPort must be between 1 and 65535"),
        "{error}"
    );
}

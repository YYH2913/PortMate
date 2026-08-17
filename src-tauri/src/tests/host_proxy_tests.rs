use super::*;

#[test]
fn portmate_host_proxy_approval_names_routes_and_remote_listener_exposure() {
    let root = tempfile::tempdir().unwrap();
    let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
    let request = IpcRequest {
        token: "authenticated-token".to_string(),
        client_id: "host-proxy-client".to_string(),
        trusted_write: false,
        command: "create_host_route".to_string(),
        args: serde_json::json!({
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
                command: "create_host_route".to_string(),
                args: serde_json::json!({
                    "mode": "local",
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
                command: "list_host_routes".to_string(),
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
                command: "list_host_routes".to_string(),
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
                command: "stop_host_route".to_string(),
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
                command: "stop_host_route".to_string(),
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
            .find(|record| record.action == "create_host_route")
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
fn mcp_rejects_host_egress_on_the_session_tunnel_tool() {
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
    assert!(error.contains("use create_host_route"), "{error}");
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
        let profile = test_shell_profile();
        let session_id = profile.id.clone();
        let state = test_app_state(profile, root.path().join("store.sqlite3"));

        let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_address = echo_listener.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            stream.write_all(b"pong").await.unwrap();
        });

        let tunnel = create_tunnel_inner(
            &state,
            CreateTunnelRequest {
                session_id: session_id.clone(),
                egress: TunnelEgress::PortmateHost,
                mode: TunnelMode::Dynamic,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
                target_host: String::new(),
                target_port: 0,
                route_rules: vec![portmate_core::TunnelRouteRule {
                    host: "127.0.0.1".to_string(),
                    port: Some(echo_address.port()),
                }],
                allow_remote_bind: false,
                label: None,
            },
        )
        .await
        .unwrap();

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

        let stopped = stop_tunnel_inner(&state, &tunnel.id).await.unwrap();
        assert!(!stopped.spec.enabled);
    });
}

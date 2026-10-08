#[cfg(unix)]
use super::*;

#[cfg(unix)]
#[test]
fn ssh_tunnel_preserves_response_after_local_write_half_close() {
    let root = tempfile::tempdir().unwrap();
    let key = root.path().join("key");
    generate_ed25519_test_key(&key);
    tauri::async_runtime::block_on(async {
        let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_port = target.local_addr().unwrap().port();
        let responder = tokio::spawn(async move {
            let (mut socket, _) = target.accept().await.unwrap();
            let mut request = Vec::new();
            socket.read_to_end(&mut request).await.unwrap();
            assert_eq!(request, b"request");
            tokio::time::sleep(Duration::from_millis(30)).await;
            socket.write_all(b"response").await.unwrap();
            socket.shutdown().await.unwrap();
        });
        let (port, _, server) = spawn_mixed_auth_test_server(&key, "user", "secret").await;
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(handle
            .authenticate_password("user", "secret")
            .await
            .unwrap()
            .success());
        let channel = handle
            .channel_open_direct_tcpip("127.0.0.1", target_port.into(), "127.0.0.1", 0)
            .await
            .unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut socket = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (local, _) = listener.accept().await.unwrap();
        let tunnel: TunnelSpec = serde_json::from_value(serde_json::json!({
            "id": "half-close", "label": "half-close", "mode": "local", "egress": "ssh",
            "bindHost": "127.0.0.1", "bindPort": 0, "targetHost": "127.0.0.1", "targetPort": target_port,
            "routeRules": [], "enabled": true
        })).unwrap();
        let pipe = tokio::spawn(pipe_ssh_channel_to_tcp(
            SshBackendChannel::from_russh(channel),
            local,
            tunnel,
            Arc::new(TunnelMetrics::default()),
        ));
        socket.write_all(b"request").await.unwrap();
        socket.shutdown().await.unwrap();
        let mut reply = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), socket.read_to_end(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reply, b"response");
        tokio::time::timeout(Duration::from_secs(3), pipe)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        responder.await.unwrap();
        server.abort();
    });
}

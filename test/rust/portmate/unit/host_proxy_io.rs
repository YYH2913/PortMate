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

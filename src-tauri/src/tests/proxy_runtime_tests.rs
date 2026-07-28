use super::*;

#[test]
fn socks5_loopback_parses_domain_connect_request() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_socks5_connect_request(&mut socket).await.unwrap()
        });

        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0_u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);

        let domain = b"example.com";
        let mut request = vec![5, 1, 0, 3, domain.len() as u8];
        request.extend_from_slice(domain);
        request.extend_from_slice(&443_u16.to_be_bytes());
        client.write_all(&request).await.unwrap();

        let target = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("SOCKS5 parser timed out")
            .expect("SOCKS5 parser task failed");
        assert_eq!(target, ("example.com".to_string(), 443));
    });
}

#[test]
fn socks5_loopback_rejects_clients_without_no_auth_method() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_socks5_connect_request(&mut socket).await.unwrap_err()
        });

        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(&[5, 1, 2]).await.unwrap();
        let mut method = [0_u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0xff]);

        let error = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("SOCKS5 rejection timed out")
            .expect("SOCKS5 rejection task failed");
        assert!(error.contains("did not offer no-authentication"));
    });
}

#[test]
fn socks5_loopback_rejects_non_connect_commands_with_reply() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_socks5_connect_request(&mut socket).await.unwrap_err()
        });

        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0_u8; 2];
        client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);
        client.write_all(&[5, 2, 0, 1]).await.unwrap();

        let mut reply = [0_u8; 10];
        client.read_exact(&mut reply).await.unwrap();
        assert_eq!(reply, socks5_reply(7));
        let error = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("SOCKS5 command rejection timed out")
            .expect("SOCKS5 command rejection task failed");
        assert!(error.contains("only SOCKS5 CONNECT"));
    });
}

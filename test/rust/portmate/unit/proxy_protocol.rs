use super::*;
use tokio::io::duplex;

async fn handshake_with_socks5_reply(reply: Vec<u8>) -> Result<(), String> {
    let (mut client, mut server) = duplex(1024);
    let server_task = tokio::spawn(async move {
        let mut greeting = [0_u8; 3];
        server.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [0x05, 0x01, 0x00]);
        server.write_all(&[0x05, 0x00]).await.unwrap();

        let mut request = [0_u8; 5];
        server.read_exact(&mut request).await.unwrap();
        assert_eq!(&request[..4], &[0x05, 0x01, 0x00, 0x03]);
        let mut target = vec![0_u8; usize::from(request[4]) + 2];
        server.read_exact(&mut target).await.unwrap();
        server.write_all(&reply).await.unwrap();
    });
    let result = perform_socks5_connect(&mut client, "target.example", 443, None, "TCP").await;
    server_task.await.unwrap();
    result
}

#[test]
fn socks5_rejects_nonzero_reserved_reply_field() {
    tauri::async_runtime::block_on(async {
        let error = handshake_with_socks5_reply(vec![0x05, 0x00, 0x01, 0x01, 127, 0, 0, 1, 0, 0])
            .await
            .unwrap_err();
        assert!(error.contains("无效保留字段"), "{error}");
    });
}

#[test]
fn socks5_rejects_empty_bound_domain() {
    tauri::async_runtime::block_on(async {
        let error = handshake_with_socks5_reply(vec![0x05, 0x00, 0x00, 0x03, 0x00])
            .await
            .unwrap_err();
        assert!(error.contains("空绑定域名"), "{error}");
    });
}

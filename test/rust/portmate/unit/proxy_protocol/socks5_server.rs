use super::*;
use tokio::io::duplex;

#[test]
fn socks5_server_handshake_has_one_total_deadline() {
    tauri::async_runtime::block_on(async {
        let (mut client, mut server) = duplex(64);
        client.write_all(&[0x05]).await.unwrap();

        let error =
            read_socks5_connect_request_with_timeout(&mut server, Duration::from_millis(20))
                .await
                .unwrap_err();
        assert!(error.contains("timed out after 20 ms"), "{error}");
    });
}

#[test]
fn socks5_server_rejects_whitespace_in_domain_target() {
    tauri::async_runtime::block_on(async {
        let (mut client, mut server) = duplex(128);
        let mut request = vec![0x05, 0x01, 0x00, 0x05, 0x01, 0x00, 0x03, 0x08];
        request.extend_from_slice(b"bad host");
        request.extend_from_slice(&443_u16.to_be_bytes());
        client.write_all(&request).await.unwrap();

        let error = read_socks5_connect_request_with_timeout(&mut server, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(error.contains("control or whitespace"), "{error}");

        let mut replies = [0_u8; 12];
        client.read_exact(&mut replies).await.unwrap();
        assert_eq!(&replies[..2], &[0x05, 0x00]);
        assert_eq!(&replies[2..], &socks5_reply(8));
    });
}

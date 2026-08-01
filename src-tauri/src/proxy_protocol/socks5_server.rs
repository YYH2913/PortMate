use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const SOCKS5_SERVER_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) async fn read_socks5_connect_request<S>(stream: &mut S) -> Result<(String, u16), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    read_socks5_connect_request_with_timeout(stream, SOCKS5_SERVER_HANDSHAKE_TIMEOUT).await
}

async fn read_socks5_connect_request_with_timeout<S>(
    stream: &mut S,
    timeout: Duration,
) -> Result<(String, u16), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(timeout, read_socks5_connect_request_inner(stream))
        .await
        .map_err(|_| {
            format!(
                "SOCKS5 handshake timed out after {} ms",
                timeout.as_millis()
            )
        })?
}

async fn read_socks5_connect_request_inner<S>(stream: &mut S) -> Result<(String, u16), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut header = [0_u8; 2];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|error| format!("SOCKS5 handshake read failed: {error}"))?;
    if header[0] != 5 {
        let _ = stream.write_all(&[5, 0xff]).await;
        return Err("only SOCKS5 is supported for dynamic tunnel".to_string());
    }
    let mut methods = vec![0_u8; header[1] as usize];
    stream
        .read_exact(&mut methods)
        .await
        .map_err(|error| format!("SOCKS5 methods read failed: {error}"))?;
    if !methods.contains(&0) {
        stream
            .write_all(&[5, 0xff])
            .await
            .map_err(|error| format!("SOCKS5 method rejection failed: {error}"))?;
        return Err("SOCKS5 client did not offer no-authentication method".to_string());
    }
    stream
        .write_all(&[5, 0])
        .await
        .map_err(|error| format!("SOCKS5 method response failed: {error}"))?;

    let mut request = [0_u8; 4];
    stream
        .read_exact(&mut request)
        .await
        .map_err(|error| format!("SOCKS5 request read failed: {error}"))?;
    if request[0] != 5 || request[2] != 0 {
        let _ = stream.write_all(&socks5_reply(1)).await;
        return Err("invalid SOCKS5 CONNECT request header".to_string());
    }
    if request[1] != 1 {
        stream.write_all(&socks5_reply(7)).await.ok();
        return Err("only SOCKS5 CONNECT is supported".to_string());
    }

    let target_host = match request[3] {
        1 => {
            let mut addr = [0_u8; 4];
            stream
                .read_exact(&mut addr)
                .await
                .map_err(|error| format!("SOCKS5 IPv4 read failed: {error}"))?;
            std::net::Ipv4Addr::from(addr).to_string()
        }
        3 => read_socks5_domain(stream).await?,
        4 => {
            let mut addr = [0_u8; 16];
            stream
                .read_exact(&mut addr)
                .await
                .map_err(|error| format!("SOCKS5 IPv6 read failed: {error}"))?;
            std::net::Ipv6Addr::from(addr).to_string()
        }
        other => {
            let _ = stream.write_all(&socks5_reply(8)).await;
            return Err(format!("unsupported SOCKS5 address type: {other}"));
        }
    };
    let mut port_bytes = [0_u8; 2];
    stream
        .read_exact(&mut port_bytes)
        .await
        .map_err(|error| format!("SOCKS5 port read failed: {error}"))?;
    let target_port = u16::from_be_bytes(port_bytes);
    if target_port == 0 {
        let _ = stream.write_all(&socks5_reply(1)).await;
        return Err("SOCKS5 target port cannot be zero".to_string());
    }
    Ok((target_host, target_port))
}

async fn read_socks5_domain<S>(stream: &mut S) -> Result<String, String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut len = [0_u8; 1];
    stream
        .read_exact(&mut len)
        .await
        .map_err(|error| format!("SOCKS5 domain length read failed: {error}"))?;
    if len[0] == 0 {
        let _ = stream.write_all(&socks5_reply(8)).await;
        return Err("SOCKS5 domain name cannot be empty".to_string());
    }
    let mut name = vec![0_u8; len[0] as usize];
    stream
        .read_exact(&mut name)
        .await
        .map_err(|error| format!("SOCKS5 domain read failed: {error}"))?;
    match String::from_utf8(name) {
        Ok(name)
            if !name
                .chars()
                .any(|character| character.is_control() || character.is_whitespace()) =>
        {
            Ok(name)
        }
        Ok(_) => {
            let _ = stream.write_all(&socks5_reply(8)).await;
            Err("SOCKS5 domain name cannot contain control or whitespace characters".to_string())
        }
        Err(_) => {
            let _ = stream.write_all(&socks5_reply(8)).await;
            Err("SOCKS5 domain name is not valid UTF-8".to_string())
        }
    }
}

pub(crate) fn socks5_reply(code: u8) -> [u8; 10] {
    [5, code, 0, 1, 0, 0, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
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

            let error =
                read_socks5_connect_request_with_timeout(&mut server, Duration::from_secs(1))
                    .await
                    .unwrap_err();
            assert!(error.contains("control or whitespace"), "{error}");

            let mut replies = [0_u8; 12];
            client.read_exact(&mut replies).await.unwrap();
            assert_eq!(&replies[..2], &[0x05, 0x00]);
            assert_eq!(&replies[2..], &socks5_reply(8));
        });
    }
}

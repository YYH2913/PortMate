use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use portmate_core::ProxyKind;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

pub(super) const MAX_HTTP_CONNECT_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_HTTP_PROXY_CREDENTIAL_BYTES: usize = 8 * 1024;
const SOCKS5_SERVER_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct ProxyCredentials {
    pub(super) username: String,
    pub(super) password: Zeroizing<String>,
}

pub(super) fn validate_proxy_credentials(
    kind: ProxyKind,
    username: &str,
    password: &str,
) -> Result<(), String> {
    if username.is_empty() {
        return Err("代理用户名不能为空".to_string());
    }
    if password.is_empty() {
        return Err("代理密码不能为空".to_string());
    }
    match kind {
        ProxyKind::HttpConnect => {
            if username
                .bytes()
                .any(|byte| matches!(byte, b':' | b'\r' | b'\n'))
            {
                return Err("HTTP CONNECT 代理用户名不能包含冒号或换行符".to_string());
            }
            if username.len().saturating_add(password.len()) > MAX_HTTP_PROXY_CREDENTIAL_BYTES {
                return Err(format!(
                    "HTTP CONNECT 代理凭据不能超过 {MAX_HTTP_PROXY_CREDENTIAL_BYTES} bytes"
                ));
            }
        }
        ProxyKind::Socks5 => {
            if username.len() > u8::MAX as usize {
                return Err("SOCKS5 代理用户名长度必须为 1-255 bytes".to_string());
            }
            if password.len() > u8::MAX as usize {
                return Err("SOCKS5 代理密码长度必须为 1-255 bytes".to_string());
            }
        }
    }
    Ok(())
}

pub(super) async fn perform_http_connect<S>(
    stream: &mut S,
    authority: &str,
    credentials: Option<&ProxyCredentials>,
    label: &str,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Some(credentials) = credentials {
        validate_proxy_credentials(
            ProxyKind::HttpConnect,
            &credentials.username,
            credentials.password.as_str(),
        )?;
    }
    let authorization = credentials.map(|credentials| {
        let mut raw = Zeroizing::new(Vec::with_capacity(
            credentials.username.len() + credentials.password.len() + 1,
        ));
        raw.extend_from_slice(credentials.username.as_bytes());
        raw.push(b':');
        raw.extend_from_slice(credentials.password.as_bytes());
        Zeroizing::new(BASE64_STANDARD.encode(raw.as_slice()))
    });
    let request = Zeroizing::new(match authorization.as_deref() {
        Some(authorization) => format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic {authorization}\r\nProxy-Connection: Keep-Alive\r\n\r\n"
        ),
        None => format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: Keep-Alive\r\n\r\n"
        ),
    });
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("{label} HTTP CONNECT 请求失败: {error}"))?;

    let mut response = Vec::with_capacity(256);
    let mut byte = [0_u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        if response.len() >= MAX_HTTP_CONNECT_RESPONSE_BYTES {
            return Err(format!(
                "{label} HTTP CONNECT 响应头超过 {} bytes",
                MAX_HTTP_CONNECT_RESPONSE_BYTES
            ));
        }
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|error| format!("{label} HTTP CONNECT 响应读取失败: {error}"))?;
        response.push(byte[0]);
    }
    let response = std::str::from_utf8(&response)
        .map_err(|_| format!("{label} HTTP CONNECT 响应不是有效 ASCII/UTF-8"))?;
    let status_line = response
        .split("\r\n")
        .next()
        .ok_or_else(|| format!("{label} HTTP CONNECT 响应缺少状态行"))?;
    let mut parts = status_line.split_whitespace();
    let version = parts.next().unwrap_or_default();
    let status = parts
        .next()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("{label} HTTP CONNECT 状态行无效: {status_line}"))?;
    if !version.starts_with("HTTP/") {
        return Err(format!("{label} HTTP CONNECT 状态行无效: {status_line}"));
    }
    if !(200..300).contains(&status) {
        return Err(format!("{label} HTTP CONNECT 被代理拒绝: {status_line}"));
    }
    Ok(())
}

pub(super) async fn perform_socks5_connect<S>(
    stream: &mut S,
    target_host: &str,
    target_port: u16,
    credentials: Option<&ProxyCredentials>,
    label: &str,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Some(credentials) = credentials {
        validate_proxy_credentials(
            ProxyKind::Socks5,
            &credentials.username,
            credentials.password.as_str(),
        )?;
    }
    let auth_method = if credentials.is_some() { 0x02 } else { 0x00 };
    stream
        .write_all(&[0x05, 0x01, auth_method])
        .await
        .map_err(|error| format!("{label} SOCKS5 协商请求失败: {error}"))?;
    let mut greeting = [0_u8; 2];
    stream
        .read_exact(&mut greeting)
        .await
        .map_err(|error| format!("{label} SOCKS5 协商响应失败: {error}"))?;
    if greeting[0] != 0x05 {
        return Err(format!("{label} SOCKS5 代理返回未知版本: {}", greeting[0]));
    }
    if greeting[1] == 0xff {
        return Err(format!("{label} SOCKS5 代理没有可接受的认证方式"));
    }
    if greeting[1] != auth_method {
        return Err(format!(
            "{label} SOCKS5 代理选择了未提供的认证方式: 0x{:02x}",
            greeting[1]
        ));
    }
    if let Some(credentials) = credentials {
        let username = credentials.username.as_bytes();
        let password = credentials.password.as_bytes();
        let mut request = Zeroizing::new(Vec::with_capacity(username.len() + password.len() + 3));
        request.extend_from_slice(&[0x01, username.len() as u8]);
        request.extend_from_slice(username);
        request.push(password.len() as u8);
        request.extend_from_slice(password);
        stream
            .write_all(request.as_slice())
            .await
            .map_err(|error| format!("{label} SOCKS5 认证请求失败: {error}"))?;
        let mut response = [0_u8; 2];
        stream
            .read_exact(&mut response)
            .await
            .map_err(|error| format!("{label} SOCKS5 认证响应失败: {error}"))?;
        if response[0] != 0x01 {
            return Err(format!("{label} SOCKS5 认证返回未知版本: {}", response[0]));
        }
        if response[1] != 0x00 {
            return Err(format!("{label} SOCKS5 用户名/密码认证失败"));
        }
    }

    let host = target_host.as_bytes();
    if host.is_empty() || host.len() > u8::MAX as usize {
        return Err(format!("{label} SOCKS5 目标主机长度必须为 1-255 bytes"));
    }
    let mut request = Vec::with_capacity(host.len() + 7);
    request.extend_from_slice(&[0x05, 0x01, 0x00, 0x03, host.len() as u8]);
    request.extend_from_slice(host);
    request.extend_from_slice(&target_port.to_be_bytes());
    stream
        .write_all(&request)
        .await
        .map_err(|error| format!("{label} SOCKS5 CONNECT 请求失败: {error}"))?;

    let mut header = [0_u8; 4];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|error| format!("{label} SOCKS5 CONNECT 响应失败: {error}"))?;
    if header[0] != 0x05 {
        return Err(format!(
            "{label} SOCKS5 CONNECT 返回未知版本: {}",
            header[0]
        ));
    }
    if header[2] != 0x00 {
        return Err(format!(
            "{label} SOCKS5 CONNECT 返回无效保留字段: 0x{:02x}",
            header[2]
        ));
    }
    if header[1] != 0x00 {
        return Err(format!(
            "{label} SOCKS5 CONNECT 被拒绝: {} (0x{:02x})",
            socks5_reply_label(header[1]),
            header[1]
        ));
    }
    let address_len = match header[3] {
        0x01 => 4,
        0x03 => {
            let mut len = [0_u8; 1];
            stream
                .read_exact(&mut len)
                .await
                .map_err(|error| format!("{label} SOCKS5 响应域名长度读取失败: {error}"))?;
            if len[0] == 0 {
                return Err(format!("{label} SOCKS5 CONNECT 返回空绑定域名"));
            }
            usize::from(len[0])
        }
        0x04 => 16,
        other => {
            return Err(format!(
                "{label} SOCKS5 CONNECT 返回未知地址类型: 0x{other:02x}"
            ));
        }
    };
    let mut bound_address_and_port = vec![0_u8; address_len + 2];
    stream
        .read_exact(&mut bound_address_and_port)
        .await
        .map_err(|error| format!("{label} SOCKS5 CONNECT 地址读取失败: {error}"))?;
    Ok(())
}

fn socks5_reply_label(reply: u8) -> &'static str {
    match reply {
        0x01 => "general failure",
        0x02 => "connection not allowed",
        0x03 => "network unreachable",
        0x04 => "host unreachable",
        0x05 => "connection refused",
        0x06 => "TTL expired",
        0x07 => "command not supported",
        0x08 => "address type not supported",
        _ => "unknown failure",
    }
}

pub(super) async fn read_socks5_connect_request<S>(stream: &mut S) -> Result<(String, u16), String>
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
        3 => {
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
                    name
                }
                Ok(_) => {
                    let _ = stream.write_all(&socks5_reply(8)).await;
                    return Err(
                        "SOCKS5 domain name cannot contain control or whitespace characters"
                            .to_string(),
                    );
                }
                Err(_) => {
                    let _ = stream.write_all(&socks5_reply(8)).await;
                    return Err("SOCKS5 domain name is not valid UTF-8".to_string());
                }
            }
        }
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

pub(super) fn socks5_reply(code: u8) -> [u8; 10] {
    [5, code, 0, 1, 0, 0, 0, 0, 0, 0]
}

#[cfg(test)]
mod tests {
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
            let error =
                handshake_with_socks5_reply(vec![0x05, 0x00, 0x01, 0x01, 127, 0, 0, 1, 0, 0])
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

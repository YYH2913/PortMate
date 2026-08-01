use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use portmate_core::{ProxyConfig, ProxyKind};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use zeroize::Zeroizing;

use super::{canonical_secret_ref, read_secret_from_store};

mod socks5_server;

pub(super) use socks5_server::{read_socks5_connect_request, socks5_reply};

pub(super) const MAX_HTTP_CONNECT_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_HTTP_PROXY_CREDENTIAL_BYTES: usize = 8 * 1024;

pub(super) struct ProxyCredentials {
    pub(super) username: String,
    pub(super) password: Zeroizing<String>,
}

pub(super) fn proxy_target_authority(host: &str, port: u16) -> Result<String, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("代理目标主机不能为空".to_string());
    }
    if host.bytes().any(|byte| byte == b'\r' || byte == b'\n') {
        return Err("代理目标主机不能包含换行符".to_string());
    }
    if host.parse::<std::net::Ipv6Addr>().is_ok() {
        Ok(format!("[{host}]:{port}"))
    } else {
        Ok(format!("{host}:{port}"))
    }
}

pub(super) fn normalized_enabled_proxy(proxy: &ProxyConfig) -> Result<Option<ProxyConfig>, String> {
    if !proxy.enabled {
        return Ok(None);
    }
    let mut proxy = proxy.clone();
    proxy.normalize();
    if proxy.host.is_empty() {
        return Err("代理主机不能为空".to_string());
    }
    if proxy
        .host
        .bytes()
        .any(|byte| byte == b'\r' || byte == b'\n')
    {
        return Err("代理主机不能包含换行符".to_string());
    }
    if proxy.port == 0 {
        return Err("代理端口必须在 1-65535 之间".to_string());
    }
    if let Some(secret_ref) = proxy.password_secret_ref.as_deref() {
        if canonical_secret_ref(secret_ref).is_none() {
            return Err("代理密码 secretRef 无效".to_string());
        }
        if proxy.username.is_empty() {
            return Err("已保存代理密码时，代理用户名不能为空".to_string());
        }
    }
    Ok(Some(proxy))
}

pub(super) fn resolve_proxy_credentials_with<ReadSecret>(
    proxy: &ProxyConfig,
    mut read_secret: ReadSecret,
) -> Result<Option<ProxyCredentials>, String>
where
    ReadSecret: FnMut(&str) -> Result<String, String>,
{
    let Some(secret_ref) = proxy.password_secret_ref.as_deref() else {
        return Ok(None);
    };
    if proxy.username.is_empty() {
        return Err("已保存代理密码时，代理用户名不能为空".to_string());
    }
    let password = Zeroizing::new(
        read_secret(secret_ref).map_err(|error| format!("代理密码读取失败: {error}"))?,
    );
    validate_proxy_credentials(proxy.kind, &proxy.username, password.as_str())?;
    Ok(Some(ProxyCredentials {
        username: proxy.username.clone(),
        password,
    }))
}

pub(super) async fn connect_target_stream(
    target_host: &str,
    target_port: u16,
    proxy: &ProxyConfig,
    label: &str,
) -> Result<TcpStream, String> {
    let authority = proxy_target_authority(target_host, target_port)?;
    let Some(proxy) = normalized_enabled_proxy(proxy)? else {
        return TcpStream::connect((target_host, target_port))
            .await
            .map_err(|error| format!("{label} 连接失败 {authority}: {error}"));
    };
    let credentials = resolve_proxy_credentials_with(&proxy, read_secret_from_store)?;
    let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port))
        .await
        .map_err(|error| {
            format!(
                "{label} 代理连接失败 {}:{}: {error}",
                proxy.host, proxy.port
            )
        })?;
    match proxy.kind {
        ProxyKind::HttpConnect => {
            perform_http_connect(&mut stream, &authority, credentials.as_ref(), label).await?;
        }
        ProxyKind::Socks5 => {
            perform_socks5_connect(
                &mut stream,
                target_host.trim(),
                target_port,
                credentials.as_ref(),
                label,
            )
            .await?;
        }
    }
    Ok(stream)
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
}

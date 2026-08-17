use super::*;
use std::fs::File;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::net::UdpSocket;

const TFTP_OPCODE_RRQ: u16 = 1;
const TFTP_OPCODE_DATA: u16 = 3;
const TFTP_OPCODE_ACK: u16 = 4;
const TFTP_OPCODE_ERROR: u16 = 5;
const TFTP_OPCODE_OACK: u16 = 6;
const TFTP_DEFAULT_BLOCK_SIZE: usize = 512;
const TFTP_MAX_BLOCK_SIZE: usize = 1_468;
const TFTP_MAX_PACKET_SIZE: usize = 65_535;
const TFTP_RETRY_COUNT: usize = 5;
const TFTP_RETRY_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq)]
struct TftpReadRequest {
    file_name: String,
    options: Vec<(String, String)>,
}

struct TftpNegotiation {
    block_size: usize,
    retry_timeout: Duration,
    option_ack: Option<Vec<u8>>,
}

pub(super) async fn transfer_file_via_tftp(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let spec = parse_tftp_receiver_endpoint(&request.destination)?
        .ok_or_else(|| "TFTP 传输必须使用 load:tftpboot 接收端点".to_string())?;
    if has_remote_transfer_prefix(&request.source) {
        return Err("TFTP 一次性服务仅支持 PortMate 本机文件".to_string());
    }
    let source_path = Path::new(&request.source);
    if local_transfer_entry(source_path, "TFTP 本地传输源")?.is_none() {
        return Err("TFTP 本地传输源不存在".to_string());
    }
    let file_name = match spec.file_name.as_deref() {
        Some(file_name) => file_name.to_string(),
        None => source_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "TFTP 本地传输源缺少可用文件名".to_string())?
            .to_string(),
    };
    validate_tftp_file_name(&file_name)?;

    let server_ip = resolve_tftp_server_ip(&spec).await?;
    let bind_host = spec.bind_host.unwrap_or(server_ip);
    let socket = UdpSocket::bind((bind_host, spec.bind_port))
        .await
        .map_err(|error| {
            let privilege_hint = if spec.bind_port != 0 && spec.bind_port < 1024 {
                "；低端口可能需要管理员权限，可改用 bindPort=0 或大于 1023 的端口"
            } else {
                ""
            };
            format!(
                "无法在 {bind_host}:{} 启动一次性 TFTP 服务: {error}{privilege_hint}",
                spec.bind_port
            )
        })?;
    let server_port = socket
        .local_addr()
        .map_err(|error| format!("无法读取 TFTP 服务监听地址: {error}"))?
        .port();
    let mut source = File::open(source_path)
        .map_err(|error| format!("无法打开 TFTP 本地传输源: {error}"))?;
    let total = source
        .metadata()
        .map_err(|error| format!("无法读取 TFTP 本地传输源信息: {error}"))?
        .len();

    let binding = transfer_modem_binding(state, &request.session_id, progress).await?;
    let commands = spec.command_lines(&file_name, server_ip, server_port)?;
    binding
        .write_runtime_bytes(state, commands.as_bytes())
        .await
        .map_err(|error| format!("启动 U-Boot TFTP 命令失败: {error}"))?;

    let deadline = Instant::now() + spec.timeout;
    serve_tftp_file(
        &socket,
        &mut source,
        &file_name,
        spec.device_ip,
        total,
        deadline,
        &binding,
        progress,
    )
    .await
}

async fn resolve_tftp_server_ip(spec: &TftpReceiverSpec) -> Result<Ipv4Addr, String> {
    if let Some(server_ip) = spec.server_ip {
        return Ok(server_ip);
    }
    if let Some(bind_host) = spec.bind_host.filter(|host| !host.is_unspecified()) {
        return Ok(bind_host);
    }
    let probe = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(|error| format!("无法创建 TFTP 路由探测 socket: {error}"))?;
    probe
        .connect((spec.device_ip, DEFAULT_TFTP_PORT))
        .await
        .map_err(|error| format!("无法推断到设备 {} 的本机路由: {error}", spec.device_ip))?;
    match probe
        .local_addr()
        .map_err(|error| format!("无法读取 TFTP 路由探测结果: {error}"))?
        .ip()
    {
        IpAddr::V4(address) if !address.is_unspecified() => Ok(address),
        _ => Err("无法自动推断 TFTP serverIp，请显式指定 serverIp".to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
async fn serve_tftp_file(
    socket: &UdpSocket,
    source: &mut File,
    file_name: &str,
    device_ip: Ipv4Addr,
    total: u64,
    deadline: Instant,
    binding: &ModemRuntimeBinding,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (read_request, peer) = wait_for_tftp_read_request(
        socket,
        file_name,
        device_ip,
        deadline,
        binding,
        progress,
    )
    .await?;
    let negotiation = negotiate_tftp_options(&read_request.options, total)?;
    if let Some(option_ack) = negotiation.option_ack.as_deref() {
        send_tftp_packet_with_ack(
            socket,
            peer,
            option_ack,
            0,
            negotiation.retry_timeout,
            deadline,
            binding,
            progress,
        )
        .await?;
    }

    let mut block = 1_u16;
    let mut bytes_done = 0_u64;
    loop {
        let remaining = total.saturating_sub(bytes_done);
        let read_length = remaining.min(negotiation.block_size as u64) as usize;
        let mut data = vec![0_u8; read_length];
        source
            .read_exact(&mut data)
            .map_err(|error| format!("读取 TFTP 本地传输源失败: {error}"))?;
        let mut packet = Vec::with_capacity(4 + data.len());
        packet.extend_from_slice(&TFTP_OPCODE_DATA.to_be_bytes());
        packet.extend_from_slice(&block.to_be_bytes());
        packet.extend_from_slice(&data);
        send_tftp_packet_with_ack(
            socket,
            peer,
            &packet,
            block,
            negotiation.retry_timeout,
            deadline,
            binding,
            progress,
        )
        .await?;
        bytes_done = bytes_done.saturating_add(data.len() as u64);
        progress.update(bytes_done, total).await?;
        if data.len() < negotiation.block_size {
            return Ok(bytes_done);
        }
        block = block.wrapping_add(1);
    }
}

async fn wait_for_tftp_read_request(
    socket: &UdpSocket,
    expected_file_name: &str,
    device_ip: Ipv4Addr,
    deadline: Instant,
    binding: &ModemRuntimeBinding,
    progress: &TransferProgressContext,
) -> Result<(TftpReadRequest, SocketAddr), String> {
    let mut packet = vec![0_u8; TFTP_MAX_PACKET_SIZE];
    loop {
        let (size, peer) = receive_tftp_packet(socket, &mut packet, deadline, binding, progress)
            .await?;
        if peer.ip() != IpAddr::V4(device_ip) {
            continue;
        }
        let request = match parse_tftp_read_request(&packet[..size]) {
            Ok(request) => request,
            Err(error) => {
                send_tftp_error(socket, peer, 4, &error).await;
                return Err(error);
            }
        };
        if request.file_name != expected_file_name {
            let error = format!(
                "设备请求了未授权的 TFTP 文件 `{}`，预期 `{expected_file_name}`",
                request.file_name
            );
            send_tftp_error(socket, peer, 1, "file not found").await;
            return Err(error);
        }
        return Ok((request, peer));
    }
}

fn parse_tftp_read_request(packet: &[u8]) -> Result<TftpReadRequest, String> {
    if packet.len() < 4 || u16::from_be_bytes([packet[0], packet[1]]) != TFTP_OPCODE_RRQ {
        return Err("收到的 TFTP 数据包不是 RRQ".to_string());
    }
    if packet.last() != Some(&0) {
        return Err("TFTP RRQ 缺少终止符".to_string());
    }
    let fields = packet[2..]
        .split(|byte| *byte == 0)
        .collect::<Vec<_>>();
    if fields.len() < 3 || fields[0].is_empty() || fields[1].is_empty() {
        return Err("TFTP RRQ 缺少文件名或传输模式".to_string());
    }
    let file_name = std::str::from_utf8(fields[0])
        .map_err(|_| "TFTP RRQ 文件名不是 UTF-8".to_string())?
        .to_string();
    validate_tftp_file_name(&file_name)?;
    let mode = std::str::from_utf8(fields[1])
        .map_err(|_| "TFTP RRQ 模式不是 UTF-8".to_string())?;
    if !mode.eq_ignore_ascii_case("octet") {
        return Err("TFTP 仅支持 octet 二进制模式".to_string());
    }
    let option_fields = &fields[2..fields.len() - 1];
    if option_fields.len() % 2 != 0 {
        return Err("TFTP RRQ 选项必须成对出现".to_string());
    }
    let mut options = Vec::with_capacity(option_fields.len() / 2);
    for pair in option_fields.chunks_exact(2) {
        let name = std::str::from_utf8(pair[0])
            .map_err(|_| "TFTP RRQ 选项名不是 UTF-8".to_string())?
            .to_ascii_lowercase();
        let value = std::str::from_utf8(pair[1])
            .map_err(|_| "TFTP RRQ 选项值不是 UTF-8".to_string())?
            .to_string();
        if name.is_empty() || value.is_empty() {
            return Err("TFTP RRQ 包含空选项".to_string());
        }
        options.push((name, value));
    }
    Ok(TftpReadRequest { file_name, options })
}

fn negotiate_tftp_options(
    requested: &[(String, String)],
    total: u64,
) -> Result<TftpNegotiation, String> {
    let mut block_size = TFTP_DEFAULT_BLOCK_SIZE;
    let mut retry_timeout = TFTP_RETRY_TIMEOUT;
    let mut accepted = Vec::new();
    for (name, value) in requested {
        match name.as_str() {
            "blksize" => {
                let requested = value
                    .parse::<usize>()
                    .map_err(|_| "TFTP blksize 选项不是整数".to_string())?;
                if requested < 8 {
                    return Err("TFTP blksize 不能小于 8".to_string());
                }
                block_size = requested.min(TFTP_MAX_BLOCK_SIZE);
                accepted.push((name.as_str(), block_size.to_string()));
            }
            "tsize" if value == "0" => {
                accepted.push((name.as_str(), total.to_string()));
            }
            "timeout" => {
                let requested = value
                    .parse::<u64>()
                    .map_err(|_| "TFTP timeout 选项不是整数".to_string())?;
                if requested == 0 || requested > u8::MAX as u64 {
                    return Err("TFTP timeout 选项必须介于 1 和 255 之间".to_string());
                }
                let seconds = requested.min(5);
                retry_timeout = Duration::from_secs(seconds);
                accepted.push((name.as_str(), seconds.to_string()));
            }
            _ => {}
        }
    }
    let option_ack = if accepted.is_empty() {
        None
    } else {
        let mut packet = TFTP_OPCODE_OACK.to_be_bytes().to_vec();
        for (name, value) in accepted {
            packet.extend_from_slice(name.as_bytes());
            packet.push(0);
            packet.extend_from_slice(value.as_bytes());
            packet.push(0);
        }
        Some(packet)
    };
    Ok(TftpNegotiation {
        block_size,
        retry_timeout,
        option_ack,
    })
}

#[allow(clippy::too_many_arguments)]
async fn send_tftp_packet_with_ack(
    socket: &UdpSocket,
    peer: SocketAddr,
    packet: &[u8],
    expected_block: u16,
    retry_timeout: Duration,
    deadline: Instant,
    binding: &ModemRuntimeBinding,
    progress: &TransferProgressContext,
) -> Result<(), String> {
    let mut response = vec![0_u8; TFTP_MAX_PACKET_SIZE];
    for _ in 0..TFTP_RETRY_COUNT {
        progress.check_cancelled()?;
        binding.ensure_current()?;
        socket
            .send_to(packet, peer)
            .await
            .map_err(|error| format!("发送 TFTP 数据包失败: {error}"))?;
        let attempt_deadline = deadline.min(Instant::now() + retry_timeout);
        loop {
            match receive_tftp_packet(
                socket,
                &mut response,
                attempt_deadline,
                binding,
                progress,
            )
            .await
            {
                Ok((size, sender)) if sender == peer && size >= 4 => {
                    let opcode = u16::from_be_bytes([response[0], response[1]]);
                    let block = u16::from_be_bytes([response[2], response[3]]);
                    if opcode == TFTP_OPCODE_ACK && block == expected_block {
                        return Ok(());
                    }
                    if opcode == TFTP_OPCODE_ERROR {
                        let message = response[4..size]
                            .split(|byte| *byte == 0)
                            .next()
                            .map(String::from_utf8_lossy)
                            .unwrap_or_default();
                        return Err(format!("设备返回 TFTP 错误 {block}: {message}"));
                    }
                }
                Ok(_) => {}
                Err(_) if Instant::now() >= attempt_deadline && Instant::now() < deadline => break,
                Err(error) => return Err(error),
            }
        }
    }
    Err(format!(
        "等待设备确认 TFTP 块 {expected_block} 超时，已重试 {TFTP_RETRY_COUNT} 次"
    ))
}

async fn receive_tftp_packet(
    socket: &UdpSocket,
    buffer: &mut [u8],
    deadline: Instant,
    binding: &ModemRuntimeBinding,
    progress: &TransferProgressContext,
) -> Result<(usize, SocketAddr), String> {
    loop {
        progress.check_cancelled()?;
        binding.ensure_current()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("等待 TFTP 设备请求或确认超时".to_string());
        }
        match tokio::time::timeout(
            remaining.min(TRANSFER_CANCEL_POLL_INTERVAL),
            socket.recv_from(buffer),
        )
        .await
        {
            Ok(Ok(packet)) => return Ok(packet),
            Ok(Err(error)) => return Err(format!("接收 TFTP 数据包失败: {error}")),
            Err(_) => {}
        }
    }
}

async fn send_tftp_error(socket: &UdpSocket, peer: SocketAddr, code: u16, message: &str) {
    let mut packet = TFTP_OPCODE_ERROR.to_be_bytes().to_vec();
    packet.extend_from_slice(&code.to_be_bytes());
    packet.extend_from_slice(message.as_bytes());
    packet.push(0);
    let _ = socket.send_to(&packet, peer).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_binary_rrq_and_negotiates_bounded_options() {
        let request = parse_tftp_read_request(
            b"\x00\x01firmware.bin\x00octet\x00blksize\x0065464\x00tsize\x000\x00timeout\x009\x00",
        )
        .unwrap();
        assert_eq!(request.file_name, "firmware.bin");
        let negotiation = negotiate_tftp_options(&request.options, 4_096).unwrap();
        assert_eq!(negotiation.block_size, TFTP_MAX_BLOCK_SIZE);
        assert_eq!(negotiation.retry_timeout, Duration::from_secs(5));
        let option_ack = negotiation.option_ack.unwrap();
        assert!(option_ack
            .windows(b"blksize\x001468\x00".len())
            .any(|window| window == b"blksize\x001468\x00"));
        assert!(option_ack
            .windows(b"tsize\x004096\x00".len())
            .any(|window| window == b"tsize\x004096\x00"));
    }

    #[test]
    fn rejects_command_injection_in_rrq_file_names() {
        assert!(parse_tftp_read_request(b"\x00\x01fw.bin;saveenv\x00octet\x00").is_err());
        assert!(parse_tftp_read_request(b"\x00\x01../fw.bin\x00octet\x00").is_err());
        assert!(parse_tftp_read_request(b"\x00\x01fw.bin\x00netascii\x00").is_err());
    }
}

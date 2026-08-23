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
const TFTP_ACK_BUFFER_SIZE: usize = 4 + TFTP_MAX_BLOCK_SIZE + 512;
// U-Boot's default tftp timeout is 5 seconds. Matching it avoids premature
// retransmits on older boot ROMs that do not negotiate a timeout option.
const TFTP_RETRY_TIMEOUT: Duration = Duration::from_secs(5);
// The AXON 1.7 and downstream U-Boot consoles echo a complete command before
// accepting the next one. At 115200 baud, 80 ms still lets the following line
// overrun the command parser and corrupts `tftpdstp`/`tftpboot`.
const TFTP_COMMAND_LINE_DELAY: Duration = Duration::from_millis(500);

#[derive(Debug, PartialEq, Eq)]
struct TftpReadRequest {
    file_name: String,
    options: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TftpRequestError {
    code: u16,
    message: String,
}

impl TftpRequestError {
    fn illegal(message: impl Into<String>) -> Self {
        Self { code: 4, message: message.into() }
    }

    fn file(message: impl Into<String>) -> Self {
        Self { code: 1, message: message.into() }
    }

    fn option(message: impl Into<String>) -> Self {
        Self { code: 8, message: message.into() }
    }
}

impl std::fmt::Display for TftpRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

struct TftpNegotiation {
    block_size: usize,
    retry_timeout: Duration,
    option_ack: Option<Vec<u8>>,
}

struct TftpSocketBinding {
    socket: UdpSocket,
    port: u16,
    fallback_from_default: bool,
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
    let TftpSocketBinding {
        socket,
        port: server_port,
        fallback_from_default,
    } = bind_tftp_socket(bind_host, spec.bind_port).await?;
    if fallback_from_default {
        eprintln!(
            "PortMate: TFTP default port 69 was unavailable on {bind_host}; using {server_port} with an explicit U-Boot server-port argument"
        );
    }
    let (mut source, total) = open_local_transfer_source(source_path, "TFTP 本地传输源")?;

    let binding = transfer_modem_binding(state, &request.session_id, progress).await?;
    // Hold the session outbound lane for the complete one-shot transfer. A
    // queued interactive write must never land between U-Boot setup lines or
    // while the serial console is carrying the TFTP command.
    let io = state.session_io();
    let outbound_lane = acquire_tftp_outbound_lane(
        &io.store_path,
        &request.session_id,
        progress,
    )
    .await?;
    let commands = spec.command_lines(&file_name, server_ip, server_port)?;
    // The original U-Boot serial console can overrun when the complete command
    // sequence is written as one 115200-baud burst. Send each command line
    // separately so the console has time to consume the preceding CR.
    let command_lines = split_tftp_command_lines(&commands);
    let setup_result = async {
        for (index, line) in command_lines.iter().enumerate() {
            progress.check_cancelled()?;
            binding.ensure_current()?;
            write_runtime_bytes_for_runtime_with_lane(
                state,
                &request.session_id,
                line.as_bytes(),
                Some(binding.runtime_id()),
                &outbound_lane,
            )
            .await
            .map_err(|error| format!("启动 U-Boot TFTP 命令失败: {error}"))?;
            if index + 1 < command_lines.len() {
                sleep_tftp_with_cancellation(progress, TFTP_COMMAND_LINE_DELAY).await?;
            }
        }
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = setup_result {
        let _ = write_runtime_bytes_for_runtime_with_lane(
            state,
            &request.session_id,
            b"\x03",
            Some(binding.runtime_id()),
            &outbound_lane,
        )
        .await;
        // Preserve the canonical cancellation string so the transfer runner
        // records a cancelled task instead of misclassifying it as a failure.
        return Err(error);
    }

    let deadline = Instant::now()
        .checked_add(spec.timeout)
        .ok_or_else(|| "TFTP 总超时超出当前平台可表示的时间范围".to_string())?;
    let result = serve_tftp_file(
        &socket,
        &mut source,
        &file_name,
        spec.device_ip,
        total,
        deadline,
        &binding,
        progress,
    )
    .await;
    if result.is_err() {
        // Leave U-Boot's network command and return it to its prompt before
        // the next transfer. TFTP has no portable cancel packet.
        let _ = write_runtime_bytes_for_runtime_with_lane(
            state,
            &request.session_id,
            b"\x03",
            Some(binding.runtime_id()),
            &outbound_lane,
        )
        .await;
    }
    result.map_err(|error| {
        if error == TRANSFER_CANCELLED_MESSAGE {
            return error;
        }
        let fallback_note = if fallback_from_default {
            "；端口 69 不可用，已使用高端口；目标 U-Boot 必须支持 `server:port:file` 语法或 CONFIG_TFTP_PORT/tftpdstp"
        } else {
            ""
        };
        format!(
            "{error}（TFTP 服务 {bind_host}:{server_port}，仅接受来自设备 {} 的请求{fallback_note}）",
            spec.device_ip,
        )
    })
}

async fn sleep_tftp_with_cancellation(
    progress: &TransferProgressContext,
    duration: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        progress.check_cancelled()?;
        let remaining = duration.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Ok(());
        }
        tokio::time::sleep(remaining.min(TRANSFER_CANCEL_POLL_INTERVAL)).await;
    }
}

async fn acquire_tftp_outbound_lane(
    store_path: &Path,
    session_id: &str,
    progress: &TransferProgressContext,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    loop {
        progress.check_cancelled()?;
        match acquire_outbound_lane_with_timeout(
            store_path,
            session_id,
            TRANSFER_CANCEL_POLL_INTERVAL,
        )
        .await
        {
            Ok(guard) => return Ok(guard),
            Err(error) if error.contains("出站队列等待超时") => continue,
            Err(error) => return Err(error),
        }
    }
}

async fn bind_tftp_socket(
    bind_host: Ipv4Addr,
    requested_port: u16,
) -> Result<TftpSocketBinding, String> {
    match UdpSocket::bind((bind_host, requested_port)).await {
        Ok(socket) => {
            let port = socket
                .local_addr()
                .map_err(|error| format!("无法读取 TFTP 服务监听地址: {error}"))?
                .port();
            Ok(TftpSocketBinding {
                socket,
                port,
                fallback_from_default: false,
            })
        }
        Err(error)
            if requested_port == DEFAULT_TFTP_PORT
                && matches!(
                    error.kind(),
                    std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
                ) =>
        {
            let fallback = UdpSocket::bind((bind_host, 0)).await.map_err(|fallback_error| {
                format!(
                    "无法在 {bind_host}:69 启动一次性 TFTP 服务: {error}；自动选择高端口也失败: {fallback_error}"
                )
            })?;
            let port = fallback
                .local_addr()
                .map_err(|fallback_error| {
                    format!("无法读取自动选择的 TFTP 服务监听地址: {fallback_error}")
                })?
                .port();
            Ok(TftpSocketBinding {
                socket: fallback,
                port,
                fallback_from_default: true,
            })
        }
        Err(error) => {
            let privilege_hint = if requested_port != 0 && requested_port < 1024 {
                "；低端口可能需要管理员权限，可改用 bindPort=0 或大于 1023 的端口"
            } else {
                ""
            };
            Err(format!(
                "无法在 {bind_host}:{requested_port} 启动一次性 TFTP 服务: {error}{privilege_hint}"
            ))
        }
    }
}

pub(super) fn split_tftp_command_lines(commands: &str) -> Vec<&str> {
    commands
        .split_inclusive('\r')
        .filter(|line| !line.is_empty())
        .collect()
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
    let negotiation = match negotiate_tftp_options(&read_request.options, total) {
        Ok(negotiation) => negotiation,
        Err(error) => {
            send_tftp_error(socket, peer, 8, &error).await;
            return Err(error);
        }
    };
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
    let mut data = vec![0_u8; negotiation.block_size];
    let mut packet = Vec::with_capacity(4 + negotiation.block_size);
    loop {
        let remaining = total.saturating_sub(bytes_done);
        let read_length = remaining.min(negotiation.block_size as u64) as usize;
        data.resize(read_length, 0);
        source
            .read_exact(&mut data)
            .map_err(|error| format!("读取 TFTP 本地传输源失败: {error}"))?;
        packet.clear();
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
                send_tftp_error(socket, peer, error.code, &error.message).await;
                return Err(error.message);
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

fn parse_tftp_read_request(packet: &[u8]) -> Result<TftpReadRequest, TftpRequestError> {
    if packet.len() < 4 || u16::from_be_bytes([packet[0], packet[1]]) != TFTP_OPCODE_RRQ {
        return Err(TftpRequestError::illegal("收到的 TFTP 数据包不是 RRQ"));
    }

    let (file_bytes, remaining) = take_tftp_field(&packet[2..])
        .ok_or_else(|| TftpRequestError::illegal("TFTP RRQ 缺少文件名终止符"))?;
    let (mode_bytes, mut option_bytes) = take_tftp_field(remaining)
        .ok_or_else(|| TftpRequestError::illegal("TFTP RRQ 缺少传输模式终止符"))?;
    let request_has_terminal_nul = packet.last() == Some(&0);
    if file_bytes.is_empty() || mode_bytes.is_empty() {
        return Err(TftpRequestError::illegal("TFTP RRQ 缺少文件名或传输模式"));
    }
    let file_name = std::str::from_utf8(file_bytes)
        .map_err(|_| TftpRequestError::file("TFTP RRQ 文件名不是 UTF-8"))?
        .to_string();
    validate_tftp_file_name(&file_name).map_err(TftpRequestError::file)?;
    let mode = std::str::from_utf8(mode_bytes)
        .map_err(|_| TftpRequestError::illegal("TFTP RRQ 模式不是 UTF-8"))?;
    if !mode.eq_ignore_ascii_case("octet") {
        return Err(TftpRequestError::illegal("TFTP 仅支持 octet 二进制模式"));
    }

    // Some boot ROMs send a fixed-size 516-byte RRQ and leave arbitrary
    // non-NUL padding after the mode. Parse complete NUL-terminated option
    // fields and ignore a bounded incomplete tail as compatibility padding.
    let mut option_fields = Vec::new();
    while let Some(separator) = option_bytes.iter().position(|byte| *byte == 0) {
        let field = &option_bytes[..separator];
        option_bytes = &option_bytes[separator + 1..];
        if field.is_empty() {
            break;
        }
        option_fields.push(field);
    }
    if option_fields.len() % 2 != 0 && request_has_terminal_nul {
        return Err(TftpRequestError::option("TFTP RRQ 选项必须成对出现"));
    }
    let mut options = Vec::with_capacity(option_fields.len() / 2);
    for pair in option_fields.chunks_exact(2) {
        let name = std::str::from_utf8(pair[0])
            .map_err(|_| TftpRequestError::option("TFTP RRQ 选项名不是 UTF-8"))?
            .to_ascii_lowercase();
        let value = std::str::from_utf8(pair[1])
            .map_err(|_| TftpRequestError::option("TFTP RRQ 选项值不是 UTF-8"))?
            .to_string();
        if name.is_empty() || value.is_empty() {
            return Err(TftpRequestError::option("TFTP RRQ 包含空选项"));
        }
        options.push((name, value));
    }
    Ok(TftpReadRequest { file_name, options })
}

fn take_tftp_field(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let separator = bytes.iter().position(|byte| *byte == 0)?;
    Some((&bytes[..separator], &bytes[separator + 1..]))
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
                // U-Boot validates that the OACK echoes its requested timeout
                // exactly. Do not clamp this value to an application-local
                // retry policy.
                let seconds = requested;
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
    // ACK/OACK/ERROR packets are small. Avoid allocating a full 64 KiB UDP
    // buffer for every data block while retaining room for negotiated options.
    let mut response = vec![0_u8; TFTP_ACK_BUFFER_SIZE];
    let mut attempts = 0_u32;
    loop {
        attempts = attempts.saturating_add(1);
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
        if Instant::now() >= deadline {
            return Err(format!(
                "等待设备确认 TFTP 块 {expected_block} 超时，已重试 {attempts} 次"
            ));
        }
    }
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
    fn default_port_binding_has_an_unprivileged_fallback() {
        tauri::async_runtime::block_on(async {
            let blocker = UdpSocket::bind((Ipv4Addr::LOCALHOST, DEFAULT_TFTP_PORT))
                .await
                .ok();
            let binding = bind_tftp_socket(Ipv4Addr::LOCALHOST, DEFAULT_TFTP_PORT)
                .await
                .expect("TFTP should fall back when port 69 cannot be used");
            if blocker.is_some() {
                assert_ne!(binding.port, DEFAULT_TFTP_PORT);
            }
            drop(binding);
            drop(blocker);
        });
    }

    #[test]
    fn parses_binary_rrq_and_negotiates_bounded_options() {
        let request = parse_tftp_read_request(
            b"\x00\x01firmware.bin\x00octet\x00blksize\x0065464\x00tsize\x000\x00timeout\x009\x00",
        )
        .unwrap();
        assert_eq!(request.file_name, "firmware.bin");
        let negotiation = negotiate_tftp_options(&request.options, 4_096).unwrap();
        assert_eq!(negotiation.block_size, TFTP_MAX_BLOCK_SIZE);
        assert_eq!(negotiation.retry_timeout, Duration::from_secs(9));
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
        let error = parse_tftp_read_request(b"\x00\x01fw.bin;saveenv\x00octet\x00")
            .expect_err("unsafe filenames must be rejected");
        assert_eq!(error.code, 1);
        assert!(parse_tftp_read_request(b"\x00\x01../fw.bin\x00octet\x00").is_err());
        assert!(parse_tftp_read_request(b"\x00\x01fw.bin\x00netascii\x00").is_err());
    }

    #[test]
    fn accepts_fixed_length_rrq_padding_after_mode() {
        let mut packet = b"\x00\x01firmware.bin\x00octet\x00".to_vec();
        packet.resize(516, 0xa5);
        let request = parse_tftp_read_request(&packet).expect("padded RRQ should remain compatible");
        assert_eq!(request.file_name, "firmware.bin");
        assert!(request.options.is_empty());
    }

    #[test]
    fn keeps_complete_options_before_fixed_length_rrq_padding() {
        let mut packet = b"\x00\x01firmware.bin\x00octet\x00blksize\x001024\x00".to_vec();
        packet.resize(516, 0xa5);
        let request = parse_tftp_read_request(&packet).expect("valid options should survive padding");
        assert_eq!(request.options, vec![("blksize".to_string(), "1024".to_string())]);
    }

    #[test]
    fn malformed_rrq_uses_illegal_operation_error_code() {
        let error = parse_tftp_read_request(b"\x00\x02firmware.bin\x00octet\x00")
            .expect_err("WRQ is not a supported RRQ");
        assert_eq!(error.code, 4);
    }
}

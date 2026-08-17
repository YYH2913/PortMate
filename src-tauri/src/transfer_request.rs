use super::*;
use std::net::Ipv4Addr;

pub(super) const DEFAULT_TFTP_PORT: u16 = 69;
pub(super) const DEFAULT_TFTP_TIMEOUT_SECONDS: u64 = 60;
pub(super) const MAX_TFTP_TIMEOUT_SECONDS: u64 = 150;

pub(super) fn prepare_transfer_request(
    profile: &SessionProfile,
    request: StartTransferRequest,
) -> Result<StartTransferRequest, String> {
    let home = native_home_path();
    prepare_transfer_request_with_home(
        profile,
        request,
        current_local_transfer_path_platform(),
        home.as_deref(),
    )
}

pub(super) fn prepare_transfer_request_with_home(
    profile: &SessionProfile,
    mut request: StartTransferRequest,
    platform: LocalTransferPathPlatform,
    home: Option<&Path>,
) -> Result<StartTransferRequest, String> {
    if has_load_receiver_prefix(&request.source) {
        return Err("load: 设备接收端点只能作为 Modem 上传目标".to_string());
    }
    let load_receiver = validate_load_receiver_endpoint(&request.destination, &request.protocol)?;
    if load_receiver && has_remote_transfer_prefix(&request.source) {
        return Err("load: 设备接收端点只支持从 PortMate 本机文件上传".to_string());
    }
    let accesses_remote = is_nonlocal_transfer_endpoint(&request.source)
        || is_nonlocal_transfer_endpoint(&request.destination);
    validate_transfer_protocol(profile, &request.protocol, accesses_remote)?;
    if let Some(path) = remote_path(&request.source) {
        validate_remote_transfer_path(path, "远端传输源路径")?;
    }
    if let Some(path) = remote_path(&request.destination) {
        validate_remote_transfer_path(path, "远端传输目标路径")?;
    }
    let default_local_dir = resolve_transfer_default_local_dir_with_home(
        profile.transfer.default_local_dir.as_deref(),
        platform,
        home,
    )?;

    request.source = resolve_default_local_transfer_path_with_home(
        &request.source,
        default_local_dir.as_deref(),
        platform,
        home,
    )?;
    request.destination = resolve_default_local_transfer_path_with_home(
        &request.destination,
        default_local_dir.as_deref(),
        platform,
        home,
    )?;
    Ok(request)
}

pub(super) fn validate_transfer_protocol(
    profile: &SessionProfile,
    protocol: &TransferProtocol,
    accesses_remote: bool,
) -> Result<(), String> {
    if !accesses_remote && matches!(protocol, TransferProtocol::Sftp | TransferProtocol::Scp) {
        return Ok(());
    }
    let ssh_like = matches!(profile.kind, SessionKind::Ssh | SessionKind::Tmux);
    if accesses_remote
        && matches!(protocol, TransferProtocol::Sftp | TransferProtocol::Scp)
        && !ssh_like
    {
        return Err(format!(
            "{} 仅支持 SSH/Tmux 会话",
            transfer_protocol_label(protocol)
        ));
    }

    let enabled = match protocol {
        TransferProtocol::Sftp => profile.transfer.sftp,
        TransferProtocol::Scp => profile.transfer.scp,
        TransferProtocol::Tftp => profile.transfer.tftp,
        TransferProtocol::Xmodem => profile.transfer.xmodem,
        TransferProtocol::Ymodem => profile.transfer.ymodem,
        TransferProtocol::Zmodem => profile.transfer.zmodem,
    };
    if enabled {
        Ok(())
    } else {
        Err(format!(
            "{} 已在 Profile 传输设置中禁用",
            transfer_protocol_label(protocol)
        ))
    }
}

pub(super) fn transfer_protocol_label(protocol: &TransferProtocol) -> &'static str {
    match protocol {
        TransferProtocol::Sftp => "SFTP",
        TransferProtocol::Scp => "SCP",
        TransferProtocol::Tftp => "TFTP",
        TransferProtocol::Xmodem => "XModem",
        TransferProtocol::Ymodem => "YModem",
        TransferProtocol::Zmodem => "ZModem",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalTransferPathPlatform {
    Unix,
    Windows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalTransferPathKind {
    Relative,
    Absolute,
    RootedWithoutDrive,
    DriveRelative,
    ForeignAnchored,
}

pub(super) fn current_local_transfer_path_platform() -> LocalTransferPathPlatform {
    if cfg!(windows) {
        LocalTransferPathPlatform::Windows
    } else {
        LocalTransferPathPlatform::Unix
    }
}

pub(super) fn classify_local_transfer_path(
    value: &str,
    platform: LocalTransferPathPlatform,
) -> LocalTransferPathKind {
    let bytes = value.as_bytes();
    let first_is_forward = bytes.first() == Some(&b'/');
    let first_is_backward = bytes.first() == Some(&b'\\');
    let first_is_separator = first_is_forward || first_is_backward;
    let second_is_separator = bytes
        .get(1)
        .is_some_and(|byte| *byte == b'/' || *byte == b'\\');
    let has_drive_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';

    match platform {
        LocalTransferPathPlatform::Unix => {
            if first_is_forward {
                LocalTransferPathKind::Absolute
            } else if first_is_backward || has_drive_prefix {
                LocalTransferPathKind::ForeignAnchored
            } else {
                LocalTransferPathKind::Relative
            }
        }
        LocalTransferPathPlatform::Windows => {
            if has_drive_prefix {
                if bytes
                    .get(2)
                    .is_some_and(|byte| *byte == b'/' || *byte == b'\\')
                {
                    LocalTransferPathKind::Absolute
                } else {
                    LocalTransferPathKind::DriveRelative
                }
            } else if first_is_separator && second_is_separator {
                LocalTransferPathKind::Absolute
            } else if first_is_separator {
                LocalTransferPathKind::RootedWithoutDrive
            } else {
                LocalTransferPathKind::Relative
            }
        }
    }
}

pub(super) fn validate_transfer_default_local_dir(profile: &SessionProfile) -> Result<(), String> {
    let home = native_home_path();
    resolve_transfer_default_local_dir_with_home(
        profile.transfer.default_local_dir.as_deref(),
        current_local_transfer_path_platform(),
        home.as_deref(),
    )
    .map(|_| ())
}

pub(super) fn resolve_transfer_default_local_dir_with_home(
    default_local_dir: Option<&str>,
    platform: LocalTransferPathPlatform,
    home: Option<&Path>,
) -> Result<Option<String>, String> {
    let Some(default_local_dir) = default_local_dir.filter(|path| !path.trim().is_empty()) else {
        return Ok(None);
    };
    let windows = platform == LocalTransferPathPlatform::Windows;
    if has_local_home_prefix(default_local_dir, windows) {
        return expand_local_home_transfer_path(default_local_dir, home, windows).map(Some);
    }
    if classify_local_transfer_path(default_local_dir, platform) != LocalTransferPathKind::Absolute
    {
        return Err("Profile 默认本地目录必须是当前平台的完整绝对路径或以 ~ 开头".to_string());
    }
    Ok(Some(default_local_dir.to_string()))
}

pub(super) fn resolve_default_local_transfer_path_with_home(
    value: &str,
    default_local_dir: Option<&str>,
    platform: LocalTransferPathPlatform,
    home: Option<&Path>,
) -> Result<String, String> {
    if is_nonlocal_transfer_endpoint(value) {
        return Ok(value.to_string());
    }
    let windows = platform == LocalTransferPathPlatform::Windows;
    if has_local_home_prefix(value, windows) {
        return expand_local_home_transfer_path(value, home, windows);
    }
    match classify_local_transfer_path(value, platform) {
        LocalTransferPathKind::Absolute => Ok(value.to_string()),
        LocalTransferPathKind::Relative => Ok(default_local_dir
            .map(|directory| {
                Path::new(directory)
                    .join(value)
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_else(|| value.to_string())),
        LocalTransferPathKind::RootedWithoutDrive => {
            Err("Windows 本地传输路径必须包含盘符或完整 UNC 前缀".to_string())
        }
        LocalTransferPathKind::DriveRelative => {
            Err("Windows 本地传输路径不能使用 drive-relative 形式".to_string())
        }
        LocalTransferPathKind::ForeignAnchored => {
            Err("本地传输路径与当前操作系统不兼容".to_string())
        }
    }
}

fn expand_local_home_transfer_path(
    value: &str,
    home: Option<&Path>,
    windows: bool,
) -> Result<String, String> {
    if local_home_relative_path(value, windows).is_none() {
        return Err("本地 ~ 路径不能包含 Windows 盘符后缀".to_string());
    }
    let home = home.ok_or_else(|| "无法解析本地 ~ 路径：系统用户主目录不可用".to_string())?;
    expand_identity_path_with_home(value, Some(home), windows)
        .into_os_string()
        .into_string()
        .map_err(|_| "本地 ~ 路径无法表示为 Unicode".to_string())
}

pub(super) fn has_remote_transfer_prefix(value: &str) -> bool {
    value.starts_with("remote:") || value.starts_with("ssh:")
}

pub(super) fn has_load_receiver_prefix(value: &str) -> bool {
    value.starts_with("load:")
}

pub(super) fn is_nonlocal_transfer_endpoint(value: &str) -> bool {
    has_remote_transfer_prefix(value) || has_load_receiver_prefix(value)
}

pub(super) fn validate_load_receiver_endpoint(
    value: &str,
    protocol: &TransferProtocol,
) -> Result<bool, String> {
    if !has_load_receiver_prefix(value) {
        return Ok(false);
    }
    match protocol {
        TransferProtocol::Tftp => parse_tftp_receiver_endpoint(value).map(|spec| spec.is_some()),
        _ => parse_load_receiver_endpoint(value, protocol).map(|spec| spec.is_some()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LoadReceiverSpec {
    pub(super) command: &'static str,
    pub(super) address: Option<String>,
    pub(super) baud_rate: Option<u32>,
}

impl LoadReceiverSpec {
    pub(super) fn command_line(&self) -> String {
        let mut command = self.command.to_string();
        if let Some(address) = self.address.as_deref() {
            command.push(' ');
            command.push_str(address);
        }
        if let Some(baud_rate) = self.baud_rate {
            command.push(' ');
            command.push_str(&baud_rate.to_string());
        }
        command.push('\r');
        command
    }
}

pub(super) fn parse_load_receiver_endpoint(
    value: &str,
    protocol: &TransferProtocol,
) -> Result<Option<LoadReceiverSpec>, String> {
    if !has_load_receiver_prefix(value) {
        return Ok(None);
    }
    if value.starts_with("load://") {
        return Err("load: 设备接收端点不能包含主机部分".to_string());
    }
    let parsed = url::Url::parse(value)
        .map_err(|error| format!("load: 设备接收端点无效: {error}"))?;
    if parsed.scheme() != "load" || parsed.fragment().is_some() {
        return Err("load: 设备接收端点格式无效".to_string());
    }

    let expected_command = match protocol {
        TransferProtocol::Xmodem => "loadx",
        TransferProtocol::Ymodem => "loady",
        TransferProtocol::Zmodem => "loadz",
        TransferProtocol::Sftp | TransferProtocol::Scp | TransferProtocol::Tftp => {
            return Err("load: 设备接收端点仅支持 X/Y/ZModem".to_string())
        }
    };
    if parsed.path() != expected_command {
        return Err(format!(
            "{} 传输必须使用 load:{expected_command} 设备接收端点",
            transfer_protocol_label(protocol)
        ));
    }

    let mut address = None;
    let mut baud_rate = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "address" if address.is_none() => {
                let value = value.into_owned();
                let digits = value
                    .strip_prefix("0x")
                    .or_else(|| value.strip_prefix("0X"))
                    .unwrap_or(&value);
                if digits.is_empty()
                    || digits.len() > 16
                    || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(
                        "load: 加载地址必须是最多 16 位的十六进制数，可带 0x 前缀"
                            .to_string(),
                    );
                }
                address = Some(value);
            }
            "baud" if baud_rate.is_none() => {
                let value = value
                    .parse::<u32>()
                    .map_err(|_| "load: 波特率必须是有效的正整数".to_string())?;
                if value == 0 {
                    return Err("load: 波特率必须大于 0".to_string());
                }
                baud_rate = Some(value);
            }
            "address" | "baud" => {
                return Err(format!("load: 参数 `{key}` 不能重复"));
            }
            _ => return Err(format!("load: 不支持参数 `{key}`")),
        }
    }
    if baud_rate.is_some() && address.is_none() {
        return Err("load: 指定波特率时必须同时指定加载地址".to_string());
    }
    Ok(Some(LoadReceiverSpec {
        command: expected_command,
        address,
        baud_rate,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TftpReceiverSpec {
    pub(super) address: Option<String>,
    pub(super) file_name: Option<String>,
    pub(super) device_ip: Ipv4Addr,
    pub(super) server_ip: Option<Ipv4Addr>,
    pub(super) bind_host: Option<Ipv4Addr>,
    pub(super) bind_port: u16,
    pub(super) timeout: Duration,
}

impl TftpReceiverSpec {
    pub(super) fn command_lines(
        &self,
        file_name: &str,
        server_ip: Ipv4Addr,
        server_port: u16,
    ) -> Result<String, String> {
        validate_tftp_file_name(file_name)?;
        let address = self.address.as_deref().unwrap_or("${loadaddr}");
        let mut commands = format!(
            "setenv ipaddr {}\rsetenv serverip {server_ip}\r",
            self.device_ip
        );
        if server_port == DEFAULT_TFTP_PORT {
            commands.push_str("setenv tftpdstp\r");
        } else {
            commands.push_str(&format!("setenv tftpdstp {server_port}\r"));
        }
        commands.push_str(&format!("tftpboot {address} {file_name}\r"));
        Ok(commands)
    }
}

pub(super) fn parse_tftp_receiver_endpoint(
    value: &str,
) -> Result<Option<TftpReceiverSpec>, String> {
    if !has_load_receiver_prefix(value) {
        return Ok(None);
    }
    if value.starts_with("load://") {
        return Err("load: TFTP 接收端点不能包含主机部分".to_string());
    }
    let parsed = url::Url::parse(value)
        .map_err(|error| format!("load: TFTP 接收端点无效: {error}"))?;
    if parsed.scheme() != "load" || parsed.path() != "tftpboot" || parsed.fragment().is_some() {
        return Err("TFTP 传输必须使用 load:tftpboot 接收端点".to_string());
    }

    let mut address = None;
    let mut file_name = None;
    let mut device_ip = None;
    let mut server_ip = None;
    let mut bind_host = None;
    let mut bind_port = None;
    let mut timeout_seconds = None;
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "address" if address.is_none() => {
                address = Some(validate_load_address(value.as_ref())?);
            }
            "fileName" if file_name.is_none() => {
                validate_tftp_file_name(value.as_ref())?;
                file_name = Some(value.into_owned());
            }
            "deviceIp" if device_ip.is_none() => {
                device_ip = Some(parse_tftp_ipv4(value.as_ref(), "deviceIp", false)?);
            }
            "serverIp" if server_ip.is_none() => {
                server_ip = Some(parse_tftp_ipv4(value.as_ref(), "serverIp", false)?);
            }
            "bindHost" if bind_host.is_none() => {
                bind_host = Some(parse_tftp_ipv4(value.as_ref(), "bindHost", true)?);
            }
            "bindPort" if bind_port.is_none() => {
                bind_port = Some(
                    value
                        .parse::<u16>()
                        .map_err(|_| "load: TFTP bindPort 必须是 0 到 65535 的整数".to_string())?,
                );
            }
            "timeoutSeconds" if timeout_seconds.is_none() => {
                let seconds = value.parse::<u64>().map_err(|_| {
                    "load: TFTP timeoutSeconds 必须是有效的正整数".to_string()
                })?;
                if !(5..=MAX_TFTP_TIMEOUT_SECONDS).contains(&seconds) {
                    return Err(format!(
                        "load: TFTP timeoutSeconds 必须介于 5 和 {MAX_TFTP_TIMEOUT_SECONDS} 之间"
                    ));
                }
                timeout_seconds = Some(seconds);
            }
            "address" | "fileName" | "deviceIp" | "serverIp" | "bindHost" | "bindPort"
            | "timeoutSeconds" => {
                return Err(format!("load: TFTP 参数 `{key}` 不能重复"));
            }
            _ => return Err(format!("load: TFTP 不支持参数 `{key}`")),
        }
    }
    let device_ip = device_ip.ok_or_else(|| "load: TFTP 必须指定 deviceIp".to_string())?;
    Ok(Some(TftpReceiverSpec {
        address,
        file_name,
        device_ip,
        server_ip,
        bind_host,
        bind_port: bind_port.unwrap_or(DEFAULT_TFTP_PORT),
        timeout: Duration::from_secs(
            timeout_seconds.unwrap_or(DEFAULT_TFTP_TIMEOUT_SECONDS),
        ),
    }))
}

fn validate_load_address(value: &str) -> Result<String, String> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    if digits.is_empty()
        || digits.len() > 16
        || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(
            "load: 加载地址必须是最多 16 位的十六进制数，可带 0x 前缀".to_string(),
        );
    }
    Ok(value.to_string())
}

fn parse_tftp_ipv4(value: &str, name: &str, allow_unspecified: bool) -> Result<Ipv4Addr, String> {
    let address = value
        .parse::<Ipv4Addr>()
        .map_err(|_| format!("load: TFTP {name} 必须是 IPv4 地址"))?;
    if address.is_multicast()
        || address == Ipv4Addr::BROADCAST
        || (!allow_unspecified && address.is_unspecified())
    {
        return Err(format!("load: TFTP {name} 不是可用的单播 IPv4 地址"));
    }
    Ok(address)
}

pub(super) fn validate_tftp_file_name(file_name: &str) -> Result<(), String> {
    if file_name.is_empty()
        || file_name.len() > 255
        || file_name.starts_with('/')
        || file_name
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        || !file_name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'/')
        })
    {
        return Err(
            "TFTP fileName 仅支持安全的相对 ASCII 路径（字母、数字、/、点、下划线、加号或连字符），且不能包含空、. 或 .. 分量"
                .to_string(),
        );
    }
    Ok(())
}

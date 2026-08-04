use super::*;

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
    let accesses_remote = has_remote_transfer_prefix(&request.source)
        || has_remote_transfer_prefix(&request.destination);
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
    let Some(default_local_dir) = default_local_dir
        .map(str::trim)
        .filter(|path| !path.is_empty())
    else {
        return Ok(None);
    };
    let windows = platform == LocalTransferPathPlatform::Windows;
    if local_home_relative_path(default_local_dir, windows).is_some() {
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
    if has_remote_transfer_prefix(value) {
        return Ok(value.to_string());
    }
    let windows = platform == LocalTransferPathPlatform::Windows;
    if local_home_relative_path(value, windows).is_some() {
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
    let home = home.ok_or_else(|| "无法解析本地 ~ 路径：系统用户主目录不可用".to_string())?;
    expand_identity_path_with_home(value, Some(home), windows)
        .into_os_string()
        .into_string()
        .map_err(|_| "本地 ~ 路径无法表示为 Unicode".to_string())
}

pub(super) fn has_remote_transfer_prefix(value: &str) -> bool {
    value.starts_with("remote:") || value.starts_with("ssh:")
}

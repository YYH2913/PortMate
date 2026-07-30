use super::*;
pub(super) async fn transfer_file_via_sftp(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);
    if let Some(source) = source_remote {
        validate_remote_transfer_path(source, "SFTP 远端源路径")?;
    }
    if let Some(destination) = destination_remote {
        validate_remote_transfer_path(destination, "SFTP 远端目标路径")?;
    }

    match (source_remote, destination_remote) {
        (None, None) => {
            copy_local_file_for_transfer(&request.source, &request.destination, progress).await
        }
        remote_paths => {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let sftp = auxiliary.sftp().await?;
            let transfer = async {
                match remote_paths {
                    (None, Some(remote_destination)) => {
                        sftp_upload(&sftp, &request.source, remote_destination, progress).await
                    }
                    (Some(remote_source), None) => {
                        sftp_download(&sftp, remote_source, &request.destination, progress).await
                    }
                    (Some(remote_source), Some(remote_destination)) => {
                        sftp_remote_copy(&sftp, remote_source, remote_destination, progress).await
                    }
                    (None, None) => unreachable!("local transfer handled before opening SFTP"),
                }
            };
            let result = await_sftp_transfer_with_cancellation(transfer, progress).await;
            result
        }
    }
}

pub(super) async fn await_sftp_transfer_with_cancellation<T, F>(
    operation: F,
    progress: &TransferProgressContext,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    progress.check_cancelled()?;
    tokio::pin!(operation);
    loop {
        tokio::select! {
            result = &mut operation => return result,
            _ = tokio::time::sleep(TRANSFER_CANCEL_POLL_INTERVAL) => {
                progress.check_cancelled()?;
            }
        }
    }
}

pub(super) async fn transfer_file_via_local_or_scp(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);
    if let Some(source) = source_remote {
        validate_remote_transfer_path(source, "SCP 远端源路径")?;
    }
    if let Some(destination) = destination_remote {
        validate_remote_transfer_path(destination, "SCP 远端目标路径")?;
    }

    match (source_remote, destination_remote) {
        (None, None) => {
            copy_local_file_for_transfer(&request.source, &request.destination, progress).await
        }
        (None, Some(remote_destination)) => {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let handle = auxiliary.handle();
            scp_upload(handle, &request.source, remote_destination, progress).await
        }
        (Some(remote_source), None) => {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let handle = auxiliary.handle();
            scp_download(handle, remote_source, &request.destination, progress).await
        }
        (Some(remote_source), Some(remote_destination)) => {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let handle = auxiliary.handle();
            remote_copy(handle, remote_source, remote_destination, progress).await
        }
    }
}

pub(super) fn remote_path(value: &str) -> Option<&str> {
    value
        .strip_prefix("remote:")
        .or_else(|| value.strip_prefix("ssh:"))
        .filter(|path| !path.trim().is_empty())
}

pub(super) fn validate_remote_transfer_path(path: &str, label: &str) -> Result<(), String> {
    let trimmed = path.trim();
    let normalized = trimmed.trim_end_matches('/');
    if trimmed.is_empty()
        || normalized.is_empty()
        || matches!(normalized, "." | ".." | "~" | "/" | "//")
        || trimmed.contains('\0')
        || remote_path_has_dot_components(normalized)
    {
        return Err(format!(
            "{label}不能为空、包含 NUL、使用 . / .. 分量或指向根目录"
        ));
    }
    Ok(())
}

pub(super) async fn copy_local_file_for_transfer(
    source: &str,
    destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let source = PathBuf::from(source);
    let destination = PathBuf::from(destination);
    let (mut input, total) = open_local_transfer_source(&source, "local transfer")?;
    prepare_local_transfer_target_path(&destination, "本地传输目标路径")?;
    let temp_destination = local_resume_part_path(&destination);
    let mut copied =
        local_resume_offset_matching_local_source(&mut input, &temp_destination, total, progress)?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, total).await?;
    }
    if total > 0 && copied == total {
        finalize_local_resume_file(&temp_destination, &destination)?;
        return Ok(copied);
    }
    let mut output = open_local_resume_writer(&temp_destination, copied)
        .map_err(|error| format!("local transfer create failed: {error}"))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        progress.check_cancelled()?;
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("local transfer read failed: {error}"))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("local transfer write failed: {error}"))?;
        copied += read as u64;
        ensure_transfer_not_oversized(copied, total, "local transfer")?;
        progress.update(copied, total).await?;
    }
    ensure_exact_transfer_size(copied, total, "local transfer")?;
    output
        .flush()
        .map_err(|error| format!("local transfer flush failed: {error}"))?;
    drop(output);
    finalize_local_resume_file(&temp_destination, &destination)?;
    Ok(copied)
}

/// A transfer writes to a temp sibling of the real destination and is only
/// renamed onto it after a full success; on any error the temp is best-effort
/// removed. Otherwise a mid-transfer failure leaves a partial file at the real
/// destination path with nothing distinguishing it from a complete one.
pub(super) fn local_resume_part_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("portmate-transfer");
    target.with_file_name(format!("{name}.portmate-part"))
}

pub(super) fn local_resume_offset(path: &Path, total: u64) -> Result<u64, String> {
    let Some(metadata) = local_transfer_entry(path, "本地断点文件")? else {
        return Ok(0);
    };
    let size = metadata.len();
    if size <= total {
        Ok(size)
    } else {
        fs::remove_file(path)
            .map_err(|error| format!("删除过大的断点文件失败 {}: {error}", path.display()))?;
        Ok(0)
    }
}

pub(super) fn open_local_transfer_reader(path: &Path, label: &str) -> Result<fs::File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options
        .open(path)
        .map_err(|error| format!("打开{label}失败 {}: {error}", path.display()))
}

pub(super) fn local_resume_offset_matching_local_source(
    source: &mut fs::File,
    part_path: &Path,
    total: u64,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let offset = local_resume_offset(part_path, total)?;
    if offset == 0 {
        return Ok(0);
    }
    source
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|error| format!("local transfer source seek failed: {error}"))?;
    let mut part = open_local_transfer_reader(part_path, "本地断点文件")?;
    let matches = compare_local_prefix(source, &mut part, offset, progress)?;
    source
        .seek(std::io::SeekFrom::Start(if matches { offset } else { 0 }))
        .map_err(|error| format!("local transfer source seek failed: {error}"))?;
    Ok(if matches { offset } else { 0 })
}

pub(super) fn compare_local_prefix(
    left: &mut fs::File,
    right: &mut fs::File,
    length: u64,
    progress: &TransferProgressContext,
) -> Result<bool, String> {
    let mut left_buffer = vec![0_u8; 64 * 1024];
    let mut right_buffer = vec![0_u8; 64 * 1024];
    let mut compared = 0_u64;
    while compared < length {
        progress.check_cancelled()?;
        let remaining = usize::try_from(length - compared).unwrap_or(usize::MAX);
        let take = remaining.min(left_buffer.len());
        match left.read_exact(&mut left_buffer[..take]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(format!("读取本地源文件前缀失败: {error}")),
        }
        match right.read_exact(&mut right_buffer[..take]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(format!("读取本地断点文件前缀失败: {error}")),
        }
        if left_buffer[..take] != right_buffer[..take] {
            return Ok(false);
        }
        compared += take as u64;
    }
    Ok(true)
}

pub(super) fn ensure_exact_transfer_size(
    copied: u64,
    total: u64,
    label: &str,
) -> Result<(), String> {
    if copied == total {
        Ok(())
    } else {
        Err(format!(
            "{label} size mismatch: copied {copied}, expected {total}"
        ))
    }
}

pub(super) fn ensure_transfer_not_oversized(
    copied: u64,
    total: u64,
    label: &str,
) -> Result<(), String> {
    if copied <= total {
        Ok(())
    } else {
        Err(format!(
            "{label} size mismatch: copied {copied}, expected {total}"
        ))
    }
}

pub(super) fn open_local_resume_writer(path: &Path, offset: u64) -> std::io::Result<fs::File> {
    if let Err(error) = local_transfer_entry(path, "本地断点文件") {
        return Err(std::io::Error::other(error));
    }
    if offset == 0 {
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        options.open(path)
    } else {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        options.open(path)
    }
}

pub(super) fn finalize_local_resume_file(temp: &Path, target: &Path) -> Result<(), String> {
    if local_transfer_entry(temp, "本地断点文件")?.is_none() {
        return Err(format!("本地断点文件不存在: {}", temp.display()));
    }
    if local_transfer_entry(target, "本地目标文件")?.is_some() {
        fs::remove_file(target)
            .map_err(|error| format!("删除旧目标文件失败 {}: {error}", target.display()))?;
    }
    fs::rename(temp, target).map_err(|error| {
        format!(
            "重命名本地目标文件失败 {} -> {}: {error}",
            temp.display(),
            target.display()
        )
    })
}

pub(super) fn local_transfer_entry(
    path: &Path,
    label: &str,
) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("{label}不能是符号链接: {}", path.display()))
        }
        Ok(metadata) if !metadata.is_file() => {
            Err(format!("{label}不是普通文件: {}", path.display()))
        }
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("检查{label}失败 {}: {error}", path.display())),
    }
}

pub(super) fn open_local_transfer_source(
    path: &Path,
    label: &str,
) -> Result<(fs::File, u64), String> {
    let metadata = local_transfer_entry(path, label)?
        .ok_or_else(|| format!("{label}不存在: {}", path.display()))?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options
        .open(path)
        .map_err(|error| format!("打开{label}失败 {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("检查{label}失败 {}: {error}", path.display()))?;
    if !opened_metadata.is_file() || opened_metadata.len() != metadata.len() {
        return Err(format!("{label}在打开前后发生变化: {}", path.display()));
    }
    Ok((file, metadata.len()))
}

pub(super) fn ensure_local_transfer_source_size(
    file: &fs::File,
    expected_size: u64,
    label: &str,
) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("检查{label}失败: {error}"))?;
    if metadata.is_file() && metadata.len() == expected_size {
        Ok(())
    } else {
        Err(format!("{label}在传输期间发生变化"))
    }
}

pub(super) fn local_transfer_source_size(path: &str) -> Result<u64, String> {
    local_transfer_entry(Path::new(path), "本地传输源")?
        .map(|metadata| metadata.len())
        .ok_or_else(|| format!("本地传输源不存在: {path}"))
}

pub(super) fn open_new_local_transfer_file(target: &Path) -> Result<(fs::File, PathBuf), String> {
    let _ = local_transfer_entry(target, "本地目标文件")?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-transfer");
    let temp = target.with_file_name(format!(".{name}.portmate-{}.part", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&temp)
        .map_err(|error| format!("创建本地传输临时文件失败 {}: {error}", temp.display()))?;
    Ok((file, temp))
}

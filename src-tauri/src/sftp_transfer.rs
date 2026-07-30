use super::*;

pub(super) async fn sftp_resume_offset(
    sftp: &SftpBackendSession,
    path: &str,
    total: u64,
) -> Result<u64, String> {
    let Some(size) = sftp_regular_file_size(sftp, path, "SFTP 断点文件").await? else {
        return Ok(0);
    };
    if size <= total {
        Ok(size)
    } else {
        sftp.remove_file(path.to_string())
            .await
            .map_err(|error| format!("SFTP 删除过大的断点文件失败 {path}: {error}"))?;
        Ok(0)
    }
}

pub(super) async fn sftp_resume_offset_matching_local_source(
    sftp: &SftpBackendSession,
    part_path: &str,
    total: u64,
    source: &mut fs::File,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let offset = sftp_resume_offset(sftp, part_path, total).await?;
    if offset == 0 {
        return Ok(0);
    }
    source
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|error| format!("SFTP 本地源文件 seek 失败: {error}"))?;
    let mut part = sftp
        .open(part_path.to_string())
        .await
        .map_err(|error| format!("SFTP 打开断点文件失败 {part_path}: {error}"))?;
    let matches = compare_sftp_and_local_prefix(&mut part, source, offset, progress).await;
    let _ = part.shutdown().await;
    let matches = matches?;
    source
        .seek(std::io::SeekFrom::Start(if matches { offset } else { 0 }))
        .map_err(|error| format!("SFTP 本地源文件 seek 失败: {error}"))?;
    Ok(if matches { offset } else { 0 })
}

pub(super) async fn local_resume_offset_matching_sftp_source(
    source: &mut SftpBackendFile,
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
        .await
        .map_err(|error| format!("SFTP 远端源文件 seek 失败: {error}"))?;
    let mut part = open_local_transfer_reader(part_path, "本地断点文件")?;
    let matches = compare_sftp_and_local_prefix(source, &mut part, offset, progress).await?;
    source
        .seek(std::io::SeekFrom::Start(if matches { offset } else { 0 }))
        .await
        .map_err(|error| format!("SFTP 远端源文件 seek 失败: {error}"))?;
    Ok(if matches { offset } else { 0 })
}

pub(super) async fn sftp_resume_offset_matching_sftp_source(
    sftp: &SftpBackendSession,
    source: &mut SftpBackendFile,
    part_path: &str,
    total: u64,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let offset = sftp_resume_offset(sftp, part_path, total).await?;
    if offset == 0 {
        return Ok(0);
    }
    source
        .seek(std::io::SeekFrom::Start(0))
        .await
        .map_err(|error| format!("SFTP 远端源文件 seek 失败: {error}"))?;
    let mut part = sftp
        .open(part_path.to_string())
        .await
        .map_err(|error| format!("SFTP 打开断点文件失败 {part_path}: {error}"))?;
    let matches = compare_sftp_prefixes(source, &mut part, offset, progress).await;
    let _ = part.shutdown().await;
    let matches = matches?;
    source
        .seek(std::io::SeekFrom::Start(if matches { offset } else { 0 }))
        .await
        .map_err(|error| format!("SFTP 远端源文件 seek 失败: {error}"))?;
    Ok(if matches { offset } else { 0 })
}

pub(super) async fn compare_sftp_and_local_prefix(
    remote: &mut SftpBackendFile,
    local: &mut fs::File,
    length: u64,
    progress: &TransferProgressContext,
) -> Result<bool, String> {
    let mut remote_buffer = vec![0_u8; 64 * 1024];
    let mut local_buffer = vec![0_u8; 64 * 1024];
    let mut compared = 0_u64;
    while compared < length {
        progress.check_cancelled()?;
        let remaining = usize::try_from(length - compared).unwrap_or(usize::MAX);
        let take = remaining.min(remote_buffer.len());
        match remote.read_exact(&mut remote_buffer[..take]).await {
            Ok(_) => {}
            Err(error) => return prefix_read_mismatch_or_error(error, "SFTP 远端断点文件"),
        }
        match std::io::Read::read_exact(local, &mut local_buffer[..take]) {
            Ok(()) => {}
            Err(error) => return prefix_read_mismatch_or_error(error, "本地源文件"),
        }
        if remote_buffer[..take] != local_buffer[..take] {
            return Ok(false);
        }
        compared += take as u64;
    }
    Ok(true)
}

pub(super) async fn compare_sftp_prefixes(
    left: &mut SftpBackendFile,
    right: &mut SftpBackendFile,
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
        match left.read_exact(&mut left_buffer[..take]).await {
            Ok(_) => {}
            Err(error) => return prefix_read_mismatch_or_error(error, "SFTP 远端源文件"),
        }
        match right.read_exact(&mut right_buffer[..take]).await {
            Ok(_) => {}
            Err(error) => return prefix_read_mismatch_or_error(error, "SFTP 远端断点文件"),
        }
        if left_buffer[..take] != right_buffer[..take] {
            return Ok(false);
        }
        compared += take as u64;
    }
    Ok(true)
}

pub(super) fn prefix_read_mismatch_or_error(
    error: std::io::Error,
    label: &str,
) -> Result<bool, String> {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        Ok(false)
    } else {
        Err(format!("读取{label}前缀失败: {error}"))
    }
}

pub(super) async fn sftp_regular_file_size(
    sftp: &SftpBackendSession,
    path: &str,
    label: &str,
) -> Result<Option<u64>, String> {
    let metadata = match sftp.symlink_metadata(path.to_string()).await {
        Ok(metadata) => metadata,
        Err(error) => {
            let exists = match sftp.try_exists(path.to_string()).await {
                Ok(exists) => exists,
                Err(exists_error) => {
                    return Err(format!(
                        "{label}无法读取远端属性 {path}: {error}; existence check failed: {exists_error}"
                    ));
                }
            };
            if exists {
                return Err(format!("{label}无法读取远端属性 {path}: {error}"));
            }
            return Ok(None);
        }
    };
    if metadata.is_symlink() {
        return Err(format!("{label}不能是符号链接: {path}"));
    }
    if !metadata.is_regular() {
        return Err(format!("{label}不是普通文件: {path}"));
    }
    Ok(Some(metadata.len()))
}

pub(super) async fn sftp_open_resume_writer(
    sftp: &SftpBackendSession,
    path: &str,
    offset: u64,
) -> Result<SftpBackendFile, String> {
    let _ = sftp_regular_file_size(sftp, path, "SFTP 断点文件").await?;
    let flags = if offset == 0 {
        OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE
    } else {
        OpenFlags::CREATE | OpenFlags::WRITE
    };
    let mut file = sftp
        .open_with_flags(path.to_string(), flags)
        .await
        .map_err(|error| format!("SFTP 打开断点文件失败 {path}: {error}"))?;
    if offset > 0 {
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|error| format!("SFTP 断点文件 seek 失败 {path}: {error}"))?;
    }
    Ok(file)
}

pub(super) async fn sftp_finalize_resume_file(
    sftp: &SftpBackendSession,
    temp: &str,
    target: &str,
) -> Result<(), String> {
    if sftp_regular_file_size(sftp, temp, "SFTP 断点文件")
        .await?
        .is_none()
    {
        return Err(format!("SFTP 断点文件不存在: {temp}"));
    }
    if sftp_regular_file_size(sftp, target, "SFTP 目标文件")
        .await?
        .is_some()
    {
        sftp.remove_file(target.to_string())
            .await
            .map_err(|error| format!("SFTP 删除旧目标文件失败 {target}: {error}"))?;
    }
    sftp.rename(temp.to_string(), target.to_string())
        .await
        .map_err(|error| format!("SFTP 重命名断点文件失败 {temp} -> {target}: {error}"))
}

pub(super) async fn sftp_upload(
    sftp: &SftpBackendSession,
    local_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut local_file, total) =
        open_local_transfer_source(Path::new(local_source), "SFTP upload")?;
    let file_name = Path::new(local_source)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-upload.bin");
    let target = sftp_destination_file_path(sftp, remote_destination, file_name).await?;
    let temp_target = remote_resume_part_path(&target);
    let mut copied = sftp_resume_offset_matching_local_source(
        sftp,
        &temp_target,
        total,
        &mut local_file,
        progress,
    )
    .await?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, total).await?;
    }
    if total > 0 && copied == total {
        sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
        return Ok(copied);
    }
    let mut remote_file = sftp_open_resume_writer(sftp, &temp_target, copied).await?;

    let copy_result: Result<u64, String> = async {
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            progress.check_cancelled()?;
            let read = local_file
                .read(&mut buffer)
                .map_err(|error| format!("读取本地文件失败 {local_source}: {error}"))?;
            if read == 0 {
                break;
            }
            remote_file
                .write_all(&buffer[..read])
                .await
                .map_err(|error| format!("SFTP 写入远端文件失败 {temp_target}: {error}"))?;
            copied += read as u64;
            ensure_transfer_not_oversized(copied, total, "SFTP upload")?;
            progress.update(copied, total).await?;
        }
        ensure_exact_transfer_size(copied, total, "SFTP upload")?;
        remote_file
            .flush()
            .await
            .map_err(|error| format!("SFTP 刷新远端文件失败 {temp_target}: {error}"))?;
        remote_file
            .shutdown()
            .await
            .map_err(|error| format!("SFTP 关闭远端文件失败 {temp_target}: {error}"))?;
        Ok(copied)
    }
    .await;

    match copy_result {
        Ok(copied) => {
            sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
            Ok(copied)
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn sftp_download(
    sftp: &SftpBackendSession,
    remote_source: &str,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let total = sftp_regular_file_size(sftp, remote_source, "SFTP 远端源文件")
        .await?
        .ok_or_else(|| format!("SFTP 远端源文件不存在: {remote_source}"))?;
    let mut remote_file = sftp
        .open(remote_source.to_string())
        .await
        .map_err(|error| format!("SFTP 打开远端文件失败 {remote_source}: {error}"))?;
    let target = local_destination_file_path(local_destination, remote_source)?;
    prepare_local_transfer_target_path(&target, "SFTP 本地目标路径")?;
    let temp_target = local_resume_part_path(&target);
    let mut copied =
        local_resume_offset_matching_sftp_source(&mut remote_file, &temp_target, total, progress)
            .await?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, total).await?;
    }
    if total > 0 && copied == total {
        finalize_local_resume_file(&temp_target, &target)?;
        let _ = remote_file.shutdown().await;
        return Ok(copied);
    }
    let mut local_file = open_local_resume_writer(&temp_target, copied)
        .map_err(|error| format!("创建本地目标文件失败 {}: {error}", temp_target.display()))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        progress.check_cancelled()?;
        let read = remote_file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("SFTP 读取远端文件失败 {remote_source}: {error}"))?;
        if read == 0 {
            break;
        }
        local_file
            .write_all(&buffer[..read])
            .map_err(|error| format!("写入本地目标文件失败 {}: {error}", temp_target.display()))?;
        copied += read as u64;
        ensure_transfer_not_oversized(copied, total, "SFTP download")?;
        progress.update(copied, total).await?;
    }
    ensure_exact_transfer_size(copied, total, "SFTP download")?;
    local_file
        .flush()
        .map_err(|error| format!("刷新本地目标文件失败 {}: {error}", temp_target.display()))?;
    drop(local_file);
    finalize_local_resume_file(&temp_target, &target)?;
    let _ = remote_file.shutdown().await;
    Ok(copied)
}

pub(super) async fn sftp_remote_copy(
    sftp: &SftpBackendSession,
    remote_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let total = sftp_regular_file_size(sftp, remote_source, "SFTP 远端源文件")
        .await?
        .ok_or_else(|| format!("SFTP 远端源文件不存在: {remote_source}"))?;
    let mut source_file = sftp
        .open(remote_source.to_string())
        .await
        .map_err(|error| format!("SFTP 打开远端源文件失败 {remote_source}: {error}"))?;
    let file_name = remote_file_name(remote_source);
    let target = sftp_destination_file_path(sftp, remote_destination, &file_name).await?;
    let temp_target = remote_resume_part_path(&target);
    let mut copied = sftp_resume_offset_matching_sftp_source(
        sftp,
        &mut source_file,
        &temp_target,
        total,
        progress,
    )
    .await?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, total).await?;
    }
    if total > 0 && copied == total {
        sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
        let _ = source_file.shutdown().await;
        return Ok(copied);
    }
    let mut target_file = sftp_open_resume_writer(sftp, &temp_target, copied).await?;

    let copy_result: Result<u64, String> = async {
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            progress.check_cancelled()?;
            let read = source_file
                .read(&mut buffer)
                .await
                .map_err(|error| format!("SFTP 读取远端源文件失败 {remote_source}: {error}"))?;
            if read == 0 {
                break;
            }
            target_file
                .write_all(&buffer[..read])
                .await
                .map_err(|error| format!("SFTP 写入远端目标文件失败 {temp_target}: {error}"))?;
            copied += read as u64;
            ensure_transfer_not_oversized(copied, total, "SFTP remote copy")?;
            progress.update(copied, total).await?;
        }
        ensure_exact_transfer_size(copied, total, "SFTP remote copy")?;
        target_file
            .flush()
            .await
            .map_err(|error| format!("SFTP 刷新远端目标文件失败 {temp_target}: {error}"))?;
        target_file
            .shutdown()
            .await
            .map_err(|error| format!("SFTP 关闭远端目标文件失败 {temp_target}: {error}"))?;
        Ok(copied)
    }
    .await;
    let _ = source_file.shutdown().await;

    match copy_result {
        Ok(copied) => {
            sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
            Ok(copied)
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn sftp_destination_file_path(
    sftp: &SftpBackendSession,
    remote_destination: &str,
    source_name: &str,
) -> Result<String, String> {
    let destination = remote_destination.trim();
    if destination.is_empty() {
        return Err("SFTP 远端目标路径不能为空".to_string());
    }

    if destination.ends_with('/') {
        sftp_create_dir_all(sftp, destination).await?;
        return Ok(remote_join_path(destination, source_name));
    }

    match sftp.metadata(destination.to_string()).await {
        Ok(metadata) if metadata.is_dir() => {
            return Ok(remote_join_path(destination, source_name));
        }
        Ok(_) => {}
        Err(metadata_error) => match sftp.try_exists(destination.to_string()).await {
            Ok(false) => {}
            Ok(true) => {
                return Err(format!(
                    "SFTP 无法读取远端目标属性 {destination}: {metadata_error}"
                ));
            }
            Err(exists_error) => {
                return Err(format!(
                    "SFTP 无法读取远端目标属性 {destination}: {metadata_error}; existence check failed: {exists_error}"
                ));
            }
        },
    }

    if let Some(parent) = remote_parent_path(destination) {
        if parent != "." && parent != "/" {
            sftp_create_dir_all(sftp, &parent).await?;
        }
    }
    Ok(destination.to_string())
}

pub(super) fn local_destination_file_path(
    local_destination: &str,
    remote_source: &str,
) -> Result<PathBuf, String> {
    let destination = expand_identity_path(local_destination.trim());
    if local_destination.trim().is_empty() {
        return Err("本地目标路径不能为空".to_string());
    }
    let source_name = remote_file_name(remote_source);
    let ends_with_separator = local_destination.ends_with('/') || local_destination.ends_with('\\');
    if destination.is_dir() || ends_with_separator {
        Ok(destination.join(source_name))
    } else {
        Ok(destination)
    }
}

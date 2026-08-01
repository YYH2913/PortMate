use super::*;

pub(crate) async fn sftp_resume_offset(
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

pub(crate) async fn sftp_resume_offset_matching_local_source(
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

pub(crate) async fn local_resume_offset_matching_sftp_source(
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

pub(crate) async fn sftp_resume_offset_matching_sftp_source(
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

pub(crate) async fn compare_sftp_and_local_prefix(
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

pub(crate) async fn compare_sftp_prefixes(
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

pub(crate) fn prefix_read_mismatch_or_error(
    error: std::io::Error,
    label: &str,
) -> Result<bool, String> {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        Ok(false)
    } else {
        Err(format!("读取{label}前缀失败: {error}"))
    }
}

pub(crate) async fn sftp_regular_file_size(
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

pub(crate) async fn sftp_open_resume_writer(
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

pub(crate) async fn sftp_finalize_resume_file(
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

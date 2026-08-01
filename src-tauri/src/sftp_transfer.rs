use super::*;

mod resume;

pub(super) use resume::*;

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

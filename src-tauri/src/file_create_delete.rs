use super::*;

pub(super) async fn sftp_remove_recursive(
    sftp: &SftpBackendSession,
    path: &str,
) -> Result<(), String> {
    let path = validate_remote_mutating_path(path)?;
    let mut stack = vec![(path.to_string(), false)];

    while let Some((current, visited)) = stack.pop() {
        let metadata = sftp
            .symlink_metadata(current.clone())
            .await
            .map_err(|error| format!("SFTP 读取远端路径失败 {current}: {error}"))?;
        let is_directory = metadata.is_dir() && !metadata.is_symlink();
        if is_directory && !visited {
            stack.push((current.clone(), true));
            let entries = sftp
                .read_dir(current.clone())
                .await
                .map_err(|error| format!("SFTP 读取远端目录失败 {current}: {error}"))?;
            for entry in entries {
                stack.push((remote_join_path(&current, &entry.file_name()), false));
            }
            continue;
        }

        if is_directory {
            sftp.remove_dir(current.clone())
                .await
                .map_err(|error| format!("SFTP 删除远端目录失败 {current}: {error}"))?;
        } else {
            sftp.remove_file(current.clone())
                .await
                .map_err(|error| format!("SFTP 删除远端文件失败 {current}: {error}"))?;
        }
    }

    Ok(())
}

pub(super) enum FileOperation {
    CreateDirectory,
    CreateFile,
    Delete,
}

pub(super) async fn file_operation_inner(
    state: &AppState,
    request: FileOperationRequest,
    operation: FileOperation,
) -> Result<(), String> {
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote file operation requires sessionId".to_string())?;
        let path = validate_remote_mutating_path(&request.path)?;
        let auxiliary = ssh_auxiliary_lease(state, session_id)?;
        let sftp = auxiliary.sftp().await?;
        let result = async {
            match operation {
                FileOperation::CreateDirectory => {
                    reject_remote_symlink_components(&sftp, &path, false, "远端目录创建路径")
                        .await?;
                    sftp_create_dir_all(&sftp, &path).await
                }
                FileOperation::CreateFile => {
                    reject_remote_symlink_components(&sftp, &path, false, "远端文件创建路径")
                        .await?;
                    let mut file = sftp
                        .open_with_flags(
                            path.clone(),
                            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
                        )
                        .await
                        .map_err(|error| format!("SFTP 新建远端文件失败 {path}: {error}"))?;
                    file.shutdown()
                        .await
                        .map_err(|error| format!("SFTP 关闭新建远端文件失败 {path}: {error}"))
                }
                FileOperation::Delete => {
                    reject_remote_symlink_components(&sftp, &path, true, "远端删除路径").await?;
                    sftp_remove_recursive(&sftp, &path).await
                }
            }
        }
        .await;
        result
    } else {
        match operation {
            FileOperation::CreateDirectory => {
                let path = validate_local_mutating_path(&request.path)?;
                reject_local_symlink_components(&path, false, "本地目录创建路径")?;
                fs::create_dir_all(&path)
                    .map_err(|error| format!("创建本地目录失败 {}: {error}", path.display()))
            }
            FileOperation::CreateFile => {
                let path = validate_local_mutating_path(&request.path)?;
                reject_local_symlink_components(&path, false, "本地文件创建路径")?;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                options.custom_flags(libc::O_NOFOLLOW);
                options
                    .open(&path)
                    .map(|_| ())
                    .map_err(|error| format!("新建本地文件失败 {}: {error}", path.display()))
            }
            FileOperation::Delete => {
                let path = validate_local_mutating_path(&request.path)?;
                reject_local_symlink_components(&path, true, "本地删除路径")?;
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|error| format!("读取本地路径失败 {}: {error}", path.display()))?;
                if metadata.is_dir() {
                    fs::remove_dir_all(&path)
                        .map_err(|error| format!("删除本地目录失败 {}: {error}", path.display()))
                } else {
                    fs::remove_file(&path)
                        .map_err(|error| format!("删除本地文件失败 {}: {error}", path.display()))
                }
            }
        }
    }
}

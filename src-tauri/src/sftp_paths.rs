use super::*;

pub(super) fn remote_resume_part_path(target: &str) -> String {
    match target.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/{name}.portmate-part"),
        None => format!("{target}.portmate-part"),
    }
}

pub(super) async fn sftp_create_dir_all(
    sftp: &SftpBackendSession,
    path: &str,
) -> Result<(), String> {
    let path = path.trim_end_matches('/');
    if path.is_empty() || path == "." || path == "/" {
        return Ok(());
    }

    let mut current = if path.starts_with('/') {
        "/".to_string()
    } else {
        String::new()
    };
    for part in path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        current = remote_join_path(&current, part);
        match sftp.symlink_metadata(current.clone()).await {
            Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => continue,
            Ok(_) => {
                return Err(format!(
                    "SFTP 创建远端目录失败 {current}: 路径已存在但不是普通目录"
                ));
            }
            Err(metadata_error) => match sftp.try_exists(current.clone()).await {
                Ok(false) => {}
                Ok(true) => {
                    return Err(format!("SFTP 创建远端目录失败 {current}: {metadata_error}"));
                }
                Err(exists_error) => {
                    return Err(format!(
                        "SFTP 创建远端目录失败 {current}: {metadata_error}; existence check failed: {exists_error}"
                    ));
                }
            },
        }
        match sftp.create_dir(current.clone()).await {
            Ok(()) => {}
            Err(error) => match sftp.symlink_metadata(current.clone()).await {
                Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {}
                _ => return Err(format!("SFTP 创建远端目录失败 {current}: {error}")),
            },
        }
    }
    Ok(())
}

pub(super) async fn reject_remote_symlink_components(
    sftp: &SftpBackendSession,
    path: &str,
    allow_final_symlink: bool,
    label: &str,
) -> Result<(), String> {
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut current = if path.starts_with('/') {
        "/".to_string()
    } else {
        String::new()
    };
    for (index, part) in parts.iter().enumerate() {
        current = remote_join_path(&current, part);
        let metadata = match sftp.symlink_metadata(current.clone()).await {
            Ok(metadata) => metadata,
            Err(error) => match sftp.try_exists(current.clone()).await {
                Ok(false) => break,
                Ok(true) => {
                    return Err(format!("无法检查{label} {current}: {error}"));
                }
                Err(exists_error) => {
                    return Err(format!(
                        "无法检查{label} {current}: {error}; existence check failed: {exists_error}"
                    ));
                }
            },
        };
        if metadata.is_symlink() && !(allow_final_symlink && index + 1 == parts.len()) {
            return Err(format!("{label}不能经过符号链接: {current}"));
        }
    }
    Ok(())
}

pub(super) fn remote_parent_path(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    let index = path.rfind('/')?;
    if index == 0 {
        Some("/".to_string())
    } else {
        Some(path[..index].to_string())
    }
}

pub(super) fn remote_file_name(path: &str) -> String {
    portable_file_name(path).unwrap_or_else(|| "portmate-file.bin".to_string())
}

pub(super) fn remote_join_path(parent: &str, name: &str) -> String {
    let name = name.trim_matches('/');
    if parent.is_empty() || parent == "." {
        name.to_string()
    } else if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

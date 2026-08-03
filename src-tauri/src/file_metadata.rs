use super::*;

pub(super) const MAX_FILE_DIRECTORY_ENTRIES: usize = 20_000;

pub(super) fn list_local_files(path: &str) -> Result<Vec<FileEntry>, String> {
    list_local_files_with_limit(path, MAX_FILE_DIRECTORY_ENTRIES)
}

pub(super) fn list_local_files_with_limit(
    path: &str,
    max_entries: usize,
) -> Result<Vec<FileEntry>, String> {
    let path = validate_native_local_path(if path.trim().is_empty() {
        "."
    } else {
        path.trim()
    })?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&path)
        .map_err(|error| format!("读取本地目录失败 {}: {error}", path.display()))?
    {
        let entry = entry.map_err(|error| format!("读取本地目录项失败: {error}"))?;
        if entries.len() >= max_entries {
            return Err(format!("目录条目超过 {max_entries} 条，请缩小目录范围"));
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|error| format!("读取本地文件元数据失败: {error}"))?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| {
                chrono::DateTime::<Utc>::from(std::time::UNIX_EPOCH + duration).to_rfc3339()
            });
        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            path: entry.path().display().to_string(),
            is_dir: metadata.is_dir() && !metadata.file_type().is_symlink(),
            size: if metadata.is_file() && !metadata.file_type().is_symlink() {
                metadata.len()
            } else {
                0
            },
            modified,
        });
    }
    sort_file_entries(&mut entries);
    Ok(entries)
}

pub(super) fn local_file_properties(path: &str) -> Result<FileProperties, String> {
    let path = validate_native_local_path(path)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("读取本地路径属性失败 {}: {error}", path.display()))?;
    let file_type = metadata.file_type();
    #[cfg(unix)]
    let permissions = {
        use std::os::unix::fs::PermissionsExt;
        Some(metadata.permissions().mode())
    };
    #[cfg(not(unix))]
    let permissions = None;
    Ok(FileProperties {
        name: path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_else(|| path.to_str().unwrap_or(""))
            .to_string(),
        path: path.display().to_string(),
        remote: false,
        kind: file_kind_label(
            metadata.is_dir(),
            metadata.is_file(),
            file_type.is_symlink(),
        ),
        is_dir: metadata.is_dir(),
        is_file: metadata.is_file(),
        is_symlink: file_type.is_symlink(),
        size: if metadata.is_file() {
            metadata.len()
        } else {
            0
        },
        permissions,
        modified: metadata.modified().ok().and_then(system_time_to_rfc3339),
        accessed: metadata.accessed().ok().and_then(system_time_to_rfc3339),
        created: metadata.created().ok().and_then(system_time_to_rfc3339),
    })
}

pub(super) async fn list_remote_files(
    sftp: &SftpBackendSession,
    path: &str,
) -> Result<Vec<FileEntry>, String> {
    let path = if path.trim().is_empty() {
        "."
    } else {
        path.trim()
    };
    list_remote_files_via_sftp(sftp, path).await
}

async fn list_remote_files_via_sftp(
    sftp: &SftpBackendSession,
    path: &str,
) -> Result<Vec<FileEntry>, String> {
    let mut entries = Vec::new();
    for entry in sftp
        .read_dir(path.to_string())
        .await
        .map_err(|error| format!("SFTP 读取远端目录失败 {path}: {error}"))?
    {
        let metadata = entry.metadata();
        let name = entry.file_name();
        let modified = metadata
            .mtime
            .and_then(|timestamp| chrono::DateTime::<Utc>::from_timestamp(timestamp.into(), 0))
            .map(|value| value.to_rfc3339());
        entries.push(FileEntry {
            path: remote_join_path(path, &name),
            name,
            is_dir: metadata.is_dir(),
            size: if metadata.is_regular() {
                metadata.len()
            } else {
                0
            },
            modified,
        });
    }
    sort_file_entries(&mut entries);
    Ok(entries)
}

pub(super) async fn remote_file_properties(
    sftp: &SftpBackendSession,
    path: &str,
) -> Result<FileProperties, String> {
    let metadata = sftp
        .symlink_metadata(path.to_string())
        .await
        .map_err(|error| format!("SFTP 读取远端属性失败 {path}: {error}"))?;
    let is_dir = metadata.is_dir();
    let is_file = metadata.is_regular();
    let is_symlink = metadata.is_symlink();
    Ok(FileProperties {
        name: remote_file_name(path),
        path: path.to_string(),
        remote: true,
        kind: file_kind_label(is_dir, is_file, is_symlink),
        is_dir,
        is_file,
        is_symlink,
        size: if is_file { metadata.len() } else { 0 },
        permissions: metadata.permissions,
        modified: metadata
            .mtime
            .and_then(|timestamp| chrono::DateTime::<Utc>::from_timestamp(timestamp.into(), 0))
            .map(|value| value.to_rfc3339()),
        accessed: None,
        created: None,
    })
}

fn file_kind_label(is_dir: bool, is_file: bool, is_symlink: bool) -> String {
    if is_symlink {
        "symlink"
    } else if is_dir {
        "directory"
    } else if is_file {
        "file"
    } else {
        "other"
    }
    .to_string()
}

fn system_time_to_rfc3339(time: std::time::SystemTime) -> Option<String> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| {
            chrono::DateTime::<Utc>::from(std::time::UNIX_EPOCH + duration).to_rfc3339()
        })
}

fn sort_file_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

pub(super) async fn list_files_inner(
    state: &AppState,
    request: ListFilesRequest,
) -> Result<Vec<FileEntry>, String> {
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote file list requires sessionId".to_string())?;
        let auxiliary = ssh_auxiliary_lease(state, session_id)?;
        let sftp = auxiliary.sftp().await?;
        list_remote_files(&sftp, &request.path).await
    } else {
        list_local_files(&request.path)
    }
}

pub(super) async fn file_properties_inner(
    state: &AppState,
    request: FilePropertiesRequest,
) -> Result<FileProperties, String> {
    if request.path.trim().is_empty() {
        return Err("属性路径不能为空".to_string());
    }
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote file properties require sessionId".to_string())?;
        let auxiliary = ssh_auxiliary_lease(state, session_id)?;
        let sftp = auxiliary.sftp().await?;
        let result = remote_file_properties(&sftp, request.path.trim()).await;
        result
    } else {
        local_file_properties(request.path.trim())
    }
}

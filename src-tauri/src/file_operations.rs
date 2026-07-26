use super::*;

pub(super) fn prepare_local_transfer_target_path(path: &Path, label: &str) -> Result<(), String> {
    reject_local_symlink_components(path, false, label)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("创建{label}目录失败 {}: {error}", parent.display()))?;
    }
    reject_local_symlink_components(path, false, label)
}

pub(super) fn portable_file_name(path: &str) -> Option<String> {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let without_drive = if trimmed.len() >= 2
        && trimmed.as_bytes()[0].is_ascii_alphabetic()
        && trimmed.as_bytes()[1] == b':'
    {
        &trimmed[2..]
    } else {
        trimmed
    };
    let name = without_drive
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .trim();
    if name.is_empty()
        || matches!(name, "." | "..")
        || name
            .chars()
            .any(|character| character.is_control() || character == '\0')
    {
        return None;
    }
    Some(name.to_string())
}

pub(super) fn list_local_files(path: &str) -> Result<Vec<FileEntry>, String> {
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
    sftp: &SftpSession,
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
    sftp: &SftpSession,
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
    sftp: &SftpSession,
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

pub(super) async fn sftp_remove_recursive(sftp: &SftpSession, path: &str) -> Result<(), String> {
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

pub(super) fn validate_remote_mutating_path(path: &str) -> Result<String, String> {
    let path = path.trim();
    let trimmed = path.trim_end_matches('/');
    if path.is_empty()
        || trimmed.is_empty()
        || matches!(trimmed, "." | ".." | "~" | "/" | "//")
        || path.contains('\0')
    {
        return Err("拒绝操作空路径、根目录或当前目录".to_string());
    }
    if remote_path_has_dot_components(trimmed) {
        return Err("拒绝包含 . 或 .. 路径分量的远端变更路径".to_string());
    }
    Ok(path.to_string())
}

/// Guards local delete/rename endpoints against the two most catastrophic
/// fat-finger paths: an empty/`.`/`~` path, and a path that resolves to a
/// filesystem root or to the user's home directory itself.
pub(super) fn validate_local_mutating_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    let trimmed_slashes = trimmed.trim_end_matches(['/', '\\']);
    if trimmed.is_empty()
        || trimmed_slashes.is_empty()
        || matches!(trimmed_slashes, "." | ".." | "~")
        || trimmed.contains('\0')
    {
        return Err("拒绝操作空路径、根目录或当前目录".to_string());
    }
    if trimmed
        .split(['/', '\\'])
        .any(|component| matches!(component, "." | ".."))
    {
        return Err("拒绝包含 . 或 .. 路径分量的本地变更路径".to_string());
    }

    let candidate = validate_native_local_path(trimmed)?;
    if candidate.parent().is_none() || is_local_filesystem_root(&candidate) {
        return Err("拒绝操作文件系统根目录".to_string());
    }
    if candidate.components().any(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    }) {
        return Err("拒绝包含 . 或 .. 路径分量的本地变更路径".to_string());
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        let candidate_check = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        let home_check = home.canonicalize().unwrap_or(home);
        if candidate_check == home_check {
            return Err("拒绝操作用户主目录".to_string());
        }
    }
    Ok(candidate)
}

pub(super) fn reject_local_symlink_components(
    path: &Path,
    allow_final_symlink: bool,
    label: &str,
) -> Result<(), String> {
    let component_count = path.components().count();
    let mut current = PathBuf::new();
    for (index, component) in path.components().enumerate() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata)
                if metadata.file_type().is_symlink()
                    && !(allow_final_symlink && index + 1 == component_count) =>
            {
                return Err(format!("{label}不能经过符号链接: {}", current.display()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(format!("检查{label}失败 {}: {error}", current.display()));
            }
        }
    }
    Ok(())
}

pub(super) fn validate_native_local_path(path: &str) -> Result<PathBuf, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        return Err("本地路径不能为空或包含 NUL".to_string());
    }
    match classify_local_transfer_path(trimmed, current_local_transfer_path_platform()) {
        LocalTransferPathKind::Relative | LocalTransferPathKind::Absolute => {
            Ok(expand_identity_path(trimmed))
        }
        LocalTransferPathKind::RootedWithoutDrive => {
            Err("Windows 本地路径必须包含盘符或完整 UNC 前缀".to_string())
        }
        LocalTransferPathKind::DriveRelative => {
            Err("Windows 本地路径不能使用 drive-relative 形式".to_string())
        }
        LocalTransferPathKind::ForeignAnchored => Err("本地路径与当前操作系统不兼容".to_string()),
    }
}

fn is_local_filesystem_root(path: &Path) -> bool {
    path.has_root() && path.file_name().is_none()
}

fn sort_file_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

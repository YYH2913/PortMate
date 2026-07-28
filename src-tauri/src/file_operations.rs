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

pub(super) enum FileOperation {
    CreateDirectory,
    CreateFile,
    Delete,
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

#[derive(Debug)]
struct LocalDeletePath {
    source: PathBuf,
    identity: PathBuf,
    source_is_dir: bool,
}

#[derive(Debug)]
struct RemoteDeletePath {
    source: String,
    source_is_dir: bool,
}

fn prepare_local_delete_paths(paths: &[String]) -> Result<Vec<LocalDeletePath>, String> {
    validate_file_batch_path_count(paths)?;
    let mut identities = HashSet::new();
    let mut plan = Vec::with_capacity(paths.len());
    for raw_path in paths {
        let source = validate_local_mutating_path(raw_path)?;
        reject_local_symlink_components(&source, true, "本地批量删除路径")?;
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("读取本地批量删除路径失败 {}: {error}", source.display()))?;
        let identity = canonical_local_entry_path(&source, "本地批量删除路径")?;
        if !identities.insert(identity.clone()) {
            return Err(format!("批量删除包含重复路径: {}", source.display()));
        }
        plan.push(LocalDeletePath {
            source,
            identity,
            source_is_dir: metadata.is_dir() && !metadata.file_type().is_symlink(),
        });
    }
    for directory in plan.iter().filter(|item| item.source_is_dir) {
        if let Some(nested) = plan.iter().find(|item| {
            item.identity != directory.identity && item.identity.starts_with(&directory.identity)
        }) {
            return Err(format!(
                "批量删除不能同时包含目录及其子项: {} 和 {}",
                directory.source.display(),
                nested.source.display()
            ));
        }
    }
    Ok(plan)
}

async fn prepare_remote_delete_paths(
    sftp: &SftpBackendSession,
    paths: &[String],
) -> Result<Vec<RemoteDeletePath>, String> {
    validate_file_batch_path_count(paths)?;
    let mut sources = HashSet::new();
    let mut plan = Vec::with_capacity(paths.len());
    for raw_path in paths {
        let source = validate_remote_mutating_path(raw_path)?;
        let source = source.trim_end_matches('/').to_string();
        if !sources.insert(source.clone()) {
            return Err(format!("批量删除包含重复路径: {source}"));
        }
        reject_remote_symlink_components(sftp, &source, true, "远端批量删除路径").await?;
        let metadata = sftp
            .symlink_metadata(source.clone())
            .await
            .map_err(|error| format!("SFTP 读取远端批量删除路径失败 {source}: {error}"))?;
        plan.push(RemoteDeletePath {
            source,
            source_is_dir: metadata.is_dir() && !metadata.is_symlink(),
        });
    }
    for directory in plan.iter().filter(|item| item.source_is_dir) {
        if let Some(nested) = plan.iter().find(|item| {
            item.source != directory.source
                && remote_path_is_within(&item.source, &directory.source)
        }) {
            return Err(format!(
                "批量删除不能同时包含目录及其子项: {} 和 {}",
                directory.source, nested.source
            ));
        }
    }
    Ok(plan)
}

pub(super) async fn delete_paths_inner(
    state: &AppState,
    request: DeletePathsRequest,
) -> Result<(), String> {
    validate_file_batch_path_count(&request.paths)?;
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote batch delete requires sessionId".to_string())?;
        let auxiliary = ssh_auxiliary_lease(state, session_id)?;
        let sftp = auxiliary.sftp().await?;
        let result = async {
            let plan = prepare_remote_delete_paths(&sftp, &request.paths).await?;
            for (completed, item) in plan.into_iter().enumerate() {
                reject_remote_symlink_components(
                    &sftp,
                    &item.source,
                    true,
                    "远端批量删除路径",
                )
                .await
                .map_err(|error| {
                    format!(
                        "SFTP 批量删除失败 {}: {error}; 已完成 {completed} 项，请刷新目录确认当前状态",
                        item.source
                    )
                })?;
                sftp_remove_recursive(&sftp, &item.source)
                    .await
                    .map_err(|error| {
                        format!(
                            "SFTP 批量删除失败 {}: {error}; 已完成 {completed} 项，请刷新目录确认当前状态",
                            item.source
                        )
                    })?;
            }
            Ok(())
        }
        .await;
        result
    } else {
        let plan = prepare_local_delete_paths(&request.paths)?;
        for (completed, item) in plan.into_iter().enumerate() {
            reject_local_symlink_components(&item.source, true, "本地批量删除路径").map_err(
                |error| {
                    format!(
                        "本地批量删除失败 {}: {error}; 已完成 {completed} 项，请刷新目录确认当前状态",
                        item.source.display()
                    )
                },
            )?;
            let metadata = fs::symlink_metadata(&item.source).map_err(|error| {
                format!(
                    "本地批量删除失败 {}: 读取路径失败 {error}; 已完成 {completed} 项，请刷新目录确认当前状态",
                    item.source.display()
                )
            })?;
            let result = if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(&item.source)
            } else {
                fs::remove_file(&item.source)
            };
            result.map_err(|error| {
                format!(
                    "本地批量删除失败 {}: {error}; 已完成 {completed} 项，请刷新目录确认当前状态",
                    item.source.display()
                )
            })?;
        }
        Ok(())
    }
}

fn ensure_local_path_missing(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!("{label}已存在: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("检查{label}失败 {}: {error}", path.display())),
    }
}

async fn ensure_remote_path_missing(
    sftp: &SftpBackendSession,
    path: &str,
    label: &str,
) -> Result<(), String> {
    match sftp.symlink_metadata(path.to_string()).await {
        Ok(_) => Err(format!("{label}已存在: {path}")),
        Err(metadata_error) => match sftp.try_exists(path.to_string()).await {
            Ok(false) => Ok(()),
            Ok(true) => Err(format!("无法检查{label} {path}: {metadata_error}")),
            Err(exists_error) => Err(format!(
                "无法检查{label} {path}: {metadata_error}; existence check failed: {exists_error}"
            )),
        },
    }
}

fn rename_local_path_without_overwrite(source: &Path, target: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStrExt;

        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| format!("本地移动源路径包含 NUL: {}", source.display()))?;
        let target = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| format!("本地移动目标路径包含 NUL: {}", target.display()))?;
        // The C strings own stable NUL-terminated path bytes for this syscall.
        let status = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }

    #[cfg(not(target_os = "linux"))]
    {
        fs::rename(source, target).map_err(|error| error.to_string())
    }
}

#[derive(Debug)]
struct LocalMovePath {
    source: PathBuf,
    source_identity: PathBuf,
    target: PathBuf,
    source_is_dir: bool,
}

#[derive(Debug)]
struct RemoteMovePath {
    source: String,
    target: String,
    source_is_dir: bool,
}

fn validate_file_batch_path_count(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("文件批量操作没有源路径".to_string());
    }
    if paths.len() > MAX_EXTERNAL_DROP_ROOTS {
        return Err(format!(
            "一次最多处理 {MAX_EXTERNAL_DROP_ROOTS} 个顶层路径，当前为 {} 个",
            paths.len()
        ));
    }
    Ok(())
}

fn canonical_local_entry_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("{label}没有文件名: {}", path.display()))?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = parent
        .canonicalize()
        .map_err(|error| format!("解析{label}父目录失败 {}: {error}", parent.display()))?;
    Ok(parent.join(name))
}

fn prepare_local_move_paths(
    paths: &[String],
    destination: &str,
) -> Result<Vec<LocalMovePath>, String> {
    validate_file_batch_path_count(paths)?;
    let destination = validate_local_mutating_path(destination)?;
    reject_local_symlink_components(&destination, false, "本地移动目标目录")?;
    let destination_metadata = fs::symlink_metadata(&destination).map_err(|error| {
        format!(
            "读取本地移动目标目录失败 {}: {error}",
            destination.display()
        )
    })?;
    if !destination_metadata.is_dir() || destination_metadata.file_type().is_symlink() {
        return Err(format!(
            "本地移动目标不是普通目录: {}",
            destination.display()
        ));
    }
    let destination = destination.canonicalize().map_err(|error| {
        format!(
            "解析本地移动目标目录失败 {}: {error}",
            destination.display()
        )
    })?;

    let mut sources = HashSet::new();
    let mut targets = HashSet::new();
    let mut plan = Vec::with_capacity(paths.len());
    for raw_path in paths {
        let source = validate_local_mutating_path(raw_path)?;
        reject_local_symlink_components(&source, true, "本地移动源路径")?;
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("读取本地移动源路径失败 {}: {error}", source.display()))?;
        let source_identity = canonical_local_entry_path(&source, "本地移动源路径")?;
        if !sources.insert(source_identity.clone()) {
            return Err(format!("移动操作包含重复源路径: {}", source.display()));
        }
        let name = source
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| format!("本地移动源路径没有文件名: {}", source.display()))?;
        let target = destination.join(name);
        if source_identity == target {
            return Err(format!("源路径已位于移动目标目录: {}", source.display()));
        }
        if metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && destination.starts_with(&source_identity)
        {
            return Err(format!(
                "拒绝将目录移动到其自身内部: {} -> {}",
                source.display(),
                destination.display()
            ));
        }
        reject_local_symlink_components(&target, false, "本地移动目标路径")?;
        if !targets.insert(target.clone()) {
            return Err(format!("移动操作包含冲突目标路径: {}", target.display()));
        }
        ensure_local_path_missing(&target, "本地移动目标路径")?;
        plan.push(LocalMovePath {
            source,
            source_identity,
            target,
            source_is_dir: metadata.is_dir() && !metadata.file_type().is_symlink(),
        });
    }

    for directory in plan.iter().filter(|item| item.source_is_dir) {
        if let Some(nested) = plan.iter().find(|item| {
            item.source_identity != directory.source_identity
                && item.source_identity.starts_with(&directory.source_identity)
        }) {
            return Err(format!(
                "移动操作不能同时包含目录及其子项: {} 和 {}",
                directory.source.display(),
                nested.source.display()
            ));
        }
    }
    Ok(plan)
}

async fn prepare_remote_move_paths(
    sftp: &SftpBackendSession,
    paths: &[String],
    destination: &str,
) -> Result<Vec<RemoteMovePath>, String> {
    validate_file_batch_path_count(paths)?;
    let destination = validate_remote_mutating_path(destination)?;
    let destination = destination.trim_end_matches('/').to_string();
    reject_remote_symlink_components(sftp, &destination, false, "远端移动目标目录").await?;
    let destination_metadata = sftp
        .symlink_metadata(destination.clone())
        .await
        .map_err(|error| format!("SFTP 读取远端移动目标目录失败 {destination}: {error}"))?;
    if !destination_metadata.is_dir() || destination_metadata.is_symlink() {
        return Err(format!("远端移动目标不是普通目录: {destination}"));
    }

    let mut sources = HashSet::new();
    let mut targets = HashSet::new();
    let mut plan = Vec::with_capacity(paths.len());
    for raw_path in paths {
        let source = validate_remote_mutating_path(raw_path)?;
        let source = source.trim_end_matches('/').to_string();
        if !sources.insert(source.clone()) {
            return Err(format!("移动操作包含重复源路径: {source}"));
        }
        reject_remote_symlink_components(sftp, &source, true, "远端移动源路径").await?;
        let metadata = sftp
            .symlink_metadata(source.clone())
            .await
            .map_err(|error| format!("SFTP 读取远端移动源路径失败 {source}: {error}"))?;
        let name = portable_file_name(&source)
            .ok_or_else(|| format!("远端移动源路径没有文件名: {source}"))?;
        let target = remote_join_path(&destination, &name);
        if source == target {
            return Err(format!("源路径已位于移动目标目录: {source}"));
        }
        if metadata.is_dir()
            && !metadata.is_symlink()
            && remote_path_is_within(&destination, &source)
        {
            return Err(format!(
                "拒绝将目录移动到其自身内部: {source} -> {destination}"
            ));
        }
        reject_remote_symlink_components(sftp, &target, false, "远端移动目标路径").await?;
        if !targets.insert(target.clone()) {
            return Err(format!("移动操作包含冲突目标路径: {target}"));
        }
        ensure_remote_path_missing(sftp, &target, "远端移动目标路径").await?;
        plan.push(RemoteMovePath {
            source,
            target,
            source_is_dir: metadata.is_dir() && !metadata.is_symlink(),
        });
    }

    for directory in plan.iter().filter(|item| item.source_is_dir) {
        if let Some(nested) = plan.iter().find(|item| {
            item.source != directory.source
                && remote_path_is_within(&item.source, &directory.source)
        }) {
            return Err(format!(
                "移动操作不能同时包含目录及其子项: {} 和 {}",
                directory.source, nested.source
            ));
        }
    }
    Ok(plan)
}

pub(super) async fn move_paths_inner(
    state: &AppState,
    request: MovePathsRequest,
) -> Result<(), String> {
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote move requires sessionId".to_string())?;
        let auxiliary = ssh_auxiliary_lease(state, session_id)?;
        let sftp = auxiliary.sftp().await?;
        let result = async {
            let plan = prepare_remote_move_paths(&sftp, &request.paths, &request.destination).await?;
            for (completed, item) in plan.into_iter().enumerate() {
                sftp.rename(item.source.clone(), item.target.clone())
                    .await
                    .map_err(|error| {
                        format!(
                            "SFTP 移动失败 {} -> {}: {error}; 已完成 {completed} 项，请刷新目录确认当前状态",
                            item.source, item.target
                        )
                    })?;
            }
            Ok(())
        }
        .await;
        result
    } else {
        let plan = prepare_local_move_paths(&request.paths, &request.destination)?;
        for (completed, item) in plan.into_iter().enumerate() {
            rename_local_path_without_overwrite(&item.source, &item.target).map_err(|error| {
                format!(
                    "本地移动失败 {} -> {}: {error}; 已完成 {completed} 项，请刷新目录确认当前状态",
                    item.source.display(),
                    item.target.display()
                )
            })?;
        }
        Ok(())
    }
}

pub(super) async fn rename_path_inner(
    state: &AppState,
    request: RenamePathRequest,
) -> Result<(), String> {
    if request.old_path.trim().is_empty() || request.new_path.trim().is_empty() {
        return Err("重命名路径不能为空".to_string());
    }
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote rename requires sessionId".to_string())?;
        let old_path = validate_remote_mutating_path(&request.old_path)?;
        let new_path = validate_remote_mutating_path(&request.new_path)?;
        if old_path.trim_end_matches('/') == new_path.trim_end_matches('/') {
            return Ok(());
        }
        let auxiliary = ssh_auxiliary_lease(state, session_id)?;
        let sftp = auxiliary.sftp().await?;
        let result = async {
            reject_remote_symlink_components(&sftp, &old_path, true, "远端重命名源路径").await?;
            reject_remote_symlink_components(&sftp, &new_path, false, "远端重命名目标路径").await?;
            ensure_remote_path_missing(&sftp, &new_path, "远端重命名目标路径").await?;
            sftp.rename(old_path.clone(), new_path.clone())
                .await
                .map_err(|error| format!("SFTP 重命名失败 {} -> {}: {error}", old_path, new_path))
        }
        .await;
        result
    } else {
        let old_path = validate_local_mutating_path(&request.old_path)?;
        let new_path = validate_local_mutating_path(&request.new_path)?;
        if old_path == new_path {
            return Ok(());
        }
        reject_local_symlink_components(&old_path, true, "本地重命名源路径")?;
        reject_local_symlink_components(&new_path, false, "本地重命名目标路径")?;
        ensure_local_path_missing(&new_path, "本地重命名目标路径")?;
        rename_local_path_without_overwrite(&old_path, &new_path).map_err(|error| {
            format!(
                "本地重命名失败 {} -> {}: {error}",
                old_path.display(),
                new_path.display()
            )
        })
    }
}

pub(super) async fn chmod_path_inner(
    state: &AppState,
    request: ChmodPathRequest,
) -> Result<(), String> {
    if request.path.trim().is_empty() {
        return Err("权限路径不能为空".to_string());
    }
    if request.mode > 0o7777 {
        return Err("权限模式必须是 0000-7777 八进制范围".to_string());
    }
    if request.remote {
        let session_id = request
            .session_id
            .as_deref()
            .ok_or_else(|| "remote chmod requires sessionId".to_string())?;
        let path = validate_remote_mutating_path(&request.path)?;
        let auxiliary = ssh_auxiliary_lease(state, session_id)?;
        let sftp = auxiliary.sftp().await?;
        let result = async {
            reject_remote_symlink_components(&sftp, &path, false, "远端 chmod 路径").await?;
            let mut metadata = sftp
                .symlink_metadata(path.clone())
                .await
                .map_err(|error| format!("SFTP 读取权限失败 {path}: {error}"))?;
            if metadata.is_symlink() {
                return Err(format!("拒绝修改远端符号链接权限: {path}"));
            }
            let file_type_bits = metadata.permissions.unwrap_or(0) & 0o170000;
            metadata.permissions = Some(file_type_bits | request.mode);
            sftp.set_metadata(path.clone(), metadata)
                .await
                .map_err(|error| format!("SFTP 设置权限失败 {path}: {error}"))
        }
        .await;
        result
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = validate_local_mutating_path(&request.path)?;
            reject_local_symlink_components(&path, false, "本地 chmod 路径")?;
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("读取本地权限失败 {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!("拒绝修改本地符号链接权限: {}", path.display()));
            }
            let mut permissions = metadata.permissions();
            permissions.set_mode(request.mode);
            fs::set_permissions(&path, permissions)
                .map_err(|error| format!("设置本地权限失败 {}: {error}", path.display()))
        }
        #[cfg(not(unix))]
        {
            let _ = state;
            Err("当前平台不支持本地 chmod".to_string())
        }
    }
}

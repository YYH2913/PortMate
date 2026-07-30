use super::*;

pub(super) fn ensure_local_path_missing(path: &Path, label: &str) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!("{label}已存在: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("检查{label}失败 {}: {error}", path.display())),
    }
}

pub(super) async fn ensure_remote_path_missing(
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

pub(super) fn validate_file_batch_path_count(paths: &[String]) -> Result<(), String> {
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

pub(super) fn canonical_local_entry_path(path: &Path, label: &str) -> Result<PathBuf, String> {
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

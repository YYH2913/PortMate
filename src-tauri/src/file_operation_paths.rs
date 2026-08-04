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
    let trimmed = path.trim_end_matches(['/', '\\']);
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
    let name = without_drive.rsplit(['/', '\\']).next().unwrap_or_default();
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

pub(super) fn validate_remote_mutating_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim_end_matches('/');
    if path.trim().is_empty()
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

/// Guards local mutations against empty/current/home/filesystem-root paths.
pub(super) fn validate_local_mutating_path(path: &str) -> Result<PathBuf, String> {
    let home = native_home_path();
    validate_local_mutating_path_with_home(path, home.as_deref())
}

pub(super) fn validate_local_mutating_path_with_home(
    path: &str,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    let trimmed_slashes = path.trim_end_matches(['/', '\\']);
    if path.trim().is_empty()
        || trimmed_slashes.is_empty()
        || matches!(trimmed_slashes, "." | ".." | "~")
        || path.contains('\0')
    {
        return Err("拒绝操作空路径、根目录或当前目录".to_string());
    }
    if path
        .split(['/', '\\'])
        .any(|component| matches!(component, "." | ".."))
    {
        return Err("拒绝包含 . 或 .. 路径分量的本地变更路径".to_string());
    }

    let candidate = validate_native_local_path(path)?;
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
    if let Some(home) = home {
        let candidate_check = candidate
            .canonicalize()
            .unwrap_or_else(|_| candidate.clone());
        let home_check = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());
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
    let home = native_home_path();
    validate_native_local_path_with_home(
        path,
        current_local_transfer_path_platform(),
        home.as_deref(),
    )
}

pub(super) fn validate_native_local_path_with_home(
    path: &str,
    platform: LocalTransferPathPlatform,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    if path.trim().is_empty() || path.contains('\0') {
        return Err("本地路径不能为空或包含 NUL".to_string());
    }
    let windows = platform == LocalTransferPathPlatform::Windows;
    if has_local_home_prefix(path, windows) {
        let relative = local_home_relative_path(path, windows)
            .ok_or_else(|| "本地 ~ 路径不能包含 Windows 盘符后缀".to_string())?;
        let home = home.ok_or_else(|| "无法解析本地 ~ 路径：系统用户主目录不可用".to_string())?;
        return Ok(home.join(relative));
    }
    match classify_local_transfer_path(path, platform) {
        LocalTransferPathKind::Relative | LocalTransferPathKind::Absolute => {
            Ok(PathBuf::from(path))
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

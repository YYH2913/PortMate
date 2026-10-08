use super::*;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileBatchPlanFile {
    pub(super) source: String,
    pub(super) relative: String,
    pub(super) size: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileBatchPlan {
    pub(super) directories: Vec<String>,
    pub(super) files: Vec<FileBatchPlanFile>,
    pub(super) skipped: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BatchTargetKind {
    Missing,
    File,
    Directory,
    Other,
}

pub(super) fn plan_local_file_batch(paths: &[String]) -> Result<FileBatchPlan, String> {
    let plan = plan_external_drop(paths, None)?;
    Ok(FileBatchPlan {
        directories: plan
            .directories
            .iter()
            .map(|path| external_relative_remote_path(path))
            .collect::<Result<Vec<_>, _>>()?,
        files: plan
            .files
            .into_iter()
            .map(|file| {
                let source = file.source.to_str().ok_or_else(|| {
                    format!("批次源路径不是有效 Unicode: {}", file.source.display())
                })?;
                Ok(FileBatchPlanFile {
                    source: source.to_string(),
                    relative: external_relative_remote_path(&file.relative)?,
                    size: file.size,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        skipped: plan.skipped,
    })
}

pub(super) async fn plan_remote_file_batch(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    paths: &[String],
) -> Result<FileBatchPlan, String> {
    let mut roots = paths
        .iter()
        .map(|path| normalize_remote_batch_source(path))
        .collect::<Result<Vec<_>, _>>()?;
    roots.sort_by_key(|path| batch_path_depth(path));
    let mut selected: Vec<String> = Vec::new();
    for path in roots {
        if !selected
            .iter()
            .any(|root| path == *root || remote_path_is_within(&path, root))
        {
            selected.push(path);
        }
    }
    let output = remote_safe_tree(handle, "plan", &selected).await?;
    let mut plan: FileBatchPlan = serde_json::from_str(&output)
        .map_err(|error| format!("invalid safe remote batch response: {error}"))?;
    if plan.files.len() > MAX_EXTERNAL_DROP_FILES
        || plan.files.len() + plan.directories.len() > MAX_EXTERNAL_DROP_ENTRIES
    {
        return Err("remote batch response exceeds entry limit".into());
    }
    for file in &plan.files {
        normalize_remote_batch_source(&file.source)?;
    }
    validate_file_batch_plan(&mut plan)?;
    Ok(plan)
}

pub(super) fn normalize_remote_batch_source(path: &str) -> Result<String, String> {
    let path = path.trim_end_matches('/');
    if path.trim().is_empty()
        || matches!(path, "." | ".." | "~")
        || path.contains('\0')
        || path == "/"
        || remote_path_has_dot_components(path)
    {
        return Err("拒绝传输空路径、当前目录或远端根目录".to_string());
    }
    Ok(path.to_string())
}

pub(super) fn remote_path_has_dot_components(path: &str) -> bool {
    path.split(['/', '\\'])
        .any(|component| matches!(component, "." | ".."))
}

pub(super) fn remote_path_is_within(path: &str, parent: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub(super) fn validate_batch_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\0')
        || path
            .chars()
            .any(|character| matches!(character, '\\' | ':'))
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return Err(format!("批次相对路径无效: {path}"));
    }
    Ok(())
}

pub(super) async fn validate_remote_batch_destination(
    sftp: &SftpBackendSession,
    path: &str,
) -> Result<String, String> {
    let path = resolve_remote_drop_destination(sftp, path).await?;
    let metadata = sftp
        .symlink_metadata(path.clone())
        .await
        .map_err(|error| format!("SFTP 读取远端目标目录失败 {path}: {error}"))?;
    if !metadata.is_dir() || metadata.is_symlink() {
        return Err(format!("远端批次目标不是普通目录: {path}"));
    }
    Ok(path)
}

pub(super) fn batch_destination_path(
    remote_destination: &str,
    local_destination: Option<&Path>,
    relative: &str,
    destination_remote: bool,
) -> Result<String, String> {
    if destination_remote {
        Ok(remote_join_path(remote_destination, relative))
    } else {
        let destination = local_destination.ok_or_else(|| "本地批次目标目录不可用".to_string())?;
        Ok(destination.join(Path::new(relative)).display().to_string())
    }
}

pub(super) async fn batch_target_kind(
    sftp: Option<&SftpBackendSession>,
    path: &str,
    remote: bool,
) -> Result<BatchTargetKind, String> {
    if remote {
        let sftp = sftp.ok_or_else(|| "远端目标检查缺少 SFTP session".to_string())?;
        let metadata = match sftp.symlink_metadata(path.to_string()).await {
            Ok(metadata) => metadata,
            Err(metadata_error) => match sftp.try_exists(path.to_string()).await {
                Ok(false) => return Ok(BatchTargetKind::Missing),
                Ok(true) => {
                    return Err(format!(
                        "无法确认远端批次目标 {path}: {metadata_error}"
                    ));
                }
                Err(exists_error) => {
                    return Err(format!(
                        "无法确认远端批次目标 {path}: {metadata_error}; existence check failed: {exists_error}"
                    ));
                }
            },
        };
        Ok(if metadata.is_symlink() {
            BatchTargetKind::Other
        } else if metadata.is_dir() {
            BatchTargetKind::Directory
        } else if metadata.is_regular() {
            BatchTargetKind::File
        } else {
            BatchTargetKind::Other
        })
    } else {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() => Ok(BatchTargetKind::Other),
            Ok(metadata) if metadata.is_dir() => Ok(BatchTargetKind::Directory),
            Ok(metadata) if metadata.is_file() => Ok(BatchTargetKind::File),
            Ok(_) => Ok(BatchTargetKind::Other),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(BatchTargetKind::Missing)
            }
            Err(error) => Err(format!("读取本地批次目标失败 {path}: {error}")),
        }
    }
}

pub(super) fn numbered_batch_relative_path(path: &str, suffix: u32) -> Result<String, String> {
    validate_batch_relative_path(path)?;
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    let (stem, extension) = name
        .rsplit_once('.')
        .filter(|(stem, extension)| !stem.is_empty() && !extension.is_empty())
        .unwrap_or((name, ""));
    let renamed = if extension.is_empty() {
        format!("{stem} ({suffix})")
    } else {
        format!("{stem} ({suffix}).{extension}")
    };
    Ok(if parent.is_empty() {
        renamed
    } else {
        format!("{parent}/{renamed}")
    })
}

pub(super) fn batch_path_depth(path: &str) -> usize {
    path.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .count()
}

pub(super) fn validate_file_batch_plan(plan: &mut FileBatchPlan) -> Result<(), String> {
    plan.directories.sort_by(|left, right| {
        batch_path_depth(left)
            .cmp(&batch_path_depth(right))
            .then_with(|| left.cmp(right))
    });
    if let Some(conflict) = plan
        .directories
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| &pair[0])
    {
        return Err(format!("文件批次包含冲突的目标目录: {conflict}"));
    }
    plan.files
        .sort_by(|left, right| left.relative.cmp(&right.relative));
    let directories = plan.directories.iter().collect::<HashSet<_>>();
    let mut files = HashSet::new();
    for file in &plan.files {
        if directories.contains(&file.relative) || !files.insert(file.relative.as_str()) {
            return Err(format!("文件批次包含冲突的目标路径: {}", file.relative));
        }
    }
    plan.skipped.sort();
    plan.skipped.dedup();
    Ok(())
}

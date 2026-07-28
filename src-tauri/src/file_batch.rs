use super::*;

pub(super) const MAX_EXTERNAL_DROP_ROOTS: usize = 512;
pub(super) const MAX_EXTERNAL_DROP_ENTRIES: usize = 20_000;
pub(super) const MAX_EXTERNAL_DROP_FILES: usize = MAX_ACTIVE_TRANSFERS_PER_SESSION;

#[derive(Debug)]
pub(super) struct ExternalDropRoot {
    pub(super) path: PathBuf,
    pub(super) is_dir: bool,
}

#[derive(Debug)]
pub(super) struct ExternalDropFile {
    pub(super) source: PathBuf,
    pub(super) relative: PathBuf,
}

#[derive(Debug, Default)]
pub(super) struct ExternalDropPlan {
    pub(super) directories: Vec<PathBuf>,
    pub(super) files: Vec<ExternalDropFile>,
    pub(super) skipped: Vec<String>,
    pub(super) total_bytes: u64,
}

#[derive(Debug)]
pub(super) struct FileBatchPlanFile {
    pub(super) source: String,
    pub(super) relative: String,
    pub(super) size: u64,
}

#[derive(Debug, Default)]
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

pub(super) async fn start_file_batch_inner(
    state: &AppState,
    request: StartFileBatchRequest,
) -> Result<ExternalDropResult, String> {
    if request.source_remote == request.destination_remote {
        return Err("文件批次必须在本地与远端面板之间传输".to_string());
    }
    {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if store.profile(&request.session_id).is_none() {
            return Err(format!("unknown session: {}", request.session_id));
        }
    }
    if request.paths.is_empty() {
        return Err("文件批次没有源路径".to_string());
    }
    if request.paths.len() > MAX_EXTERNAL_DROP_ROOTS {
        return Err(format!(
            "一次最多选择 {MAX_EXTERNAL_DROP_ROOTS} 个顶层路径，当前为 {} 个",
            request.paths.len()
        ));
    }

    let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
    let sftp = auxiliary.sftp().await?;
    let result = async {
        let mut plan = if request.source_remote {
            plan_remote_file_batch(&sftp, &request.paths).await?
        } else {
            plan_local_file_batch(&request.paths)?
        };
        let local_destination = if request.destination_remote {
            validate_remote_batch_destination(&sftp, &request.destination).await?;
            None
        } else {
            Some(validate_local_drop_destination(&request.destination)?)
        };

        let mut directory_targets = Vec::with_capacity(plan.directories.len());
        for relative in &plan.directories {
            validate_batch_relative_path(relative)?;
            let target = batch_destination_path(
                request.destination.trim(),
                local_destination.as_deref(),
                relative,
                request.destination_remote,
            )?;
            match batch_target_kind(Some(&sftp), &target, request.destination_remote).await? {
                BatchTargetKind::Missing => directory_targets.push(target),
                BatchTargetKind::Directory => {}
                BatchTargetKind::File | BatchTargetKind::Other => {
                    return Err(format!("目标目录路径已被非目录占用: {target}"));
                }
            }
        }

        let mut reserved_targets = HashSet::new();
        let mut prepared_files = Vec::with_capacity(plan.files.len());
        let mut total_bytes = 0_u64;
        for file in plan.files {
            validate_batch_relative_path(&file.relative)?;
            let mut relative = file.relative.clone();
            let mut target = batch_destination_path(
                request.destination.trim(),
                local_destination.as_deref(),
                &relative,
                request.destination_remote,
            )?;
            let kind = batch_target_kind(Some(&sftp), &target, request.destination_remote).await?;
            let conflict =
                kind != BatchTargetKind::Missing || !reserved_targets.insert(target.clone());
            if conflict {
                match request.conflict_policy {
                    TransferConflictPolicy::Fail => {
                        return Err(format!("目标文件已存在或批次内冲突: {target}"));
                    }
                    TransferConflictPolicy::Skip => {
                        plan.skipped
                            .push(format!("{} (destination exists: {target})", file.source));
                        continue;
                    }
                    TransferConflictPolicy::Overwrite => {
                        if !matches!(kind, BatchTargetKind::File | BatchTargetKind::Missing) {
                            return Err(format!("拒绝用文件覆盖非普通文件目标: {target}"));
                        }
                        reserved_targets.insert(target.clone());
                    }
                    TransferConflictPolicy::Rename => {
                        let mut renamed = None;
                        for suffix in 1..=10_000_u32 {
                            let candidate_relative =
                                numbered_batch_relative_path(&file.relative, suffix)?;
                            let candidate = batch_destination_path(
                                request.destination.trim(),
                                local_destination.as_deref(),
                                &candidate_relative,
                                request.destination_remote,
                            )?;
                            if !reserved_targets.contains(&candidate)
                                && batch_target_kind(
                                    Some(&sftp),
                                    &candidate,
                                    request.destination_remote,
                                )
                                .await?
                                    == BatchTargetKind::Missing
                            {
                                renamed = Some((candidate_relative, candidate));
                                break;
                            }
                        }
                        let (candidate_relative, candidate) = renamed
                            .ok_or_else(|| format!("无法为冲突目标生成可用名称: {target}"))?;
                        relative = candidate_relative;
                        target = candidate;
                        reserved_targets.insert(target.clone());
                    }
                }
            }
            total_bytes = total_bytes
                .checked_add(file.size)
                .ok_or_else(|| "文件批次总大小溢出".to_string())?;
            prepared_files.push((file.source, relative, target));
        }

        ensure_transfer_batch_capacity(state, &request.session_id, prepared_files.len())?;

        directory_targets.sort_by_key(|path| batch_path_depth(path));
        for target in &directory_targets {
            if request.destination_remote {
                sftp_create_dir_all(&sftp, target).await?;
            } else {
                fs::create_dir_all(target)
                    .map_err(|error| format!("创建本地批次目录失败 {target}: {error}"))?;
            }
        }

        Ok::<_, String>((
            prepared_files,
            directory_targets.len(),
            plan.skipped,
            total_bytes,
        ))
    }
    .await;
    let (prepared_files, directories_prepared, mut skipped, total_bytes) = result?;

    let mut tasks = Vec::with_capacity(prepared_files.len());
    for (source, _relative, target) in prepared_files {
        let source = if request.source_remote {
            format!("remote:{source}")
        } else {
            source
        };
        let destination = if request.destination_remote {
            format!("remote:{target}")
        } else {
            target
        };
        tasks.push(
            start_transfer_inner(
                state,
                StartTransferRequest {
                    session_id: request.session_id.clone(),
                    protocol: TransferProtocol::Sftp,
                    source,
                    destination,
                },
            )
            .await?,
        );
    }
    skipped.sort();
    skipped.dedup();
    Ok(ExternalDropResult {
        tasks,
        directories_prepared,
        skipped,
        total_bytes,
    })
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
                    size: fs::metadata(&file.source)
                        .map_err(|error| {
                            format!("读取批次源文件失败 {}: {error}", file.source.display())
                        })?
                        .len(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        skipped: plan.skipped,
    })
}

pub(super) async fn plan_remote_file_batch(
    sftp: &SftpBackendSession,
    paths: &[String],
) -> Result<FileBatchPlan, String> {
    let mut roots = Vec::new();
    let mut skipped = Vec::new();
    for raw in paths {
        let path = normalize_remote_batch_source(raw)?;
        if roots.iter().any(|existing: &(String, bool)| {
            path == existing.0 || (existing.1 && remote_path_is_within(&path, &existing.0))
        }) {
            skipped.push(format!("{path} (already included)"));
            continue;
        }
        let metadata = sftp
            .symlink_metadata(path.clone())
            .await
            .map_err(|error| format!("SFTP 读取批次源路径失败 {path}: {error}"))?;
        if metadata.is_symlink() {
            skipped.push(format!("{path} (symbolic link)"));
            continue;
        }
        if !metadata.is_dir() && !metadata.is_regular() {
            skipped.push(format!("{path} (not a regular file or directory)"));
            continue;
        }
        roots.push((path, metadata.is_dir()));
    }

    let mut plan = FileBatchPlan {
        skipped,
        ..FileBatchPlan::default()
    };
    let mut visited = 0_usize;
    for (root, _) in roots {
        let root_name = remote_file_name(&root);
        validate_batch_relative_path(&root_name)?;
        let mut stack = vec![(root, root_name)];
        while let Some((source, relative)) = stack.pop() {
            visited += 1;
            if visited > MAX_EXTERNAL_DROP_ENTRIES {
                return Err(format!(
                    "远端目录超过 {MAX_EXTERNAL_DROP_ENTRIES} 个条目，请缩小批次"
                ));
            }
            let metadata = sftp
                .symlink_metadata(source.clone())
                .await
                .map_err(|error| format!("SFTP 读取远端目录项失败 {source}: {error}"))?;
            if metadata.is_symlink() {
                plan.skipped.push(format!("{source} (symbolic link)"));
                continue;
            }
            if metadata.is_dir() {
                plan.directories.push(relative.clone());
                let mut children = sftp
                    .read_dir(source.clone())
                    .await
                    .map_err(|error| format!("SFTP 读取远端目录失败 {source}: {error}"))?
                    .collect::<Vec<_>>();
                children.sort_by_key(|entry| entry.file_name());
                for child in children.into_iter().rev() {
                    let name = child.file_name();
                    if matches!(name.as_str(), "." | "..") {
                        continue;
                    }
                    validate_batch_relative_path(&name)?;
                    stack.push((
                        remote_join_path(&source, &name),
                        remote_join_path(&relative, &name),
                    ));
                }
            } else if metadata.is_regular() {
                if plan.files.len() >= MAX_EXTERNAL_DROP_FILES {
                    return Err(format!(
                        "一次最多传输 {MAX_EXTERNAL_DROP_FILES} 个文件，请缩小批次"
                    ));
                }
                plan.files.push(FileBatchPlanFile {
                    source,
                    relative,
                    size: metadata.len(),
                });
            } else {
                plan.skipped
                    .push(format!("{source} (not a regular file or directory)"));
            }
        }
    }
    validate_file_batch_plan(&mut plan)?;
    Ok(plan)
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

pub(super) fn normalize_remote_batch_source(path: &str) -> Result<String, String> {
    let path = path.trim().trim_end_matches('/');
    if path.is_empty()
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
    path.split('/')
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
) -> Result<(), String> {
    validate_remote_drop_destination(path)?;
    let metadata = sftp
        .symlink_metadata(path.trim().to_string())
        .await
        .map_err(|error| format!("SFTP 读取远端目标目录失败 {}: {error}", path.trim()))?;
    if !metadata.is_dir() || metadata.is_symlink() {
        return Err(format!("远端批次目标不是普通目录: {}", path.trim()));
    }
    Ok(())
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
        let Ok(metadata) = sftp.symlink_metadata(path.to_string()).await else {
            return Ok(BatchTargetKind::Missing);
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
        .map_or((name, ""), |parts| parts);
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

pub(super) async fn apply_external_drop_conflicts(
    plan: &mut ExternalDropPlan,
    sftp: Option<&SftpBackendSession>,
    destination: &str,
    local_destination: Option<&Path>,
    remote: bool,
    policy: TransferConflictPolicy,
) -> Result<(), String> {
    for relative in &plan.directories {
        let relative = external_relative_remote_path(relative)?;
        let target = batch_destination_path(destination, local_destination, &relative, remote)?;
        match batch_target_kind(sftp, &target, remote).await? {
            BatchTargetKind::Missing | BatchTargetKind::Directory => {}
            BatchTargetKind::File | BatchTargetKind::Other => {
                return Err(format!("目标目录路径已被非目录占用: {target}"));
            }
        }
    }

    let mut prepared = Vec::with_capacity(plan.files.len());
    let mut reserved = HashSet::new();
    let mut total_bytes = 0_u64;
    for mut file in std::mem::take(&mut plan.files) {
        let original_relative = external_relative_remote_path(&file.relative)?;
        let mut relative = original_relative.clone();
        let mut target = batch_destination_path(destination, local_destination, &relative, remote)?;
        let kind = batch_target_kind(sftp, &target, remote).await?;
        let conflict = kind != BatchTargetKind::Missing || !reserved.insert(target.clone());
        if conflict {
            match policy {
                TransferConflictPolicy::Fail => {
                    return Err(format!("目标文件已存在或批次内冲突: {target}"));
                }
                TransferConflictPolicy::Skip => {
                    plan.skipped.push(format!(
                        "{} (destination exists: {target})",
                        file.source.display()
                    ));
                    continue;
                }
                TransferConflictPolicy::Overwrite => {
                    if !matches!(kind, BatchTargetKind::File | BatchTargetKind::Missing) {
                        return Err(format!("拒绝用文件覆盖非普通文件目标: {target}"));
                    }
                    reserved.insert(target.clone());
                }
                TransferConflictPolicy::Rename => {
                    let mut renamed = None;
                    for suffix in 1..=10_000_u32 {
                        let candidate_relative =
                            numbered_batch_relative_path(&original_relative, suffix)?;
                        let candidate = batch_destination_path(
                            destination,
                            local_destination,
                            &candidate_relative,
                            remote,
                        )?;
                        if !reserved.contains(&candidate)
                            && batch_target_kind(sftp, &candidate, remote).await?
                                == BatchTargetKind::Missing
                        {
                            renamed = Some((candidate_relative, candidate));
                            break;
                        }
                    }
                    let (candidate_relative, candidate) =
                        renamed.ok_or_else(|| format!("无法为冲突目标生成可用名称: {target}"))?;
                    relative = candidate_relative;
                    target = candidate;
                    reserved.insert(target);
                    file.relative = PathBuf::from(relative);
                }
            }
        }
        let size = fs::metadata(&file.source)
            .map_err(|error| format!("读取拖放源文件失败 {}: {error}", file.source.display()))?
            .len();
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| "拖放批次总大小溢出".to_string())?;
        prepared.push(file);
    }
    plan.files = prepared;
    plan.total_bytes = total_bytes;
    Ok(())
}

pub(super) async fn start_external_drop_inner(
    state: &AppState,
    request: StartExternalDropRequest,
) -> Result<ExternalDropResult, String> {
    {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        if store.profile(&request.session_id).is_none() {
            return Err(format!("unknown session: {}", request.session_id));
        }
    }

    let local_destination = if request.remote {
        validate_remote_drop_destination(&request.destination)?;
        None
    } else {
        Some(validate_local_drop_destination(&request.destination)?)
    };
    let mut plan = plan_external_drop(&request.paths, local_destination.as_deref())?;

    if request.remote {
        if !plan.directories.is_empty() || !plan.files.is_empty() {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let sftp = auxiliary.sftp().await?;
            let result = async {
                apply_external_drop_conflicts(
                    &mut plan,
                    Some(&sftp),
                    request.destination.trim(),
                    None,
                    true,
                    request.conflict_policy,
                )
                .await?;
                ensure_transfer_batch_capacity(state, &request.session_id, plan.files.len())?;
                sftp_create_dir_all(&sftp, request.destination.trim()).await?;
                for relative in &plan.directories {
                    let target = remote_join_path(
                        request.destination.trim(),
                        &external_relative_remote_path(relative)?,
                    );
                    sftp_create_dir_all(&sftp, &target).await?;
                }
                Ok::<(), String>(())
            }
            .await;
            result?;
        }
    } else if let Some(destination) = &local_destination {
        apply_external_drop_conflicts(
            &mut plan,
            None,
            &destination.display().to_string(),
            Some(destination),
            false,
            request.conflict_policy,
        )
        .await?;
        ensure_transfer_batch_capacity(state, &request.session_id, plan.files.len())?;
        for relative in &plan.directories {
            let target = destination.join(relative);
            fs::create_dir_all(&target)
                .map_err(|error| format!("创建拖放目标目录失败 {}: {error}", target.display()))?;
        }
    }

    let directories_prepared = plan.directories.len();
    let total_bytes = plan.total_bytes;
    let skipped = plan.skipped;
    let mut tasks = Vec::with_capacity(plan.files.len());
    for file in plan.files {
        let source = file
            .source
            .to_str()
            .ok_or_else(|| format!("拖放源路径不是有效 Unicode: {}", file.source.display()))?
            .to_string();
        let destination = if request.remote {
            format!(
                "remote:{}",
                remote_join_path(
                    request.destination.trim(),
                    &external_relative_remote_path(&file.relative)?,
                )
            )
        } else {
            local_destination
                .as_ref()
                .expect("local drop destination must be available")
                .join(&file.relative)
                .display()
                .to_string()
        };
        tasks.push(
            start_transfer_inner(
                state,
                StartTransferRequest {
                    session_id: request.session_id.clone(),
                    protocol: TransferProtocol::Sftp,
                    source,
                    destination,
                },
            )
            .await?,
        );
    }

    Ok(ExternalDropResult {
        tasks,
        directories_prepared,
        skipped,
        total_bytes,
    })
}

pub(super) fn validate_remote_drop_destination(path: &str) -> Result<(), String> {
    let path = path.trim().trim_end_matches('/');
    if path.is_empty()
        || matches!(path, "." | ".." | "~")
        || path.contains('\0')
        || path == "/"
        || remote_path_has_dot_components(path)
    {
        return Err("远端拖放目标路径不能为空、包含 NUL、使用 . / .. 分量或指向根目录".to_string());
    }
    Ok(())
}

pub(super) fn validate_local_drop_destination(path: &str) -> Result<PathBuf, String> {
    let destination = validate_native_local_path(path)?;
    let metadata = fs::metadata(&destination).map_err(|error| {
        format!(
            "读取本地拖放目标目录失败 {}: {error}",
            destination.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!("本地拖放目标不是目录: {}", destination.display()));
    }
    destination
        .canonicalize()
        .map_err(|error| format!("解析本地拖放目标失败 {}: {error}", destination.display()))
}

pub(super) fn plan_external_drop(
    paths: &[String],
    local_destination: Option<&Path>,
) -> Result<ExternalDropPlan, String> {
    if paths.is_empty() {
        return Err("拖放批次没有源路径".to_string());
    }
    if paths.len() > MAX_EXTERNAL_DROP_ROOTS {
        return Err(format!(
            "一次最多拖放 {MAX_EXTERNAL_DROP_ROOTS} 个顶层路径，当前为 {} 个",
            paths.len()
        ));
    }

    let mut candidates = Vec::with_capacity(paths.len());
    let mut skipped = Vec::new();
    for raw_path in paths {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() || trimmed.contains('\0') {
            return Err("拖放源路径不能为空或包含 NUL".to_string());
        }
        let source = expand_identity_path(trimmed);
        let metadata = fs::symlink_metadata(&source)
            .map_err(|error| format!("读取拖放源路径失败 {}: {error}", source.display()))?;
        if metadata.file_type().is_symlink() {
            skipped.push(format!("{} (symbolic link)", source.display()));
            continue;
        }
        if !metadata.is_dir() && !metadata.is_file() {
            skipped.push(format!(
                "{} (not a regular file or directory)",
                source.display()
            ));
            continue;
        }
        let source = source
            .canonicalize()
            .map_err(|error| format!("解析拖放源路径失败 {}: {error}", source.display()))?;
        candidates.push(ExternalDropRoot {
            path: source,
            is_dir: metadata.is_dir(),
        });
    }
    candidates.sort_by(|left, right| {
        left.path
            .components()
            .count()
            .cmp(&right.path.components().count())
            .then_with(|| left.path.cmp(&right.path))
    });

    let mut roots: Vec<ExternalDropRoot> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if roots.iter().any(|root| {
            candidate.path == root.path || (root.is_dir && candidate.path.starts_with(&root.path))
        }) {
            skipped.push(format!("{} (already included)", candidate.path.display()));
            continue;
        }
        if candidate.is_dir
            && local_destination.is_some_and(|destination| destination.starts_with(&candidate.path))
        {
            return Err(format!(
                "拒绝把目录复制到自身或其子目录: {}",
                candidate.path.display()
            ));
        }
        if let Some(destination) = local_destination {
            if let Some(name) = candidate.path.file_name() {
                let target = destination.join(name);
                if target.exists()
                    && target
                        .canonicalize()
                        .is_ok_and(|target| target == candidate.path)
                {
                    return Err(format!(
                        "拒绝把路径复制到自身: {}",
                        candidate.path.display()
                    ));
                }
            }
        }
        roots.push(candidate);
    }

    let mut plan = ExternalDropPlan {
        skipped,
        ..ExternalDropPlan::default()
    };
    let mut visited = 0_usize;
    for root in roots {
        let Some(root_name) = root.path.file_name().map(|name| name.to_os_string()) else {
            return Err(format!("拒绝拖放文件系统根路径: {}", root.path.display()));
        };
        if root_name.to_str().is_none() {
            plan.skipped.push(format!(
                "{} (path is not valid Unicode)",
                root.path.display()
            ));
            continue;
        }
        let mut stack = vec![(root.path, PathBuf::from(root_name))];
        while let Some((source, relative)) = stack.pop() {
            visited += 1;
            if visited > MAX_EXTERNAL_DROP_ENTRIES {
                return Err(format!(
                    "拖放目录超过 {MAX_EXTERNAL_DROP_ENTRIES} 个条目，请缩小批次"
                ));
            }
            if external_relative_remote_path(&relative).is_err() {
                plan.skipped
                    .push(format!("{} (path is not valid Unicode)", source.display()));
                continue;
            }
            let metadata = fs::symlink_metadata(&source)
                .map_err(|error| format!("读取拖放目录项失败 {}: {error}", source.display()))?;
            if metadata.file_type().is_symlink() {
                plan.skipped
                    .push(format!("{} (symbolic link)", source.display()));
                continue;
            }
            if metadata.is_dir() {
                plan.directories.push(relative.clone());
                let mut children = fs::read_dir(&source)
                    .map_err(|error| format!("读取拖放目录失败 {}: {error}", source.display()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| format!("读取拖放目录项失败 {}: {error}", source.display()))?;
                children.sort_by_key(|entry| entry.file_name());
                for child in children.into_iter().rev() {
                    stack.push((child.path(), relative.join(child.file_name())));
                }
            } else if metadata.is_file() {
                if let Some(destination) = local_destination {
                    let target = destination.join(&relative);
                    if target.exists() && target.canonicalize().is_ok_and(|target| target == source)
                    {
                        return Err(format!("拒绝把文件复制到自身: {}", source.display()));
                    }
                }
                plan.total_bytes = plan
                    .total_bytes
                    .checked_add(metadata.len())
                    .ok_or_else(|| "拖放批次总大小溢出".to_string())?;
                if plan.files.len() >= MAX_EXTERNAL_DROP_FILES {
                    return Err(format!(
                        "一次最多拖放 {MAX_EXTERNAL_DROP_FILES} 个文件，请缩小批次"
                    ));
                }
                plan.files.push(ExternalDropFile { source, relative });
            } else {
                plan.skipped.push(format!(
                    "{} (not a regular file or directory)",
                    source.display()
                ));
            }
        }
    }

    plan.directories.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    if let Some(conflict) = plan
        .directories
        .windows(2)
        .find(|pair| pair[0] == pair[1])
        .map(|pair| &pair[0])
    {
        return Err(format!(
            "拖放批次包含冲突的目标目录: {}",
            conflict.display()
        ));
    }
    plan.files
        .sort_by(|left, right| left.relative.cmp(&right.relative));
    let directory_targets = plan.directories.iter().cloned().collect::<HashSet<_>>();
    let mut file_targets = HashSet::new();
    for file in &plan.files {
        if directory_targets.contains(&file.relative) || !file_targets.insert(file.relative.clone())
        {
            return Err(format!(
                "拖放批次包含冲突的目标路径: {}",
                file.relative.display()
            ));
        }
    }
    plan.skipped.sort();
    plan.skipped.dedup();
    Ok(plan)
}

pub(super) fn external_relative_remote_path(path: &Path) -> Result<String, String> {
    let mut remote = String::new();
    for component in path.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(format!("拖放相对路径无效: {}", path.display()));
        };
        let part = part
            .to_str()
            .ok_or_else(|| format!("拖放路径不是有效 Unicode: {}", path.display()))?;
        remote = remote_join_path(&remote, part);
    }
    if remote.is_empty() {
        Err("拖放相对路径不能为空".to_string())
    } else {
        Ok(remote)
    }
}

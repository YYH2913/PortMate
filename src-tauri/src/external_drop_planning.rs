use super::*;

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

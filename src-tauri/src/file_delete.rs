use super::*;

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

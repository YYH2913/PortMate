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

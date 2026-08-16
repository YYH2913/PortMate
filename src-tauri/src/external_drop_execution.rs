use super::*;

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
        None
    } else {
        Some(validate_local_drop_destination(&request.destination)?)
    };
    let mut plan = plan_external_drop(&request.paths, local_destination.as_deref())?;
    let mut resolved_remote_destination = None;
    let mut planned_ssh_runtime_id = None;

    if request.remote {
        if !plan.directories.is_empty() || !plan.files.is_empty() {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            auxiliary.ensure_current(state, "远端拖放规划")?;
            planned_ssh_runtime_id = Some(auxiliary.runtime_id().to_string());
            let sftp = auxiliary.sftp().await?;
            let result = async {
                let destination =
                    resolve_remote_drop_destination(&sftp, &request.destination).await?;
                resolved_remote_destination = Some(destination.clone());
                apply_external_drop_conflicts(
                    &mut plan,
                    Some(&sftp),
                    &destination,
                    None,
                    true,
                    request.conflict_policy,
                )
                .await?;
                ensure_transfer_batch_capacity(state, &request.session_id, plan.files.len())?;
                sftp_create_dir_all(&sftp, &destination).await?;
                for relative in &plan.directories {
                    let target =
                        remote_join_path(&destination, &external_relative_remote_path(relative)?);
                    sftp_create_dir_all(&sftp, &target).await?;
                }
                Ok::<(), String>(())
            }
            .await;
            drop(sftp);
            auxiliary.ensure_current(state, "远端拖放规划")?;
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
                    resolved_remote_destination
                        .as_deref()
                        .unwrap_or(&request.destination),
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
        let transfer = StartTransferRequest {
            session_id: request.session_id.clone(),
            protocol: TransferProtocol::Sftp,
            source,
            destination,
        };
        tasks.push(if let Some(runtime_id) = planned_ssh_runtime_id.as_deref() {
            start_transfer_inner_for_ssh_runtime(state, transfer, runtime_id).await?
        } else {
            start_transfer_inner(state, transfer).await?
        });
    }

    Ok(ExternalDropResult {
        tasks,
        directories_prepared,
        skipped,
        total_bytes,
    })
}

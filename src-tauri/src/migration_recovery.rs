use super::*;

fn profile_secret_migration_projection_disposition(
    store: &SessionStore,
    payload: &ProfileSecretMigrationJournalPayload,
) -> ProfileSecretMigrationRecoveryDisposition {
    let mut all_before = true;
    let mut all_after = true;
    for profile in &payload.profiles {
        let Some(current) = store
            .profiles
            .iter()
            .find(|current| current.id == profile.profile_id)
        else {
            return ProfileSecretMigrationRecoveryDisposition::Conflict;
        };
        let Ok(current) = profile_secret_migration_projection(current) else {
            return ProfileSecretMigrationRecoveryDisposition::Conflict;
        };
        all_before &= current == profile.before;
        all_after &= current == profile.after;
    }
    match (all_before, all_after) {
        (true, false) => ProfileSecretMigrationRecoveryDisposition::NotCommitted,
        (false, true) => ProfileSecretMigrationRecoveryDisposition::Committed,
        _ => ProfileSecretMigrationRecoveryDisposition::Conflict,
    }
}

pub(super) fn profile_secret_migration_disposition(
    store: &SessionStore,
    journal: &LoadedProfileSecretMigrationJournal,
) -> ProfileSecretMigrationRecoveryDisposition {
    let projection = profile_secret_migration_projection_disposition(store, &journal.payload);
    match (projection, journal.state) {
        (
            ProfileSecretMigrationRecoveryDisposition::NotCommitted,
            ProfileSecretMigrationJournalState::TargetWritePending
            | ProfileSecretMigrationJournalState::TargetsVerified
            | ProfileSecretMigrationJournalState::TargetCleanupPending,
        ) => ProfileSecretMigrationRecoveryDisposition::NotCommitted,
        (
            ProfileSecretMigrationRecoveryDisposition::Committed,
            ProfileSecretMigrationJournalState::ProfilesCommitted
            | ProfileSecretMigrationJournalState::SourceCleanupPending,
        ) => ProfileSecretMigrationRecoveryDisposition::Committed,
        _ => ProfileSecretMigrationRecoveryDisposition::Conflict,
    }
}

pub(super) fn profile_secret_migration_recovery_summary(
    store: &SessionStore,
    journal: &LoadedProfileSecretMigrationJournal,
    portable_vault_ready: bool,
) -> ProfileSecretMigrationRecoverySummary {
    let disposition = profile_secret_migration_disposition(store, journal);
    let conflict = disposition == ProfileSecretMigrationRecoveryDisposition::Conflict;
    let requires_portable_vault_unlock = !portable_vault_ready && !conflict;
    let message = match (disposition, requires_portable_vault_unlock) {
        (ProfileSecretMigrationRecoveryDisposition::NotCommitted, true) => {
            "Profile 仍使用原凭据；请重新解锁 portable vault 后核对并回收目标副本".to_string()
        }
        (ProfileSecretMigrationRecoveryDisposition::NotCommitted, false) => {
            "Profile 仍使用原凭据；可核对源凭据并回收未引用的目标副本".to_string()
        }
        (ProfileSecretMigrationRecoveryDisposition::Committed, true) => {
            "Profile 已切换到目标凭据；请重新解锁 portable vault 后核对并完成源清理".to_string()
        }
        (ProfileSecretMigrationRecoveryDisposition::Committed, false) => {
            "Profile 已切换到目标凭据；可核对两侧内容并完成源清理".to_string()
        }
        (ProfileSecretMigrationRecoveryDisposition::Conflict, _) => {
            "Profile 凭据投影与迁移记录冲突；已冻结两侧 secret，需要人工核对".to_string()
        }
    };
    ProfileSecretMigrationRecoverySummary {
        migration_id: journal.payload.migration_id.clone(),
        state: journal.state,
        disposition,
        target_storage: journal.payload.target_storage,
        cleanup_source: journal.payload.cleanup_source,
        profile_count: journal.payload.profiles.len(),
        secret_count: journal.payload.items.len(),
        requires_portable_vault_unlock,
        can_recover: !conflict,
        message,
        created_at: journal.created_at,
        updated_at: journal.updated_at,
    }
}

pub(super) fn migration_source_ref_is_in_flight(
    store: &SessionStore,
    payload: &ProfileSecretMigrationJournalPayload,
    source_ref: &str,
) -> bool {
    payload.profiles.iter().any(|profile| {
        profile_secret_projection_ref_counts(&profile.before).contains_key(source_ref)
            && store.runtimes.iter().any(|runtime| {
                runtime.session_id == profile.profile_id
                    && matches!(
                        runtime.status,
                        SessionStatus::Connecting | SessionStatus::Reconnecting
                    )
            })
    })
}

fn profile_secret_migration_needs_resolution<JournalUpdate>(
    journal: &LoadedProfileSecretMigrationJournal,
    reason: String,
    journal_update: &mut JournalUpdate,
) -> Result<ProfileSecretMigrationRecoveryOutcome, String>
where
    JournalUpdate: FnMut(ProfileSecretMigrationJournalEvent) -> Result<(), String>,
{
    if journal.state != ProfileSecretMigrationJournalState::NeedsResolution {
        journal_update(ProfileSecretMigrationJournalEvent::Transition {
            migration_id: journal.payload.migration_id.clone(),
            state: ProfileSecretMigrationJournalState::NeedsResolution,
        })
        .map_err(|error| format!("无法冻结冲突的凭据迁移恢复记录: {error}"))?;
    }
    Ok(ProfileSecretMigrationRecoveryOutcome {
        resolved: false,
        action: "needs-resolution".to_string(),
        warnings: vec![reason],
    })
}

fn blocked_profile_secret_migration_recovery(
    message: String,
) -> ProfileSecretMigrationRecoveryOutcome {
    ProfileSecretMigrationRecoveryOutcome {
        resolved: false,
        action: "blocked".to_string(),
        warnings: vec![message],
    }
}

pub(super) fn recover_profile_secret_migration_with_io<ProbeSecret, DeleteBatch, JournalUpdate>(
    store: &SessionStore,
    journal: &LoadedProfileSecretMigrationJournal,
    mut probe_secret: ProbeSecret,
    mut delete_batch: DeleteBatch,
    mut journal_update: JournalUpdate,
) -> Result<ProfileSecretMigrationRecoveryOutcome, String>
where
    ProbeSecret: FnMut(&str) -> SecretProbeResult,
    DeleteBatch: FnMut(SecretStorage, &[String]) -> SecretBatchDeleteOutcome,
    JournalUpdate: FnMut(ProfileSecretMigrationJournalEvent) -> Result<(), String>,
{
    let migration_id = &journal.payload.migration_id;
    match profile_secret_migration_disposition(store, journal) {
        ProfileSecretMigrationRecoveryDisposition::Conflict => {
            return profile_secret_migration_needs_resolution(
                journal,
                "Profile 出现混合、缺失或第三种凭据投影；自动恢复未修改 Profile 或 provider"
                    .to_string(),
                &mut journal_update,
            );
        }
        ProfileSecretMigrationRecoveryDisposition::NotCommitted => {
            for item in &journal.payload.items {
                match probe_secret(&item.source_ref) {
                    SecretProbeResult::Present(_) => {}
                    SecretProbeResult::Missing => {
                        return profile_secret_migration_needs_resolution(
                            journal,
                            format!(
                                "Profile 仍引用源 secret，但 provider 中已缺失: {}",
                                item.source_ref
                            ),
                            &mut journal_update,
                        );
                    }
                    SecretProbeResult::Unavailable(error) => {
                        return Ok(blocked_profile_secret_migration_recovery(format!(
                            "无法核对源 secret {}: {error}",
                            item.source_ref
                        )));
                    }
                }
            }
            for item in &journal.payload.items {
                let usage_count = secret_ref_usage_count(store, &item.target_ref);
                if usage_count > 0 {
                    return profile_secret_migration_needs_resolution(
                        journal,
                        format!(
                            "未提交迁移的目标 secret 已被 {usage_count} 个当前 Profile 引用: {}",
                            item.target_ref
                        ),
                        &mut journal_update,
                    );
                }
            }
            let mut present_targets = Vec::new();
            for item in &journal.payload.items {
                match probe_secret(&item.target_ref) {
                    SecretProbeResult::Present(_) => present_targets.push(item.target_ref.clone()),
                    SecretProbeResult::Missing => {}
                    SecretProbeResult::Unavailable(error) => {
                        return Ok(blocked_profile_secret_migration_recovery(format!(
                            "无法核对目标 secret {}: {error}",
                            item.target_ref
                        )));
                    }
                }
            }
            journal_update(ProfileSecretMigrationJournalEvent::Transition {
                migration_id: migration_id.clone(),
                state: ProfileSecretMigrationJournalState::TargetCleanupPending,
            })
            .map_err(|error| format!("目标回滚前无法保存恢复 checkpoint: {error}"))?;
            let cleanup = if present_targets.is_empty() {
                SecretBatchDeleteOutcome {
                    results: BTreeMap::new(),
                    portable_vault_requires_reunlock: false,
                }
            } else {
                delete_batch(journal.payload.target_storage, &present_targets)
            };
            let mut warnings = cleanup
                .results
                .iter()
                .filter_map(|(secret_ref, result)| {
                    result
                        .as_ref()
                        .err()
                        .map(|error| format!("目标 secret {secret_ref} 回收失败: {error}"))
                })
                .collect::<Vec<_>>();
            let cleanup_complete = !cleanup.portable_vault_requires_reunlock
                && present_targets
                    .iter()
                    .all(|secret_ref| matches!(cleanup.results.get(secret_ref), Some(Ok(()))));
            if cleanup.portable_vault_requires_reunlock {
                warnings.push("portable vault 回滚提交状态需要重新锁定并解锁后核对".to_string());
            }
            if !cleanup_complete {
                return Ok(ProfileSecretMigrationRecoveryOutcome {
                    resolved: false,
                    action: "rollback-pending".to_string(),
                    warnings,
                });
            }
            if let Err(error) = journal_update(ProfileSecretMigrationJournalEvent::Clear {
                migration_id: migration_id.clone(),
            }) {
                warnings.push(format!("目标已回滚，但恢复记录清除失败: {error}"));
                return Ok(ProfileSecretMigrationRecoveryOutcome {
                    resolved: false,
                    action: "rollback-complete-journal-pending".to_string(),
                    warnings,
                });
            }
            return Ok(ProfileSecretMigrationRecoveryOutcome {
                resolved: true,
                action: "rolled-back-targets".to_string(),
                warnings,
            });
        }
        ProfileSecretMigrationRecoveryDisposition::Committed => {}
    }

    let mut target_values = HashMap::new();
    for item in &journal.payload.items {
        match probe_secret(&item.target_ref) {
            SecretProbeResult::Present(secret) => {
                target_values.insert(item.target_ref.as_str(), secret);
            }
            SecretProbeResult::Missing => {
                return profile_secret_migration_needs_resolution(
                    journal,
                    format!(
                        "Profile 已引用目标 secret，但 provider 中已缺失: {}",
                        item.target_ref
                    ),
                    &mut journal_update,
                );
            }
            SecretProbeResult::Unavailable(error) => {
                return Ok(blocked_profile_secret_migration_recovery(format!(
                    "无法核对目标 secret {}: {error}",
                    item.target_ref
                )));
            }
        }
    }

    let mut present_sources = HashSet::new();
    for item in &journal.payload.items {
        match probe_secret(&item.source_ref) {
            SecretProbeResult::Present(source) => {
                let target = target_values
                    .get(item.target_ref.as_str())
                    .expect("all target values were probed");
                if source.as_str() != target.as_str() {
                    return profile_secret_migration_needs_resolution(
                        journal,
                        format!(
                            "源与目标 secret 内容不一致，已保留两侧: {} -> {}",
                            item.source_ref, item.target_ref
                        ),
                        &mut journal_update,
                    );
                }
                present_sources.insert(item.source_ref.clone());
            }
            SecretProbeResult::Missing => {}
            SecretProbeResult::Unavailable(error) => {
                return Ok(blocked_profile_secret_migration_recovery(format!(
                    "无法核对源 secret {}: {error}",
                    item.source_ref
                )));
            }
        }
    }

    let mut deletable_sources = Vec::new();
    let mut deferred_in_flight = Vec::new();
    if journal.payload.cleanup_source {
        for item in &journal.payload.items {
            if !present_sources.contains(&item.source_ref)
                || secret_ref_usage_count(store, &item.source_ref) > 0
            {
                continue;
            }
            if migration_source_ref_is_in_flight(store, &journal.payload, &item.source_ref) {
                deferred_in_flight.push(item.source_ref.clone());
            } else {
                deletable_sources.push(item.source_ref.clone());
            }
        }
        journal_update(ProfileSecretMigrationJournalEvent::Transition {
            migration_id: migration_id.clone(),
            state: ProfileSecretMigrationJournalState::SourceCleanupPending,
        })
        .map_err(|error| format!("源清理前无法保存恢复 checkpoint: {error}"))?;
    }

    let source_storage = match journal.payload.target_storage {
        SecretStorage::Native => SecretStorage::Portable,
        SecretStorage::Portable => SecretStorage::Native,
    };
    let cleanup = if deletable_sources.is_empty() {
        SecretBatchDeleteOutcome {
            results: BTreeMap::new(),
            portable_vault_requires_reunlock: false,
        }
    } else {
        delete_batch(source_storage, &deletable_sources)
    };
    let mut warnings = cleanup
        .results
        .iter()
        .filter_map(|(secret_ref, result)| {
            result
                .as_ref()
                .err()
                .map(|error| format!("源 secret {secret_ref} 清理失败: {error}"))
        })
        .collect::<Vec<_>>();
    if !deferred_in_flight.is_empty() {
        warnings.push(format!(
            "{} 个源 secret 仍被建连中的会话使用，已延后清理",
            deferred_in_flight.len()
        ));
    }
    if cleanup.portable_vault_requires_reunlock {
        warnings.push("portable vault 清理提交状态需要重新锁定并解锁后核对".to_string());
    }
    let cleanup_complete = !cleanup.portable_vault_requires_reunlock
        && deferred_in_flight.is_empty()
        && deletable_sources
            .iter()
            .all(|secret_ref| matches!(cleanup.results.get(secret_ref), Some(Ok(()))));
    if !cleanup_complete {
        return Ok(ProfileSecretMigrationRecoveryOutcome {
            resolved: false,
            action: "source-cleanup-pending".to_string(),
            warnings,
        });
    }
    if let Err(error) = journal_update(ProfileSecretMigrationJournalEvent::Clear {
        migration_id: migration_id.clone(),
    }) {
        warnings.push(format!("迁移已核对完成，但恢复记录清除失败: {error}"));
        return Ok(ProfileSecretMigrationRecoveryOutcome {
            resolved: false,
            action: "cleanup-complete-journal-pending".to_string(),
            warnings,
        });
    }
    Ok(ProfileSecretMigrationRecoveryOutcome {
        resolved: true,
        action: "finalized-source-cleanup".to_string(),
        warnings,
    })
}

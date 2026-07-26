use super::*;

#[cfg(test)]
pub(super) fn migrate_profile_secrets_with_io<ReadSecret, WriteBatch, DeleteBatch, PersistStore>(
    store: &mut SessionStore,
    request: &ProfileSecretMigrationRequest,
    read_secret: ReadSecret,
    write_batch: WriteBatch,
    delete_batch: DeleteBatch,
    mut persist_store: PersistStore,
) -> Result<ProfileSecretMigrationResponse, String>
where
    ReadSecret: FnMut(&str) -> Result<String, String>,
    WriteBatch: FnMut(SecretStorage, &[PreparedProfileSecretMigration]) -> Result<bool, String>,
    DeleteBatch: FnMut(SecretStorage, &[String]) -> SecretBatchDeleteOutcome,
    PersistStore: FnMut(&SessionStore, &[String], &[String]) -> ProfileSecretStoreCommit,
{
    migrate_profile_secrets_with_journal_io(
        store,
        request,
        read_secret,
        write_batch,
        delete_batch,
        |next_store, affected_profile_ids, target_refs, _| {
            persist_store(next_store, affected_profile_ids, target_refs)
        },
        |_| Ok(()),
    )
}

pub(super) fn migrate_profile_secrets_with_journal_io<
    ReadSecret,
    WriteBatch,
    DeleteBatch,
    PersistStore,
    JournalUpdate,
>(
    store: &mut SessionStore,
    request: &ProfileSecretMigrationRequest,
    mut read_secret: ReadSecret,
    mut write_batch: WriteBatch,
    mut delete_batch: DeleteBatch,
    mut persist_store: PersistStore,
    mut journal_update: JournalUpdate,
) -> Result<ProfileSecretMigrationResponse, String>
where
    ReadSecret: FnMut(&str) -> Result<String, String>,
    WriteBatch: FnMut(SecretStorage, &[PreparedProfileSecretMigration]) -> Result<bool, String>,
    DeleteBatch: FnMut(SecretStorage, &[String]) -> SecretBatchDeleteOutcome,
    PersistStore: FnMut(&SessionStore, &[String], &[String], &str) -> ProfileSecretStoreCommit,
    JournalUpdate: FnMut(ProfileSecretMigrationJournalEvent) -> Result<(), String>,
{
    let plan = build_profile_secret_migration_plan(store, request)?;
    if plan.source_ref_counts.is_empty() {
        return Ok(ProfileSecretMigrationResponse {
            migration_id: None,
            recovery_pending: false,
            target_storage: request.target_storage,
            selected_profile_count: plan.preview.selected_profile_count,
            migrated_profile_count: 0,
            migrated_reference_count: 0,
            migrated_secret_count: 0,
            summaries: Vec::new(),
            items: Vec::new(),
            warnings: (plan.preview.excluded_reserved_reference_count > 0)
                .then(|| {
                    format!(
                        "已排除 {} 个 MCP token 保留引用",
                        plan.preview.excluded_reserved_reference_count
                    )
                })
                .into_iter()
                .collect(),
            portable_vault_requires_reunlock: false,
        });
    }

    let mut prepared = Vec::with_capacity(plan.source_ref_counts.len());
    for source_ref in plan.source_ref_counts.keys() {
        let secret = read_secret(source_ref).map_err(|error| {
            format!("凭据迁移预检失败，尚未写入任何目标 secret ({source_ref}): {error}")
        })?;
        prepared.push(PreparedProfileSecretMigration {
            source_ref: source_ref.clone(),
            target_ref: new_secret_ref(request.target_storage),
            secret: Zeroizing::new(secret),
        });
    }

    let replacements = prepared
        .iter()
        .map(|item| (item.source_ref.clone(), item.target_ref.clone()))
        .collect::<HashMap<_, _>>();
    let target_refs = prepared
        .iter()
        .map(|item| item.target_ref.clone())
        .collect::<Vec<_>>();
    let mut next_store = store.clone();
    let selected = plan
        .selected_profile_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let replaced = next_store
        .profiles
        .iter_mut()
        .filter(|profile| selected.contains(profile.id.as_str()))
        .map(|profile| replace_profile_secret_refs(profile, &replacements))
        .sum::<usize>();
    if replaced != plan.preview.eligible_reference_count {
        return Err(format!(
            "凭据迁移内部引用计数不一致: expected {}, got {replaced}",
            plan.preview.eligible_reference_count
        ));
    }

    let journal =
        build_profile_secret_migration_journal(store, &next_store, &plan, request, &prepared)?;
    let migration_id = journal.migration_id.clone();
    journal_update(ProfileSecretMigrationJournalEvent::Prepared(journal))
        .map_err(|error| format!("凭据迁移恢复记录未能在目标写入前持久化，操作已中止: {error}"))?;

    let mut portable_vault_requires_reunlock = match write_batch(request.target_storage, &prepared)
    {
        Ok(requires_reunlock) => requires_reunlock,
        Err(error) => {
            let _ = journal_update(ProfileSecretMigrationJournalEvent::Transition {
                migration_id: migration_id.clone(),
                state: ProfileSecretMigrationJournalState::TargetCleanupPending,
            });
            return Err(format!(
                "{PROFILE_SECRET_MIGRATION_RESTART_REQUIRED} 凭据迁移目标写入失败，Profile 引用保持不变；恢复记录 {migration_id} 已保留用于核对目标副本: {error}"
            ));
        }
    };
    if portable_vault_requires_reunlock {
        let checkpoint = journal_update(ProfileSecretMigrationJournalEvent::Transition {
            migration_id: migration_id.clone(),
            state: ProfileSecretMigrationJournalState::TargetCleanupPending,
        });
        let checkpoint_warning = checkpoint
            .err()
            .map(|error| format!("；恢复记录 checkpoint 更新失败: {error}"))
            .unwrap_or_default();
        return Err(format!(
            "{PROFILE_SECRET_MIGRATION_RESTART_REQUIRED} portable vault 目标 snapshot 已提交，但版本指纹无法确认；Profile 引用保持不变，恢复记录 {migration_id} 已保留，锁定并重新解锁 vault 后再核对{checkpoint_warning}"
        ));
    }
    if let Err(error) = journal_update(ProfileSecretMigrationJournalEvent::Transition {
        migration_id: migration_id.clone(),
        state: ProfileSecretMigrationJournalState::TargetsVerified,
    }) {
        return Err(format!(
            "{PROFILE_SECRET_MIGRATION_RESTART_REQUIRED} 目标 secret 已写入，但恢复 checkpoint 保存失败；为避免误删，Profile 原引用与两侧 secret 均已保留，恢复记录 {migration_id} 待重启核对: {error}"
        ));
    }

    match persist_store(
        &next_store,
        &plan.affected_profile_ids,
        &target_refs,
        &migration_id,
    ) {
        ProfileSecretStoreCommit::Committed { warning } => {
            let mut warnings = Vec::new();
            if let Some(warning) = warning {
                warnings.push(warning);
            }
            *store = next_store;
            let mut journal_checkpoint_failed = false;
            if let Err(error) = journal_update(ProfileSecretMigrationJournalEvent::Transition {
                migration_id: migration_id.clone(),
                state: ProfileSecretMigrationJournalState::SourceCleanupPending,
            }) {
                journal_checkpoint_failed = true;
                warnings.push(format!(
                    "Profile 已提交，但恢复记录 checkpoint 更新失败: {error}"
                ));
            }

            let mut deletable = Vec::new();
            let mut remaining = BTreeMap::new();
            for source_ref in plan.source_ref_counts.keys() {
                let count = secret_ref_usage_count(store, source_ref);
                remaining.insert(source_ref.clone(), count);
                if request.cleanup_source
                    && !journal_checkpoint_failed
                    && count == 0
                    && !plan.in_flight_source_refs.contains(source_ref)
                {
                    deletable.push(source_ref.clone());
                }
            }
            let source_storage = match request.target_storage {
                SecretStorage::Native => SecretStorage::Portable,
                SecretStorage::Portable => SecretStorage::Native,
            };
            let cleanup = if deletable.is_empty() {
                SecretBatchDeleteOutcome {
                    results: BTreeMap::new(),
                    portable_vault_requires_reunlock: false,
                }
            } else {
                delete_batch(source_storage, &deletable)
            };
            portable_vault_requires_reunlock |= cleanup.portable_vault_requires_reunlock;

            let target_by_source = prepared
                .iter()
                .map(|item| (item.source_ref.as_str(), item.target_ref.as_str()))
                .collect::<HashMap<_, _>>();
            let mut items = Vec::with_capacity(plan.source_ref_counts.len());
            for (source_ref, reference_count) in &plan.source_ref_counts {
                let remaining_source_references = remaining[source_ref];
                let (cleanup_status, cleanup_warning) = if !request.cleanup_source {
                    (ProfileSecretCleanupStatus::RetainedByRequest, None)
                } else if remaining_source_references > 0 {
                    (ProfileSecretCleanupStatus::RetainedShared, None)
                } else if plan.in_flight_source_refs.contains(source_ref) {
                    (ProfileSecretCleanupStatus::RetainedInUse, None)
                } else if journal_checkpoint_failed {
                    let warning = format!(
                        "旧 secret {source_ref} 因恢复 checkpoint 未确认而保留"
                    );
                    (ProfileSecretCleanupStatus::Failed, Some(warning))
                } else {
                    match cleanup.results.get(source_ref) {
                        Some(Ok(())) => (ProfileSecretCleanupStatus::Deleted, None),
                        Some(Err(error)) => {
                            let warning = format!("旧 secret {source_ref} 清理失败: {error}");
                            warnings.push(warning.clone());
                            (ProfileSecretCleanupStatus::Failed, Some(warning))
                        }
                        None => {
                            let warning = format!("旧 secret {source_ref} 未返回清理结果");
                            warnings.push(warning.clone());
                            (ProfileSecretCleanupStatus::Failed, Some(warning))
                        }
                    }
                };
                items.push(ProfileSecretMigrationItem {
                    source_ref: source_ref.clone(),
                    target_ref: target_by_source[source_ref.as_str()].to_string(),
                    reference_count: *reference_count,
                    remaining_source_references,
                    cleanup_status,
                    cleanup_warning,
                });
            }
            if plan.preview.excluded_reserved_reference_count > 0 {
                warnings.push(format!(
                    "已排除 {} 个 MCP token 保留引用",
                    plan.preview.excluded_reserved_reference_count
                ));
            }
            if portable_vault_requires_reunlock {
                warnings.push(
                    "Stronghold snapshot 已提交，但版本指纹刷新失败；请锁定并重新解锁 portable vault"
                        .to_string(),
                );
            }
            let summaries_by_id = store
                .summaries()
                .into_iter()
                .map(|summary| (summary.profile.id.clone(), summary))
                .collect::<HashMap<_, _>>();
            let summaries = plan
                .affected_profile_ids
                .iter()
                .filter_map(|profile_id| summaries_by_id.get(profile_id).cloned())
                .collect::<Vec<_>>();
            let mut recovery_pending = journal_checkpoint_failed
                || portable_vault_requires_reunlock
                || items.iter().any(|item| {
                    matches!(
                        item.cleanup_status,
                        ProfileSecretCleanupStatus::Failed
                            | ProfileSecretCleanupStatus::RetainedInUse
                    )
                });
            if recovery_pending {
                if let Err(error) =
                    journal_update(ProfileSecretMigrationJournalEvent::Transition {
                        migration_id: migration_id.clone(),
                        state: ProfileSecretMigrationJournalState::SourceCleanupPending,
                    })
                {
                    warnings.push(format!("恢复记录保持 pending 失败: {error}"));
                }
            } else if let Err(error) = journal_update(ProfileSecretMigrationJournalEvent::Clear {
                migration_id: migration_id.clone(),
            }) {
                recovery_pending = true;
                warnings.push(format!("迁移已完成，但恢复记录清除失败: {error}"));
            }
            Ok(ProfileSecretMigrationResponse {
                migration_id: Some(migration_id),
                recovery_pending,
                target_storage: request.target_storage,
                selected_profile_count: plan.preview.selected_profile_count,
                migrated_profile_count: plan.affected_profile_ids.len(),
                migrated_reference_count: plan.preview.eligible_reference_count,
                migrated_secret_count: plan.preview.eligible_secret_count,
                summaries,
                items,
                warnings,
                portable_vault_requires_reunlock,
            })
        }
        ProfileSecretStoreCommit::NotCommitted(error) => {
            let checkpoint = journal_update(ProfileSecretMigrationJournalEvent::Transition {
                migration_id: migration_id.clone(),
                state: ProfileSecretMigrationJournalState::TargetCleanupPending,
            });
            let cleanup = if checkpoint.is_ok() {
                delete_batch(request.target_storage, &target_refs)
            } else {
                SecretBatchDeleteOutcome {
                    results: BTreeMap::new(),
                    portable_vault_requires_reunlock: false,
                }
            };
            let cleanup_complete = !cleanup.portable_vault_requires_reunlock
                && target_refs
                    .iter()
                    .all(|target_ref| matches!(cleanup.results.get(target_ref), Some(Ok(()))));
            let mut recovery_pending = checkpoint.is_err() || !cleanup_complete;
            let mut journal_error = checkpoint.err();
            if !recovery_pending {
                if let Err(error) =
                    journal_update(ProfileSecretMigrationJournalEvent::Clear {
                        migration_id: migration_id.clone(),
                    })
                {
                    recovery_pending = true;
                    journal_error = Some(error);
                }
            }
            let mut message = migration_error_with_cleanup(
                format!("凭据迁移 Profile 保存失败，原引用保持不变: {error}"),
                &cleanup,
            );
            if let Some(journal_error) = journal_error {
                message.push_str(&format!("；恢复记录更新失败: {journal_error}"));
            }
            if recovery_pending {
                Err(format!(
                    "{PROFILE_SECRET_MIGRATION_RESTART_REQUIRED} {message}；恢复记录 {migration_id} 已保留"
                ))
            } else {
                Err(message)
            }
        }
        ProfileSecretStoreCommit::Unknown(error) => Err(format!(
            "{PROFILE_SECRET_MIGRATION_RESTART_REQUIRED} 凭据迁移 Profile 保存结果无法确认，原引用继续留在当前进程且新目标 secret 已保留；恢复记录 {migration_id} 已持久化，请重启 PortMate 后核对: {error}"
        )),
    }
}

pub(super) fn read_persisted_store_for_migration(path: &Path) -> Result<SessionStore, String> {
    let raw = if path.extension().and_then(|value| value.to_str()) == Some("sqlite3") {
        let connection = SqliteConnection::open(path).map_err(|error| {
            format!(
                "failed to open PortMate SQLite store {}: {error}",
                path.display()
            )
        })?;
        connection
            .query_row(
                "select value from kv where key = ?1",
                params![STORE_KEY],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| format!("failed to verify PortMate SQLite store: {error}"))?
    } else {
        fs::read_to_string(path).map_err(|error| {
            format!(
                "failed to verify PortMate JSON store {}: {error}",
                path.display()
            )
        })?
    };
    serde_json::from_str(&raw)
        .map_err(|error| format!("failed to decode persisted PortMate store: {error}"))
}

pub(super) fn persist_profile_secret_migration(
    path: &Path,
    next_store: &SessionStore,
    _affected_profile_ids: &[String],
    _target_refs: &[String],
    migration_id: &str,
) -> ProfileSecretStoreCommit {
    match save_store_with_profile_secret_migration_checkpoint(path, next_store, migration_id) {
        Ok(()) => ProfileSecretStoreCommit::Committed { warning: None },
        Err(save_error) => {
            let persisted = read_persisted_store_for_migration(path);
            let journal = load_profile_secret_migration_journal(path);
            match (persisted, journal) {
                (Ok(persisted), Ok(Some(journal)))
                    if journal.payload.migration_id == migration_id =>
                {
                    match profile_secret_migration_disposition(&persisted, &journal) {
                        ProfileSecretMigrationRecoveryDisposition::Committed => {
                            ProfileSecretStoreCommit::Committed {
                                warning: Some(format!(
                                    "Profile 保存返回错误，但磁盘 Profile 与 journal 已精确验证为已提交: {save_error}"
                                )),
                            }
                        }
                        ProfileSecretMigrationRecoveryDisposition::NotCommitted => {
                            ProfileSecretStoreCommit::NotCommitted(save_error)
                        }
                        ProfileSecretMigrationRecoveryDisposition::Conflict => {
                            ProfileSecretStoreCommit::Unknown(format!(
                                "{save_error}; 磁盘 Profile 与 journal 投影冲突"
                            ))
                        }
                    }
                }
                (Ok(_), Ok(Some(journal))) => ProfileSecretStoreCommit::Unknown(format!(
                    "{save_error}; 当前恢复记录 ID {} 与迁移 {migration_id} 不一致",
                    journal.payload.migration_id
                )),
                (Ok(_), Ok(None)) => ProfileSecretStoreCommit::Unknown(format!(
                    "{save_error}; 提交后找不到迁移恢复记录"
                )),
                (Err(verify_error), _) => ProfileSecretStoreCommit::Unknown(format!(
                    "{save_error}; 无法读取磁盘状态: {verify_error}"
                )),
                (_, Err(verify_error)) => ProfileSecretStoreCommit::Unknown(format!(
                    "{save_error}; 无法读取迁移恢复记录: {verify_error}"
                )),
            }
        }
    }
}

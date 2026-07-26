use super::*;

pub(super) fn journal_transition_allowed(
    current: ProfileSecretMigrationJournalState,
    next: ProfileSecretMigrationJournalState,
) -> bool {
    current == next
        || matches!(
            (current, next),
            (
                ProfileSecretMigrationJournalState::TargetWritePending,
                ProfileSecretMigrationJournalState::TargetsVerified
                    | ProfileSecretMigrationJournalState::TargetCleanupPending
                    | ProfileSecretMigrationJournalState::NeedsResolution
            ) | (
                ProfileSecretMigrationJournalState::TargetsVerified,
                ProfileSecretMigrationJournalState::ProfilesCommitted
                    | ProfileSecretMigrationJournalState::TargetCleanupPending
                    | ProfileSecretMigrationJournalState::NeedsResolution
            ) | (
                ProfileSecretMigrationJournalState::ProfilesCommitted,
                ProfileSecretMigrationJournalState::SourceCleanupPending
                    | ProfileSecretMigrationJournalState::NeedsResolution
            ) | (
                ProfileSecretMigrationJournalState::SourceCleanupPending
                    | ProfileSecretMigrationJournalState::TargetCleanupPending,
                ProfileSecretMigrationJournalState::NeedsResolution
            )
        )
}

pub(super) fn persist_profile_secret_migration_journal_event(
    path: &Path,
    event: ProfileSecretMigrationJournalEvent,
) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()) != Some("sqlite3") {
        return Err("凭据迁移恢复记录只支持 SQLite SessionStore".to_string());
    }
    let prepared_json = match &event {
        ProfileSecretMigrationJournalEvent::Prepared(payload) => {
            validate_profile_secret_migration_journal(payload)?;
            let payload_json = serde_json::to_string(payload)
                .map_err(|error| format!("无法编码凭据迁移恢复记录: {error}"))?;
            if payload_json.len() as u64 > MAX_PROFILE_SECRET_MIGRATION_JOURNAL_BYTES {
                return Err("凭据迁移恢复记录超过大小限制".to_string());
            }
            Some(payload_json)
        }
        _ => None,
    };
    let verification = match &event {
        ProfileSecretMigrationJournalEvent::Prepared(payload) => {
            ProfileSecretMigrationJournalVerification::Active {
                migration_id: payload.migration_id.clone(),
                state: ProfileSecretMigrationJournalState::TargetWritePending,
                payload_json: prepared_json.clone(),
            }
        }
        ProfileSecretMigrationJournalEvent::Transition {
            migration_id,
            state,
        } => ProfileSecretMigrationJournalVerification::Active {
            migration_id: migration_id.clone(),
            state: *state,
            payload_json: None,
        },
        ProfileSecretMigrationJournalEvent::Clear { migration_id } => {
            ProfileSecretMigrationJournalVerification::Cleared {
                migration_id: migration_id.clone(),
            }
        }
    };
    mutate_store_metadata_checked(
        path,
        move |connection| match event {
            ProfileSecretMigrationJournalEvent::Prepared(payload) => {
                connection
                    .execute(
                        "insert into profile_secret_migrations
                     (id, state, active, payload_json, created_at, updated_at)
                     values (?1, ?2, 1, ?3,
                             strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                             strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                        params![
                            &payload.migration_id,
                            ProfileSecretMigrationJournalState::TargetWritePending.as_str(),
                            prepared_json.expect("prepared journal JSON must exist")
                        ],
                    )
                    .map_err(|error| format!("无法创建凭据迁移恢复记录: {error}"))?;
                Ok(())
            }
            ProfileSecretMigrationJournalEvent::Transition {
                migration_id,
                state,
            } => {
                if state == ProfileSecretMigrationJournalState::ProfilesCommitted {
                    return Err(
                        "profiles-committed 只能与 Profile store 在同一事务提交".to_string()
                    );
                }
                let current = connection
                    .query_row(
                        "select state from profile_secret_migrations where id = ?1 and active = 1",
                        params![&migration_id],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| format!("无法读取凭据迁移恢复状态: {error}"))?;
                let current = ProfileSecretMigrationJournalState::parse(&current)?;
                if !journal_transition_allowed(current, state) {
                    return Err(format!(
                        "拒绝无效的凭据迁移恢复状态转换: {} -> {}",
                        current.as_str(),
                        state.as_str()
                    ));
                }
                let updated = connection
                    .execute(
                        "update profile_secret_migrations
                     set state = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                     where id = ?2 and active = 1",
                        params![state.as_str(), &migration_id],
                    )
                    .map_err(|error| format!("无法更新凭据迁移恢复状态: {error}"))?;
                if updated != 1 {
                    return Err(format!("凭据迁移恢复记录不存在: {migration_id}"));
                }
                Ok(())
            }
            ProfileSecretMigrationJournalEvent::Clear { migration_id } => {
                let deleted = connection
                    .execute(
                        "delete from profile_secret_migrations where id = ?1 and active = 1",
                        params![&migration_id],
                    )
                    .map_err(|error| format!("无法清除凭据迁移恢复记录: {error}"))?;
                if deleted != 1 {
                    return Err(format!("凭据迁移恢复记录不存在: {migration_id}"));
                }
                Ok(())
            }
        },
        move |connection| verify_profile_secret_migration_journal_event(connection, &verification),
    )
}

fn verify_profile_secret_migration_journal_event(
    connection: &SqliteConnection,
    verification: &ProfileSecretMigrationJournalVerification,
) -> Result<(), String> {
    match verification {
        ProfileSecretMigrationJournalVerification::Active {
            migration_id,
            state,
            payload_json,
        } => {
            let (persisted_state, persisted_payload) = connection
                .query_row(
                    "select state, payload_json from profile_secret_migrations
                     where id = ?1 and active = 1",
                    params![migration_id],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .map_err(|error| format!("无法读回凭据迁移恢复记录: {error}"))?;
            if persisted_state != state.as_str() {
                return Err(format!(
                    "凭据迁移恢复记录状态读回不一致: expected {}, got {persisted_state}",
                    state.as_str()
                ));
            }
            if payload_json
                .as_ref()
                .is_some_and(|expected| expected != &persisted_payload)
            {
                return Err("凭据迁移恢复记录 payload 读回不一致".to_string());
            }
            Ok(())
        }
        ProfileSecretMigrationJournalVerification::Cleared { migration_id } => {
            let remaining = connection
                .query_row(
                    "select count(*) from profile_secret_migrations where id = ?1",
                    params![migration_id],
                    |row| row.get::<_, usize>(0),
                )
                .map_err(|error| format!("无法验证凭据迁移恢复记录已清除: {error}"))?;
            if remaining != 0 {
                return Err(format!("凭据迁移恢复记录未被清除: {migration_id}"));
            }
            Ok(())
        }
    }
}

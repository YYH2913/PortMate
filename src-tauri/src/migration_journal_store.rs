use super::*;

impl ProfileSecretMigrationJournalState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::TargetWritePending => "target-write-pending",
            Self::TargetsVerified => "targets-verified",
            Self::ProfilesCommitted => "profiles-committed",
            Self::SourceCleanupPending => "source-cleanup-pending",
            Self::TargetCleanupPending => "target-cleanup-pending",
            Self::NeedsResolution => "needs-resolution",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "target-write-pending" => Ok(Self::TargetWritePending),
            "targets-verified" => Ok(Self::TargetsVerified),
            "profiles-committed" => Ok(Self::ProfilesCommitted),
            "source-cleanup-pending" => Ok(Self::SourceCleanupPending),
            "target-cleanup-pending" => Ok(Self::TargetCleanupPending),
            "needs-resolution" => Ok(Self::NeedsResolution),
            _ => Err("未知的凭据迁移恢复状态".to_string()),
        }
    }
}

fn validate_journal_projection(
    projection: &ProfileSecretMigrationJournalProjection,
    label: &str,
) -> Result<(), String> {
    for identity_id in projection.identity_secret_refs.keys() {
        if identity_id.is_empty() || identity_id.trim() != identity_id {
            return Err(format!("凭据迁移恢复记录包含无效的 {label} identity ID"));
        }
    }
    for secret_ref in profile_secret_projection_ref_counts(projection).keys() {
        if canonical_secret_ref(secret_ref).as_deref() != Some(secret_ref.as_str()) {
            return Err(format!("凭据迁移恢复记录包含无效的 {label} secretRef"));
        }
    }
    Ok(())
}

pub(super) fn validate_profile_secret_migration_journal(
    payload: &ProfileSecretMigrationJournalPayload,
) -> Result<(), String> {
    if payload.version != PROFILE_SECRET_MIGRATION_JOURNAL_VERSION {
        return Err(format!("不支持的凭据迁移恢复记录版本: {}", payload.version));
    }
    Uuid::parse_str(&payload.migration_id).map_err(|_| "凭据迁移恢复记录 ID 无效".to_string())?;
    if payload.plan_token.len() != 64
        || !payload
            .plan_token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("凭据迁移恢复记录 plan token 无效".to_string());
    }
    if payload.selected_profile_ids.is_empty()
        || payload.profiles.is_empty()
        || payload.items.is_empty()
        || payload.profiles.len() > MAX_PROFILE_SECRET_MIGRATION_PROFILES
        || payload.items.len() > MAX_PROFILE_SECRET_MIGRATION_ITEMS
    {
        return Err("凭据迁移恢复记录范围无效".to_string());
    }
    let mut selected = HashSet::new();
    for profile_id in &payload.selected_profile_ids {
        if profile_id.trim() != profile_id
            || profile_id.is_empty()
            || !selected.insert(profile_id.as_str())
        {
            return Err("凭据迁移恢复记录包含无效或重复 Profile ID".to_string());
        }
    }
    let mut mappings = HashMap::new();
    let mut targets = HashSet::new();
    for item in &payload.items {
        if item.reference_count == 0
            || canonical_secret_ref(&item.source_ref).as_deref() != Some(item.source_ref.as_str())
            || canonical_secret_ref(&item.target_ref).as_deref() != Some(item.target_ref.as_str())
            || is_reserved_internal_secret_ref(&item.source_ref)
            || is_reserved_internal_secret_ref(&item.target_ref)
            || secret_ref_storage(&item.source_ref) == payload.target_storage
            || secret_ref_storage(&item.target_ref) != payload.target_storage
            || mappings
                .insert(item.source_ref.as_str(), item.target_ref.as_str())
                .is_some()
            || !targets.insert(item.target_ref.as_str())
        {
            return Err("凭据迁移恢复记录包含无效或重复 secret 映射".to_string());
        }
        let target_account = item
            .target_ref
            .split_once(':')
            .map(|(_, account)| account)
            .unwrap_or_default();
        Uuid::parse_str(target_account)
            .map_err(|_| "凭据迁移恢复记录 targetRef 不是 PortMate UUID".to_string())?;
    }
    let mut profile_ids = HashSet::new();
    let mut mapped_totals = BTreeMap::<String, usize>::new();
    for profile in &payload.profiles {
        if !selected.contains(profile.profile_id.as_str())
            || !profile_ids.insert(profile.profile_id.as_str())
        {
            return Err("凭据迁移恢复记录包含未知或重复的受影响 Profile".to_string());
        }
        validate_journal_projection(&profile.before, "before")?;
        validate_journal_projection(&profile.after, "after")?;
        let before_counts = profile_secret_projection_ref_counts(&profile.before);
        let mut expected_after = profile.before.clone();
        let replaced = replace_journal_projection_refs(&mut expected_after, &mappings);
        for (secret_ref, count) in before_counts {
            if mappings.contains_key(secret_ref.as_str()) {
                *mapped_totals.entry(secret_ref).or_default() += count;
            }
        }
        if expected_after != profile.after || replaced == 0 {
            return Err("凭据迁移恢复记录的 before/after Profile 投影不一致".to_string());
        }
    }
    for item in &payload.items {
        if mapped_totals.get(&item.source_ref).copied() != Some(item.reference_count) {
            return Err("凭据迁移恢复记录的引用计数不一致".to_string());
        }
    }
    Ok(())
}

pub(super) fn parse_journal_timestamp(
    value: &str,
    label: &str,
) -> Result<chrono::DateTime<Utc>, String> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| format!("凭据迁移恢复记录 {label} 时间无效: {error}"))
}

pub(super) fn load_profile_secret_migration_journal_from_connection(
    connection: &SqliteConnection,
) -> Result<Option<LoadedProfileSecretMigrationJournal>, String> {
    let row = connection.query_row(
        "select id, state, payload_json, created_at, updated_at
         from profile_secret_migrations where active = 1 limit 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    );
    let (migration_id, state, payload_json, created_at, updated_at) = match row {
        Ok(row) => row,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(error) => return Err(format!("无法读取凭据迁移恢复记录: {error}")),
    };
    if payload_json.len() as u64 > MAX_PROFILE_SECRET_MIGRATION_JOURNAL_BYTES {
        return Err("凭据迁移恢复记录超过大小限制".to_string());
    }
    let payload = serde_json::from_str::<ProfileSecretMigrationJournalPayload>(&payload_json)
        .map_err(|error| {
            format!(
                "凭据迁移恢复记录 JSON 损坏（line {}, column {}）",
                error.line(),
                error.column()
            )
        })?;
    validate_profile_secret_migration_journal(&payload)?;
    if payload.migration_id != migration_id {
        return Err("凭据迁移恢复记录 row ID 与 payload ID 不一致".to_string());
    }
    Ok(Some(LoadedProfileSecretMigrationJournal {
        state: ProfileSecretMigrationJournalState::parse(&state)?,
        payload,
        created_at: parse_journal_timestamp(&created_at, "createdAt")?,
        updated_at: parse_journal_timestamp(&updated_at, "updatedAt")?,
    }))
}

pub(super) fn load_active_profile_secret_migration_journal_metadata(
    connection: &SqliteConnection,
) -> Result<Option<ActiveProfileSecretMigrationJournalMetadata>, String> {
    let row = connection.query_row(
        "select id, state, length(cast(payload_json as blob)), created_at, updated_at
         from profile_secret_migrations where active = 1 limit 1",
        [],
        |row| {
            Ok(ActiveProfileSecretMigrationJournalMetadata {
                row_id: row.get(0)?,
                state: row.get(1)?,
                payload_bytes: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            })
        },
    );
    match row {
        Ok(row) => Ok(Some(row)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(format!("无法读取凭据迁移恢复记录的受限诊断元数据: {error}")),
    }
}

pub(super) fn load_profile_secret_migration_journal(
    path: &Path,
) -> Result<Option<LoadedProfileSecretMigrationJournal>, String> {
    if path.extension().and_then(|value| value.to_str()) != Some("sqlite3") {
        return Err("凭据迁移恢复记录只支持 SQLite SessionStore".to_string());
    }
    let connection = SqliteConnection::open(path)
        .map_err(|error| format!("无法打开 SQLite 读取凭据迁移恢复记录: {error}"))?;
    ensure_store_schema(&connection)?;
    load_profile_secret_migration_journal_from_connection(&connection)
}

pub(super) fn ensure_no_pending_profile_secret_migration(path: &Path) -> Result<(), String> {
    if let Some(journal) = load_profile_secret_migration_journal(path)? {
        return Err(format!(
            "存在待恢复的凭据迁移 {}（{}），请先核对并恢复后再修改凭据",
            journal.payload.migration_id,
            journal.state.as_str()
        ));
    }
    Ok(())
}

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

use super::*;

pub(crate) fn read_persisted_store_for_migration(path: &Path) -> Result<SessionStore, String> {
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

pub(crate) fn persist_profile_secret_migration(
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

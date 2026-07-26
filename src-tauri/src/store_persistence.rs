use super::*;

pub(super) fn load_store(path: &Path) -> Result<SessionStore, String> {
    let snapshot_lock = lock_store_snapshot(path)?;
    let initialize_store = !path.exists();
    let store = if !initialize_store {
        normalize_loaded_store_checked(load_store_sqlite(path)?)?
    } else {
        let legacy_path = path.with_file_name(LEGACY_JSON_STORE_FILE_NAME);
        if legacy_path.exists() {
            load_store_json(&legacy_path)?
        } else {
            SessionStore::default()
        }
    };
    if initialize_store {
        save_store_contents(path, &store)?;
    }
    let version = store_snapshot_version(path)?;
    STORE_SNAPSHOT_VERSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?
        .insert(path.to_path_buf(), version);
    drop(snapshot_lock);
    Ok(store)
}

pub(super) fn save_store(path: &Path, store: &SessionStore) -> Result<(), String> {
    let snapshot_lock = lock_store_snapshot(path)?;
    let current = store_snapshot_version(path)?;
    let mut expected = {
        let mut versions = STORE_SNAPSHOT_VERSIONS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|error| error.to_string())?;
        *versions.entry(path.to_path_buf()).or_insert(current)
    };
    let result = save_store_checked_locked(path, store, &mut expected, current);
    STORE_SNAPSHOT_VERSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?
        .insert(path.to_path_buf(), expected);
    drop(snapshot_lock);
    result
}

pub(super) fn save_store_with_profile_secret_migration_checkpoint(
    path: &Path,
    store: &SessionStore,
    migration_id: &str,
) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()) != Some("sqlite3") {
        return Err("凭据迁移恢复记录只支持 SQLite SessionStore".to_string());
    }
    let snapshot_lock = lock_store_snapshot(path)?;
    let current = store_snapshot_version(path)?;
    let mut expected = {
        let mut versions = STORE_SNAPSHOT_VERSIONS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|error| error.to_string())?;
        *versions.entry(path.to_path_buf()).or_insert(current)
    };
    let result = save_store_checked_locked_with_writer(
        path,
        store,
        &mut expected,
        current,
        |path, store| {
            save_store_contents_with_profile_secret_migration_checkpoint(path, store, migration_id)
        },
    );
    STORE_SNAPSHOT_VERSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?
        .insert(path.to_path_buf(), expected);
    drop(snapshot_lock);
    result
}

pub(super) fn mutate_store_metadata_checked<F, V>(
    path: &Path,
    mutation: F,
    verification: V,
) -> Result<(), String>
where
    F: FnOnce(&SqliteConnection) -> Result<(), String>,
    V: FnOnce(&SqliteConnection) -> Result<(), String>,
{
    let snapshot_lock = lock_store_snapshot(path)?;
    let current = store_snapshot_version(path)?;
    let mut expected = {
        let mut versions = STORE_SNAPSHOT_VERSIONS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|error| error.to_string())?;
        *versions.entry(path.to_path_buf()).or_insert(current)
    };
    let write_result = if expected == StoreSnapshotVersion::UnknownAfterCommit {
        Err("PortMate store 上次提交后无法刷新版本，请重启应用后再保存".to_string())
    } else if expected != current {
        Err("PortMate store 已被另一实例修改，已拒绝陈旧写入；请重启应用加载最新数据".to_string())
    } else {
        (|| {
            let connection = SqliteConnection::open(path)
                .map_err(|error| format!("无法打开 SQLite 更新迁移恢复记录: {error}"))?;
            connection
                .execute_batch("PRAGMA synchronous = FULL;")
                .map_err(|error| format!("无法启用迁移恢复记录完整同步: {error}"))?;
            ensure_store_schema(&connection)?;
            connection
                .execute_batch("BEGIN IMMEDIATE;")
                .map_err(|error| format!("无法开始迁移恢复记录事务: {error}"))?;
            let transaction_result = (|| {
                mutation(&connection)?;
                connection
                    .execute(
                        "insert into metadata (key, value) values ('storeRevision', ?1)
                         on conflict(key) do update set value = excluded.value",
                        params![Uuid::new_v4().to_string()],
                    )
                    .map_err(|error| format!("无法更新迁移恢复记录 revision: {error}"))?;
                Ok(())
            })();
            if let Err(error) = transaction_result {
                let _ = connection.execute_batch("ROLLBACK;");
                return Err(error);
            }
            connection
                .execute_batch("COMMIT;")
                .map_err(|error| format!("无法提交迁移恢复记录: {error}"))?;
            verification(&connection)?;
            Ok(())
        })()
    };
    let result = match write_result {
        Ok(()) => match store_snapshot_version(path) {
            Ok(StoreSnapshotVersion::Sha256(version)) => {
                expected = StoreSnapshotVersion::Sha256(version);
                Ok(())
            }
            Ok(StoreSnapshotVersion::Missing) | Ok(StoreSnapshotVersion::UnknownAfterCommit) => {
                expected = StoreSnapshotVersion::UnknownAfterCommit;
                Err("迁移恢复记录已写入，但提交后无法验证；请重启 PortMate".to_string())
            }
            Err(error) => {
                expected = StoreSnapshotVersion::UnknownAfterCommit;
                Err(format!(
                    "迁移恢复记录已写入，但提交后版本验证失败；请重启 PortMate: {error}"
                ))
            }
        },
        Err(error) => {
            if store_snapshot_version(path).is_ok_and(|after| after != current) {
                expected = StoreSnapshotVersion::UnknownAfterCommit;
            }
            Err(error)
        }
    };
    STORE_SNAPSHOT_VERSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?
        .insert(path.to_path_buf(), expected);
    drop(snapshot_lock);
    result
}

#[cfg(test)]
pub(super) fn save_store_with_expected_snapshot_version(
    path: &Path,
    store: &SessionStore,
    expected: &mut StoreSnapshotVersion,
) -> Result<(), String> {
    let snapshot_lock = lock_store_snapshot(path)?;
    let current = store_snapshot_version(path)?;
    let result = save_store_checked_locked(path, store, expected, current);
    drop(snapshot_lock);
    result
}

fn save_store_checked_locked(
    path: &Path,
    store: &SessionStore,
    expected: &mut StoreSnapshotVersion,
    current: StoreSnapshotVersion,
) -> Result<(), String> {
    save_store_checked_locked_with_writer(path, store, expected, current, save_store_contents)
}

fn save_store_checked_locked_with_writer<F>(
    path: &Path,
    store: &SessionStore,
    expected: &mut StoreSnapshotVersion,
    current: StoreSnapshotVersion,
    write_contents: F,
) -> Result<(), String>
where
    F: FnOnce(&Path, &SessionStore) -> Result<(), String>,
{
    store.validate_profile_count()?;
    if *expected == StoreSnapshotVersion::UnknownAfterCommit {
        return Err("PortMate store 上次提交后无法刷新版本，请重启应用后再保存".to_string());
    }
    if *expected != current {
        return Err(
            "PortMate store 已被另一实例修改，已拒绝陈旧写入；请重启应用加载最新数据".to_string(),
        );
    }
    if let Err(error) = write_contents(path, store) {
        if store_snapshot_version(path).is_ok_and(|after| after != current) {
            *expected = StoreSnapshotVersion::UnknownAfterCommit;
        }
        return Err(error);
    }
    match store_snapshot_version(path) {
        Ok(StoreSnapshotVersion::Sha256(version)) => {
            *expected = StoreSnapshotVersion::Sha256(version);
            Ok(())
        }
        Ok(StoreSnapshotVersion::Missing) | Ok(StoreSnapshotVersion::UnknownAfterCommit) => {
            *expected = StoreSnapshotVersion::UnknownAfterCommit;
            Err(
                "PortMate store 写入已完成，但持久化快照无法读回验证；请重启应用后再继续保存"
                    .to_string(),
            )
        }
        Err(error) => {
            *expected = StoreSnapshotVersion::UnknownAfterCommit;
            Err(format!(
                "PortMate store 写入已完成，但提交后版本验证失败；请重启应用: {error}"
            ))
        }
    }
}

fn save_store_contents(path: &Path, store: &SessionStore) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()) == Some("sqlite3") {
        save_store_sqlite(path, store)?;
        let legacy_path = path.with_file_name(LEGACY_JSON_STORE_FILE_NAME);
        if let Err(error) = save_store_json(&legacy_path, store) {
            eprintln!("PortMate: failed to update JSON compatibility store: {error}");
        }
        return Ok(());
    }
    save_store_json(path, store)
}

fn save_store_contents_with_profile_secret_migration_checkpoint(
    path: &Path,
    store: &SessionStore,
    migration_id: &str,
) -> Result<(), String> {
    save_store_sqlite_with_profile_secret_migration_checkpoint(
        path,
        store,
        Some((
            migration_id,
            ProfileSecretMigrationJournalState::ProfilesCommitted,
        )),
    )?;
    let legacy_path = path.with_file_name(LEGACY_JSON_STORE_FILE_NAME);
    if let Err(error) = save_store_json(&legacy_path, store) {
        eprintln!("PortMate: failed to update JSON compatibility store: {error}");
    }
    Ok(())
}

pub(super) fn save_store_json(path: &Path, store: &SessionStore) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("failed to serialize PortMate store: {error}"))?;
    write_private_atomic_file(path, &bytes, "PortMate JSON compatibility store")
}

pub(super) fn persist_store_arc(
    path: &Path,
    store: &Arc<Mutex<SessionStore>>,
) -> Result<(), String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    persist_applied_store(&store, path, "runtime event stream")
}

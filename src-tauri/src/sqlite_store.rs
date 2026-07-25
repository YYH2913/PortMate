use std::{fs, path::Path};

use portmate_core::SessionStore;
use rusqlite::{params, Connection as SqliteConnection};
use uuid::Uuid;

use super::{
    journal_transition_allowed, sqlite_mirror::save_store_sqlite_tables,
    sqlite_schema::ensure_store_schema, ProfileSecretMigrationJournalState, STORE_KEY,
};

pub(super) fn load_store_sqlite(path: &Path) -> Result<SessionStore, String> {
    let connection = SqliteConnection::open(path).map_err(|error| {
        format!(
            "failed to open PortMate SQLite store {}: {error}",
            path.display()
        )
    })?;
    ensure_store_schema(&connection)?;
    let raw = connection
        .query_row(
            "select value from kv where key = ?1",
            params![STORE_KEY],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("failed to read PortMate SQLite store: {error}"))?;
    serde_json::from_str::<SessionStore>(&raw)
        .map_err(|error| format!("failed to parse SQLite store {}: {error}", path.display()))
}

pub(super) fn save_store_sqlite(path: &Path, store: &SessionStore) -> Result<(), String> {
    save_store_sqlite_with_profile_secret_migration_checkpoint(path, store, None)
}

pub(super) fn save_store_sqlite_with_profile_secret_migration_checkpoint(
    path: &Path,
    store: &SessionStore,
    migration_checkpoint: Option<(&str, ProfileSecretMigrationJournalState)>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create PortMate data directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let connection = SqliteConnection::open(path).map_err(|error| {
        format!(
            "failed to open PortMate SQLite store {}: {error}",
            path.display()
        )
    })?;
    connection
        .execute_batch("PRAGMA synchronous = FULL;")
        .map_err(|error| format!("failed to enable full SQLite synchronization: {error}"))?;
    ensure_store_schema(&connection)?;
    let bytes = serde_json::to_string_pretty(store)
        .map_err(|error| format!("failed to serialize PortMate store: {error}"))?;

    // Dropping this call-local connection rolls back an incomplete transaction,
    // so the canonical snapshot and typed mirrors cannot commit independently.
    connection
        .execute_batch("BEGIN IMMEDIATE;")
        .map_err(|error| format!("failed to start PortMate SQLite transaction: {error}"))?;
    connection
        .execute(
            "insert into kv (key, value, updated_at) values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
             on conflict(key) do update set value = excluded.value, updated_at = excluded.updated_at",
            params![STORE_KEY, &bytes],
        )
        .map_err(|error| format!("failed to save PortMate SQLite store: {error}"))?;
    save_store_sqlite_tables(&connection, store)?;
    if let Some((migration_id, state)) = migration_checkpoint {
        let current = connection
            .query_row(
                "select state from profile_secret_migrations where id = ?1 and active = 1",
                params![migration_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| {
                format!("failed to read profile secret migration checkpoint: {error}")
            })?;
        let current = ProfileSecretMigrationJournalState::parse(&current)?;
        if !journal_transition_allowed(current, state) {
            return Err(format!(
                "refusing invalid profile secret migration checkpoint: {} -> {}",
                current.as_str(),
                state.as_str()
            ));
        }
        let updated = connection
            .execute(
                "update profile_secret_migrations
                 set state = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now')
                 where id = ?2 and active = 1",
                params![state.as_str(), migration_id],
            )
            .map_err(|error| format!("failed to checkpoint profile secret migration: {error}"))?;
        if updated != 1 {
            return Err(format!(
                "active profile secret migration journal is missing: {migration_id}"
            ));
        }
    }
    connection
        .execute(
            "insert into metadata (key, value) values ('storeRevision', ?1)
                on conflict(key) do update set value = excluded.value",
            params![Uuid::new_v4().to_string()],
        )
        .map_err(|error| format!("failed to update PortMate store revision: {error}"))?;
    connection
        .execute_batch("COMMIT;")
        .map_err(|error| format!("failed to commit PortMate SQLite transaction: {error}"))?;
    let persisted_bytes = connection
        .query_row(
            "select value from kv where key = ?1",
            params![STORE_KEY],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| format!("failed to read back PortMate SQLite store: {error}"))?;
    if persisted_bytes != bytes {
        return Err("PortMate SQLite store read-back did not match committed contents".to_string());
    }
    if let Some((migration_id, state)) = migration_checkpoint {
        let persisted_state = connection
            .query_row(
                "select state from profile_secret_migrations where id = ?1 and active = 1",
                params![migration_id],
                |row| row.get::<_, String>(0),
            )
            .map_err(|error| {
                format!("failed to read back profile secret migration checkpoint: {error}")
            })?;
        if persisted_state != state.as_str() {
            return Err(format!(
                "profile secret migration checkpoint read-back mismatch: expected {}, got {persisted_state}",
                state.as_str()
            ));
        }
    }
    Ok(())
}

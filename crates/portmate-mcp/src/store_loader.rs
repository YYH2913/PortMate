use anyhow::{Context, Result};
use portmate_core::SessionStore;
use rusqlite::{params, Connection as SqliteConnection, OpenFlags as SqliteOpenFlags};
use std::fs;
use std::path::Path;

pub(crate) const STORE_KEY: &str = "session-store";

pub(crate) fn load_store_from_path(path: &Path) -> Result<SessionStore> {
    let store = if path.extension().and_then(|value| value.to_str()) == Some("sqlite3") {
        let connection =
            SqliteConnection::open_with_flags(path, SqliteOpenFlags::SQLITE_OPEN_READ_ONLY)
                .with_context(|| {
                    format!("failed to open MCP Store `{}` read-only", path.display())
                })?;
        let raw = connection
            .query_row(
                "select value from kv where key = ?1",
                params![STORE_KEY],
                |row| row.get::<_, String>(0),
            )
            .with_context(|| {
                format!(
                    "failed to read `{STORE_KEY}` from MCP Store `{}`",
                    path.display()
                )
            })?;
        serde_json::from_str::<SessionStore>(&raw).with_context(|| {
            format!(
                "MCP Store `{}` contains an invalid `{STORE_KEY}` snapshot",
                path.display()
            )
        })?
    } else {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read MCP Store `{}`", path.display()))?;
        serde_json::from_str::<SessionStore>(&raw)
            .with_context(|| format!("MCP Store `{}` contains invalid JSON", path.display()))?
    };
    prepare_loaded_store(store)
}

pub(crate) fn prepare_loaded_store(mut store: SessionStore) -> Result<SessionStore> {
    store.validate_profile_count().map_err(anyhow::Error::msg)?;
    store.custom_scripts =
        portmate_core::normalize_loaded_custom_scripts(std::mem::take(&mut store.custom_scripts));
    portmate_core::redact_custom_script_event_bodies(&mut store.events);
    store.normalize_bounded_histories();
    Ok(store)
}

#[cfg(test)]
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../test/rust/portmate-mcp/support/store_loader.rs"
));

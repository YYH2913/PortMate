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
    store.normalize_bounded_histories();
    Ok(store)
}

#[cfg(test)]
pub(crate) fn ensure_store_schema(connection: &SqliteConnection) -> Result<()> {
    connection.execute_batch(
        "create table if not exists kv (
            key text primary key not null,
            value text not null,
            updated_at text not null
        );
        create table if not exists metadata (
            key text primary key not null,
            value text not null
        );
        create table if not exists profiles (
            id text primary key not null,
            name text not null,
            kind text not null,
            group_name text not null,
            tags_json text not null,
            connection_json text not null,
            terminal_json text not null,
            logging_json text not null,
            triggers_json text not null,
            transfer_json text not null,
            updated_at text not null
        );
        create table if not exists runtimes (
            session_id text primary key not null,
            pane_id text not null,
            status text not null,
            title text not null,
            cwd text,
            connected_since text,
            last_activity text not null,
            active_transport text not null,
            raw_json text not null
        );
        create table if not exists events (
            id text primary key not null,
            session_id text not null,
            pane_id text not null,
            ts text not null,
            direction text not null,
            stream text not null,
            bytes_ref text,
            text text,
            annotations_json text not null,
            raw_json text not null
        );
        create table if not exists transfers (
            id text primary key not null,
            session_id text not null,
            protocol text not null,
            source text not null,
            destination text not null,
            bytes_total integer not null,
            bytes_done integer not null,
            status text not null,
            message text,
            raw_json text not null
        );
        create table if not exists trusted_host_keys (
            id text primary key not null,
            profile_id text,
            alias text not null,
            host text not null,
            port integer not null,
            algorithm text not null,
            fingerprint_sha256 text not null,
            public_key_base64 text not null,
            scope text not null,
            label text,
            first_seen text not null,
            last_seen text not null,
            raw_json text not null
        );
        create table if not exists mcp_grants (
            client_id text primary key not null,
            name text not null,
            scopes_json text not null,
            allowed_sessions_json text not null,
            expires_at text,
            revoked_at text,
            raw_json text not null
        );
        create table if not exists mcp_audit (
            id text primary key not null,
            ts text not null,
            actor text not null,
            action text not null,
            session_id text,
            decision text not null,
            details_json text not null,
            raw_json text not null
        );
        create table if not exists timeline_marks (
            id text primary key not null,
            session_id text not null,
            ts text not null,
            label text not null,
            details text,
            raw_json text not null
        );
        create table if not exists sysmon_snapshots (
            session_id text not null,
            ts text not null,
            uptime_seconds integer not null,
            cpu_percent real not null,
            memory_percent real not null,
            rx_kbps real not null,
            tx_kbps real not null,
            raw_json text not null,
            primary key (session_id, ts)
        );
        create index if not exists idx_events_session_ts on events(session_id, ts);
        create index if not exists idx_events_text on events(text);
        create index if not exists idx_transfers_session on transfers(session_id);
        create index if not exists idx_host_keys_alias on trusted_host_keys(alias, port, algorithm);
        create index if not exists idx_audit_session_ts on mcp_audit(session_id, ts);
        create index if not exists idx_timeline_session_ts on timeline_marks(session_id, ts);
        create index if not exists idx_sysmon_session_ts on sysmon_snapshots(session_id, ts);
        insert into metadata (key, value) values ('schemaVersion', '2')
            on conflict(key) do update set value = excluded.value;",
    )?;
    Ok(())
}

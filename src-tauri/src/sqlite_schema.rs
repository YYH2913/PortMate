use rusqlite::{params, Connection as SqliteConnection};

pub(super) const SQLITE_SCHEMA_VERSION: &str = "4";

pub(super) fn ensure_store_schema(connection: &SqliteConnection) -> Result<(), String> {
    connection
        .execute_batch(
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
                last_disconnect text,
                last_disconnect_reason text,
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
                details_json text not null,
                raw_json text not null,
                primary key (session_id, ts)
            );
            create table if not exists profile_secret_migrations (
                id text primary key not null,
                state text not null,
                active integer not null check (active in (0, 1)),
                payload_json text not null,
                created_at text not null,
                updated_at text not null
            );
            create index if not exists idx_events_session_ts on events(session_id, ts);
            create index if not exists idx_events_text on events(text);
            create index if not exists idx_transfers_session on transfers(session_id);
            create index if not exists idx_host_keys_alias on trusted_host_keys(alias, port, algorithm);
            create index if not exists idx_audit_session_ts on mcp_audit(session_id, ts);
            create index if not exists idx_timeline_session_ts on timeline_marks(session_id, ts);
            create index if not exists idx_sysmon_session_ts on sysmon_snapshots(session_id, ts);
            create unique index if not exists idx_profile_secret_migrations_active
                on profile_secret_migrations(active) where active = 1;",
        )
        .map_err(|error| format!("failed to initialize PortMate SQLite schema: {error}"))?;
    ensure_sqlite_column(
        connection,
        "runtimes",
        "last_disconnect",
        "alter table runtimes add column last_disconnect text",
    )?;
    ensure_sqlite_column(
        connection,
        "runtimes",
        "last_disconnect_reason",
        "alter table runtimes add column last_disconnect_reason text",
    )?;
    ensure_sqlite_column(
        connection,
        "sysmon_snapshots",
        "details_json",
        "alter table sysmon_snapshots add column details_json text not null default '{}'",
    )?;
    connection
        .execute(
            "insert into metadata (key, value) values ('schemaVersion', ?1)
             on conflict(key) do update set value = excluded.value",
            params![SQLITE_SCHEMA_VERSION],
        )
        .map_err(|error| format!("failed to update SQLite schema version: {error}"))?;
    Ok(())
}

fn ensure_sqlite_column(
    connection: &SqliteConnection,
    table: &str,
    column: &str,
    alter_sql: &str,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("pragma table_info({table})"))
        .map_err(|error| format!("failed to inspect SQLite table {table}: {error}"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|error| format!("failed to inspect SQLite columns for {table}: {error}"))?;
    for existing in columns {
        let existing = existing
            .map_err(|error| format!("failed to read SQLite columns for {table}: {error}"))?;
        if existing == column {
            return Ok(());
        }
    }
    connection
        .execute_batch(alter_sql)
        .map_err(|error| format!("failed to add SQLite column {table}.{column}: {error}"))?;
    Ok(())
}

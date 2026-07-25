use std::collections::{HashMap, HashSet};

use portmate_core::{SessionStore, SysmonDisk, SysmonNetworkInterface, SysmonProcess};
use rusqlite::{params, Connection as SqliteConnection};
use serde::Serialize;

use super::sqlite_schema::SQLITE_SCHEMA_VERSION;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SysmonSnapshotDetails<'a> {
    load_average: [f32; 3],
    memory_total_bytes: u64,
    memory_available_bytes: u64,
    processes: &'a [SysmonProcess],
    disks: &'a [SysmonDisk],
    network_interfaces: &'a [SysmonNetworkInterface],
}

pub(super) fn save_store_sqlite_tables(
    connection: &SqliteConnection,
    store: &SessionStore,
) -> Result<(), String> {
    connection
        .execute_batch(
            "delete from profiles;
             delete from runtimes;
             delete from transfers;
             delete from trusted_host_keys;
             delete from mcp_grants;",
        )
        .map_err(|error| format!("failed to clear PortMate SQLite mirror tables: {error}"))?;

    let mirrored_events =
        sqlite_string_map(connection, "select id, raw_json from events", "event")?;
    let mirrored_event_ids = mirrored_events.keys().cloned().collect::<HashSet<_>>();
    let current_event_ids = store
        .events
        .iter()
        .map(|event| event.id.clone())
        .collect::<HashSet<_>>();
    let mirrored_audit = sqlite_string_map(
        connection,
        "select id, raw_json from mcp_audit",
        "MCP audit",
    )?;
    let mirrored_audit_ids = mirrored_audit.keys().cloned().collect::<HashSet<_>>();
    let current_audit_ids = store
        .audit
        .iter()
        .map(|record| record.id.clone())
        .collect::<HashSet<_>>();
    let mirrored_timeline_ids =
        sqlite_string_keys(connection, "select id from timeline_marks", "timeline mark")?;
    let current_timeline_ids = store
        .timeline
        .iter()
        .map(|mark| mark.id.clone())
        .collect::<HashSet<_>>();
    let mirrored_sysmon_keys = sqlite_sysmon_keys(connection)?;
    let current_sysmon_keys = store
        .sysmon
        .iter()
        .map(|snapshot| (snapshot.session_id.clone(), snapshot.ts.to_rfc3339()))
        .collect::<HashSet<_>>();

    for profile in &store.profiles {
        connection
            .execute(
                "insert into profiles (
                    id, name, kind, group_name, tags_json, connection_json, terminal_json,
                    logging_json, triggers_json, transfer_json, updated_at
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                params![
                    profile.id,
                    profile.name,
                    enum_text(&profile.kind)?,
                    profile.group,
                    json_text(&profile.tags)?,
                    json_text(&profile.connection)?,
                    json_text(&profile.terminal)?,
                    json_text(&profile.logging)?,
                    json_text(&profile.triggers)?,
                    json_text(&profile.transfer)?,
                ],
            )
            .map_err(|error| format!("failed to mirror profile {}: {error}", profile.id))?;
    }

    for runtime in &store.runtimes {
        connection
            .execute(
                "insert into runtimes (
                    session_id, pane_id, status, title, cwd, connected_since, last_activity,
                    last_disconnect, last_disconnect_reason, active_transport, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    runtime.session_id,
                    runtime.pane_id,
                    enum_text(&runtime.status)?,
                    runtime.title,
                    runtime.cwd,
                    runtime.connected_since.map(|value| value.to_rfc3339()),
                    runtime.last_activity.to_rfc3339(),
                    runtime.last_disconnect.map(|value| value.to_rfc3339()),
                    runtime.last_disconnect_reason,
                    enum_text(&runtime.active_transport)?,
                    json_text(runtime)?,
                ],
            )
            .map_err(|error| format!("failed to mirror runtime {}: {error}", runtime.session_id))?;
    }

    for event in &store.events {
        let raw_json = json_text(event)?;
        if mirrored_events.get(&event.id) == Some(&raw_json) {
            continue;
        }
        if mirrored_events.contains_key(&event.id) {
            connection
                .execute(
                    "update events set
                        session_id = ?2, pane_id = ?3, ts = ?4, direction = ?5, stream = ?6,
                        bytes_ref = ?7, text = ?8, annotations_json = ?9, raw_json = ?10
                     where id = ?1",
                    params![
                        event.id,
                        event.session_id,
                        event.pane_id,
                        event.ts.to_rfc3339(),
                        enum_text(&event.direction)?,
                        enum_text(&event.stream)?,
                        event.bytes_ref,
                        event.text,
                        json_text(&event.annotations)?,
                        raw_json,
                    ],
                )
                .map_err(|error| {
                    format!("failed to update mirrored event {}: {error}", event.id)
                })?;
        } else {
            connection
                .execute(
                    "insert into events (
                    id, session_id, pane_id, ts, direction, stream, bytes_ref, text,
                    annotations_json, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        event.id,
                        event.session_id,
                        event.pane_id,
                        event.ts.to_rfc3339(),
                        enum_text(&event.direction)?,
                        enum_text(&event.stream)?,
                        event.bytes_ref,
                        event.text,
                        json_text(&event.annotations)?,
                        raw_json,
                    ],
                )
                .map_err(|error| format!("failed to mirror event {}: {error}", event.id))?;
        }
    }
    for id in mirrored_event_ids.difference(&current_event_ids) {
        connection
            .execute("delete from events where id = ?1", params![id])
            .map_err(|error| format!("failed to remove stale mirrored event {id}: {error}"))?;
    }

    for transfer in &store.transfers {
        connection
            .execute(
                "insert into transfers (
                    id, session_id, protocol, source, destination, bytes_total, bytes_done,
                    status, message, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    transfer.id,
                    transfer.session_id,
                    enum_text(&transfer.protocol)?,
                    transfer.source,
                    transfer.destination,
                    transfer.bytes_total as i64,
                    transfer.bytes_done as i64,
                    enum_text(&transfer.status)?,
                    transfer.message,
                    json_text(transfer)?,
                ],
            )
            .map_err(|error| format!("failed to mirror transfer {}: {error}", transfer.id))?;
    }

    for key in &store.host_keys.keys {
        connection
            .execute(
                "insert into trusted_host_keys (
                    id, profile_id, alias, host, port, algorithm, fingerprint_sha256,
                    public_key_base64, scope, label, first_seen, last_seen, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    key.id,
                    key.profile_id,
                    key.alias,
                    key.host,
                    i64::from(key.port),
                    key.algorithm,
                    key.fingerprint_sha256,
                    key.public_key_base64,
                    enum_text(&key.scope)?,
                    key.label,
                    key.first_seen.to_rfc3339(),
                    key.last_seen.to_rfc3339(),
                    json_text(key)?,
                ],
            )
            .map_err(|error| format!("failed to mirror host key {}: {error}", key.id))?;
    }

    for grant in &store.grants {
        connection
            .execute(
                "insert into mcp_grants (
                    client_id, name, scopes_json, allowed_sessions_json, expires_at, revoked_at, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    grant.client_id,
                    grant.name,
                    json_text(&grant.scopes)?,
                    json_text(&grant.allowed_sessions)?,
                    grant.expires_at.map(|value| value.to_rfc3339()),
                    grant.revoked_at.map(|value| value.to_rfc3339()),
                    json_text(grant)?,
                ],
            )
            .map_err(|error| format!("failed to mirror MCP grant {}: {error}", grant.client_id))?;
    }

    for record in &store.audit {
        let raw_json = json_text(record)?;
        if mirrored_audit.get(&record.id) == Some(&raw_json) {
            continue;
        }
        if mirrored_audit.contains_key(&record.id) {
            connection
                .execute(
                    "update mcp_audit set
                        ts = ?2, actor = ?3, action = ?4, session_id = ?5, decision = ?6,
                        details_json = ?7, raw_json = ?8
                     where id = ?1",
                    params![
                        record.id,
                        record.ts.to_rfc3339(),
                        record.actor,
                        record.action,
                        record.session_id,
                        record.decision,
                        json_text(&record.details)?,
                        raw_json,
                    ],
                )
                .map_err(|error| {
                    format!("failed to update mirrored MCP audit {}: {error}", record.id)
                })?;
        } else {
            connection
                .execute(
                    "insert into mcp_audit (
                        id, ts, actor, action, session_id, decision, details_json, raw_json
                    ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        record.id,
                        record.ts.to_rfc3339(),
                        record.actor,
                        record.action,
                        record.session_id,
                        record.decision,
                        json_text(&record.details)?,
                        raw_json,
                    ],
                )
                .map_err(|error| format!("failed to mirror MCP audit {}: {error}", record.id))?;
        }
    }
    for id in mirrored_audit_ids.difference(&current_audit_ids) {
        connection
            .execute("delete from mcp_audit where id = ?1", params![id])
            .map_err(|error| format!("failed to remove stale mirrored MCP audit {id}: {error}"))?;
    }

    for mark in &store.timeline {
        if mirrored_timeline_ids.contains(&mark.id) {
            continue;
        }
        connection
            .execute(
                "insert into timeline_marks (
                    id, session_id, ts, label, details, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    mark.id,
                    mark.session_id,
                    mark.ts.to_rfc3339(),
                    mark.label,
                    mark.details,
                    json_text(mark)?,
                ],
            )
            .map_err(|error| format!("failed to mirror timeline mark {}: {error}", mark.id))?;
    }
    for id in mirrored_timeline_ids.difference(&current_timeline_ids) {
        connection
            .execute("delete from timeline_marks where id = ?1", params![id])
            .map_err(|error| {
                format!("failed to remove stale mirrored timeline mark {id}: {error}")
            })?;
    }

    for snapshot in &store.sysmon {
        let key = (snapshot.session_id.clone(), snapshot.ts.to_rfc3339());
        if mirrored_sysmon_keys.contains(&key) {
            continue;
        }
        connection
            .execute(
                "insert into sysmon_snapshots (
                    session_id, ts, uptime_seconds, cpu_percent, memory_percent, rx_kbps, tx_kbps,
                    details_json, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    snapshot.session_id,
                    snapshot.ts.to_rfc3339(),
                    snapshot.uptime_seconds as i64,
                    snapshot.cpu_percent,
                    snapshot.memory_percent,
                    snapshot.rx_kbps,
                    snapshot.tx_kbps,
                    json_text(&SysmonSnapshotDetails {
                        load_average: snapshot.load_average,
                        memory_total_bytes: snapshot.memory_total_bytes,
                        memory_available_bytes: snapshot.memory_available_bytes,
                        processes: &snapshot.processes,
                        disks: &snapshot.disks,
                        network_interfaces: &snapshot.network_interfaces,
                    })?,
                    json_text(snapshot)?,
                ],
            )
            .map_err(|error| {
                format!(
                    "failed to mirror sysmon snapshot {} {}: {error}",
                    snapshot.session_id, snapshot.ts
                )
            })?;
    }
    for (session_id, ts) in mirrored_sysmon_keys.difference(&current_sysmon_keys) {
        connection
            .execute(
                "delete from sysmon_snapshots where session_id = ?1 and ts = ?2",
                params![session_id, ts],
            )
            .map_err(|error| {
                format!(
                    "failed to remove stale mirrored sysmon snapshot {session_id} {ts}: {error}"
                )
            })?;
    }

    connection
        .execute(
            "insert into metadata (key, value) values ('schemaVersion', ?1)
                on conflict(key) do update set value = excluded.value",
            params![SQLITE_SCHEMA_VERSION],
        )
        .map_err(|error| format!("failed to update SQLite schema version: {error}"))?;
    Ok(())
}

fn sqlite_string_keys(
    connection: &SqliteConnection,
    query: &str,
    label: &str,
) -> Result<HashSet<String>, String> {
    let mut statement = connection
        .prepare(query)
        .map_err(|error| format!("failed to query mirrored {label} keys: {error}"))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| format!("failed to read mirrored {label} keys: {error}"))?;
    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(
            row.map_err(|error| format!("failed to decode mirrored {label} key: {error}"))?,
        );
    }
    Ok(keys)
}

fn sqlite_string_map(
    connection: &SqliteConnection,
    query: &str,
    label: &str,
) -> Result<HashMap<String, String>, String> {
    let mut statement = connection
        .prepare(query)
        .map_err(|error| format!("failed to query mirrored {label} values: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("failed to read mirrored {label} values: {error}"))?;
    let mut values = HashMap::new();
    for row in rows {
        let (key, value) =
            row.map_err(|error| format!("failed to decode mirrored {label} value: {error}"))?;
        values.insert(key, value);
    }
    Ok(values)
}

fn sqlite_sysmon_keys(connection: &SqliteConnection) -> Result<HashSet<(String, String)>, String> {
    let mut statement = connection
        .prepare("select session_id, ts from sysmon_snapshots")
        .map_err(|error| format!("failed to query mirrored sysmon keys: {error}"))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("failed to read mirrored sysmon keys: {error}"))?;
    let mut keys = HashSet::new();
    for row in rows {
        keys.insert(row.map_err(|error| format!("failed to decode mirrored sysmon key: {error}"))?);
    }
    Ok(keys)
}

fn json_text<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value)
        .map_err(|error| format!("failed to encode SQLite JSON mirror: {error}"))
}

fn enum_text<T: Serialize>(value: &T) -> Result<String, String> {
    let value = serde_json::to_value(value)
        .map_err(|error| format!("failed to encode enum for SQLite mirror: {error}"))?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "expected enum to serialize as a string".to_string())
}

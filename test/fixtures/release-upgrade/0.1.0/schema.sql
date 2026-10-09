create table kv (
    key text primary key not null,
    value text not null,
    updated_at text not null
);
create table metadata (
    key text primary key not null,
    value text not null
);
create table profiles (
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
create table runtimes (
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
create table events (
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
create table transfers (
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
create table trusted_host_keys (
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
create table mcp_grants (
    client_id text primary key not null,
    name text not null,
    scopes_json text not null,
    allowed_sessions_json text not null,
    expires_at text,
    revoked_at text,
    raw_json text not null
);
create table mcp_audit (
    id text primary key not null,
    ts text not null,
    actor text not null,
    action text not null,
    session_id text,
    decision text not null,
    details_json text not null,
    raw_json text not null
);
create table timeline_marks (
    id text primary key not null,
    session_id text not null,
    ts text not null,
    label text not null,
    details text,
    raw_json text not null
);
create table sysmon_snapshots (
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
create table profile_secret_migrations (
    id text primary key not null,
    state text not null,
    active integer not null check (active in (0, 1)),
    payload_json text not null,
    created_at text not null,
    updated_at text not null
);
create index idx_events_session_ts on events(session_id, ts);
create index idx_events_text on events(text);
create index idx_transfers_session on transfers(session_id);
create index idx_host_keys_alias on trusted_host_keys(alias, port, algorithm);
create index idx_audit_session_ts on mcp_audit(session_id, ts);
create index idx_timeline_session_ts on timeline_marks(session_id, ts);
create index idx_sysmon_session_ts on sysmon_snapshots(session_id, ts);
create unique index idx_profile_secret_migrations_active
    on profile_secret_migrations(active) where active = 1;
insert into metadata (key, value) values ('schemaVersion', '4');

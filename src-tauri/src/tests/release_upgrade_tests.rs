use super::*;

const PREVIOUS_STORE: &str =
    include_str!("../../../tests/fixtures/release-upgrade/0.1.0/session-store.json");
const PREVIOUS_JOURNAL: &str =
    include_str!("../../../tests/fixtures/release-upgrade/0.1.0/profile-secret-migration.json");
const PREVIOUS_LOG: &[u8] =
    include_bytes!("../../../tests/fixtures/release-upgrade/0.1.0/logs/release-ssh-1.jsonl");
const PREVIOUS_BROWSER_STATE: &[u8] =
    include_bytes!("../../../tests/fixtures/release-upgrade/0.1.0/browser-state.json");
const PREVIOUS_SCHEMA: &str =
    include_str!("../../../tests/fixtures/release-upgrade/0.1.0/schema.sql");

fn required_string<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("release fixture field {key} must be a string"))
}

fn optional_string(value: &serde_json::Value, key: &str) -> Option<String> {
    value[key].as_str().map(ToOwned::to_owned)
}

fn write_previous_release_store(path: &Path) {
    let store: serde_json::Value = serde_json::from_str(PREVIOUS_STORE).unwrap();
    let connection = SqliteConnection::open(path).unwrap();
    connection.execute_batch(PREVIOUS_SCHEMA).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
    connection
        .execute(
            "insert into kv (key, value, updated_at) values (?1, ?2, ?3)",
            params![STORE_KEY, PREVIOUS_STORE, "2026-08-11T11:07:00Z"],
        )
        .unwrap();
    connection
        .execute(
            "insert into metadata (key, value) values ('sourceRelease', '0.1.0')",
            [],
        )
        .unwrap();
    connection
        .execute(
            "insert into metadata (key, value) values ('storeRevision', ?1)",
            params!["44444444-4444-4444-8444-444444444444"],
        )
        .unwrap();

    for profile in store["profiles"].as_array().unwrap() {
        connection
            .execute(
                "insert into profiles (
                    id, name, kind, group_name, tags_json, connection_json, terminal_json,
                    logging_json, triggers_json, transfer_json, updated_at
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    required_string(profile, "id"),
                    required_string(profile, "name"),
                    required_string(profile, "kind"),
                    required_string(profile, "group"),
                    profile["tags"].to_string(),
                    profile["connection"].to_string(),
                    profile["terminal"].to_string(),
                    profile["logging"].to_string(),
                    profile["triggers"].to_string(),
                    profile["transfer"].to_string(),
                    "2026-08-11T11:07:00Z",
                ],
            )
            .unwrap();
    }

    for runtime in store["runtimes"].as_array().unwrap() {
        connection
            .execute(
                "insert into runtimes (
                    session_id, pane_id, status, title, cwd, connected_since, last_activity,
                    last_disconnect, last_disconnect_reason, active_transport, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    required_string(runtime, "sessionId"),
                    required_string(runtime, "paneId"),
                    required_string(runtime, "status"),
                    required_string(runtime, "title"),
                    optional_string(runtime, "cwd"),
                    optional_string(runtime, "connectedSince"),
                    required_string(runtime, "lastActivity"),
                    optional_string(runtime, "lastDisconnect"),
                    optional_string(runtime, "lastDisconnectReason"),
                    required_string(runtime, "activeTransport"),
                    runtime.to_string(),
                ],
            )
            .unwrap();
    }

    for event in store["events"].as_array().unwrap() {
        connection
            .execute(
                "insert into events (
                    id, session_id, pane_id, ts, direction, stream, bytes_ref, text,
                    annotations_json, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    required_string(event, "id"),
                    required_string(event, "sessionId"),
                    required_string(event, "paneId"),
                    required_string(event, "ts"),
                    required_string(event, "direction"),
                    required_string(event, "stream"),
                    optional_string(event, "bytesRef"),
                    optional_string(event, "text"),
                    event["annotations"].to_string(),
                    event.to_string(),
                ],
            )
            .unwrap();
    }

    for key in store["hostKeys"]["keys"].as_array().unwrap() {
        connection
            .execute(
                "insert into trusted_host_keys (
                    id, profile_id, alias, host, port, algorithm, fingerprint_sha256,
                    public_key_base64, scope, label, first_seen, last_seen, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    required_string(key, "id"),
                    optional_string(key, "profileId"),
                    required_string(key, "alias"),
                    required_string(key, "host"),
                    key["port"].as_u64().unwrap() as i64,
                    required_string(key, "algorithm"),
                    required_string(key, "fingerprintSha256"),
                    required_string(key, "publicKeyBase64"),
                    required_string(key, "scope"),
                    optional_string(key, "label"),
                    required_string(key, "firstSeen"),
                    required_string(key, "lastSeen"),
                    key.to_string(),
                ],
            )
            .unwrap();
    }

    for grant in store["grants"].as_array().unwrap() {
        connection
            .execute(
                "insert into mcp_grants (
                    client_id, name, scopes_json, allowed_sessions_json, expires_at,
                    revoked_at, raw_json
                ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    required_string(grant, "clientId"),
                    required_string(grant, "name"),
                    grant["scopes"].to_string(),
                    grant["allowedSessions"].to_string(),
                    optional_string(grant, "expiresAt"),
                    optional_string(grant, "revokedAt"),
                    grant.to_string(),
                ],
            )
            .unwrap();
    }

    let journal: serde_json::Value = serde_json::from_str(PREVIOUS_JOURNAL).unwrap();
    connection
        .execute(
            "insert into profile_secret_migrations
                (id, state, active, payload_json, created_at, updated_at)
             values (?1, ?2, 1, ?3, ?4, ?5)",
            params![
                required_string(&journal["payload"], "migrationId"),
                required_string(&journal, "state"),
                journal["payload"].to_string(),
                required_string(&journal, "createdAt"),
                required_string(&journal, "updatedAt"),
            ],
        )
        .unwrap();
    connection.execute_batch("COMMIT;").unwrap();
}

fn assert_release_data(store: &SessionStore) {
    assert_eq!(store.profiles.len(), 1);
    let profile = &store.profiles[0];
    assert_eq!(profile.id, "release-ssh-1");
    assert_eq!(profile.name, "0.1.0 Lab Router");
    let ConnectionConfig::Ssh(ssh) = &profile.connection else {
        panic!("release fixture SSH profile changed transport");
    };
    assert_eq!(
        ssh.password_secret_ref.as_deref(),
        Some("keychain:release-password")
    );
    assert_eq!(
        ssh.passphrase_secret_ref.as_deref(),
        Some("keychain:release-passphrase")
    );
    assert_eq!(ssh.tunnels.len(), 1);
    assert!(ssh.tunnels[0].route_rules.is_empty());
    assert_eq!(store.host_keys.keys.len(), 1);
    assert_eq!(store.host_keys.keys[0].id, "release-host-key-1");
    assert_eq!(store.grants.len(), 1);
    assert_eq!(store.grants[0].client_id, "release-upgrade-client");
    assert_eq!(store.grants[0].allowed_sessions, ["release-ssh-1"]);
    assert_eq!(store.events.len(), 1);
    assert_eq!(store.events[0].id, "release-event-1");
    assert_eq!(
        store.events[0]
            .annotations
            .get("fixture")
            .map(String::as_str),
        Some("previous-release")
    );
    assert_eq!(
        store
            .command_history
            .iter()
            .map(|entry| entry.command.as_str())
            .collect::<Vec<_>>(),
        ["show version", "show interfaces"]
    );
    assert!(store.command_history_migrated);
    assert_eq!(store.command_history_revision, 2);
    assert_eq!(store.mcp_http_settings.client_host, "127.0.0.1");
}

#[test]
fn previous_release_app_data_survives_directory_store_mirror_and_journal_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join(LEGACY_APP_IDENTIFIER);
    let current = root.path().join("dev.portmate.desktop");
    fs::create_dir_all(legacy.join("logs")).unwrap();
    let webview_state_path = Path::new("WebKit/WebsiteData/LocalStorage/release-fixture.json");
    fs::create_dir_all(legacy.join(webview_state_path.parent().unwrap())).unwrap();
    let legacy_store = legacy.join(STORE_FILE_NAME);
    write_previous_release_store(&legacy_store);
    fs::write(legacy.join("logs/release-ssh-1.jsonl"), PREVIOUS_LOG).unwrap();
    fs::write(legacy.join(webview_state_path), PREVIOUS_BROWSER_STATE).unwrap();

    let before = SqliteConnection::open(&legacy_store).unwrap();
    let old_connection_json: String = before
        .query_row(
            "select connection_json from profiles where id = 'release-ssh-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!old_connection_json.contains("routeRules"));
    let old_canonical: String = before
        .query_row(
            "select value from kv where key = ?1",
            params![STORE_KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!old_canonical.contains("clientHost"));
    drop(before);

    migrate_legacy_app_data_dir(root.path(), &current).unwrap();

    assert!(!legacy.exists());
    let store_path = current.join(STORE_FILE_NAME);
    assert_eq!(
        fs::read(current.join("logs/release-ssh-1.jsonl")).unwrap(),
        PREVIOUS_LOG
    );
    assert_eq!(
        fs::read(current.join(webview_state_path)).unwrap(),
        PREVIOUS_BROWSER_STATE
    );
    let loaded = load_store(&store_path).unwrap();
    assert_release_data(&loaded);
    let journal = load_profile_secret_migration_journal(&store_path)
        .unwrap()
        .expect("0.1.0 credential journal disappeared during loading");
    assert_eq!(
        journal.state,
        ProfileSecretMigrationJournalState::TargetWritePending
    );
    assert_eq!(
        journal.payload.migration_id,
        "33333333-3333-4333-8333-333333333333"
    );
    assert_eq!(journal.payload.profiles[0].profile_id, "release-ssh-1");

    save_store(&store_path, &loaded).unwrap();
    flush_json_compatibility_snapshot(&store_path, Duration::from_secs(5)).unwrap();
    let reloaded = load_store(&store_path).unwrap();
    assert_release_data(&reloaded);
    let compatibility = load_store_json(&current.join(LEGACY_JSON_STORE_FILE_NAME)).unwrap();
    assert_release_data(&compatibility);

    let connection = SqliteConnection::open(&store_path).unwrap();
    let source_release: String = connection
        .query_row(
            "select value from metadata where key = 'sourceRelease'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(source_release, "0.1.0");
    for (table, expected) in [
        ("profiles", 1_i64),
        ("runtimes", 1),
        ("events", 1),
        ("trusted_host_keys", 1),
        ("mcp_grants", 1),
        ("profile_secret_migrations", 1),
    ] {
        let count: i64 = connection
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, expected, "upgraded mirror table {table}");
    }
    let upgraded_connection_json: String = connection
        .query_row(
            "select connection_json from profiles where id = 'release-ssh-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(upgraded_connection_json.contains("routeRules"));
    let upgraded_canonical: String = connection
        .query_row(
            "select value from kv where key = ?1",
            params![STORE_KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert!(upgraded_canonical.contains("clientHost"));
    drop(connection);

    let preserved_journal = load_profile_secret_migration_journal(&store_path)
        .unwrap()
        .expect("credential journal disappeared after the upgraded Store was rewritten");
    assert_eq!(preserved_journal.state, journal.state);
    assert_eq!(preserved_journal.payload, journal.payload);
    assert_eq!(preserved_journal.created_at, journal.created_at);
    assert_eq!(preserved_journal.updated_at, journal.updated_at);
}

#[test]
fn previous_release_upgrade_refuses_two_nonempty_app_data_directories() {
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join(LEGACY_APP_IDENTIFIER);
    let current = root.path().join("dev.portmate.desktop");
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&current).unwrap();
    let legacy_store = legacy.join(STORE_FILE_NAME);
    write_previous_release_store(&legacy_store);
    let current_store = current.join(STORE_FILE_NAME);
    fs::write(&current_store, b"current-store-must-remain-unchanged").unwrap();
    let legacy_before = fs::read(&legacy_store).unwrap();

    let error = migrate_legacy_app_data_dir(root.path(), &current).unwrap_err();

    assert!(error.contains("refusing to merge"), "{error}");
    assert_eq!(fs::read(&legacy_store).unwrap(), legacy_before);
    assert_eq!(
        fs::read(&current_store).unwrap(),
        b"current-store-must-remain-unchanged"
    );
}

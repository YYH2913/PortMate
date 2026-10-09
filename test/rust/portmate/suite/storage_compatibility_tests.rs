#[test]
fn json_compatibility_store_is_private_atomic_and_symlink_safe() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join(LEGACY_JSON_STORE_FILE_NAME);
    let mut store = SessionStore::default();
    store.upsert_profile(test_shell_profile());
    fs::write(&store_path, b"old snapshot").unwrap();

    save_store_json(&store_path, &store).unwrap();

    assert_eq!(
        load_store_json(&store_path).unwrap().profiles[0].name,
        "Bench/Device"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};

        let protected_path = temp.path().join("protected.txt");
        assert_eq!(
            fs::metadata(&store_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::write(&protected_path, b"must remain unchanged").unwrap();
        let predictable_temp_path = store_path.with_file_name(format!(
            "{}.tmp",
            store_path.file_name().unwrap().to_string_lossy()
        ));
        symlink(&protected_path, &predictable_temp_path).unwrap();
        store.profiles[0].name = "Updated profile".to_string();

        save_store_json(&store_path, &store).unwrap();

        assert_eq!(fs::read(&protected_path).unwrap(), b"must remain unchanged");
        assert!(fs::symlink_metadata(&predictable_temp_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            load_store_json(&store_path).unwrap().profiles[0].name,
            "Updated profile"
        );

        fs::remove_file(&store_path).unwrap();
        symlink(&protected_path, &store_path).unwrap();
        save_store_json(&store_path, &store).unwrap();

        assert_eq!(fs::read(&protected_path).unwrap(), b"must remain unchanged");
        assert!(!fs::symlink_metadata(&store_path)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::metadata(&store_path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn json_compatibility_snapshot_queue_coalesces_to_the_latest_store() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join("portmate-store.sqlite3");
    let compatibility_path = temp.path().join(LEGACY_JSON_STORE_FILE_NAME);
    let mut first = SessionStore::default();
    first.upsert_profile(test_shell_profile());
    let mut latest = first.clone();
    latest.profiles[0].name = "latest compatibility snapshot".to_string();

    enqueue_json_compatibility_snapshot(&store_path, &first).unwrap();
    enqueue_json_compatibility_snapshot(&store_path, &latest).unwrap();
    flush_json_compatibility_snapshot(&store_path, Duration::from_secs(5)).unwrap();

    let persisted = load_store_json(&compatibility_path).unwrap();
    assert_eq!(persisted.profiles[0].name, "latest compatibility snapshot");
}

#[test]
fn sqlite_save_updates_the_json_compatibility_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join("portmate-store.sqlite3");
    let compatibility_path = temp.path().join(LEGACY_JSON_STORE_FILE_NAME);
    let mut store = SessionStore::default();
    store.upsert_profile(test_shell_profile());
    let history_recorded_at = Utc::now().timestamp_millis();
    store
        .record_command_history(
            "git status".to_string(),
            Some("test-session".to_string()),
            100,
            30,
            history_recorded_at,
        )
        .unwrap();

    save_store(&store_path, &store).unwrap();

    let compatibility =
        serde_json::to_value(load_store_json(&compatibility_path).unwrap()).unwrap();
    let canonical = serde_json::to_value(load_store_sqlite(&store_path).unwrap()).unwrap();
    assert_eq!(compatibility, canonical);
    assert_eq!(canonical["commandHistory"][0]["command"], "git status");
    assert_eq!(canonical["commandHistoryRevision"], 1);
}

#[test]
fn sqlite_mirror_incrementally_syncs_history_tables() {
    let root = std::env::temp_dir().join(format!("portmate-sqlite-mirror-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    store.record_system_event(&session_id, "first event");
    store
        .send_text("test-user", &session_id, "second event")
        .unwrap();
    store.record_timeline_mark(TimelineMark {
        id: "timeline-1".to_string(),
        session_id: session_id.clone(),
        ts: Utc::now(),
        label: "checkpoint".to_string(),
        details: None,
    });
    store.record_sysmon_snapshot(SysmonSnapshot {
        session_id: session_id.clone(),
        ts: Utc::now(),
        uptime_seconds: 10,
        cpu_percent: 1.0,
        memory_percent: 2.0,
        rx_kbps: 3.0,
        tx_kbps: 4.0,
        load_average: [0.1, 0.2, 0.3],
        memory_total_bytes: 1_024,
        memory_available_bytes: 512,
        processes: Vec::new(),
        disks: Vec::new(),
        network_interfaces: Vec::new(),
    });
    save_store(&store_path, &store).unwrap();

    let connection = SqliteConnection::open(&store_path).unwrap();
    let schema_version: String = connection
        .query_row(
            "select value from metadata where key = 'schemaVersion'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(schema_version, SQLITE_SCHEMA_VERSION);
    let details_json: String = connection
        .query_row(
            "select details_json from sysmon_snapshots where session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .unwrap();
    let details: serde_json::Value = serde_json::from_str(&details_json).unwrap();
    assert_eq!(details["loadAverage"], serde_json::json!([0.1, 0.2, 0.3]));
    assert_eq!(details["memoryTotalBytes"], 1_024);
    connection
        .execute_batch(
            "create table mirror_test_counts (
                table_name text not null,
                action text not null,
                count integer not null,
                primary key (table_name, action)
            );
            insert into mirror_test_counts values
                ('events', 'insert', 0), ('events', 'delete', 0),
                ('mcp_audit', 'insert', 0), ('mcp_audit', 'update', 0),
                ('mcp_audit', 'delete', 0),
                ('timeline_marks', 'insert', 0), ('timeline_marks', 'delete', 0),
                ('sysmon_snapshots', 'insert', 0), ('sysmon_snapshots', 'delete', 0);
            create trigger mirror_test_events_insert after insert on events begin
                update mirror_test_counts set count = count + 1
                where table_name = 'events' and action = 'insert';
            end;
            create trigger mirror_test_events_delete after delete on events begin
                update mirror_test_counts set count = count + 1
                where table_name = 'events' and action = 'delete';
            end;
            create trigger mirror_test_audit_insert after insert on mcp_audit begin
                update mirror_test_counts set count = count + 1
                where table_name = 'mcp_audit' and action = 'insert';
            end;
            create trigger mirror_test_audit_update after update on mcp_audit begin
                update mirror_test_counts set count = count + 1
                where table_name = 'mcp_audit' and action = 'update';
            end;
            create trigger mirror_test_audit_delete after delete on mcp_audit begin
                update mirror_test_counts set count = count + 1
                where table_name = 'mcp_audit' and action = 'delete';
            end;
            create trigger mirror_test_timeline_insert after insert on timeline_marks begin
                update mirror_test_counts set count = count + 1
                where table_name = 'timeline_marks' and action = 'insert';
            end;
            create trigger mirror_test_timeline_delete after delete on timeline_marks begin
                update mirror_test_counts set count = count + 1
                where table_name = 'timeline_marks' and action = 'delete';
            end;
            create trigger mirror_test_sysmon_insert after insert on sysmon_snapshots begin
                update mirror_test_counts set count = count + 1
                where table_name = 'sysmon_snapshots' and action = 'insert';
            end;
            create trigger mirror_test_sysmon_delete after delete on sysmon_snapshots begin
                update mirror_test_counts set count = count + 1
                where table_name = 'sysmon_snapshots' and action = 'delete';
            end;",
        )
        .unwrap();
    drop(connection);

    let updated_event_id = store.events[0].id.clone();
    store.events[0]
        .annotations
        .insert("loggingError".to_string(), "text shard failed".to_string());
    let updated_audit_id = store.audit[0].id.clone();
    store.audit[0].decision = "succeeded".to_string();
    store.record_system_event(&session_id, "third event");
    save_store(&store_path, &store).unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    let mirror_count = |table: &str, action: &str| -> i64 {
        connection
            .query_row(
                "select count from mirror_test_counts where table_name = ?1 and action = ?2",
                params![table, action],
                |row| row.get(0),
            )
            .unwrap()
    };
    assert_eq!(mirror_count("events", "insert"), 1);
    assert_eq!(mirror_count("events", "delete"), 0);
    assert_eq!(mirror_count("mcp_audit", "insert"), 0);
    assert_eq!(mirror_count("mcp_audit", "update"), 1);
    assert_eq!(mirror_count("mcp_audit", "delete"), 0);
    for table in ["timeline_marks", "sysmon_snapshots"] {
        assert_eq!(mirror_count(table, "insert"), 0, "{table}");
        assert_eq!(mirror_count(table, "delete"), 0, "{table}");
    }
    let (updated_raw_json, updated_annotations_json): (String, String) = connection
        .query_row(
            "select raw_json, annotations_json from events where id = ?1",
            params![updated_event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let updated_event: SessionEvent = serde_json::from_str(&updated_raw_json).unwrap();
    assert_eq!(
        updated_event
            .annotations
            .get("loggingError")
            .map(String::as_str),
        Some("text shard failed")
    );
    let updated_annotations: BTreeMap<String, String> =
        serde_json::from_str(&updated_annotations_json).unwrap();
    assert_eq!(
        updated_annotations.get("loggingError").map(String::as_str),
        Some("text shard failed")
    );
    let (updated_decision, updated_audit_json): (String, String) = connection
        .query_row(
            "select decision, raw_json from mcp_audit where id = ?1",
            params![updated_audit_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(updated_decision, "succeeded");
    let updated_audit: AuditRecord = serde_json::from_str(&updated_audit_json).unwrap();
    assert_eq!(updated_audit.decision, "succeeded");
    connection
        .execute("update mirror_test_counts set count = 0", [])
        .unwrap();
    drop(connection);

    let removed_event_id = store.events[0].id.clone();
    store.events.remove(0);
    store.audit.clear();
    store.timeline.clear();
    store.sysmon.clear();
    save_store(&store_path, &store).unwrap();

    let connection = SqliteConnection::open(&store_path).unwrap();
    let mirror_count = |table: &str, action: &str| -> i64 {
        connection
            .query_row(
                "select count from mirror_test_counts where table_name = ?1 and action = ?2",
                params![table, action],
                |row| row.get(0),
            )
            .unwrap()
    };
    for table in ["events", "mcp_audit", "timeline_marks", "sysmon_snapshots"] {
        assert_eq!(mirror_count(table, "insert"), 0, "{table}");
        assert_eq!(mirror_count(table, "delete"), 1, "{table}");
    }
    let removed_count: i64 = connection
        .query_row(
            "select count(*) from events where id = ?1",
            params![removed_event_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(removed_count, 0);
    drop(connection);

    let loaded = load_store_sqlite(&store_path).unwrap();
    assert_eq!(loaded.events, store.events);
    assert!(loaded.audit.is_empty());
    assert!(loaded.timeline.is_empty());
    assert!(loaded.sysmon.is_empty());
    let _ = fs::remove_dir_all(root);
}
#[test]
fn sysmon_schema_migrates_existing_summary_rows_to_details_json() {
    let root = std::env::temp_dir().join(format!("portmate-sysmon-schema-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute_batch(
            "create table sysmon_snapshots (
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
            insert into sysmon_snapshots values
                ('legacy', '2026-07-14T10:00:00Z', 1, 2.0, 3.0, 4.0, 5.0, '{}');",
        )
        .unwrap();

    ensure_store_schema(&connection).unwrap();
    let details: String = connection
        .query_row(
            "select details_json from sysmon_snapshots where session_id = 'legacy'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(details, "{}");
    let schema_version: String = connection
        .query_row(
            "select value from metadata where key = 'schemaVersion'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(schema_version, SQLITE_SCHEMA_VERSION);
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

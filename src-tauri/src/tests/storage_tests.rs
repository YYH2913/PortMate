use super::*;

#[test]
fn json_compatibility_store_is_private_atomic_and_symlink_safe() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join(LEGACY_JSON_STORE_FILE_NAME);
    let protected_path = temp.path().join("protected.txt");
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

    save_store(&store_path, &store).unwrap();

    let compatibility =
        serde_json::to_value(load_store_json(&compatibility_path).unwrap()).unwrap();
    let canonical = serde_json::to_value(load_store_sqlite(&store_path).unwrap()).unwrap();
    assert_eq!(compatibility, canonical);
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

#[test]
fn store_snapshot_cas_rejects_a_stale_second_instance() {
    let root = std::env::temp_dir().join(format!("portmate-store-cas-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let mut initial = SessionStore::default();
    initial.upsert_profile(test_shell_profile());
    save_store(&store_path, &initial).unwrap();

    let mut second_instance_version = store_snapshot_version(&store_path).unwrap();
    let mut first_instance = load_store(&store_path).unwrap();
    let mut second_instance = first_instance.clone();
    second_instance.profiles[0].name = "saved by second instance".to_string();
    save_store_with_expected_snapshot_version(
        &store_path,
        &second_instance,
        &mut second_instance_version,
    )
    .unwrap();

    first_instance.profiles[0].name = "stale first instance".to_string();
    let preflight_error = verify_store_snapshot_is_current(&store_path).unwrap_err();
    assert!(
        preflight_error.contains(PROFILE_SECRET_MIGRATION_RESTART_REQUIRED),
        "{preflight_error}"
    );
    let error = save_store(&store_path, &first_instance).unwrap_err();
    assert!(error.contains("另一实例修改"), "{error}");
    let persisted = load_store_sqlite(&store_path).unwrap();
    assert_eq!(persisted.profiles[0].name, "saved by second instance");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn verified_store_commit_repairs_an_unknown_cached_version() {
    let root = std::env::temp_dir().join(format!("portmate-store-verify-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let mut store = SessionStore::default();
    store.upsert_profile(test_shell_profile());
    save_store(&store_path, &store).unwrap();
    store.profiles[0].name = "verified commit".to_string();
    save_store(&store_path, &store).unwrap();

    STORE_SNAPSHOT_VERSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(store_path.clone(), StoreSnapshotVersion::UnknownAfterCommit);
    assert!(verify_persisted_store_commit(&store_path, &store).unwrap());
    let cached = STORE_SNAPSHOT_VERSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()[&store_path];
    assert!(matches!(cached, StoreSnapshotVersion::Sha256(_)));

    let mut different = store.clone();
    different.profiles[0].name = "not persisted".to_string();
    assert!(!verify_persisted_store_commit(&store_path, &different).unwrap());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn journal_mutations_are_durable_barriers_and_advance_store_revision() {
    let fixture = test_migration_journal_fixture();
    let root = std::env::temp_dir().join(format!("portmate-journal-cas-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    save_store(&store_path, &fixture.before).unwrap();
    let version_before = store_snapshot_version(&store_path).unwrap();

    persist_profile_secret_migration_journal_event(
        &store_path,
        ProfileSecretMigrationJournalEvent::Prepared(fixture.journal.payload.clone()),
    )
    .unwrap();
    let version_prepared = store_snapshot_version(&store_path).unwrap();
    assert_ne!(version_prepared, version_before);
    let loaded = load_profile_secret_migration_journal(&store_path)
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.state,
        ProfileSecretMigrationJournalState::TargetWritePending
    );
    assert_eq!(loaded.payload, fixture.journal.payload);
    let encoded = serde_json::to_string(&loaded.payload).unwrap();
    for secret in ["private-a", "private-b"] {
        assert!(!encoded.contains(secret));
        assert!(!encoded.contains(&format!("{:x}", Sha256::digest(secret.as_bytes()))));
    }

    let mut stale = version_before;
    let error = save_store_with_expected_snapshot_version(&store_path, &fixture.before, &mut stale)
        .unwrap_err();
    assert!(error.contains("另一实例修改"), "{error}");
    persist_profile_secret_migration_journal_event(
        &store_path,
        ProfileSecretMigrationJournalEvent::Transition {
            migration_id: fixture.journal.payload.migration_id.clone(),
            state: ProfileSecretMigrationJournalState::TargetsVerified,
        },
    )
    .unwrap();
    let version_verified = store_snapshot_version(&store_path).unwrap();
    assert_ne!(version_verified, version_prepared);
    assert_eq!(
        load_profile_secret_migration_journal(&store_path)
            .unwrap()
            .unwrap()
            .state,
        ProfileSecretMigrationJournalState::TargetsVerified
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn profile_and_profiles_committed_checkpoint_are_one_validated_transaction() {
    let fixture = test_migration_journal_fixture();
    let root = std::env::temp_dir().join(format!("portmate-journal-atomic-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    save_store(&store_path, &fixture.before).unwrap();
    persist_profile_secret_migration_journal_event(
        &store_path,
        ProfileSecretMigrationJournalEvent::Prepared(fixture.journal.payload.clone()),
    )
    .unwrap();

    let error = save_store_with_profile_secret_migration_checkpoint(
        &store_path,
        &fixture.after,
        &fixture.journal.payload.migration_id,
    )
    .unwrap_err();
    assert!(error.contains("invalid profile secret migration checkpoint"));
    assert_eq!(
        profile_secret_migration_projection(&load_store_sqlite(&store_path).unwrap().profiles[0])
            .unwrap(),
        fixture.journal.payload.profiles[0].before
    );
    assert_eq!(
        load_profile_secret_migration_journal(&store_path)
            .unwrap()
            .unwrap()
            .state,
        ProfileSecretMigrationJournalState::TargetWritePending
    );

    persist_profile_secret_migration_journal_event(
        &store_path,
        ProfileSecretMigrationJournalEvent::Transition {
            migration_id: fixture.journal.payload.migration_id.clone(),
            state: ProfileSecretMigrationJournalState::TargetsVerified,
        },
    )
    .unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute_batch(
            "create trigger fail_profile_migration_checkpoint
             before update of state on profile_secret_migrations
             when new.state = 'profiles-committed'
             begin select raise(abort, 'injected checkpoint failure'); end;",
        )
        .unwrap();
    drop(connection);
    assert!(save_store_with_profile_secret_migration_checkpoint(
        &store_path,
        &fixture.after,
        &fixture.journal.payload.migration_id,
    )
    .is_err());
    assert_eq!(
        profile_secret_migration_projection(&load_store_sqlite(&store_path).unwrap().profiles[0])
            .unwrap(),
        fixture.journal.payload.profiles[0].before
    );
    assert_eq!(
        load_profile_secret_migration_journal(&store_path)
            .unwrap()
            .unwrap()
            .state,
        ProfileSecretMigrationJournalState::TargetsVerified
    );

    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute_batch("drop trigger fail_profile_migration_checkpoint;")
        .unwrap();
    drop(connection);
    save_store_with_profile_secret_migration_checkpoint(
        &store_path,
        &fixture.after,
        &fixture.journal.payload.migration_id,
    )
    .unwrap();
    assert_eq!(
        profile_secret_migration_projection(&load_store_sqlite(&store_path).unwrap().profiles[0])
            .unwrap(),
        fixture.journal.payload.profiles[0].after
    );
    assert_eq!(
        load_profile_secret_migration_journal(&store_path)
            .unwrap()
            .unwrap()
            .state,
        ProfileSecretMigrationJournalState::ProfilesCommitted
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn prepared_journal_is_visible_before_any_target_side_effect() {
    let fixture = test_migration_journal_fixture();
    let root = std::env::temp_dir().join(format!("portmate-journal-barrier-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    save_store(&store_path, &fixture.before).unwrap();
    let target_side_effect = std::cell::Cell::new(false);
    let mut in_memory = fixture.before.clone();
    let error = migrate_profile_secrets_with_journal_io(
        &mut in_memory,
        &ProfileSecretMigrationRequest {
            target_storage: SecretStorage::Portable,
            profile_ids: vec!["ssh-session-1".to_string()],
            cleanup_source: true,
        },
        |secret_ref| {
            fixture
                .values
                .get(secret_ref)
                .cloned()
                .ok_or_else(|| "missing".to_string())
        },
        |_, _| {
            let loaded = load_profile_secret_migration_journal(&store_path)
                .unwrap()
                .unwrap();
            assert_eq!(
                loaded.state,
                ProfileSecretMigrationJournalState::TargetWritePending
            );
            assert_eq!(
                profile_secret_migration_projection(
                    &load_store_sqlite(&store_path).unwrap().profiles[0]
                )
                .unwrap(),
                loaded.payload.profiles[0].before
            );
            target_side_effect.set(true);
            Err("injected provider failure".to_string())
        },
        |_, _| panic!("write failure recovery must be driven by the durable journal"),
        |_, _, _, _| panic!("Profile commit must not run after target write failure"),
        |event| persist_profile_secret_migration_journal_event(&store_path, event),
    )
    .unwrap_err();
    assert!(target_side_effect.get());
    assert!(error.contains("injected provider failure"));
    assert_eq!(
        profile_secret_migration_projection(&in_memory.profiles[0]).unwrap(),
        fixture.journal.payload.profiles[0].before
    );
    assert_eq!(
        load_profile_secret_migration_journal(&store_path)
            .unwrap()
            .unwrap()
            .state,
        ProfileSecretMigrationJournalState::TargetCleanupPending
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corrupt_active_journal_freezes_new_credential_mutations() {
    let fixture = test_migration_journal_fixture();
    let root = std::env::temp_dir().join(format!("portmate-journal-corrupt-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    save_store(&store_path, &fixture.before).unwrap();
    persist_profile_secret_migration_journal_event(
        &store_path,
        ProfileSecretMigrationJournalEvent::Prepared(fixture.journal.payload.clone()),
    )
    .unwrap();
    assert!(ensure_no_pending_profile_secret_migration(&store_path)
        .unwrap_err()
        .contains("待恢复"));
    let mut unsupported = fixture.journal.payload.clone();
    unsupported.version = PROFILE_SECRET_MIGRATION_JOURNAL_VERSION + 1;
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute(
            "update profile_secret_migrations set payload_json = ?1 where active = 1",
            params![serde_json::to_string(&unsupported).unwrap()],
        )
        .unwrap();
    drop(connection);
    assert!(ensure_no_pending_profile_secret_migration(&store_path)
        .unwrap_err()
        .contains("不支持"));
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute(
            "update profile_secret_migrations set payload_json = '{broken' where active = 1",
            [],
        )
        .unwrap();
    drop(connection);
    assert!(ensure_no_pending_profile_secret_migration(&store_path)
        .unwrap_err()
        .contains("JSON 损坏"));
    let connection = SqliteConnection::open(&store_path).unwrap();
    let active = connection
        .query_row(
            "select count(*) from profile_secret_migrations where active = 1",
            [],
            |row| row.get::<_, usize>(0),
        )
        .unwrap();
    assert_eq!(active, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn store_snapshot_version_detects_kv_changes_without_revision_changes() {
    let root = std::env::temp_dir().join(format!("portmate-store-cas-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let mut initial = SessionStore::default();
    initial.upsert_profile(test_shell_profile());
    save_store(&store_path, &initial).unwrap();

    let mut expected = store_snapshot_version(&store_path).unwrap();
    let mut externally_changed = initial.clone();
    externally_changed.profiles[0].name = "legacy writer".to_string();
    let raw = serde_json::to_string_pretty(&externally_changed).unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute(
            "update kv set value = ?1 where key = ?2",
            params![raw, STORE_KEY],
        )
        .unwrap();
    drop(connection);

    assert_ne!(store_snapshot_version(&store_path).unwrap(), expected);
    let error = save_store_with_expected_snapshot_version(&store_path, &initial, &mut expected)
        .unwrap_err();
    assert!(error.contains("另一实例修改"), "{error}");
    assert_eq!(
        load_store_sqlite(&store_path).unwrap().profiles[0].name,
        "legacy writer"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn state_file_hash_matches_in_memory_sha256_across_reader_chunks() {
    let root = std::env::temp_dir().join(format!("portmate-file-hash-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("snapshot.bin");
    let bytes = (0..STATE_FILE_HASH_BUFFER_BYTES.saturating_add(17))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&path, &bytes).unwrap();

    assert_eq!(
        sha256_file_digest(&path).unwrap(),
        <[u8; 32]>::from(Sha256::digest(&bytes))
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn portable_vault_salt_rejects_data_after_its_fixed_length() {
    let root = std::env::temp_dir().join(format!("portmate-salt-length-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let path = root.join(PORTABLE_VAULT_SALT_FILE_NAME);
    fs::write(
        &path,
        vec![0_u8; portmate_kdf::SALT_LENGTH.saturating_add(1)],
    )
    .unwrap();

    let error = read_portable_vault_salt(&path).unwrap_err();
    assert!(error.contains("长度无效"), "{error}");
    assert!(error.contains("got"), "{error}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn loading_an_existing_store_does_not_advance_its_revision() {
    let root = std::env::temp_dir().join(format!("portmate-store-load-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let mut store = SessionStore::default();
    store.upsert_profile(test_shell_profile());
    save_store(&store_path, &store).unwrap();
    let mut first_instance_version = store_snapshot_version(&store_path).unwrap();

    let second_instance = load_store(&store_path).unwrap();
    assert_eq!(
        store_snapshot_version(&store_path).unwrap(),
        first_instance_version
    );
    save_store_with_expected_snapshot_version(
        &store_path,
        &second_instance,
        &mut first_instance_version,
    )
    .unwrap();

    let _ = fs::remove_dir_all(root);
}

#[test]
fn loading_corrupt_sqlite_fails_without_replacing_the_snapshot() {
    let root = std::env::temp_dir().join(format!("portmate-store-load-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let mut store = SessionStore::default();
    store.upsert_profile(test_shell_profile());
    save_store(&store_path, &store).unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute(
            "update kv set value = ?1 where key = ?2",
            params!["{not-json", STORE_KEY],
        )
        .unwrap();
    drop(connection);

    let error = load_store(&store_path).unwrap_err();
    assert!(error.contains("failed to parse SQLite store"), "{error}");
    let connection = SqliteConnection::open(&store_path).unwrap();
    let persisted: String = connection
        .query_row(
            "select value from kv where key = ?1",
            params![STORE_KEY],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted, "{not-json");

    let _ = fs::remove_dir_all(root);
}

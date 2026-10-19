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
            |row| row.get::<_, i64>(0),
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
    let bytes_before_load = fs::read(&store_path).unwrap();

    let second_instance = load_store(&store_path).unwrap();
    assert_eq!(fs::read(&store_path).unwrap(), bytes_before_load);
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
fn saving_an_identical_store_preserves_bytes_and_revision_but_real_edits_commit() {
    let root = canonical_test_tempdir();
    let store_path = root.path().join("portmate-store.sqlite3");
    let mut store = SessionStore::default();
    store.upsert_profile(test_shell_profile());
    save_store(&store_path, &store).unwrap();
    let before = fs::read(&store_path).unwrap();
    let revision = store_snapshot_version(&store_path).unwrap();

    for _ in 0..2 {
        save_store(&store_path, &store).unwrap();
        assert_eq!(fs::read(&store_path).unwrap(), before);
        assert_eq!(store_snapshot_version(&store_path).unwrap(), revision);
    }

    store.profiles[0].name = "actual edit".to_string();
    save_store(&store_path, &store).unwrap();
    assert_ne!(fs::read(&store_path).unwrap(), before);
    assert_ne!(store_snapshot_version(&store_path).unwrap(), revision);
    assert_eq!(
        load_store_sqlite(&store_path).unwrap().profiles[0].name,
        "actual edit"
    );
    let connection = SqliteConnection::open(&store_path).unwrap();
    let mirrored_name: String = connection
        .query_row(
            "select name from profiles where id = ?1",
            params![store.profiles[0].id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mirrored_name, "actual edit");
}

#[test]
fn identical_store_saves_still_reject_stale_and_unknown_versions() {
    let root = canonical_test_tempdir();
    let store_path = root.path().join("portmate-store.sqlite3");
    let store = SessionStore::default();
    save_store(&store_path, &store).unwrap();
    let stale = store_snapshot_version(&store_path).unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute(
            "update metadata set value = ?1 where key = 'storeRevision'",
            params!["external-revision"],
        )
        .unwrap();
    drop(connection);
    let before = fs::read(&store_path).unwrap();
    for (mut expected, diagnostic) in [
        (stale, "另一实例修改"),
        (StoreSnapshotVersion::UnknownAfterCommit, "上次提交后"),
    ] {
        let error = save_store_with_expected_snapshot_version(&store_path, &store, &mut expected)
            .unwrap_err();
        assert!(error.contains(diagnostic), "{error}");
        assert_eq!(fs::read(&store_path).unwrap(), before);
    }
}

#[test]
fn identical_store_saves_still_upgrade_schema_and_rebuild_new_mirrors() {
    for repair_during_load in [false, true] {
        for damage in [
            "drop table profiles",
            "update metadata set value = '3' where key = 'schemaVersion'",
        ] {
            let root = canonical_test_tempdir();
            let store_path = root.path().join("portmate-store.sqlite3");
            let mut store = SessionStore::default();
            store.upsert_profile(test_shell_profile());
            save_store(&store_path, &store).unwrap();
            let before = store_snapshot_version(&store_path).unwrap();
            let connection = SqliteConnection::open(&store_path).unwrap();
            connection.execute_batch(damage).unwrap();
            drop(connection);
            if repair_during_load {
                load_store(&store_path).unwrap();
            }
            save_store(&store_path, &store).unwrap();
            assert_ne!(store_snapshot_version(&store_path).unwrap(), before);
            let connection = SqliteConnection::open(&store_path).unwrap();
            let count: i64 = connection
                .query_row("select count(*) from profiles", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 1);
            let schema: String = connection
                .query_row(
                    "select value from metadata where key = 'schemaVersion'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(schema, crate::sqlite_schema::SQLITE_SCHEMA_VERSION);
            let pending: bool = connection
                .query_row(
                    "select exists(select 1 from metadata where key = ?1)",
                    params![crate::sqlite_schema::SQLITE_MIRROR_REBUILD_KEY],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!pending);
            drop(connection);
            let repaired = fs::read(&store_path).unwrap();
            save_store(&store_path, &store).unwrap();
            assert_eq!(fs::read(&store_path).unwrap(), repaired);
        }
    }
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

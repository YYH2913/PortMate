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

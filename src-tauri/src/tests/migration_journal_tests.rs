#[test]
fn migration_does_not_commit_profiles_when_portable_target_commit_is_unknown() {
    let fixture = test_migration_journal_fixture();
    let mut store = fixture.before.clone();
    let error = migrate_profile_secrets_with_journal_io(
        &mut store,
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
        |storage, _| {
            assert_eq!(storage, SecretStorage::Portable);
            Ok(true)
        },
        |_, _| panic!("unknown portable commit must preserve both providers"),
        |_, _, _, _| panic!("unknown portable commit must not switch Profile refs"),
        |_| Ok(()),
    )
    .unwrap_err();
    assert!(error.contains(PROFILE_SECRET_MIGRATION_RESTART_REQUIRED));
    assert!(error.contains("版本指纹无法确认"));
    assert_eq!(
        profile_secret_migration_projection(&store.profiles[0]).unwrap(),
        fixture.journal.payload.profiles[0].before
    );
}

#[test]
fn migration_preserves_sources_when_cleanup_checkpoint_cannot_be_verified() {
    let fixture = test_migration_journal_fixture();
    let mut store = fixture.before.clone();
    let delete_called = std::cell::Cell::new(false);
    let response = migrate_profile_secrets_with_journal_io(
        &mut store,
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
        |_, _| Ok(false),
        |_, _| {
            delete_called.set(true);
            SecretBatchDeleteOutcome {
                results: BTreeMap::new(),
                portable_vault_requires_reunlock: false,
            }
        },
        |_, _, _, _| ProfileSecretStoreCommit::Committed { warning: None },
        |event| match event {
            ProfileSecretMigrationJournalEvent::Transition {
                state: ProfileSecretMigrationJournalState::SourceCleanupPending,
                ..
            } => Err("injected checkpoint read-back failure".to_string()),
            _ => Ok(()),
        },
    )
    .unwrap();
    assert!(!delete_called.get());
    assert!(response.recovery_pending);
    assert!(response
        .warnings
        .iter()
        .any(|warning| warning.contains("checkpoint")));
    assert!(response
        .items
        .iter()
        .all(|item| item.cleanup_status == ProfileSecretCleanupStatus::Failed));
    let projection = profile_secret_migration_projection(&store.profiles[0]).unwrap();
    assert_ne!(projection, fixture.journal.payload.profiles[0].before);
    assert!(projection
        .password_secret_ref
        .as_deref()
        .is_some_and(|secret_ref| secret_ref.starts_with("stronghold:")));
    assert!(projection
        .passphrase_secret_ref
        .as_deref()
        .is_some_and(|secret_ref| secret_ref.starts_with("stronghold:")));
}

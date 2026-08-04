use super::*;

#[test]
fn migration_projection_classifies_old_new_and_slot_conflicts() {
    let fixture = test_migration_journal_fixture();
    assert_eq!(
        profile_secret_migration_disposition(&fixture.before, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::NotCommitted
    );

    let mut committed = fixture.journal.clone();
    committed.state = ProfileSecretMigrationJournalState::ProfilesCommitted;
    assert_eq!(
        profile_secret_migration_disposition(&fixture.after, &committed),
        ProfileSecretMigrationRecoveryDisposition::Committed
    );

    let mut partial = fixture.before.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut partial.profiles[0].connection {
        ssh.password_secret_ref = fixture
            .journal
            .payload
            .items
            .first()
            .map(|item| item.target_ref.clone());
    }
    assert_eq!(
        profile_secret_migration_disposition(&partial, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::Conflict
    );

    let mut swapped = fixture.before.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut swapped.profiles[0].connection {
        std::mem::swap(&mut ssh.password_secret_ref, &mut ssh.passphrase_secret_ref);
    }
    assert_eq!(
        profile_secret_migration_disposition(&swapped, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::Conflict,
        "equal ref counts must not hide credential-slot swaps"
    );

    let mut missing = fixture.before.clone();
    missing.profiles.clear();
    assert_eq!(
        profile_secret_migration_disposition(&missing, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::Conflict
    );
    assert_eq!(
        profile_secret_migration_disposition(&fixture.after, &fixture.journal),
        ProfileSecretMigrationRecoveryDisposition::Conflict,
        "a NEW projection cannot be paired with a pre-commit journal state"
    );
}

#[test]
fn migration_diagnostic_reports_slots_and_provider_evidence_without_secret_material() {
    let fixture = test_migration_journal_fixture();
    let mut mixed = fixture.before.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut mixed.profiles[0].connection {
        ssh.password_secret_ref = Some(fixture.journal.payload.items[0].target_ref.clone());
    }
    let payload_bytes = serde_json::to_vec(&fixture.journal.payload).unwrap().len() as u64;
    let metadata = ActiveProfileSecretMigrationJournalMetadata {
        row_id: fixture.journal.payload.migration_id.clone(),
        state: fixture.journal.state.as_str().to_string(),
        payload_bytes,
        created_at: fixture.journal.created_at.to_rfc3339(),
        updated_at: fixture.journal.updated_at.to_rfc3339(),
    };
    let report = build_profile_secret_migration_diagnostic_report(
        &mixed,
        &fixture.journal,
        &metadata,
        |secret_ref| match fixture.values.get(secret_ref) {
            Some(value) => SecretProbeResult::Present(Zeroizing::new(value.clone())),
            None => SecretProbeResult::Missing,
        },
        ProfileSecretMigrationDiagnosticPortableVault {
            exists: Some(true),
            unlocked: Some(true),
            recovery_ready: Some(true),
            error: None,
        },
    );
    let json = serde_json::to_value(&report).unwrap();
    let encoded = serde_json::to_string(&json).unwrap();
    assert!(!encoded.contains("private-a"));
    assert!(!encoded.contains("private-b"));
    assert_eq!(json["containsSecretMaterial"], false);
    assert_eq!(json["journal"]["disposition"], "conflict");
    assert_eq!(json["profiles"][0]["status"], "conflict");
    assert_eq!(json["secrets"][0]["source"]["status"], "present");
    assert_eq!(json["secrets"][0]["target"]["status"], "present");
    assert_eq!(json["secrets"][0]["contentsMatch"], true);
}

#[test]
fn corrupt_migration_diagnostic_never_exports_the_raw_payload_or_state() {
    let fixture = test_migration_journal_fixture();
    let root = std::env::temp_dir().join(format!(
        "portmate-migration-diagnostic-corrupt-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    save_store(&store_path, &fixture.before).unwrap();
    persist_profile_secret_migration_journal_event(
        &store_path,
        ProfileSecretMigrationJournalEvent::Prepared(fixture.journal.payload.clone()),
    )
    .unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    connection
        .execute(
            "update profile_secret_migrations
             set state = 'TOP-SECRET-STATE', payload_json = ?1 where active = 1",
            params![serde_json::to_string("TOP-SECRET-JOURNAL-BODY").unwrap()],
        )
        .unwrap();
    drop(connection);

    let result = export_profile_secret_migration_diagnostics_with_io(
        &store_path,
        &fixture.before,
        |_| panic!("a corrupt journal must not probe providers"),
        ProfileSecretMigrationDiagnosticPortableVault {
            exists: Some(true),
            unlocked: Some(false),
            recovery_ready: Some(false),
            error: None,
        },
    )
    .unwrap();
    assert!(!result.journal_valid);
    let exported = fs::read_to_string(&result.path).unwrap();
    assert!(!exported.contains("TOP-SECRET-STATE"));
    assert!(!exported.contains("TOP-SECRET-JOURNAL-BODY"));
    assert!(exported.contains("未包含原始 payload"));
    assert!(exported.contains("JSON 损坏"));
    let checksum = fs::read_to_string(&result.checksum_path).unwrap();
    assert!(checksum.starts_with(&result.sha256));
    assert_eq!(result.sha256, sha256_hex(exported.as_bytes()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&result.checksum_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let _ = fs::remove_dir_all(root);
}

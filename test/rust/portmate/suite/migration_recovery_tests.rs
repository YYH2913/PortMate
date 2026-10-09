use super::*;

#[test]
fn recovery_rolls_back_only_unreferenced_targets_for_old_projection() {
    let fixture = test_migration_journal_fixture();
    let values = std::rc::Rc::new(std::cell::RefCell::new(fixture.values.clone()));
    let deleted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.before,
        &fixture.journal,
        {
            let values = std::rc::Rc::clone(&values);
            move |secret_ref| match values.borrow().get(secret_ref).cloned() {
                Some(value) => SecretProbeResult::Present(Zeroizing::new(value)),
                None => SecretProbeResult::Missing,
            }
        },
        {
            let values = std::rc::Rc::clone(&values);
            let deleted = std::rc::Rc::clone(&deleted);
            move |storage, secret_refs| {
                assert_eq!(storage, SecretStorage::Portable);
                let results = secret_refs
                    .iter()
                    .map(|secret_ref| {
                        values.borrow_mut().remove(secret_ref);
                        deleted.borrow_mut().push(secret_ref.clone());
                        (secret_ref.clone(), Ok(()))
                    })
                    .collect();
                SecretBatchDeleteOutcome {
                    results,
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        {
            let events = std::rc::Rc::clone(&events);
            move |event| {
                events.borrow_mut().push(match event {
                    ProfileSecretMigrationJournalEvent::Prepared(_) => "prepared".to_string(),
                    ProfileSecretMigrationJournalEvent::Transition { state, .. } => {
                        state.as_str().to_string()
                    }
                    ProfileSecretMigrationJournalEvent::Clear { .. } => "clear".to_string(),
                });
                Ok(())
            }
        },
    )
    .unwrap();
    assert!(outcome.resolved);
    assert_eq!(outcome.action, "rolled-back-targets");
    assert_eq!(deleted.borrow().len(), fixture.journal.payload.items.len());
    assert_eq!(
        events.borrow().as_slice(),
        ["target-cleanup-pending", "clear"]
    );
    for item in &fixture.journal.payload.items {
        assert!(values.borrow().contains_key(&item.source_ref));
        assert!(!values.borrow().contains_key(&item.target_ref));
    }
}

#[test]
fn recovery_freezes_old_projection_when_source_is_missing_or_provider_unavailable() {
    let fixture = test_migration_journal_fixture();
    let missing_source = fixture.journal.payload.items[0].source_ref.clone();
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.before,
        &fixture.journal,
        |secret_ref| {
            if secret_ref == missing_source {
                SecretProbeResult::Missing
            } else {
                SecretProbeResult::Present(Zeroizing::new("private".to_string()))
            }
        },
        |_, _| panic!("missing authoritative source must preserve targets"),
        |event| {
            assert!(matches!(
                event,
                ProfileSecretMigrationJournalEvent::Transition {
                    state: ProfileSecretMigrationJournalState::NeedsResolution,
                    ..
                }
            ));
            Ok(())
        },
    )
    .unwrap();
    assert!(!outcome.resolved);
    assert_eq!(outcome.action, "needs-resolution");

    let outcome = recover_profile_secret_migration_with_io(
        &fixture.before,
        &fixture.journal,
        |_| SecretProbeResult::Unavailable("keyring service offline".to_string()),
        |_, _| panic!("unavailable provider must not be treated as missing"),
        |_| panic!("provider unavailability must keep the current journal state"),
    )
    .unwrap();
    assert_eq!(outcome.action, "blocked");
    assert!(outcome.warnings[0].contains("offline"));
}

#[test]
fn recovery_finalizes_new_projection_only_after_exact_secret_verification() {
    let fixture = test_migration_journal_fixture();
    let mut journal = fixture.journal.clone();
    journal.state = ProfileSecretMigrationJournalState::ProfilesCommitted;
    let values = std::rc::Rc::new(std::cell::RefCell::new(fixture.values.clone()));
    let deleted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        {
            let values = std::rc::Rc::clone(&values);
            move |secret_ref| match values.borrow().get(secret_ref).cloned() {
                Some(value) => SecretProbeResult::Present(Zeroizing::new(value)),
                None => SecretProbeResult::Missing,
            }
        },
        {
            let values = std::rc::Rc::clone(&values);
            let deleted = std::rc::Rc::clone(&deleted);
            move |storage, secret_refs| {
                assert_eq!(storage, SecretStorage::Native);
                let results = secret_refs
                    .iter()
                    .map(|secret_ref| {
                        values.borrow_mut().remove(secret_ref);
                        deleted.borrow_mut().push(secret_ref.clone());
                        (secret_ref.clone(), Ok(()))
                    })
                    .collect();
                SecretBatchDeleteOutcome {
                    results,
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(outcome.resolved);
    assert_eq!(outcome.action, "finalized-source-cleanup");
    assert_eq!(deleted.borrow().len(), journal.payload.items.len());
    for item in &journal.payload.items {
        assert!(!values.borrow().contains_key(&item.source_ref));
        assert!(values.borrow().contains_key(&item.target_ref));
    }

    let mismatched_source = journal.payload.items[0].source_ref.clone();
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        |secret_ref| {
            let value = if secret_ref == mismatched_source {
                "changed-source"
            } else if secret_ref.ends_with("11111111-1111-4111-8111-111111111111") {
                "private-a"
            } else {
                "private-b"
            };
            SecretProbeResult::Present(Zeroizing::new(value.to_string()))
        },
        |_, _| panic!("mismatched values must preserve both providers"),
        |event| {
            assert!(matches!(
                event,
                ProfileSecretMigrationJournalEvent::Transition {
                    state: ProfileSecretMigrationJournalState::NeedsResolution,
                    ..
                }
            ));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(outcome.action, "needs-resolution");
}

#[test]
fn recovery_retries_partial_source_cleanup_idempotently() {
    let fixture = test_migration_journal_fixture();
    let mut journal = fixture.journal.clone();
    journal.state = ProfileSecretMigrationJournalState::ProfilesCommitted;
    let values = std::rc::Rc::new(std::cell::RefCell::new(fixture.values.clone()));
    let first_delete = std::cell::Cell::new(true);
    let first = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        {
            let values = std::rc::Rc::clone(&values);
            move |secret_ref| match values.borrow().get(secret_ref).cloned() {
                Some(value) => SecretProbeResult::Present(Zeroizing::new(value)),
                None => SecretProbeResult::Missing,
            }
        },
        {
            let values = std::rc::Rc::clone(&values);
            move |_, secret_refs| {
                let results = secret_refs
                    .iter()
                    .enumerate()
                    .map(|(index, secret_ref)| {
                        if first_delete.get() && index == 1 {
                            (
                                secret_ref.clone(),
                                Err("injected keyring failure".to_string()),
                            )
                        } else {
                            values.borrow_mut().remove(secret_ref);
                            (secret_ref.clone(), Ok(()))
                        }
                    })
                    .collect();
                first_delete.set(false);
                SecretBatchDeleteOutcome {
                    results,
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(!first.resolved);
    assert_eq!(first.action, "source-cleanup-pending");
    assert_eq!(
        fixture
            .journal
            .payload
            .items
            .iter()
            .filter(|item| values.borrow().contains_key(&item.source_ref))
            .count(),
        1
    );

    journal.state = ProfileSecretMigrationJournalState::SourceCleanupPending;
    let second = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        {
            let values = std::rc::Rc::clone(&values);
            move |secret_ref| match values.borrow().get(secret_ref).cloned() {
                Some(value) => SecretProbeResult::Present(Zeroizing::new(value)),
                None => SecretProbeResult::Missing,
            }
        },
        {
            let values = std::rc::Rc::clone(&values);
            move |_, secret_refs| SecretBatchDeleteOutcome {
                results: secret_refs
                    .iter()
                    .map(|secret_ref| {
                        values.borrow_mut().remove(secret_ref);
                        (secret_ref.clone(), Ok(()))
                    })
                    .collect(),
                portable_vault_requires_reunlock: false,
            }
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(second.resolved);
    assert_eq!(second.action, "finalized-source-cleanup");
    assert!(fixture
        .journal
        .payload
        .items
        .iter()
        .all(|item| !values.borrow().contains_key(&item.source_ref)));
}

#[test]
fn recovery_freezes_new_projection_when_a_target_is_missing() {
    let fixture = test_migration_journal_fixture();
    let mut journal = fixture.journal.clone();
    journal.state = ProfileSecretMigrationJournalState::ProfilesCommitted;
    let missing_target = journal.payload.items[0].target_ref.clone();
    let outcome = recover_profile_secret_migration_with_io(
        &fixture.after,
        &journal,
        |secret_ref| {
            if secret_ref == missing_target {
                SecretProbeResult::Missing
            } else {
                let value = if secret_ref.contains("source-a")
                    || secret_ref.ends_with("11111111-1111-4111-8111-111111111111")
                {
                    "private-a"
                } else {
                    "private-b"
                };
                SecretProbeResult::Present(Zeroizing::new(value.to_string()))
            }
        },
        |_, _| panic!("missing authoritative target must preserve source copies"),
        |event| {
            assert!(matches!(
                event,
                ProfileSecretMigrationJournalEvent::Transition {
                    state: ProfileSecretMigrationJournalState::NeedsResolution,
                    ..
                }
            ));
            Ok(())
        },
    )
    .unwrap();
    assert!(!outcome.resolved);
    assert_eq!(outcome.action, "needs-resolution");
}

#[test]
fn recovery_preserves_both_sides_for_mixed_projection_without_provider_access() {
    let fixture = test_migration_journal_fixture();
    let mut mixed = fixture.before.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut mixed.profiles[0].connection {
        ssh.password_secret_ref = Some(fixture.journal.payload.items[0].target_ref.clone());
    }
    let outcome = recover_profile_secret_migration_with_io(
        &mixed,
        &fixture.journal,
        |_| panic!("conflict classification must precede provider access"),
        |_, _| panic!("mixed projection must preserve both providers"),
        |event| {
            assert!(matches!(
                event,
                ProfileSecretMigrationJournalEvent::Transition {
                    state: ProfileSecretMigrationJournalState::NeedsResolution,
                    ..
                }
            ));
            Ok(())
        },
    )
    .unwrap();
    assert!(!outcome.resolved);
    assert_eq!(outcome.action, "needs-resolution");
}

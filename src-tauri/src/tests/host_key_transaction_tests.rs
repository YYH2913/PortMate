#[test]
fn host_key_mutation_changes_memory_only_after_persistence_succeeds() {
    let mut store = SessionStore::default();
    let mut profile = test_ssh_profile();
    let key = portmate_core::TrustedHostKey {
        id: "host-key-1".to_string(),
        profile_id: Some(profile.id.clone()),
        alias: "bench-device".to_string(),
        host: "192.0.2.10".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:test".to_string(),
        public_key_base64: "YWJj".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    };
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.trusted_host_keys.push(key.clone());
    }
    store.upsert_profile(profile);
    store.host_keys.keys.push(key);
    let before = serde_json::to_value(&store).unwrap();

    let error = commit_store_mutation_with(
        &mut store,
        |next_store| {
            Ok(delete_host_keys_from_store(
                next_store,
                &["host-key-1".to_string()],
            ))
        },
        |next_store| {
            assert!(next_store.host_keys.keys.is_empty());
            Err("disk full".to_string())
        },
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "disk full");
    assert_eq!(serde_json::to_value(&store).unwrap(), before);

    let saved = commit_store_mutation_with(
        &mut store,
        |next_store| {
            Ok(delete_host_keys_from_store(
                next_store,
                &["host-key-1".to_string()],
            ))
        },
        |_| Ok(()),
        |_| panic!("successful persistence must not be reverified"),
    )
    .unwrap();
    assert!(saved.keys.is_empty());
    assert!(store.host_keys.keys.is_empty());
    let profile = store.profile("ssh-session-1").unwrap();
    let trusted_host_keys = match profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh.trusted_host_keys,
        _ => panic!("expected SSH profile"),
    };
    assert!(trusted_host_keys.is_empty());
}

#[test]
fn tracked_store_mutation_rolls_back_state_events_and_outbox_together() {
    let mut store = SessionStore::default();
    store.upsert_profile(test_ssh_profile());
    let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
    store.set_system_event_notifier(sender).unwrap();
    let retained_event_id = store
        .record_system_event_tracked("ssh-session-1", "retained before transaction")
        .unwrap();
    let before = serde_json::to_value(&store).unwrap();

    let error = commit_tracked_store_mutation_with(
        &mut store,
        |next_store| {
            next_store.host_keys.keys.push(TrustedHostKey {
                id: "rolled-back-key".to_string(),
                profile_id: Some("ssh-session-1".to_string()),
                alias: "bench-device".to_string(),
                host: "192.0.2.10".to_string(),
                port: 22,
                algorithm: "ssh-ed25519".to_string(),
                fingerprint_sha256: "SHA256:test".to_string(),
                public_key_base64: "YWJj".to_string(),
                scope: HostKeyScope::Profile,
                label: None,
                first_seen: Utc::now(),
                last_seen: Utc::now(),
            });
            let event_id = next_store
                .record_system_event_tracked("ssh-session-1", "rolled back transaction")
                .unwrap();
            Ok(((), vec![event_id]))
        },
        |_| Err("disk full".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "disk full");
    assert_eq!(serde_json::to_value(&store).unwrap(), before);
    let queued = store.drain_system_event_outbox();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].0.id, retained_event_id);

    commit_tracked_store_mutation_with(
        &mut store,
        |next_store| {
            let event_id = next_store
                .record_system_event_tracked("ssh-session-1", "committed transaction")
                .unwrap();
            Ok(((), vec![event_id]))
        },
        |_| Err("post-commit verification failed".to_string()),
        |_| Ok(true),
    )
    .unwrap();
    let queued = store.drain_system_event_outbox();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].0.text.as_deref(), Some("committed transaction"));
}

#[test]
fn applied_system_event_persistence_failure_keeps_remote_truth_in_memory() {
    let mut store = SessionStore::default();
    store.upsert_profile(test_ssh_profile());

    let error = record_applied_system_event_with(
        &mut store,
        "ssh-session-1",
        "PortMate: tmux kill-pane (%7)".to_string(),
        |_| Err("disk full".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();

    assert_eq!(error, "disk full");
    let event = store.events.last().unwrap();
    assert_eq!(event.text.as_deref(), Some("PortMate: tmux kill-pane (%7)"));
    assert!(event
        .annotations
        .get("loggingError")
        .is_some_and(|error| error.contains("disk full")));
}

#[test]
fn applied_system_event_accepts_a_verified_post_commit_error() {
    let mut store = SessionStore::default();
    store.upsert_profile(test_ssh_profile());

    record_applied_system_event_with(
        &mut store,
        "ssh-session-1",
        "PortMate: tmux select-layout (lab:1)".to_string(),
        |_| Err("post-commit verification failed".to_string()),
        |_| Ok(true),
    )
    .unwrap();

    let event = store.events.last().unwrap();
    assert_eq!(
        event.text.as_deref(),
        Some("PortMate: tmux select-layout (lab:1)")
    );
    assert!(!event.annotations.contains_key("loggingError"));
}

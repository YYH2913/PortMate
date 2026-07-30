use super::*;

#[test]
fn security_command_types_keep_stable_json_contracts() {
    let decision = serde_json::to_value(HostKeyDecisionRequest {
        profile_id: "ssh-1".to_string(),
        observation: HostKeyObservation {
            host: "router.example".to_string(),
            port: 22,
            alias: Some("router".to_string()),
            algorithm: "ssh-ed25519".to_string(),
            public_key_base64: "YWJj".to_string(),
        },
        decision: HostKeyDecision::TrustOnce,
    })
    .unwrap();
    assert_eq!(decision["profileId"], "ssh-1");
    assert_eq!(decision["observation"]["publicKeyBase64"], "YWJj");
    assert_eq!(decision["decision"], "trust-once");

    let delete: ClientIdentityDeleteRequest = serde_json::from_value(serde_json::json!({
        "profileId": "ssh-1",
        "identityId": "identity-1"
    }))
    .unwrap();
    assert_eq!(delete.profile_id, "ssh-1");
    assert_eq!(delete.identity_id, "identity-1");
    assert!(!delete.delete_secret);
}

#[test]
fn temporary_host_key_trust_matches_without_persisting() {
    let mut store = SessionStore::default();
    let profile = test_ssh_profile();
    let profile_id = profile.id.clone();
    store.upsert_profile(profile);
    let observation = HostKeyObservation {
        host: "192.0.2.10".to_string(),
        port: 22,
        alias: Some("bench-device".to_string()),
        algorithm: "ssh-ed25519".to_string(),
        public_key_base64: "YWJj".to_string(),
    };

    let key = temporary_trusted_host_key(&store, &profile_id, &observation).unwrap();
    assert!(store.host_keys.keys.is_empty());

    let mut host_keys = store.host_keys.clone();
    host_keys.keys.push(key);
    let policy = match store.profile(&profile_id).unwrap().connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.host_key_policy,
        _ => panic!("expected SSH profile"),
    };
    assert!(matches!(
        host_keys
            .evaluate(&profile_id, &policy, &observation)
            .unwrap(),
        HostKeyEvaluation::Trusted { .. }
    ));
    assert!(one_time_trusts_observation(
        &host_keys.keys,
        &profile_id,
        &policy,
        &observation
    ));

    let mut different_host = observation.clone();
    different_host.host = "192.0.2.11".to_string();
    assert!(!one_time_trusts_observation(
        &host_keys.keys,
        &profile_id,
        &policy,
        &different_host
    ));
    assert!(store.host_keys.keys.is_empty());
}

#[test]
fn persistent_host_key_decisions_add_and_replace_profile_mirrors() {
    let mut store = SessionStore::default();
    let profile = test_ssh_profile();
    let profile_id = profile.id.clone();
    store.upsert_profile(profile);
    let first_observation = HostKeyObservation {
        host: "192.0.2.10".to_string(),
        port: 22,
        alias: Some("bench-device".to_string()),
        algorithm: "ssh-ed25519".to_string(),
        public_key_base64: "YWJj".to_string(),
    };

    let first = apply_persistent_host_key_decision(
        &mut store,
        &profile_id,
        &first_observation,
        HostKeyDecision::AppendToProfile,
    )
    .unwrap()
    .unwrap();
    let profile_keys = match &store.profile(&profile_id).unwrap().connection {
        ConnectionConfig::Ssh(ssh) => ssh.trusted_host_keys.clone(),
        _ => panic!("expected SSH profile"),
    };
    assert_eq!(store.host_keys.keys, vec![first.clone()]);
    assert_eq!(profile_keys, vec![first.clone()]);

    let mut replacement_observation = first_observation;
    replacement_observation.public_key_base64 = "ZGVm".to_string();
    let replacement = apply_persistent_host_key_decision(
        &mut store,
        &profile_id,
        &replacement_observation,
        HostKeyDecision::ReplaceForProfile,
    )
    .unwrap()
    .unwrap();
    assert_ne!(replacement.id, first.id);
    let profile_keys = match &store.profile(&profile_id).unwrap().connection {
        ConnectionConfig::Ssh(ssh) => ssh.trusted_host_keys.clone(),
        _ => panic!("expected SSH profile"),
    };
    assert_eq!(store.host_keys.keys, vec![replacement.clone()]);
    assert_eq!(profile_keys, vec![replacement]);
}

#[test]
fn successful_host_key_verification_touches_store_and_profile_copies() {
    let mut store = SessionStore::default();
    let mut profile = test_ssh_profile();
    let profile_id = profile.id.clone();
    let observation = HostKeyObservation {
        host: "192.0.2.10".to_string(),
        port: 22,
        alias: Some("bench-device".to_string()),
        algorithm: "ssh-ed25519".to_string(),
        public_key_base64: "YWJj".to_string(),
    };
    let fingerprint = observation.fingerprint_sha256().unwrap();
    let first_seen = Utc::now() - chrono::Duration::days(2);
    let previous_seen = first_seen + chrono::Duration::hours(1);
    let seen_at = previous_seen + chrono::Duration::hours(1);
    let key = TrustedHostKey {
        id: "host-key-match".to_string(),
        profile_id: Some(profile_id.clone()),
        alias: "bench-device".to_string(),
        host: observation.host.clone(),
        port: observation.port,
        algorithm: observation.algorithm.clone(),
        fingerprint_sha256: fingerprint.clone(),
        public_key_base64: observation.public_key_base64.clone(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen,
        last_seen: previous_seen,
    };
    let mut other_algorithm = key.clone();
    other_algorithm.id = "host-key-other-algorithm".to_string();
    other_algorithm.algorithm = "rsa-sha2-512".to_string();
    let mut other_alias = key.clone();
    other_alias.id = "host-key-other-alias".to_string();
    other_alias.alias = "other-device".to_string();

    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.trusted_host_keys = vec![key.clone(), other_algorithm.clone(), other_alias.clone()];
    }
    let policy = match &profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh.host_key_policy.clone(),
        _ => panic!("expected SSH profile"),
    };
    store.upsert_profile(profile);
    store.host_keys.keys = vec![key, other_algorithm, other_alias];

    assert!(
        touch_observed_host_key(&mut store, &profile_id, &policy, &observation, seen_at,).unwrap()
    );

    let matching = store
        .host_keys
        .keys
        .iter()
        .find(|key| key.id == "host-key-match")
        .unwrap();
    assert_eq!(matching.first_seen, first_seen);
    assert_eq!(matching.last_seen, seen_at);
    assert_eq!(
        store
            .host_keys
            .keys
            .iter()
            .find(|key| key.id == "host-key-other-algorithm")
            .unwrap()
            .last_seen,
        previous_seen
    );
    assert_eq!(
        store
            .host_keys
            .keys
            .iter()
            .find(|key| key.id == "host-key-other-alias")
            .unwrap()
            .last_seen,
        previous_seen
    );

    let saved_profile = store.profile(&profile_id).unwrap();
    let profile_keys = match saved_profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh.trusted_host_keys,
        _ => panic!("expected SSH profile"),
    };
    assert_eq!(
        profile_keys
            .iter()
            .find(|key| key.id == "host-key-match")
            .unwrap()
            .last_seen,
        seen_at
    );
    assert_eq!(
        profile_keys
            .iter()
            .find(|key| key.id == "host-key-other-algorithm")
            .unwrap()
            .last_seen,
        previous_seen
    );
}

#[test]
fn update_host_key_edits_store_and_profile_copies() {
    let mut store = SessionStore::default();
    let mut profile = test_ssh_profile();
    let key = portmate_core::TrustedHostKey {
        id: "host-key-1".to_string(),
        profile_id: Some(profile.id.clone()),
        alias: "old-alias".to_string(),
        host: "old-host".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:test".to_string(),
        public_key_base64: "YWJj".to_string(),
        scope: HostKeyScope::Profile,
        label: Some("old label".to_string()),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    };
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.trusted_host_keys.push(key.clone());
    }
    let profile_id = profile.id.clone();
    store.upsert_profile(profile);
    store.host_keys.keys.push(key.clone());

    let next = update_host_key_in_store(
        &mut store,
        HostKeyUpdateRequest {
            key_id: "host-key-1".to_string(),
            expected_key: key,
            profile_id: Some(profile_id.clone()),
            alias: " new-alias ".to_string(),
            host: " new-host ".to_string(),
            port: 2222,
            scope: HostKeyScope::Profile,
            label: Some(" new label ".to_string()),
        },
    )
    .unwrap();

    let edited = next
        .keys
        .iter()
        .find(|key| key.id == "host-key-1")
        .expect("edited host key should remain in store");
    assert_eq!(edited.alias, "new-alias");
    assert_eq!(edited.host, "new-host");
    assert_eq!(edited.port, 2222);
    assert_eq!(edited.profile_id.as_deref(), Some(profile_id.as_str()));
    assert_eq!(edited.label.as_deref(), Some("new label"));

    let saved_profile = store.profile(&profile_id).unwrap();
    let profile_copy = match &saved_profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh.trusted_host_keys.first().unwrap(),
        _ => panic!("expected SSH profile"),
    };
    assert_eq!(profile_copy.alias, "new-alias");
    assert_eq!(profile_copy.host, "new-host");
    assert_eq!(profile_copy.port, 2222);
    assert_eq!(profile_copy.label.as_deref(), Some("new label"));
}

#[test]
fn concurrent_host_key_edits_merge_fields_and_reject_conflicts() {
    let now = Utc::now();
    let expected = TrustedHostKey {
        id: "host-key-1".to_string(),
        profile_id: Some("ssh-session-1".to_string()),
        alias: "original-alias".to_string(),
        host: "original-host".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:test".to_string(),
        public_key_base64: "YWJj".to_string(),
        scope: HostKeyScope::Profile,
        label: Some("original label".to_string()),
        first_seen: now,
        last_seen: now,
    };
    let mut current = expected.clone();
    current.label = Some("current label".to_string());
    current.last_seen = now + chrono::Duration::seconds(1);
    let mut incoming = expected.clone();
    incoming.alias = "incoming-alias".to_string();

    let merged = merge_expected_host_key_update(&current, &expected, incoming.clone()).unwrap();
    assert_eq!(merged.alias, "incoming-alias");
    assert_eq!(merged.label.as_deref(), Some("current label"));
    assert_eq!(merged.last_seen, current.last_seen);
    assert_eq!(merged.fingerprint_sha256, current.fingerprint_sha256);

    let mut conflicting_current = expected.clone();
    conflicting_current.alias = "current-alias".to_string();
    let error =
        merge_expected_host_key_update(&conflicting_current, &expected, incoming).unwrap_err();
    assert!(error.contains("Host Key 字段"), "{error}");
    assert!(!error.contains("Profile 字段"), "{error}");
    assert!(error.contains("hostKey.alias"), "{error}");
    assert!(!error.contains("current-alias"), "{error}");
    assert!(!error.contains("incoming-alias"), "{error}");

    let mut wrong_expected = expected.clone();
    wrong_expected.id = "host-key-2".to_string();
    assert!(
        merge_expected_host_key_update(&current, &wrong_expected, expected)
            .unwrap_err()
            .contains("不是同一个 host key")
    );
}

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

#[test]
fn update_host_key_rejects_invalid_profile_scope() {
    let mut store = SessionStore::default();
    store.host_keys.keys.push(portmate_core::TrustedHostKey {
        id: "host-key-1".to_string(),
        profile_id: None,
        alias: "alias".to_string(),
        host: "host".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:test".to_string(),
        public_key_base64: "YWJj".to_string(),
        scope: HostKeyScope::User,
        label: None,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    });
    let expected_key = store.host_keys.keys[0].clone();

    let error = update_host_key_in_store(
        &mut store,
        HostKeyUpdateRequest {
            key_id: "host-key-1".to_string(),
            expected_key,
            profile_id: None,
            alias: "alias".to_string(),
            host: "host".to_string(),
            port: 22,
            scope: HostKeyScope::Profile,
            label: None,
        },
    )
    .unwrap_err();
    assert!(error.contains("必须选择 Profile"));
}

#[test]
fn delete_host_keys_removes_global_and_profile_copies() {
    let mut store = SessionStore::default();
    let mut profile = test_ssh_profile();
    let profile_id = profile.id.clone();
    let key_a = portmate_core::TrustedHostKey {
        id: "host-key-a".to_string(),
        profile_id: Some(profile_id.clone()),
        alias: "alias-a".to_string(),
        host: "host-a".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:a".to_string(),
        public_key_base64: "YQ==".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    };
    let key_b = portmate_core::TrustedHostKey {
        id: "host-key-b".to_string(),
        profile_id: Some(profile_id.clone()),
        alias: "alias-b".to_string(),
        host: "host-b".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:b".to_string(),
        public_key_base64: "Yg==".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    };
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.trusted_host_keys.push(key_a.clone());
        ssh.trusted_host_keys.push(key_b.clone());
    }
    store.upsert_profile(profile);
    store.host_keys.keys.push(key_a);
    store.host_keys.keys.push(key_b);

    let next = delete_host_keys_from_store(&mut store, &["host-key-a".to_string()]);
    assert_eq!(next.keys.len(), 1);
    assert_eq!(next.keys[0].id, "host-key-b");

    let saved_profile = store.profile(&profile_id).unwrap();
    let profile_keys = match &saved_profile.connection {
        ConnectionConfig::Ssh(ssh) => &ssh.trusted_host_keys,
        _ => panic!("expected SSH profile"),
    };
    assert_eq!(profile_keys.len(), 1);
    assert_eq!(profile_keys[0].id, "host-key-b");
}

#[test]
fn one_time_host_key_snapshot_keeps_multi_hop_trust_until_success() {
    let one_time = Arc::new(Mutex::new(HashMap::new()));
    let key = portmate_core::TrustedHostKey {
        id: "one-time-key".to_string(),
        profile_id: Some("ssh-session-1".to_string()),
        alias: "jump:bastion:22".to_string(),
        host: "bastion".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:test".to_string(),
        public_key_base64: "YWJj".to_string(),
        scope: HostKeyScope::Profile,
        label: Some("trust once".to_string()),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    };
    let mut target_key = key.clone();
    target_key.id = "one-time-target-key".to_string();
    target_key.alias = "target:22".to_string();
    target_key.host = "target".to_string();
    remember_one_time_host_key_in(&one_time, "ssh-session-1", key.clone()).unwrap();
    remember_one_time_host_key_in(&one_time, "ssh-session-1", target_key.clone()).unwrap();

    assert_eq!(
        one_time_host_keys_snapshot_from(&one_time, "ssh-session-1").unwrap(),
        vec![key.clone(), target_key.clone()]
    );
    assert_eq!(
        one_time_host_keys_snapshot_from(&one_time, "ssh-session-1").unwrap(),
        vec![key.clone(), target_key.clone()]
    );
    let consumed = take_one_time_host_keys_from(&one_time, "ssh-session-1").unwrap();
    assert_eq!(consumed, vec![key.clone(), target_key.clone()]);
    assert!(one_time_host_keys_snapshot_from(&one_time, "ssh-session-1")
        .unwrap()
        .is_empty());
    restore_one_time_host_keys_in(&one_time, "ssh-session-1", consumed.clone()).unwrap();
    restore_one_time_host_keys_in(&one_time, "ssh-session-1", consumed).unwrap();
    assert_eq!(
        one_time_host_keys_snapshot_from(&one_time, "ssh-session-1").unwrap(),
        vec![key, target_key]
    );
}

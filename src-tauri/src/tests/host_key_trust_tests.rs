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

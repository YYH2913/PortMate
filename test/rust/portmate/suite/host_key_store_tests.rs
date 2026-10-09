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

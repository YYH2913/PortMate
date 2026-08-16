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
fn scanned_host_key_trust_is_bound_to_the_current_ssh_connection_snapshot() {
    let scanned = normalize_session_profile(test_ssh_profile());
    let ssh = ssh_connection(&scanned).unwrap();
    let observation = HostKeyObservation {
        host: ssh.endpoint.host.clone(),
        port: ssh.endpoint.port,
        alias: ssh.host_key_policy.alias.clone(),
        algorithm: "ssh-ed25519".to_string(),
        public_key_base64: "YWJj".to_string(),
    };
    let mut store = SessionStore::default();
    store.upsert_profile(scanned.clone());

    assert_eq!(
        validate_scanned_host_key_profile_snapshot(&store, &scanned, &observation).unwrap(),
        ssh.host_key_policy
    );

    let mut scanned_with_jump = scanned.clone();
    ssh_connection_mut(&mut scanned_with_jump)
        .unwrap()
        .jumps
        .push(portmate_core::JumpHop {
            host: "jump.example".to_string(),
            port: 2222,
            username: "operator".to_string(),
            password_secret_ref: None,
            passphrase_secret_ref: None,
            identity_ref: None,
            host_key_policy: None,
        });
    let scanned_with_jump = normalize_session_profile(scanned_with_jump);
    let jump_ssh = ssh_connection(&scanned_with_jump).unwrap();
    let jump_policy = jump_host_key_policy(jump_ssh, &jump_ssh.jumps[0]);
    let jump_observation = HostKeyObservation {
        host: "jump.example".to_string(),
        port: 2222,
        alias: jump_policy.alias.clone(),
        algorithm: "ssh-ed25519".to_string(),
        public_key_base64: "YWJj".to_string(),
    };
    store.upsert_profile(scanned_with_jump.clone());
    assert_eq!(
        validate_scanned_host_key_profile_snapshot(
            &store,
            &scanned_with_jump,
            &jump_observation,
        )
        .unwrap(),
        jump_policy
    );

    store.upsert_profile(scanned.clone());
    let mut mirrored = store.profile(&scanned.id).unwrap();
    let mirrored_ssh = ssh_connection_mut(&mut mirrored).unwrap();
    mirrored_ssh.trusted_host_keys.push(TrustedHostKey {
        id: "concurrent-host-key".to_string(),
        profile_id: Some(scanned.id.clone()),
        alias: observation.alias.clone().unwrap(),
        host: observation.host.clone(),
        port: observation.port,
        algorithm: observation.algorithm.clone(),
        fingerprint_sha256: "SHA256:concurrent".to_string(),
        public_key_base64: "ZGVm".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    });
    store.upsert_profile(mirrored);
    validate_scanned_host_key_profile_snapshot(&store, &scanned, &observation).unwrap();

    let mut changed = store.profile(&scanned.id).unwrap();
    ssh_connection_mut(&mut changed).unwrap().endpoint.host = "changed.example".to_string();
    store.upsert_profile(changed);
    let error = validate_scanned_host_key_profile_snapshot(&store, &scanned, &observation)
        .unwrap_err();
    assert!(error.contains("SSH 配置已在 Host Key 扫描后变化"), "{error}");

    store.upsert_profile(scanned.clone());
    let mut wrong_observation = observation.clone();
    wrong_observation.host = "other.example".to_string();
    let error = validate_scanned_host_key_profile_snapshot(
        &store,
        &scanned,
        &wrong_observation,
    )
    .unwrap_err();
    assert!(error.contains("当前 SSH 目标或 Jump Host 不匹配"), "{error}");

    store.delete_profile(&scanned.id).unwrap();
    let error = validate_scanned_host_key_profile_snapshot(&store, &scanned, &observation)
        .unwrap_err();
    assert!(error.contains("Profile 已删除"), "{error}");
}

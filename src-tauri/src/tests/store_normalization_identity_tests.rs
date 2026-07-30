#[test]
fn normalize_loaded_store_keeps_colliding_profile_identities_separate() {
    let mut store = SessionStore::default();
    let now = Utc::now();
    let mut first_profile = test_ssh_profile();
    first_profile.id = "edge".to_string();
    first_profile.name = "Primary edge".to_string();
    let ConnectionConfig::Ssh(first_ssh) = &mut first_profile.connection else {
        unreachable!("test profile must use SSH");
    };
    first_ssh.host_key_policy.alias = None;
    first_ssh.trusted_host_keys.push(TrustedHostKey {
        id: "embedded-primary-key".to_string(),
        profile_id: Some("edge".to_string()),
        alias: "embedded-primary".to_string(),
        host: "embedded-primary".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:embedded-primary".to_string(),
        public_key_base64: "AAAA".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: now,
        last_seen: now,
    });
    let mut second_profile = first_profile.clone();
    second_profile.id = " edge ".to_string();
    second_profile.name = "Legacy spaced edge".to_string();
    let ConnectionConfig::Ssh(second_ssh) = &mut second_profile.connection else {
        unreachable!("test profile must use SSH");
    };
    second_ssh.trusted_host_keys[0].id = "embedded-legacy-key".to_string();
    second_ssh.trusted_host_keys[0].profile_id = Some(" edge ".to_string());
    second_ssh.trusted_host_keys[0].fingerprint_sha256 = "SHA256:embedded-legacy".to_string();
    store.upsert_profile(first_profile);
    store.upsert_profile(second_profile);
    store
        .record_stream_event(
            "edge",
            EventDirection::Inbound,
            EventStream::Stdout,
            "primary output",
        )
        .unwrap();
    store
        .record_stream_event(
            " edge ",
            EventDirection::Inbound,
            EventStream::Stdout,
            "legacy output",
        )
        .unwrap();
    store.events.push(SessionEvent {
        id: "ambiguous-event".to_string(),
        session_id: "\tedge\n".to_string(),
        pane_id: "\tedge\n:main".to_string(),
        ts: Utc::now(),
        direction: EventDirection::Inbound,
        stream: EventStream::Stdout,
        bytes_ref: None,
        text: Some("ambiguous output".to_string()),
        annotations: BTreeMap::new(),
    });
    store.host_keys.keys.extend([
        TrustedHostKey {
            id: "primary-key".to_string(),
            profile_id: Some("edge".to_string()),
            alias: "primary".to_string(),
            host: "primary".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:primary".to_string(),
            public_key_base64: "AAAA".to_string(),
            scope: HostKeyScope::Profile,
            label: None,
            first_seen: now,
            last_seen: now,
        },
        TrustedHostKey {
            id: "legacy-key".to_string(),
            profile_id: Some(" edge ".to_string()),
            alias: "legacy".to_string(),
            host: "legacy".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:legacy".to_string(),
            public_key_base64: "AAAA".to_string(),
            scope: HostKeyScope::Profile,
            label: None,
            first_seen: now,
            last_seen: now,
        },
    ]);
    store.grants.extend([
        McpGrant {
            client_id: "primary-reader".to_string(),
            name: "Primary reader".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: vec!["edge".to_string()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        },
        McpGrant {
            client_id: "legacy-reader".to_string(),
            name: "Legacy reader".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: vec![" edge ".to_string()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        },
    ]);
    store.one_keys.push(OneKeyCredential {
        id: "collision-one-key".to_string(),
        label: "Collision OneKey".to_string(),
        kind: OneKeyKind::Account,
        username: "operator".to_string(),
        password_secret_ref: Some("keychain:collision-password".to_string()),
        passphrase_secret_ref: None,
        identity: None,
        session_ids: vec!["edge".to_string(), " edge ".to_string()],
        created_at: now,
        updated_at: now,
    });

    let normalized = normalize_loaded_store(store);
    let primary = normalized
        .profiles
        .iter()
        .find(|profile| profile.name == "Primary edge")
        .unwrap();
    let legacy = normalized
        .profiles
        .iter()
        .find(|profile| profile.name == "Legacy spaced edge")
        .unwrap();

    assert_eq!(primary.id, "edge");
    assert_eq!(legacy.id, "edge:loaded:2");
    let ConnectionConfig::Ssh(primary_ssh) = &primary.connection else {
        unreachable!("normalized profile must use SSH");
    };
    let ConnectionConfig::Ssh(legacy_ssh) = &legacy.connection else {
        unreachable!("normalized profile must use SSH");
    };
    assert_eq!(primary_ssh.host_key_policy.alias.as_deref(), Some("edge"));
    assert_eq!(
        legacy_ssh.host_key_policy.alias.as_deref(),
        Some("edge:loaded:2")
    );
    assert_eq!(
        primary_ssh.trusted_host_keys[0].profile_id.as_deref(),
        Some("edge")
    );
    assert_eq!(
        legacy_ssh.trusted_host_keys[0].profile_id.as_deref(),
        Some("edge:loaded:2")
    );
    assert_eq!(normalized.profiles.len(), 2);
    assert_eq!(
        normalized
            .profiles
            .iter()
            .map(|profile| profile.id.as_str())
            .collect::<HashSet<_>>()
            .len(),
        2
    );
    assert_eq!(
        normalized.tail_log(&primary.id, 10)[0].text.as_deref(),
        Some("primary output")
    );
    assert_eq!(
        normalized.tail_log(&legacy.id, 10)[0].text.as_deref(),
        Some("legacy output")
    );
    assert!(!normalized
        .events
        .iter()
        .any(|event| event.id == "ambiguous-event"));
    assert_eq!(
        normalized
            .host_keys
            .keys
            .iter()
            .find(|key| key.id == "primary-key")
            .and_then(|key| key.profile_id.as_deref()),
        Some(primary.id.as_str())
    );
    assert_eq!(
        normalized
            .host_keys
            .keys
            .iter()
            .find(|key| key.id == "legacy-key")
            .and_then(|key| key.profile_id.as_deref()),
        Some(legacy.id.as_str())
    );
    assert!(normalized.mcp_can_read("primary-reader", McpScope::ReadLogs, Some(&primary.id)));
    assert!(!normalized.mcp_can_read("primary-reader", McpScope::ReadLogs, Some(&legacy.id)));
    assert!(normalized.mcp_can_read("legacy-reader", McpScope::ReadLogs, Some(&legacy.id)));
    assert_eq!(
        normalized.one_keys[0].session_ids,
        [primary.id.as_str(), legacy.id.as_str()]
    );
    assert!(normalized
        .runtimes
        .iter()
        .any(|runtime| { runtime.session_id == primary.id && runtime.title == "Primary edge" }));
    assert!(normalized.runtimes.iter().any(|runtime| {
        runtime.session_id == legacy.id && runtime.title == "Legacy spaced edge"
    }));

    let normalized_ids = normalized
        .profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    let normalized_again = normalize_loaded_store(normalized);
    assert_eq!(
        normalized_again
            .profiles
            .iter()
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>(),
        normalized_ids
    );
}


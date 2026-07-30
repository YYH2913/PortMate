#[test]
fn normalize_loaded_store_quarantines_conflicting_mcp_grants_before_mirroring() {
    let mut store = SessionStore::default();
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    store.upsert_profile(profile);
    let valid = McpGrant {
        client_id: " reader ".to_string(),
        name: " Reader ".to_string(),
        scopes: vec![McpScope::ReadLogs],
        allowed_sessions: vec![format!(" {session_id} ")],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    };
    store.grants.push(valid.clone());
    store.grants.push(valid);
    store.grants.push(McpGrant {
        client_id: "reader".to_string(),
        name: "Conflicting reader".to_string(),
        scopes: vec![McpScope::ReadSessions],
        allowed_sessions: Vec::new(),
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    store.grants.push(McpGrant {
        client_id: "bad\nclient".to_string(),
        name: "Invalid".to_string(),
        scopes: vec![McpScope::ReadSessions],
        allowed_sessions: Vec::new(),
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });

    let normalized = normalize_loaded_store(store);
    assert_eq!(normalized.grants.len(), 2);
    let reader = normalized
        .grants
        .iter()
        .find(|grant| grant.client_id == "reader")
        .unwrap();
    assert_eq!(reader.name, "Reader");
    assert_eq!(reader.scopes, [McpScope::ReadLogs]);
    assert_eq!(
        reader.allowed_sessions.as_slice(),
        std::slice::from_ref(&session_id)
    );
    let quarantine = normalized
        .grants
        .iter()
        .find(|grant| {
            grant
                .client_id
                .starts_with("portmate:invalid-loaded-grant:")
        })
        .unwrap();
    assert!(quarantine.scopes.is_empty());
    assert!(quarantine.revoked_at.is_some());
    assert!(normalized.mcp_can_read("reader", McpScope::ReadLogs, Some(&session_id)));
    assert!(!normalized.mcp_can_read("reader", McpScope::ReadSessions, None));
    assert!(!normalized.mcp_can_read("unknown", McpScope::ReadSessions, None));

    let root = std::env::temp_dir().join(format!("portmate-grant-load-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    save_store(&store_path, &normalized).unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    let mirrored: usize = connection
        .query_row("select count(*) from mcp_grants", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mirrored, 2);
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn normalize_loaded_store_repairs_duplicate_mirror_keys() {
    let mut store = SessionStore::default();
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    store.upsert_profile(profile);
    let now = Utc::now();

    let event = SessionEvent {
        id: "duplicate-event".to_string(),
        session_id: session_id.clone(),
        pane_id: format!("{session_id}:main"),
        ts: now,
        direction: EventDirection::Inbound,
        stream: EventStream::Stdout,
        bytes_ref: None,
        text: Some("first event".to_string()),
        annotations: BTreeMap::new(),
    };
    let mut duplicate_event = event.clone();
    duplicate_event.text = Some("second event".to_string());
    store.events.extend([event, duplicate_event]);

    let transfer = TransferTask {
        id: "duplicate-transfer".to_string(),
        session_id: session_id.clone(),
        protocol: TransferProtocol::Sftp,
        source: "first-source".to_string(),
        destination: "first-destination".to_string(),
        bytes_total: 1,
        bytes_done: 1,
        status: TransferStatus::Completed,
        message: None,
        started_at: Some(now),
        finished_at: Some(now),
        average_bytes_per_second: Some(1.0),
    };
    let mut duplicate_transfer = transfer.clone();
    duplicate_transfer.source = "second-source".to_string();
    store.transfers.extend([transfer, duplicate_transfer]);

    let audit = AuditRecord {
        id: "duplicate-audit".to_string(),
        ts: now,
        actor: "desktop-user".to_string(),
        action: "first-action".to_string(),
        session_id: Some(session_id.clone()),
        decision: "recorded".to_string(),
        details: BTreeMap::new(),
    };
    let mut duplicate_audit = audit.clone();
    duplicate_audit.action = "second-action".to_string();
    store.audit.extend([audit, duplicate_audit]);

    let timeline = TimelineMark {
        id: "duplicate-timeline".to_string(),
        session_id: session_id.clone(),
        ts: now,
        label: "first mark".to_string(),
        details: None,
    };
    let mut duplicate_timeline = timeline.clone();
    duplicate_timeline.label = "second mark".to_string();
    store.timeline.extend([timeline, duplicate_timeline]);

    let key = TrustedHostKey {
        id: "duplicate-host-key".to_string(),
        profile_id: Some(session_id.clone()),
        alias: "first-host".to_string(),
        host: "first-host".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:first".to_string(),
        public_key_base64: "AAAA".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: now,
        last_seen: now,
    };
    let mut duplicate_key = key.clone();
    duplicate_key.alias = "second-host".to_string();
    duplicate_key.host = "second-host".to_string();
    duplicate_key.fingerprint_sha256 = "SHA256:second".to_string();
    store.host_keys.keys.extend([key, duplicate_key]);

    let snapshot = test_sysmon_snapshot(&session_id);
    let mut duplicate_snapshot = snapshot.clone();
    duplicate_snapshot.uptime_seconds = snapshot.uptime_seconds + 1;
    store.sysmon.extend([snapshot, duplicate_snapshot]);

    let normalized = normalize_loaded_store(store);
    assert_eq!(
        normalized
            .events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        ["duplicate-event", "duplicate-event:loaded:2"]
    );
    assert_eq!(
        normalized
            .transfers
            .iter()
            .map(|transfer| transfer.id.as_str())
            .collect::<Vec<_>>(),
        ["duplicate-transfer", "duplicate-transfer:loaded:2"]
    );
    assert_eq!(
        normalized
            .audit
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        ["duplicate-audit", "duplicate-audit:loaded:2"]
    );
    assert_eq!(
        normalized
            .timeline
            .iter()
            .map(|mark| mark.id.as_str())
            .collect::<Vec<_>>(),
        ["duplicate-timeline", "duplicate-timeline:loaded:2"]
    );
    assert_eq!(
        normalized
            .host_keys
            .keys
            .iter()
            .map(|key| key.id.as_str())
            .collect::<Vec<_>>(),
        ["duplicate-host-key", "duplicate-host-key:loaded:2"]
    );
    assert_eq!(normalized.sysmon.len(), 1);
    assert_eq!(normalized.sysmon[0].uptime_seconds, 61);

    let root = std::env::temp_dir().join(format!("portmate-duplicate-load-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    save_store(&store_path, &normalized).unwrap();
    let connection = SqliteConnection::open(&store_path).unwrap();
    for (table, expected) in [
        ("events", 2),
        ("transfers", 2),
        ("trusted_host_keys", 2),
        ("mcp_audit", 2),
        ("timeline_marks", 2),
        ("sysmon_snapshots", 1),
    ] {
        let count: usize = connection
            .query_row(&format!("select count(*) from {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, expected, "{table}");
    }
    drop(connection);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn loaded_mirror_id_repair_preserves_unique_suffix_ids() {
    let mut ids = vec![
        "duplicate".to_string(),
        "duplicate".to_string(),
        "duplicate:loaded:2".to_string(),
    ];

    normalize_loaded_record_ids(
        &mut ids,
        "event",
        |id| id.as_str(),
        |id, normalized| *id = normalized,
    );

    assert_eq!(
        ids,
        ["duplicate", "duplicate:loaded:3", "duplicate:loaded:2",]
    );
}

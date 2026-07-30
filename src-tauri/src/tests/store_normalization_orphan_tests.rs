#[test]
fn normalize_loaded_store_marks_orphaned_active_transfers_interrupted() {
    let mut store = SessionStore::default();
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    store.upsert_profile(profile);
    let started_at = Utc::now() - chrono::Duration::seconds(2);
    for (id, status, started_at, finished_at) in [
        ("queued", TransferStatus::Queued, None, None),
        ("running", TransferStatus::Running, Some(started_at), None),
        (
            "completed",
            TransferStatus::Completed,
            Some(started_at),
            Some(Utc::now()),
        ),
    ] {
        store.record_transfer(TransferTask {
            id: id.to_string(),
            session_id: session_id.clone(),
            protocol: TransferProtocol::Sftp,
            source: "source.bin".to_string(),
            destination: "destination.bin".to_string(),
            bytes_total: 1_024,
            bytes_done: 1_024,
            status,
            message: None,
            started_at,
            finished_at,
            average_bytes_per_second: None,
        });
    }

    let normalized = normalize_loaded_store(store);
    for id in ["queued", "running"] {
        let task = normalized.transfer_by_id(id).unwrap();
        assert_eq!(task.status, TransferStatus::Failed);
        assert_eq!(
            task.message.as_deref(),
            Some("interrupted by previous PortMate shutdown")
        );
        assert!(task.finished_at.is_some());
    }
    assert!(normalized
        .transfer_by_id("running")
        .unwrap()
        .average_bytes_per_second
        .is_some());
    assert_eq!(
        normalized.transfer_by_id("completed").unwrap().status,
        TransferStatus::Completed
    );
}

#[test]
fn normalize_loaded_store_prunes_orphaned_session_state_and_revokes_access() {
    let mut store = SessionStore::default();
    let profile = test_shell_profile();
    let current_session_id = profile.id.clone();
    let orphaned_session_id = "deleted-session";
    store.upsert_profile(profile);
    let now = Utc::now();

    let mut orphaned_runtime = store.runtimes[0].clone();
    orphaned_runtime.session_id = orphaned_session_id.to_string();
    orphaned_runtime.pane_id = format!("{orphaned_session_id}:main");
    store.runtimes.push(orphaned_runtime);
    store.events.push(SessionEvent {
        id: "orphaned-event".to_string(),
        session_id: orphaned_session_id.to_string(),
        pane_id: format!("{orphaned_session_id}:main"),
        ts: now,
        direction: EventDirection::Inbound,
        stream: EventStream::Stdout,
        bytes_ref: Some("v2:orphaned.raw:0:6:digest".to_string()),
        text: Some("secret orphaned output".to_string()),
        annotations: BTreeMap::new(),
    });
    store.record_transfer(TransferTask {
        id: "orphaned-transfer".to_string(),
        session_id: orphaned_session_id.to_string(),
        protocol: TransferProtocol::Sftp,
        source: "orphaned-source".to_string(),
        destination: "orphaned-destination".to_string(),
        bytes_total: 1,
        bytes_done: 0,
        status: TransferStatus::Running,
        message: None,
        started_at: Some(now),
        finished_at: None,
        average_bytes_per_second: None,
    });
    store.record_timeline_mark(TimelineMark {
        id: "orphaned-mark".to_string(),
        session_id: orphaned_session_id.to_string(),
        ts: now,
        label: "orphaned timeline".to_string(),
        details: None,
    });
    store.record_sysmon_snapshot(SysmonSnapshot {
        session_id: orphaned_session_id.to_string(),
        ts: now,
        uptime_seconds: 1,
        cpu_percent: 2.0,
        memory_percent: 3.0,
        rx_kbps: 4.0,
        tx_kbps: 5.0,
        load_average: [0.0; 3],
        memory_total_bytes: 0,
        memory_available_bytes: 0,
        processes: Vec::new(),
        disks: Vec::new(),
        network_interfaces: Vec::new(),
    });
    store.record_audit(AuditRecord {
        id: "orphaned-audit".to_string(),
        ts: now,
        actor: "desktop-user".to_string(),
        action: "deleted-profile-action".to_string(),
        session_id: Some(orphaned_session_id.to_string()),
        decision: "recorded".to_string(),
        details: BTreeMap::new(),
    });
    store.host_keys.keys.extend([
        TrustedHostKey {
            id: "orphaned-profile-key".to_string(),
            profile_id: Some(orphaned_session_id.to_string()),
            alias: "deleted-device".to_string(),
            host: "deleted-device".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:orphaned-profile".to_string(),
            public_key_base64: "AAAA".to_string(),
            scope: HostKeyScope::Profile,
            label: None,
            first_seen: now,
            last_seen: now,
        },
        TrustedHostKey {
            id: "orphaned-project-key".to_string(),
            profile_id: Some(orphaned_session_id.to_string()),
            alias: "shared-device".to_string(),
            host: "shared-device".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:orphaned-project".to_string(),
            public_key_base64: "AAAA".to_string(),
            scope: HostKeyScope::Project,
            label: None,
            first_seen: now,
            last_seen: now,
        },
    ]);
    store.grants.extend([
        McpGrant {
            client_id: "orphaned-reader".to_string(),
            name: "Orphaned reader".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: vec![orphaned_session_id.to_string()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        },
        McpGrant {
            client_id: "orphaned-reader".to_string(),
            name: "Orphaned reader".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: vec![orphaned_session_id.to_string()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        },
        McpGrant {
            client_id: "mixed-reader".to_string(),
            name: "Mixed reader".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: vec![orphaned_session_id.to_string(), current_session_id.clone()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        },
        McpGrant {
            client_id: "global-reader".to_string(),
            name: "Global reader".to_string(),
            scopes: vec![McpScope::ReadLogs],
            allowed_sessions: Vec::new(),
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        },
    ]);

    let mut normalized = normalize_loaded_store(store);

    assert_eq!(normalized.grants.len(), 3);
    assert!(!normalized.grants.iter().any(|grant| grant
        .client_id
        .starts_with("portmate:invalid-loaded-grant:")));
    assert!(normalized
        .runtimes
        .iter()
        .all(|runtime| runtime.session_id != orphaned_session_id));
    assert!(normalized.tail_log(orphaned_session_id, 10).is_empty());
    assert!(normalized.transfer_by_id("orphaned-transfer").is_none());
    assert!(normalized.timeline_for(orphaned_session_id).is_empty());
    assert!(normalized.sysmon_for(orphaned_session_id).is_none());
    assert!(normalized
        .audit
        .iter()
        .any(|record| record.id == "orphaned-audit"));
    assert!(normalized
        .host_keys
        .keys
        .iter()
        .all(|key| key.id != "orphaned-profile-key"));
    let project_key = normalized
        .host_keys
        .keys
        .iter()
        .find(|key| key.id == "orphaned-project-key")
        .unwrap();
    assert!(project_key.profile_id.is_none());

    let orphaned_grant = normalized
        .grants
        .iter()
        .find(|grant| grant.client_id == "orphaned-reader")
        .unwrap();
    assert!(orphaned_grant.allowed_sessions.is_empty());
    assert!(orphaned_grant.revoked_at.is_some());
    let mixed_grant = normalized
        .grants
        .iter()
        .find(|grant| grant.client_id == "mixed-reader")
        .unwrap();
    assert_eq!(mixed_grant.allowed_sessions, [current_session_id.as_str()]);
    assert!(mixed_grant.revoked_at.is_none());
    let global_grant = normalized
        .grants
        .iter()
        .find(|grant| grant.client_id == "global-reader")
        .unwrap();
    assert!(global_grant.allowed_sessions.is_empty());
    assert!(global_grant.revoked_at.is_none());

    let mut recreated = test_shell_profile();
    recreated.id = orphaned_session_id.to_string();
    normalized.upsert_profile(recreated);
    assert!(!normalized.mcp_can_read(
        "orphaned-reader",
        McpScope::ReadLogs,
        Some(orphaned_session_id)
    ));
    assert!(normalized.mcp_can_read(
        "mixed-reader",
        McpScope::ReadLogs,
        Some(&current_session_id)
    ));
    assert!(normalized.mcp_can_read(
        "global-reader",
        McpScope::ReadLogs,
        Some(orphaned_session_id)
    ));
}


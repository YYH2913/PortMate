use super::*;

#[test]
fn normalize_loaded_store_preserves_inactive_runtime_diagnostics() {
    let mut store = SessionStore::default();
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    store.upsert_profile(profile);
    let last_activity = Utc::now() - chrono::Duration::minutes(5);
    let last_disconnect = Utc::now() - chrono::Duration::minutes(4);
    let runtime = store
        .runtimes
        .iter_mut()
        .find(|runtime| runtime.session_id == session_id)
        .unwrap();
    runtime.status = SessionStatus::Error;
    runtime.connected_since = Some(Utc::now() - chrono::Duration::hours(1));
    runtime.pane_id = "custom-pane".to_string();
    runtime.title = "dynamic shell title".to_string();
    runtime.cwd = Some("/tmp/worktree".to_string());
    runtime.last_activity = last_activity;
    runtime.last_disconnect = Some(last_disconnect);
    runtime.last_disconnect_reason = Some("network timeout".to_string());

    let normalized = normalize_loaded_store(store);
    let runtime = normalized
        .runtimes
        .iter()
        .find(|runtime| runtime.session_id == session_id)
        .unwrap();

    assert_eq!(runtime.status, SessionStatus::Disconnected);
    assert!(runtime.connected_since.is_none());
    assert_eq!(runtime.active_transport, SessionKind::Shell);
    assert_eq!(runtime.pane_id, "custom-pane");
    assert_eq!(runtime.title, "dynamic shell title");
    assert_eq!(runtime.cwd.as_deref(), Some("/tmp/worktree"));
    assert_eq!(runtime.last_activity, last_activity);
    assert_eq!(runtime.last_disconnect, Some(last_disconnect));
    assert_eq!(
        runtime.last_disconnect_reason.as_deref(),
        Some("network timeout")
    );
}

#[test]
fn normalize_loaded_store_rejects_oversized_profile_collections() {
    let mut store = SessionStore::default();
    store.profiles = vec![test_shell_profile(); portmate_core::MAX_SESSION_PROFILES + 1];

    let error = normalize_loaded_store_checked(store).unwrap_err();

    assert!(error.contains(&portmate_core::MAX_SESSION_PROFILES.to_string()));
}

#[test]
fn normalize_loaded_store_records_interrupted_active_runtime_diagnostics() {
    let loaded_at = Utc::now();
    let previous_disconnect = loaded_at - chrono::Duration::minutes(5);
    for (status, saved_disconnect, expected_disconnect, expected_reason) in [
        (
            SessionStatus::Connected,
            Some(previous_disconnect),
            loaded_at,
            "connection interrupted by previous PortMate shutdown",
        ),
        (
            SessionStatus::Connecting,
            None,
            loaded_at,
            "connection attempt interrupted by previous PortMate shutdown",
        ),
        (
            SessionStatus::Reconnecting,
            Some(previous_disconnect),
            previous_disconnect,
            "reconnect interrupted by previous PortMate shutdown",
        ),
        (
            SessionStatus::Reconnecting,
            None,
            loaded_at,
            "reconnect interrupted by previous PortMate shutdown",
        ),
    ] {
        let mut store = SessionStore::default();
        let profile = test_shell_profile();
        let session_id = profile.id.clone();
        store.upsert_profile(profile);
        let runtime = store
            .runtimes
            .iter_mut()
            .find(|runtime| runtime.session_id == session_id)
            .unwrap();
        runtime.status = status;
        runtime.connected_since = Some(loaded_at - chrono::Duration::hours(1));
        runtime.last_disconnect = saved_disconnect;
        runtime.last_disconnect_reason = Some("stale diagnostic".to_string());

        let normalized = normalize_loaded_store_at(store, loaded_at);
        let runtime = normalized
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == session_id)
            .unwrap();

        assert_eq!(runtime.status, SessionStatus::Disconnected);
        assert!(runtime.connected_since.is_none());
        assert_eq!(runtime.last_disconnect, Some(expected_disconnect));
        assert_eq!(
            runtime.last_disconnect_reason.as_deref(),
            Some(expected_reason)
        );

        let normalized_again =
            normalize_loaded_store_at(normalized, loaded_at + chrono::Duration::minutes(1));
        let runtime = normalized_again
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == session_id)
            .unwrap();
        assert_eq!(runtime.last_disconnect, Some(expected_disconnect));
        assert_eq!(
            runtime.last_disconnect_reason.as_deref(),
            Some(expected_reason)
        );
    }
}

#[test]
fn normalize_loaded_store_bounds_legacy_disconnect_reasons() {
    let mut store = SessionStore::default();
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    store.upsert_profile(profile);
    store.runtimes[0].last_disconnect_reason =
        Some(format!("  legacy\n reason {}  ", "界".repeat(300)));

    let normalized = normalize_loaded_store(store);
    let reason = normalized
        .runtimes
        .iter()
        .find(|runtime| runtime.session_id == session_id)
        .and_then(|runtime| runtime.last_disconnect_reason.as_deref())
        .unwrap();

    assert!(reason.starts_with("legacy reason 界"));
    assert!(reason.ends_with("..."));
    assert!(!reason.contains('\n'));
    assert_eq!(
        reason.chars().count(),
        portmate_core::MAX_SESSION_DISCONNECT_REASON_CHARACTERS
    );
}

#[test]
fn normalize_loaded_store_remaps_trimmed_profile_references() {
    let mut store = SessionStore::default();
    let mut profile = test_shell_profile();
    profile.id = " legacy-session ".to_string();
    let original_id = profile.id.clone();
    store.upsert_profile(profile);
    store
        .record_stream_event(
            &original_id,
            EventDirection::Inbound,
            EventStream::Stdout,
            "legacy event",
        )
        .unwrap();
    store.record_transfer(TransferTask {
        id: "legacy-transfer".to_string(),
        session_id: original_id.clone(),
        protocol: TransferProtocol::Sftp,
        source: "source".to_string(),
        destination: "destination".to_string(),
        bytes_total: 1,
        bytes_done: 1,
        status: TransferStatus::Completed,
        message: None,
        started_at: None,
        finished_at: Some(Utc::now()),
        average_bytes_per_second: None,
    });
    store.record_audit(AuditRecord {
        id: "legacy-audit".to_string(),
        ts: Utc::now(),
        actor: "test".to_string(),
        action: "load".to_string(),
        session_id: Some(original_id.clone()),
        decision: "recorded".to_string(),
        details: BTreeMap::new(),
    });
    store.record_timeline_mark(TimelineMark {
        id: "legacy-mark".to_string(),
        session_id: original_id.clone(),
        ts: Utc::now(),
        label: "loaded".to_string(),
        details: None,
    });
    store.record_sysmon_snapshot(SysmonSnapshot {
        session_id: original_id.clone(),
        ts: Utc::now(),
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
    store.host_keys.keys.push(TrustedHostKey {
        id: "legacy-host-key".to_string(),
        profile_id: Some(original_id.clone()),
        alias: " legacy-session ".to_string(),
        host: "example.test".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:test".to_string(),
        public_key_base64: "AAAA".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    });
    store.grants.push(McpGrant {
        client_id: "legacy-reader".to_string(),
        name: "Legacy reader".to_string(),
        scopes: vec![McpScope::ReadSessions],
        allowed_sessions: vec![original_id.clone()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    store.one_keys.push(OneKeyCredential {
        id: "legacy-one-key".to_string(),
        label: "Legacy".to_string(),
        kind: OneKeyKind::Account,
        username: "operator".to_string(),
        password_secret_ref: Some("keychain:legacy-password".to_string()),
        passphrase_secret_ref: None,
        identity: None,
        session_ids: vec![original_id],
        created_at: Utc::now(),
        updated_at: Utc::now(),
    });
    store.runtimes[0].session_id = "\tlegacy-session\n".to_string();
    store.runtimes[0].pane_id = "\tlegacy-session\n:main".to_string();

    let normalized = normalize_loaded_store(store);
    let expected = "legacy-session";
    let runtime = normalized.runtimes.first().unwrap();
    assert_eq!(normalized.profiles[0].id, expected);
    assert_eq!(runtime.session_id, expected);
    assert_eq!(runtime.pane_id, format!("{expected}:main"));
    assert_eq!(normalized.events[0].session_id, expected);
    assert_eq!(normalized.events[0].pane_id, format!("{expected}:main"));
    assert_eq!(normalized.transfers[0].session_id, expected);
    assert_eq!(normalized.audit[0].session_id.as_deref(), Some(expected));
    assert_eq!(normalized.timeline[0].session_id, expected);
    assert_eq!(normalized.sysmon[0].session_id, expected);
    assert_eq!(
        normalized.host_keys.keys[0].profile_id.as_deref(),
        Some(expected)
    );
    assert_eq!(normalized.host_keys.keys[0].alias, expected);
    assert_eq!(normalized.grants[0].allowed_sessions, [expected]);
    assert_eq!(normalized.one_keys[0].session_ids, [expected]);
    assert_eq!(normalized.tail_log(expected, 10).len(), 1);
    assert!(normalized.sysmon_for(expected).is_some());
}

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

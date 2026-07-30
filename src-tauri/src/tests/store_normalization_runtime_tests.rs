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


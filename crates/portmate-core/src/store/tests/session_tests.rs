use super::*;

#[test]
fn upsert_profile_creates_runtime_for_new_session() {
    let mut store = SessionStore::default();
    let summary = store.upsert_profile(SessionProfile {
        id: "new-session".to_string(),
        name: "new session".to_string(),
        kind: SessionKind::Serial,
        group: "serial".to_string(),
        tags: Vec::new(),
        connection: ConnectionConfig::Serial(SerialConnection {
            port: "COM7".to_string(),
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".to_string(),
            flow_control: "none".to_string(),
            dtr: false,
            rts: false,
            reconnect: true,
            reconnect_delay_ms: DEFAULT_SERIAL_RECONNECT_DELAY_MS,
            receive_idle_timeout_enabled: false,
            receive_idle_timeout_seconds: DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS,
        }),
        terminal: TerminalSettings::default(),
        logging: LoggingSettings::default(),
        triggers: Vec::new(),
        transfer: TransferSettings::default(),
    });

    assert_eq!(summary.profile.id, "new-session");
    assert_eq!(summary.runtime.status, SessionStatus::Disconnected);
    assert_eq!(store.summaries().len(), 1);
}

#[test]
fn profile_capacity_allows_updates_at_limit_and_rejects_oversized_stores() {
    let mut store = test_store();
    let profile = store.profiles[0].clone();
    store.profiles = (0..MAX_SESSION_PROFILES)
        .map(|index| {
            let mut profile = profile.clone();
            profile.id = format!("session-{index}");
            profile
        })
        .collect();

    assert!(store.validate_profile_count().is_ok());
    assert!(store.validate_profile_capacity("session-0").is_ok());
    let capacity_error = store.validate_profile_capacity("new-session").unwrap_err();
    assert!(capacity_error.contains(&MAX_SESSION_PROFILES.to_string()));

    let mut overflow = profile;
    overflow.id = "overflow-session".to_string();
    store.profiles.push(overflow);
    let count_error = store.validate_profile_count().unwrap_err();
    assert!(count_error.contains(&MAX_SESSION_PROFILES.to_string()));
    assert!(store.validate_profile_capacity("session-0").is_err());
}

#[test]
fn upsert_profile_preserves_active_transport_until_disconnect() {
    let mut store = test_store();
    let mut profile = store.profile("test-session").unwrap();
    profile.kind = SessionKind::Serial;
    profile.connection = ConnectionConfig::Serial(SerialConnection {
        port: "COM7".to_string(),
        baud_rate: 115_200,
        data_bits: 8,
        stop_bits: 1,
        parity: "none".to_string(),
        flow_control: "none".to_string(),
        dtr: false,
        rts: false,
        reconnect: true,
        reconnect_delay_ms: DEFAULT_SERIAL_RECONNECT_DELAY_MS,
        receive_idle_timeout_enabled: false,
        receive_idle_timeout_seconds: DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS,
    });

    let active = store.upsert_profile(profile);
    assert_eq!(active.profile.kind, SessionKind::Serial);
    assert_eq!(active.runtime.status, SessionStatus::Connected);
    assert_eq!(active.runtime.active_transport, SessionKind::Shell);

    let disconnected = store
        .set_runtime_status("test-session", SessionStatus::Disconnected)
        .unwrap();
    assert_eq!(disconnected.runtime.active_transport, SessionKind::Serial);
}

#[test]
fn open_and_close_session_updates_runtime_and_log() {
    let mut store = test_store();
    let opened = store.open_session("test-session").unwrap();
    assert_eq!(opened.runtime.status, SessionStatus::Connected);
    assert!(store.screen("test-session").unwrap().contains("connected"));

    let closed = store.close_session("test-session").unwrap();
    assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
    assert!(closed.runtime.last_disconnect.is_some());
    assert_eq!(
        closed.runtime.last_disconnect_reason.as_deref(),
        Some("user closed session")
    );
    assert!(store
        .screen("test-session")
        .unwrap()
        .contains("disconnected"));
}

#[test]
fn delete_profile_rejects_live_sessions_and_active_transfers() {
    let mut store = test_store();
    let live_error = store.delete_profile("test-session").unwrap_err();
    assert!(live_error.contains("must be disconnected"));
    assert!(store.profile("test-session").is_some());

    store.runtimes[0].status = SessionStatus::Disconnected;
    store.record_transfer(test_transfer("queued".to_string(), TransferStatus::Queued));
    let transfer_error = store.delete_profile("test-session").unwrap_err();
    assert!(transfer_error.contains("active transfer"));
    assert!(store.profile("test-session").is_some());
}

#[test]
fn delete_profile_cascades_without_widening_grants_or_project_trust() {
    let mut store = test_store();
    store.runtimes[0].status = SessionStatus::Disconnected;
    let (event_sender, _event_receiver) = std::sync::mpsc::sync_channel(1);
    store.set_system_event_notifier(event_sender).unwrap();
    store.record_system_event("test-session", "queued deletion event");
    let mut other_profile = store.profiles[0].clone();
    other_profile.id = "other-session".to_string();
    other_profile.name = "other session".to_string();
    store.upsert_profile(other_profile);
    store
        .record_stream_event(
            "test-session",
            EventDirection::Inbound,
            EventStream::Stdout,
            "deleted event",
        )
        .unwrap();
    store
        .record_stream_event(
            "other-session",
            EventDirection::Inbound,
            EventStream::Stdout,
            "retained event",
        )
        .unwrap();
    store.record_transfer(test_transfer(
        "completed".to_string(),
        TransferStatus::Completed,
    ));
    let now = Utc::now();
    store.record_timeline_mark(TimelineMark {
        id: "timeline-delete".to_string(),
        session_id: "test-session".to_string(),
        ts: now,
        label: "delete me".to_string(),
        details: None,
    });
    store.record_sysmon_snapshot(SysmonSnapshot {
        session_id: "test-session".to_string(),
        ts: now,
        uptime_seconds: 1,
        cpu_percent: 0.0,
        memory_percent: 0.0,
        rx_kbps: 0.0,
        tx_kbps: 0.0,
        load_average: [0.0; 3],
        memory_total_bytes: 0,
        memory_available_bytes: 0,
        processes: Vec::new(),
        disks: Vec::new(),
        network_interfaces: Vec::new(),
    });
    store.record_audit(AuditRecord {
        id: "audit-delete".to_string(),
        ts: now,
        actor: "desktop-user".to_string(),
        action: "profile-test".to_string(),
        session_id: Some("test-session".to_string()),
        decision: "recorded".to_string(),
        details: BTreeMap::new(),
    });
    store.host_keys.keys.extend([
        TrustedHostKey {
            id: "profile-key".to_string(),
            profile_id: Some("test-session".to_string()),
            alias: "device".to_string(),
            host: "device".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:profile".to_string(),
            public_key_base64: "AAAAC3NzaC1lZDI1NTE5AAAAIA==".to_string(),
            scope: HostKeyScope::Profile,
            label: None,
            first_seen: now,
            last_seen: now,
        },
        TrustedHostKey {
            id: "project-key".to_string(),
            profile_id: Some("test-session".to_string()),
            alias: "shared".to_string(),
            host: "shared".to_string(),
            port: 22,
            algorithm: "ssh-ed25519".to_string(),
            fingerprint_sha256: "SHA256:project".to_string(),
            public_key_base64: "AAAAC3NzaC1lZDI1NTE5AAAAIA==".to_string(),
            scope: HostKeyScope::Project,
            label: None,
            first_seen: now,
            last_seen: now,
        },
    ]);
    store.one_keys.push(OneKeyCredential {
        id: "one-key".to_string(),
        label: "shared login".to_string(),
        kind: OneKeyKind::Ssh,
        username: "operator".to_string(),
        password_secret_ref: Some("keyring:one-key".to_string()),
        passphrase_secret_ref: None,
        identity: Some(OneKeyIdentity {
            source_profile_id: "test-session".to_string(),
            identity: IdentityRef {
                id: "identity".to_string(),
                label: "identity".to_string(),
                source: IdentitySource::Agent,
                fingerprint_sha256: Some("SHA256:identity".to_string()),
                path: None,
                secret_ref: None,
            },
        }),
        session_ids: vec!["test-session".to_string(), "other-session".to_string()],
        created_at: now,
        updated_at: now,
    });
    store.grants.push(McpGrant {
        client_id: "mixed".to_string(),
        name: "mixed".to_string(),
        scopes: vec![McpScope::ReadLogs],
        allowed_sessions: vec!["test-session".to_string(), "other-session".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });

    let deleted = store.delete_profile("test-session").unwrap();

    assert_eq!(deleted.id, "test-session");
    assert!(store.profile("test-session").is_none());
    assert!(store.profile("other-session").is_some());
    assert!(store
        .events
        .iter()
        .all(|event| event.session_id != "test-session"));
    assert!(store.drain_system_event_outbox().is_empty());
    assert!(store
        .events
        .iter()
        .any(|event| event.session_id == "other-session"));
    assert!(store
        .transfers
        .iter()
        .all(|transfer| transfer.session_id != "test-session"));
    assert!(store.timeline.is_empty());
    assert!(store.sysmon.is_empty());
    assert!(store.audit.iter().any(|record| record.id == "audit-delete"));
    assert!(!store
        .host_keys
        .keys
        .iter()
        .any(|key| key.id == "profile-key"));
    assert!(store
        .host_keys
        .keys
        .iter()
        .any(|key| key.id == "project-key" && key.profile_id.is_none()));
    assert_eq!(store.one_keys[0].session_ids, ["other-session"]);
    assert!(store.one_keys[0].identity.is_none());
    assert!(store.one_keys[0].updated_at >= now);

    let scoped = store
        .grants
        .iter()
        .find(|grant| grant.client_id == "test-client")
        .unwrap();
    assert!(scoped.allowed_sessions.is_empty());
    assert!(scoped.revoked_at.is_some());
    assert!(!scoped.allows(McpScope::ReadLogs, Some("other-session"), Utc::now()));
    let mixed = store
        .grants
        .iter()
        .find(|grant| grant.client_id == "mixed")
        .unwrap();
    assert_eq!(mixed.allowed_sessions, ["other-session"]);
    assert!(mixed.revoked_at.is_none());
    let global = store
        .grants
        .iter()
        .find(|grant| grant.client_id == "readonly")
        .unwrap();
    assert!(global.allowed_sessions.is_empty());
    assert!(global.revoked_at.is_none());
}

#[test]
fn runtime_status_reason_records_disconnect_health() {
    let mut store = test_store();
    let summary = store
        .set_runtime_status_with_reason(
            "test-session",
            SessionStatus::Reconnecting,
            Some("network timeout".to_string()),
        )
        .unwrap();

    assert_eq!(summary.runtime.status, SessionStatus::Reconnecting);
    assert!(summary.runtime.last_disconnect.is_some());
    assert_eq!(
        summary.runtime.last_disconnect_reason.as_deref(),
        Some("network timeout")
    );
}

#[test]
fn runtime_disconnect_reason_is_normalized_and_unicode_bounded() {
    let mut store = test_store();
    let summary = store
        .set_runtime_status_with_reason(
            "test-session",
            SessionStatus::Error,
            Some(format!("  socket\n  closed  {}  ", "界".repeat(300))),
        )
        .unwrap();
    let reason = summary.runtime.last_disconnect_reason.unwrap();

    assert!(reason.starts_with("socket closed 界"));
    assert!(reason.ends_with("..."));
    assert!(!reason.contains('\n'));
    assert_eq!(
        reason.chars().count(),
        MAX_SESSION_DISCONNECT_REASON_CHARACTERS
    );

    let fallback = store
        .set_runtime_status_with_reason(
            "test-session",
            SessionStatus::Error,
            Some(" \n\t ".to_string()),
        )
        .unwrap();
    assert_eq!(
        fallback.runtime.last_disconnect_reason.as_deref(),
        Some("connection error")
    );

    let exact = "界".repeat(MAX_SESSION_DISCONNECT_REASON_CHARACTERS);
    assert_eq!(
        normalize_session_disconnect_reason(&exact).as_deref(),
        Some(exact.as_str())
    );
    let oversized = format!(
        "{}{}",
        " \n\t".repeat(100_000),
        "界".repeat(MAX_SESSION_DISCONNECT_REASON_CHARACTERS + 1)
    );
    let bounded = normalize_session_disconnect_reason(&oversized).unwrap();
    assert_eq!(
        bounded.chars().count(),
        MAX_SESSION_DISCONNECT_REASON_CHARACTERS
    );
    assert!(bounded.ends_with("..."));
}

#[test]
fn runtime_health_preserves_first_disconnect_time_during_one_outage() {
    let mut store = test_store();
    store
        .set_runtime_status("test-session", SessionStatus::Connected)
        .unwrap();
    let reconnecting = store
        .set_runtime_status_with_reason(
            "test-session",
            SessionStatus::Reconnecting,
            Some("network timeout".to_string()),
        )
        .unwrap();
    let disconnected_at = reconnecting.runtime.last_disconnect.unwrap();

    let retry_failed = store
        .set_runtime_status_with_reason(
            "test-session",
            SessionStatus::Reconnecting,
            Some("SSH reconnect failed: connection refused".to_string()),
        )
        .unwrap();
    assert_eq!(retry_failed.runtime.last_disconnect, Some(disconnected_at));
    assert_eq!(
        retry_failed.runtime.last_disconnect_reason.as_deref(),
        Some("SSH reconnect failed: connection refused")
    );

    let stopped = store
        .set_runtime_status_with_reason(
            "test-session",
            SessionStatus::Disconnected,
            Some("automatic reconnect disabled".to_string()),
        )
        .unwrap();
    assert_eq!(stopped.runtime.last_disconnect, Some(disconnected_at));
    assert_eq!(
        stopped.runtime.last_disconnect_reason.as_deref(),
        Some("automatic reconnect disabled")
    );

    let closed_again = store.close_session("test-session").unwrap();
    assert_eq!(closed_again.runtime.last_disconnect, Some(disconnected_at));
    assert_eq!(
        closed_again.runtime.last_disconnect_reason.as_deref(),
        Some("user closed session")
    );
}

#[test]
fn runtime_health_records_a_new_time_after_recovery() {
    let mut store = test_store();
    let old_disconnect = Utc::now() - chrono::Duration::hours(1);
    let runtime = store
        .runtimes
        .iter_mut()
        .find(|runtime| runtime.session_id == "test-session")
        .unwrap();
    runtime.status = SessionStatus::Connected;
    runtime.last_disconnect = Some(old_disconnect);
    runtime.last_disconnect_reason = Some("older outage".to_string());

    let reconnecting = store
        .set_runtime_status_with_reason(
            "test-session",
            SessionStatus::Reconnecting,
            Some("new transport loss".to_string()),
        )
        .unwrap();
    assert!(reconnecting.runtime.last_disconnect.unwrap() > old_disconnect);
    assert_eq!(
        reconnecting.runtime.last_disconnect_reason.as_deref(),
        Some("new transport loss")
    );
}

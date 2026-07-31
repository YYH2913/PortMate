use super::*;

#[test]
fn stream_events_are_bounded_per_session() {
    let mut store = test_store();
    for index in 0..(MAX_EVENTS_PER_SESSION + EVENT_TRIM_BATCH + 64) {
        store
            .record_stream_event(
                "test-session",
                EventDirection::Inbound,
                EventStream::Stdout,
                format!("line {index}"),
            )
            .unwrap();
    }

    let events = store.tail_log("test-session", usize::MAX);
    assert!(events.len() <= MAX_EVENTS_PER_SESSION + EVENT_TRIM_BATCH);
    assert_ne!(
        events.first().and_then(|event| event.text.as_deref()),
        Some("line 0")
    );
}

#[test]
fn command_history_is_deduplicated_unicode_bounded_and_revisioned() {
    let mut store = test_store();
    let now = 1_800_000_000_000_i64;
    let old = now - 31 * 24 * 60 * 60 * 1_000;
    let future = now + 60_000;
    let entries = vec![
        CommandHistoryEntry {
            command: "git status".to_string(),
            recorded_at: future,
        },
        CommandHistoryEntry {
            command: "npm test".to_string(),
            recorded_at: now - 1,
        },
        CommandHistoryEntry {
            command: "git status".to_string(),
            recorded_at: now - 2,
        },
        CommandHistoryEntry {
            command: "expired".to_string(),
            recorded_at: old,
        },
        CommandHistoryEntry {
            command: "bad\0command".to_string(),
            recorded_at: now,
        },
        CommandHistoryEntry {
            command: "界".repeat(MAX_COMMAND_HISTORY_COMMAND_CHARACTERS + 1),
            recorded_at: now,
        },
    ];

    let normalized = store
        .replace_command_history(&entries, 10, 30, now)
        .unwrap();
    assert_eq!(
        normalized,
        vec![
            CommandHistoryEntry {
                command: "git status".to_string(),
                recorded_at: now,
            },
            CommandHistoryEntry {
                command: "npm test".to_string(),
                recorded_at: now - 1,
            },
        ]
    );
    assert!(store.command_history_migrated);
    assert_eq!(store.command_history_revision, 1);

    store
        .replace_command_history(&normalized, 10, 30, now)
        .unwrap();
    assert_eq!(store.command_history_revision, 1);
    let recorded = store
        .record_command_history("npm test".to_string(), 10, 30, now + 1)
        .unwrap();
    assert_eq!(recorded[0].command, "npm test");
    assert_eq!(recorded.len(), 2);
    assert_eq!(store.command_history_revision, 2);
    assert!(store
        .record_command_history("bad\0command".to_string(), 10, 30, now + 2)
        .is_err());

    let merged = store
        .merge_command_history(
            &[CommandHistoryEntry {
                command: "local while disabled".to_string(),
                recorded_at: now + 2,
            }],
            10,
            30,
            now + 3,
        )
        .unwrap();
    assert_eq!(
        merged
            .iter()
            .map(|entry| entry.command.as_str())
            .collect::<Vec<_>>(),
        vec!["local while disabled", "npm test", "git status"]
    );

    store.command_history_revision = 9_007_199_254_740_991;
    let before = store.command_history.clone();
    assert!(store
        .record_command_history("must fail closed".to_string(), 10, 30, now + 4)
        .is_err());
    assert_eq!(store.command_history, before);
}

#[test]
fn command_history_rejects_invalid_policy_and_bounds_utf8_storage() {
    let now = 1_800_000_000_000_i64;
    assert!(SessionStore::normalized_command_history(&[], 0, 0, now).is_err());
    assert!(
        SessionStore::normalized_command_history(&[], MAX_COMMAND_HISTORY_ENTRIES + 1, 0, now)
            .is_err()
    );
    assert!(SessionStore::normalized_command_history(
        &[],
        1,
        MAX_COMMAND_HISTORY_RETENTION_DAYS + 1,
        now
    )
    .is_err());

    let command = "\"\\\n界".repeat(2_000);
    let entries = (0..200)
        .map(|index| CommandHistoryEntry {
            command: format!("{index}-{command}"),
            recorded_at: now - index,
        })
        .collect::<Vec<_>>();
    let normalized =
        SessionStore::normalized_command_history(&entries, MAX_COMMAND_HISTORY_ENTRIES, 0, now)
            .unwrap();
    let snapshot = serde_json::json!({ "version": 2, "entries": normalized });
    assert!(serde_json::to_vec(&snapshot).unwrap().len() <= MAX_COMMAND_HISTORY_STORAGE_BYTES);
}

#[test]
fn loaded_event_histories_are_normalized_and_rebuild_the_count_cache() {
    let mut store = test_store();
    let overflow = 37;
    store.events = (0..(MAX_EVENTS_PER_SESSION + overflow))
        .map(|index| SessionEvent {
            id: format!("loaded-{index}"),
            session_id: "test-session".to_string(),
            pane_id: "test-session:main".to_string(),
            ts: Utc::now(),
            direction: EventDirection::Inbound,
            stream: EventStream::Stdout,
            bytes_ref: None,
            text: Some(format!("loaded line {index}")),
            annotations: BTreeMap::new(),
        })
        .collect();
    store
        .event_counts
        .insert("test-session".to_string(), usize::MAX);

    store.normalize_bounded_histories();

    assert_eq!(store.events.len(), MAX_EVENTS_PER_SESSION);
    assert_eq!(
        store.events.first().and_then(|event| event.text.as_deref()),
        Some("loaded line 37")
    );
    assert_eq!(
        store.event_counts.get("test-session"),
        Some(&MAX_EVENTS_PER_SESSION)
    );

    store
        .record_stream_event(
            "test-session",
            EventDirection::Inbound,
            EventStream::Stdout,
            "next line",
        )
        .unwrap();
    assert_eq!(store.events.len(), MAX_EVENTS_PER_SESSION + 1);
    assert_eq!(
        store.event_counts.get("test-session"),
        Some(&(MAX_EVENTS_PER_SESSION + 1))
    );
}

#[test]
fn auxiliary_histories_are_bounded_per_scope_and_keep_active_transfers() {
    let mut store = test_store();
    let now = Utc::now();

    for index in 0..(MAX_AUDIT_RECORDS_PER_SCOPE + AUX_HISTORY_TRIM_BATCH + 2) {
        store.record_audit(AuditRecord {
            id: format!("audit-{index}"),
            ts: now + chrono::Duration::milliseconds(index as i64),
            actor: "test".to_string(),
            action: "send_text".to_string(),
            session_id: Some("test-session".to_string()),
            decision: "recorded".to_string(),
            details: BTreeMap::new(),
        });
    }
    store.record_audit(AuditRecord {
        id: "audit-global".to_string(),
        ts: now,
        actor: "test".to_string(),
        action: "global".to_string(),
        session_id: None,
        decision: "recorded".to_string(),
        details: BTreeMap::new(),
    });
    assert!(store.audit.len() <= MAX_AUDIT_RECORDS_PER_SCOPE + AUX_HISTORY_TRIM_BATCH + 1);
    assert!(!store.audit.iter().any(|record| record.id == "audit-0"));
    assert!(store.audit.iter().any(|record| record.id == "audit-global"));

    for index in 0..(MAX_TIMELINE_MARKS_PER_SESSION + AUX_HISTORY_TRIM_BATCH + 2) {
        store.record_timeline_mark(TimelineMark {
            id: format!("timeline-{index}"),
            session_id: "test-session".to_string(),
            ts: now + chrono::Duration::milliseconds(index as i64),
            label: "checkpoint".to_string(),
            details: None,
        });
    }
    assert!(store.timeline.len() <= MAX_TIMELINE_MARKS_PER_SESSION + AUX_HISTORY_TRIM_BATCH);
    assert!(!store.timeline.iter().any(|mark| mark.id == "timeline-0"));

    for index in 0..(MAX_SYSMON_SNAPSHOTS_PER_SESSION + AUX_HISTORY_TRIM_BATCH + 2) {
        store.record_sysmon_snapshot(SysmonSnapshot {
            session_id: "test-session".to_string(),
            ts: now + chrono::Duration::milliseconds(index as i64),
            uptime_seconds: index as u64,
            cpu_percent: 1.0,
            memory_percent: 2.0,
            rx_kbps: 3.0,
            tx_kbps: 4.0,
            load_average: [0.0; 3],
            memory_total_bytes: 0,
            memory_available_bytes: 0,
            processes: Vec::new(),
            disks: Vec::new(),
            network_interfaces: Vec::new(),
        });
    }
    assert!(store.sysmon.len() <= MAX_SYSMON_SNAPSHOTS_PER_SESSION + AUX_HISTORY_TRIM_BATCH);
    assert_ne!(store.sysmon[0].uptime_seconds, 0);
    let recent_sysmon = store.sysmon_history_for("test-session", 3);
    assert_eq!(recent_sysmon.len(), 3);
    assert!(recent_sysmon
        .windows(2)
        .all(|pair| pair[0].ts <= pair[1].ts));
    assert_eq!(
        recent_sysmon.last().map(|snapshot| snapshot.uptime_seconds),
        store
            .sysmon
            .iter()
            .rev()
            .find(|snapshot| snapshot.session_id == "test-session")
            .map(|snapshot| snapshot.uptime_seconds)
    );
    assert!(store.sysmon_history_for("test-session", 0).is_empty());

    for index in 0..(MAX_TERMINAL_TRANSFERS_PER_SESSION + 2) {
        store.record_transfer(test_transfer(
            format!("completed-{index}"),
            TransferStatus::Completed,
        ));
    }
    store.record_transfer(test_transfer("queued".to_string(), TransferStatus::Queued));
    store.record_transfer(test_transfer(
        "running".to_string(),
        TransferStatus::Running,
    ));
    assert_eq!(
        store
            .transfers
            .iter()
            .filter(|transfer| transfer.status == TransferStatus::Completed)
            .count(),
        MAX_TERMINAL_TRANSFERS_PER_SESSION
    );
    assert!(store.transfer_by_id("queued").is_some());
    assert!(store.transfer_by_id("running").is_some());
    assert!(store.transfer_by_id("completed-0").is_none());

    let queued = store
        .transfers
        .iter_mut()
        .find(|transfer| transfer.id == "queued")
        .unwrap();
    queued.status = TransferStatus::Completed;
    queued.finished_at = Some(now + chrono::Duration::days(1));
    store.trim_transfer_history("test-session");
    assert!(store.transfer_by_id("queued").is_some());
    assert_eq!(
        store
            .transfers
            .iter()
            .filter(|transfer| transfer.status == TransferStatus::Completed)
            .count(),
        MAX_TERMINAL_TRANSFERS_PER_SESSION
    );

    store.normalize_bounded_histories();
    assert_eq!(
        store
            .audit
            .iter()
            .filter(|record| record.session_id.as_deref() == Some("test-session"))
            .count(),
        MAX_AUDIT_RECORDS_PER_SCOPE
    );
    assert_eq!(store.timeline.len(), MAX_TIMELINE_MARKS_PER_SESSION);
    assert_eq!(store.sysmon.len(), MAX_SYSMON_SNAPSHOTS_PER_SESSION);
}

#[test]
fn search_logs_limits_to_recent_matches_in_chronological_order() {
    let mut store = test_store();
    for text in ["match old", "unrelated", "match middle", "match newest"] {
        store
            .record_stream_event(
                "test-session",
                EventDirection::Inbound,
                EventStream::Stdout,
                text,
            )
            .unwrap();
    }

    let latest = store.search_logs("MATCH", Some("test-session"), 1);
    assert_eq!(latest.len(), 1);
    assert_eq!(latest[0].text.as_deref(), Some("match newest"));

    let latest_two = store.search_logs("match", Some("test-session"), 2);
    assert_eq!(
        latest_two
            .iter()
            .filter_map(|event| event.text.as_deref())
            .collect::<Vec<_>>(),
        vec!["match middle", "match newest"]
    );
    assert!(store
        .search_logs("match", Some("other-session"), 10)
        .is_empty());
}

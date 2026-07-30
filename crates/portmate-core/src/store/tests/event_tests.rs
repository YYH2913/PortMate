use super::*;

#[test]
fn send_text_redacts_and_audits() {
    let mut store = test_store();
    let event = store
        .send_text("test", "test-session", "password=hunter2\n")
        .unwrap();
    assert!(!event.text.unwrap().contains("hunter2"));
    assert_eq!(store.audit.last().unwrap().action, "send_text");
}

#[test]
fn send_text_preserves_raw_bytes_reference_while_redacting_text() {
    let mut store = test_store();
    let event = store
        .send_text_with_bytes_ref(
            "test",
            "test-session",
            "password=hunter2\n",
            Some("v2:session.raw:0:3:digest".to_string()),
        )
        .unwrap();
    assert_eq!(
        event.bytes_ref.as_deref(),
        Some("v2:session.raw:0:3:digest")
    );
    assert!(!event.text.unwrap().contains("hunter2"));
}

#[test]
fn send_text_updates_runtime_activity_with_event_timestamp() {
    let mut store = test_store();
    let previous = Utc::now() - chrono::Duration::minutes(5);
    store.runtimes[0].last_activity = previous;

    let event = store
        .send_text("test", "test-session", "show version\n")
        .unwrap();
    let runtime = store
        .runtimes
        .iter()
        .find(|runtime| runtime.session_id == "test-session")
        .unwrap();

    assert!(runtime.last_activity > previous);
    assert_eq!(runtime.last_activity, event.ts);
    assert_eq!(store.audit.last().unwrap().ts, event.ts);
}

#[test]
fn binary_control_events_allow_bytes_without_fake_text() {
    let mut store = test_store();
    let event = store
        .record_event(
            "test-session",
            EventDirection::Outbound,
            EventStream::Control,
            None,
            Some("v2:session.raw:0:3:digest".to_string()),
            BTreeMap::from([("origin".to_string(), "telnet-negotiation".to_string())]),
        )
        .unwrap();
    assert!(event.text.is_none());
    assert!(event.bytes_ref.is_some());
    assert_eq!(
        event.annotations.get("origin").map(String::as_str),
        Some("telnet-negotiation")
    );
}

#[test]
fn deferred_profile_delete_keeps_shared_outbox_until_commit() {
    let mut store = test_store();
    store.runtimes[0].status = SessionStatus::Disconnected;
    let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
    store.set_system_event_notifier(sender).unwrap();
    store.record_system_event("test-session", "queued before transaction");

    let mut next_store = store.clone();
    next_store
        .delete_profile_deferred_system_event_cleanup("test-session")
        .unwrap();

    let queued_after_rollback = store.drain_system_event_outbox();
    assert_eq!(queued_after_rollback.len(), 1);
    assert_eq!(
        queued_after_rollback[0].0.text.as_deref(),
        Some("queued before transaction")
    );

    store.record_system_event("test-session", "queued before commit");
    next_store.discard_system_events_for_session("test-session");
    assert!(store.drain_system_event_outbox().is_empty());
}

#[test]
fn system_events_cannot_recreate_deleted_profile_history() {
    let mut store = test_store();
    store.runtimes[0].status = SessionStatus::Disconnected;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    store.set_system_event_notifier(sender).unwrap();
    store.delete_profile("test-session").unwrap();

    store.record_system_event("test-session", "late worker diagnostic");

    assert!(store.events.is_empty());
    assert!(store.drain_system_event_outbox().is_empty());
    assert!(receiver.try_recv().is_err());
}

#[test]
fn system_event_notifier_coalesces_direct_and_lifecycle_events() {
    let mut store = test_store();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    store.set_system_event_notifier(sender).unwrap();

    store.record_system_event("test-session", "PortMate: direct diagnostic");
    store.open_session("test-session").unwrap();
    store.close_session("test-session").unwrap();

    receiver
        .recv_timeout(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(receiver.try_recv().is_err());
    let queued = store.drain_system_event_outbox();
    assert_eq!(queued.len(), 3);
    assert!(queued.iter().all(|(event, profile)| {
        event.direction == EventDirection::System
            && event.stream == EventStream::Control
            && profile
                .as_ref()
                .is_some_and(|profile| profile.id == "test-session")
    }));
    let events = store.events.iter().rev().take(3).collect::<Vec<_>>();
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|event| {
        event.direction == EventDirection::System
            && event.stream == EventStream::Control
            && event.bytes_ref.is_none()
    }));
    assert!(events[2]
        .text
        .as_deref()
        .is_some_and(|text| text.contains("direct diagnostic")));
    assert!(events[1]
        .text
        .as_deref()
        .is_some_and(|text| text.contains("connected")));
    assert!(events[0]
        .text
        .as_deref()
        .is_some_and(|text| text.contains("disconnected")));
}

#[test]
fn tracked_system_event_can_be_removed_from_the_outbox_exactly() {
    let mut store = test_store();
    let (sender, _receiver) = std::sync::mpsc::sync_channel(1);
    store.set_system_event_notifier(sender).unwrap();

    let rolled_back = store
        .record_system_event_tracked("test-session", "rolled back event")
        .unwrap();
    let retained = store
        .record_system_event_tracked("test-session", "retained event")
        .unwrap();
    store.discard_queued_system_event(&rolled_back);

    let queued = store.drain_system_event_outbox();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].0.id, retained);
    assert!(store.events.iter().any(|event| event.id == rolled_back));
    assert!(store.events.iter().any(|event| event.id == retained));
}

#[test]
fn system_event_outbox_is_bounded_and_reports_overflow() {
    let mut store = test_store();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    store.set_system_event_notifier(sender).unwrap();

    for index in 0..=MAX_SYSTEM_EVENT_OUTBOX {
        store.record_system_event("test-session", format!("diagnostic {index}"));
    }

    assert_eq!(
        store.drain_system_event_outbox().len(),
        MAX_SYSTEM_EVENT_OUTBOX
    );
    assert!(store.events.last().is_some_and(|event| {
        event
            .annotations
            .get("loggingError")
            .is_some_and(|error| error.contains("backlog exceeded"))
    }));

    drop(receiver);
    store.record_system_event("test-session", "disconnected worker one");
    store.record_system_event("test-session", "disconnected worker two");
    assert!(store.events.iter().rev().take(2).all(|event| {
        event
            .annotations
            .get("loggingError")
            .is_some_and(|error| error.contains("worker disconnected"))
    }));
}

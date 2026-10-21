use super::*;

#[test]
fn update_flush_retains_unfinished_logs_for_retry_and_isolates_stores() {
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.sqlite3");
    let second = root.path().join("second.sqlite3");
    let pending = Arc::new(InboundLogQueueState::new());
    let other = Arc::new(InboundLogQueueState::new());
    INBOUND_LOG_DRAINS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .extend([
            (first.clone(), Arc::clone(&pending)),
            (second.clone(), Arc::clone(&other)),
        ]);
    assert!(flush_inbound_log_queues(&first, Duration::ZERO).is_err());
    assert!(
        INBOUND_LOG_ACCEPTING.load(Ordering::SeqCst),
        "failed installation must allow logging to continue"
    );
    pending.finish();
    assert!(flush_inbound_log_queues(&first, Duration::from_millis(10)).is_ok());
    assert!(
        flush_inbound_log_queues(&second, Duration::ZERO).is_err(),
        "another Store's pending logs must remain tracked"
    );
    other.finish();
    assert!(flush_inbound_log_queues(&second, Duration::from_millis(10)).is_ok());
}

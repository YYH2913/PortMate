#[test]
fn append_log_bytes_returns_stable_byte_refs() {
    let root = std::env::temp_dir().join(format!("portmate-log-test-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let mut profile = test_shell_profile();
    profile.logging.path_template = "../bad/{profile}/{date}/{session}.jsonl".to_string();

    let first = append_log_bytes(&store_path, &profile, "raw", b"abc").unwrap();
    let second = append_log_bytes(&store_path, &profile, "raw", b"de").unwrap();
    let raw_path = log_shard_path(&store_path, &profile, "raw").unwrap();
    let raw = fs::read(&raw_path).unwrap();

    assert_eq!(raw, b"abcde");
    let first_ref = parse_log_bytes_ref(&first).unwrap();
    let second_ref = parse_log_bytes_ref(&second).unwrap();
    assert_eq!((first_ref.offset, first_ref.length), (0, 3));
    assert_eq!((second_ref.offset, second_ref.length), (3, 2));
    assert!(first_ref.sha256.is_some());
    assert!(second_ref.sha256.is_some());
    assert!(raw_path.starts_with(log_root(&store_path)));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn append_log_bytes_rejects_symlink_targets() {
    let root = std::env::temp_dir().join(format!("portmate-log-symlink-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let profile = test_shell_profile();
    let raw_path = log_shard_path(&store_path, &profile, "raw").unwrap();
    fs::create_dir_all(raw_path.parent().unwrap()).unwrap();
    let protected = root.join("protected.raw");
    fs::write(&protected, b"protected").unwrap();
    std::os::unix::fs::symlink(&protected, &raw_path).unwrap();

    let error = append_log_bytes(&store_path, &profile, "raw", b"should not write").unwrap_err();
    assert!(error.contains("symbolic link"), "unexpected error: {error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn text_log_events_include_microsecond_metadata_on_every_line() {
    let event = SessionEvent {
        id: "event-1".to_string(),
        session_id: "session:1".to_string(),
        pane_id: "session:1:main".to_string(),
        ts: DateTime::parse_from_rfc3339("2026-07-15T12:34:56.123456789Z")
            .unwrap()
            .with_timezone(&Utc),
        direction: EventDirection::Inbound,
        stream: EventStream::Stderr,
        bytes_ref: None,
        text: Some("first\nsecond".to_string()),
        annotations: BTreeMap::from([("commandId".to_string(), "command-1".to_string())]),
    };
    let prefix = "[2026-07-15T12:34:56.123456Z] [inbound/stderr] [session=session:1] \
                  [pane=session:1:main] [command=command-1] ";

    assert_eq!(
        format_text_log_event(&event, event.text.as_deref().unwrap()),
        format!("{prefix}first\n{prefix}second\n")
    );
    assert_eq!(text_log_field("field]\\\n\t"), "field\\]\\\\\\n\\t");
}

#[test]
fn inbound_event_refs_exact_transport_bytes_before_text_decoding() {
    let root = std::env::temp_dir().join(format!("portmate-log-inbound-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let mut profile = test_shell_profile();
    profile.logging.enabled = true;
    profile.logging.raw = true;
    profile.logging.text = false;
    profile.logging.jsonl = false;
    let state = test_app_state(profile.clone(), store_path.clone());
    let wire = [b'A', 0xff, 0x00, 0x80, b'B'];

    record_channel_bytes(
        &state.session_io(),
        &profile.id,
        None,
        EventStream::Stdout,
        &wire,
        String::from_utf8_lossy(&wire).to_string(),
    );

    let event = state.store.lock().unwrap().events.last().cloned().unwrap();
    assert_ne!(event.text.as_deref().unwrap().as_bytes(), wire);
    let reference = event.bytes_ref.as_deref().unwrap();
    assert_eq!(read_log_bytes_ref(&store_path, reference).unwrap().2, wire);

    let _ = fs::remove_dir_all(root);
}

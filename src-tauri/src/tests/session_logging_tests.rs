use super::*;

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
fn text_log_events_include_millisecond_metadata_on_every_line() {
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
    let prefix = "[2026-07-15T12:34:56.123Z] [inbound/stderr] [session=session:1] \
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

#[test]
fn explicit_commands_associate_events_until_the_next_user_input() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 19];
            socket.read_exact(&mut received).await.unwrap();
            received
        });

        let mut profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        profile.logging.enabled = true;
        profile.logging.jsonl = true;
        let root = std::env::temp_dir().join(format!("portmate-command-log-{}", Uuid::new_v4()));
        let store_path = root.join("portmate-store.sqlite3");
        let state = test_app_state(profile.clone(), store_path.clone());
        let stream = TcpStream::connect(address).await.unwrap();
        let (_reader, writer) = stream.into_split();
        let (tap, _) = broadcast::channel(8);
        state.tcp.lock().unwrap().insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: Uuid::new_v4().to_string(),
                writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
                tap,
                closed: Arc::new(AtomicBool::new(false)),
                telnet: None,
            },
        );
        let io = state.session_io();

        let first = run_command_inner_with_context(
            io.clone(),
            profile.id.clone(),
            "first\n".to_string(),
            "desktop-user",
            Some("run_command"),
        )
        .await
        .unwrap();
        let first_command_id = first.annotations.get("commandId").cloned().unwrap();
        assert_eq!(
            first.annotations.get("commandState").map(String::as_str),
            Some("started")
        );
        record_channel_bytes(
            &io,
            &profile.id,
            None,
            EventStream::Stdout,
            b"first output\n",
            "first output\n".to_string(),
        );
        let first_output = state.store.lock().unwrap().events.last().cloned().unwrap();
        assert_eq!(
            first_output.annotations.get("commandId"),
            Some(&first_command_id)
        );
        assert!(!first_output.annotations.contains_key("commandState"));

        let second = run_command_inner_with_context(
            io.clone(),
            profile.id.clone(),
            "second\n".to_string(),
            "desktop-user",
            Some("run_command"),
        )
        .await
        .unwrap();
        let second_command_id = second.annotations.get("commandId").cloned().unwrap();
        assert_ne!(second_command_id, first_command_id);
        record_channel_bytes(
            &io,
            &profile.id,
            None,
            EventStream::Stderr,
            b"second output\n",
            "second output\n".to_string(),
        );
        let second_output = state.store.lock().unwrap().events.last().cloned().unwrap();
        assert_eq!(
            second_output.annotations.get("commandId"),
            Some(&second_command_id)
        );

        let manual = send_text_inner(io.clone(), profile.id.clone(), "manual".to_string())
            .await
            .unwrap();
        assert!(!manual.annotations.contains_key("commandId"));
        assert!(active_command_id(&io, &profile.id).is_none());
        record_channel_bytes(
            &io,
            &profile.id,
            None,
            EventStream::Stdout,
            b"manual output\n",
            "manual output\n".to_string(),
        );
        let manual_output = state.store.lock().unwrap().events.last().cloned().unwrap();
        assert!(!manual_output.annotations.contains_key("commandId"));

        persist_store_arc(&store_path, &state.store).unwrap();
        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("command association TCP server timed out")
            .expect("command association TCP server failed");
        assert_eq!(&received, b"first\nsecond\nmanual");

        let persisted = load_store_sqlite(&store_path).unwrap();
        let persisted_first = persisted
            .events
            .iter()
            .find(|event| event.id == first.id)
            .unwrap();
        let persisted_output = persisted
            .events
            .iter()
            .find(|event| event.id == second_output.id)
            .unwrap();
        assert_eq!(
            persisted_first.annotations.get("commandId"),
            Some(&first_command_id)
        );
        assert_eq!(
            persisted_output.annotations.get("commandId"),
            Some(&second_command_id)
        );

        let jsonl_path = log_shard_path(&store_path, &profile, "jsonl").unwrap();
        let jsonl_events = fs::read_to_string(jsonl_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<SessionEvent>(line).unwrap())
            .collect::<Vec<_>>();
        assert!(jsonl_events.iter().any(|event| {
            event.id == first.id && event.annotations.get("commandId") == Some(&first_command_id)
        }));
        assert!(jsonl_events.iter().any(|event| {
            event.id == second_output.id
                && event.annotations.get("commandId") == Some(&second_command_id)
        }));

        let text_path = log_shard_path(&store_path, &profile, "txt").unwrap();
        let text_log = fs::read_to_string(text_path).unwrap();
        assert!(text_log.contains(&format!(
            "[outbound/stdout] [session={}] [pane={}:main] [command={}] first",
            profile.id, profile.id, first_command_id
        )));
        assert!(text_log.contains(&format!(
            "[inbound/stderr] [session={}] [pane={}:main] [command={}] second output",
            profile.id, profile.id, second_command_id
        )));
        assert!(text_log.contains(&format!(
            "[inbound/stdout] [session={}] [pane={}:main] [command=-] manual output",
            profile.id, profile.id
        )));

        let bundle = state
            .store
            .lock()
            .unwrap()
            .export_session_bundle_redacted(&profile.id);
        let bundle_events =
            serde_json::from_value::<Vec<SessionEvent>>(bundle.get("events").cloned().unwrap())
                .unwrap();
        assert!(bundle_events.iter().any(|event| {
            event.id == second_output.id
                && event.annotations.get("commandId") == Some(&second_command_id)
        }));

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn failed_explicit_command_write_does_not_leave_an_active_command() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!("portmate-command-failed-{}", Uuid::new_v4()));
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let io = state.session_io();

        let error = run_command_inner_with_context(
            io.clone(),
            profile.id.clone(),
            "status\n".to_string(),
            "desktop-user",
            Some("run_command"),
        )
        .await
        .unwrap_err();

        assert!(error.contains("尚未连接"), "unexpected error: {error}");
        assert!(active_command_id(&io, &profile.id).is_none());
        assert!(state.store.lock().unwrap().events.is_empty());
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn system_event_sink_writes_redacted_text_and_jsonl_once_without_raw() {
    let root = std::env::temp_dir().join(format!("portmate-system-sink-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let mut profile = test_shell_profile();
    profile.logging.enabled = true;
    profile.logging.raw = true;
    profile.logging.text = true;
    profile.logging.jsonl = true;
    profile.logging.redact_secrets = true;
    let state = test_app_state(profile.clone(), store_path.clone());
    let raw_path = log_shard_path(&store_path, &profile, "raw").unwrap();
    append_log_bytes(&store_path, &profile, "raw", b"raw-sentinel").unwrap();
    install_system_event_sink(&state).unwrap();

    let event_ids = {
        let mut store = state.store.lock().unwrap();
        store.record_system_event(&profile.id, "PortMate: password=hunter2");
        store.open_session(&profile.id).unwrap();
        store.close_session(&profile.id).unwrap();
        let event_ids = store
            .events
            .iter()
            .rev()
            .take(3)
            .map(|event| event.id.clone())
            .collect::<HashSet<_>>();
        save_store(&store_path, &store).unwrap();
        save_store(&store_path, &store).unwrap();
        event_ids
    };

    shutdown_system_event_sink(&state);
    let jsonl_path = log_shard_path(&store_path, &profile, "jsonl").unwrap();
    let jsonl = fs::read_to_string(&jsonl_path).unwrap();

    let persisted = jsonl
        .lines()
        .map(|line| serde_json::from_str::<SessionEvent>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(persisted.len(), 3);
    assert_eq!(
        persisted
            .iter()
            .map(|event| event.id.clone())
            .collect::<HashSet<_>>(),
        event_ids
    );
    assert!(persisted.iter().all(|event| {
        event.direction == EventDirection::System
            && event.stream == EventStream::Control
            && event.bytes_ref.is_none()
            && !event
                .text
                .as_deref()
                .unwrap_or_default()
                .contains("hunter2")
    }));
    let text_path = log_shard_path(&store_path, &profile, "txt").unwrap();
    let text = fs::read_to_string(text_path).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert!(!text.contains("hunter2"));
    assert_eq!(fs::read(raw_path).unwrap(), b"raw-sentinel");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn trigger_system_jsonl_follows_the_inbound_event_that_caused_it() {
    let root = std::env::temp_dir().join(format!("portmate-trigger-sink-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let mut profile = test_shell_profile();
    profile.logging.enabled = true;
    profile.logging.raw = false;
    profile.logging.text = false;
    profile.logging.jsonl = true;
    profile.triggers = vec![portmate_core::TriggerSpec {
        id: "cause-trigger".to_string(),
        label: "Cause".to_string(),
        matcher: portmate_core::TriggerMatcher::Contains {
            text: "CAUSE".to_string(),
            case_sensitive: true,
        },
        actions: vec![TriggerAction::TimelineMark {
            label: "cause-mark".to_string(),
        }],
        enabled: true,
    }];
    let state = test_app_state(profile.clone(), store_path.clone());
    install_system_event_sink(&state).unwrap();

    record_channel_bytes(
        &state.session_io(),
        &profile.id,
        None,
        EventStream::Stdout,
        b"CAUSE\n",
        "CAUSE\n".to_string(),
    );
    shutdown_system_event_sink(&state);

    let jsonl_path = log_shard_path(&store_path, &profile, "jsonl").unwrap();
    let events = fs::read_to_string(jsonl_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<SessionEvent>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].direction, EventDirection::Inbound);
    assert_eq!(events[0].text.as_deref(), Some("CAUSE\n"));
    assert_eq!(events[1].direction, EventDirection::System);
    assert!(events[1]
        .text
        .as_deref()
        .is_some_and(|text| text.contains("trigger matched (Cause)")));
    assert!(events.iter().all(|event| event.bytes_ref.is_none()));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn outbound_persistence_failure_returns_a_success_event_without_payload_loss() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 7];
            socket.read_exact(&mut received).await.unwrap();
            assert_eq!(&received, b"status\n");
        });

        let root = std::env::temp_dir().join(format!("portmate-outbound-save-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let blocked_parent = root.join("not-a-directory");
        fs::write(&blocked_parent, b"block sqlite parent creation").unwrap();
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let state = test_app_state(profile.clone(), blocked_parent.join("store.sqlite3"));
        let stream = TcpStream::connect(address).await.unwrap();
        let (_reader, writer) = stream.into_split();
        let (tap, _) = broadcast::channel(8);
        state.tcp.lock().unwrap().insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: Uuid::new_v4().to_string(),
                writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
                tap,
                closed: Arc::new(AtomicBool::new(false)),
                telnet: None,
            },
        );

        let event = send_text_inner(
            state.session_io(),
            profile.id.clone(),
            "status\n".to_string(),
        )
        .await
        .expect("a logging failure must not turn a successful transport write into an error");
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("TCP server timed out")
            .expect("TCP server failed");

        assert_eq!(event.direction, EventDirection::Outbound);
        assert_eq!(event.text.as_deref(), Some("status\n"));
        assert!(event.annotations.contains_key("loggingError"));
        let stored = state
            .store
            .lock()
            .unwrap()
            .events
            .iter()
            .find(|stored| stored.id == event.id)
            .cloned()
            .unwrap();
        assert_eq!(stored.annotations, event.annotations);

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn outbound_shard_failures_are_reported_without_reversible_binary_text() {
    let root = std::env::temp_dir().join(format!("portmate-outbound-shard-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("logs"), b"block shard directory creation").unwrap();
    let mut profile = test_shell_profile();
    profile.logging.enabled = true;
    profile.logging.raw = true;
    profile.logging.text = true;
    profile.logging.jsonl = true;
    let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
    let payload = b"password=hunter2";
    let summary = format_outbound_byte_summary(payload);

    let event = record_outbound_user_event_with_context(
        &state.session_io(),
        &profile.id,
        &summary,
        payload,
        "desktop-user",
        Some("send_bytes"),
        BTreeMap::new(),
    );

    assert_eq!(event.text.as_deref(), Some("Binary payload: 16 bytes"));
    assert!(!event.text.as_deref().unwrap().contains("hunter2"));
    assert!(!event.text.as_deref().unwrap().contains("70 61 73"));
    let logging_error = event.annotations.get("loggingError").unwrap();
    assert!(logging_error.contains("raw shard append failed"));
    assert!(logging_error.contains("text shard append failed"));
    assert!(logging_error.contains("JSONL shard append failed"));
    let stored = state
        .store
        .lock()
        .unwrap()
        .events
        .iter()
        .find(|stored| stored.id == event.id)
        .cloned()
        .unwrap();
    assert_eq!(stored.annotations, event.annotations);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn text_log_failure_is_persisted_in_the_following_jsonl_event() {
    let root = std::env::temp_dir().join(format!("portmate-text-shard-failure-{}", Uuid::new_v4()));
    let store_path = root.join("store.sqlite3");
    let mut profile = test_shell_profile();
    profile.logging.enabled = true;
    profile.logging.raw = false;
    profile.logging.text = true;
    profile.logging.jsonl = true;
    let state = test_app_state(profile.clone(), store_path.clone());
    let text_path = log_shard_path(&store_path, &profile, "txt").unwrap();
    fs::create_dir_all(&text_path).unwrap();

    let event = record_outbound_user_event_with_context(
        &state.session_io(),
        &profile.id,
        "status\n",
        b"status\n",
        "desktop-user",
        Some("send_text"),
        BTreeMap::new(),
    );

    assert!(event
        .annotations
        .get("loggingError")
        .is_some_and(|error| error.contains("text shard append failed")));
    let jsonl_path = log_shard_path(&store_path, &profile, "jsonl").unwrap();
    let jsonl_event =
        serde_json::from_str::<SessionEvent>(fs::read_to_string(jsonl_path).unwrap().trim_end())
            .unwrap();
    assert_eq!(jsonl_event.annotations, event.annotations);
    let stored = state
        .store
        .lock()
        .unwrap()
        .events
        .iter()
        .find(|stored| stored.id == event.id)
        .cloned()
        .unwrap();
    assert_eq!(stored.annotations, event.annotations);
    let persisted = load_store_sqlite(&store_path).unwrap();
    assert_eq!(
        persisted
            .events
            .iter()
            .find(|stored| stored.id == event.id)
            .unwrap()
            .annotations,
        event.annotations
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn bytes_ref_detects_recreated_shards_and_reads_legacy_refs() {
    let root = std::env::temp_dir().join(format!("portmate-log-ref-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let profile = test_shell_profile();
    let old_ref = append_log_bytes(&store_path, &profile, "raw", b"AAAA").unwrap();
    let parsed = parse_log_bytes_ref(&old_ref).unwrap();
    let legacy_ref = format!("{}:0:4", parsed.relative);
    assert_eq!(
        read_log_bytes_ref(&store_path, &legacy_ref).unwrap().2,
        b"AAAA"
    );
    let ambiguous_path = log_root(&store_path).join("v2:legacy.raw");
    fs::write(&ambiguous_path, b"CCCC").unwrap();
    assert_eq!(
        read_log_bytes_ref(&store_path, "v2:legacy.raw:0:4")
            .unwrap()
            .2,
        b"CCCC"
    );

    delete_log_shards_inner(&store_path, std::slice::from_ref(&parsed.relative)).unwrap();
    let new_ref = append_log_bytes(&store_path, &profile, "raw", b"BBBB").unwrap();
    let error = read_log_bytes_ref(&store_path, &old_ref).unwrap_err();
    assert!(
        error.contains("content mismatch"),
        "unexpected error: {error}"
    );
    assert_eq!(
        read_log_bytes_ref(&store_path, &new_ref).unwrap().2,
        b"BBBB"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_log_bytes_serializes_concurrent_writers() {
    let root = std::env::temp_dir().join(format!("portmate-log-race-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let mut profile = test_shell_profile();
    profile.logging.path_template = "shared/{date}/transport.jsonl".to_string();
    let barrier = Arc::new(std::sync::Barrier::new(48));
    let results = Arc::new(Mutex::new(Vec::new()));

    std::thread::scope(|scope| {
        for index in 0_u8..48 {
            let barrier = Arc::clone(&barrier);
            let results = Arc::clone(&results);
            let store_path = store_path.clone();
            let profile = profile.clone();
            scope.spawn(move || {
                let payload = vec![index, 0xff, 0x00, 0x80, index.wrapping_add(1)];
                barrier.wait();
                let reference = append_log_bytes(&store_path, &profile, "raw", &payload).unwrap();
                results.lock().unwrap().push((payload, reference));
            });
        }
    });

    let results = results.lock().unwrap();
    assert_eq!(results.len(), 48);
    let mut ranges = Vec::new();
    for (expected, reference) in results.iter() {
        let (_, offset, actual) = read_log_bytes_ref(&store_path, reference).unwrap();
        assert_eq!(&actual, expected);
        ranges.push((offset, offset + actual.len() as u64));
    }
    ranges.sort_unstable();
    let mut expected_offset = 0_u64;
    for (start, end) in ranges {
        assert_eq!(start, expected_offset);
        expected_offset = end;
    }
    assert_eq!(expected_offset, 48 * 5);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_retention_prunes_only_expired_profile_shards() {
    let root = std::env::temp_dir().join(format!("portmate-log-retention-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let now = SystemTime::now();
    let mut profile = test_shell_profile();
    profile.logging.retention_days = 30;
    let empty = prune_expired_log_shards_for_profile(&store_path, &profile, now).unwrap();
    assert_eq!(empty.deleted, 0);
    let old_path =
        log_root(&store_path).join(log_shard_relative_path(&profile, "2026-05-01", "raw"));
    let fresh_path =
        log_root(&store_path).join(log_shard_relative_path(&profile, "2026-07-01", "txt"));
    let mut other = profile.clone();
    other.id = "session:2".to_string();
    other.name = "Other Device".to_string();
    let other_path =
        log_root(&store_path).join(log_shard_relative_path(&other, "2026-05-01", "jsonl"));
    for path in [&old_path, &fresh_path, &other_path] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"data").unwrap();
    }
    fs::File::options()
        .write(true)
        .open(&old_path)
        .unwrap()
        .set_modified(now - Duration::from_secs(31 * 86_400))
        .unwrap();
    fs::File::options()
        .write(true)
        .open(&fresh_path)
        .unwrap()
        .set_modified(now - Duration::from_secs(29 * 86_400))
        .unwrap();
    fs::File::options()
        .write(true)
        .open(&other_path)
        .unwrap()
        .set_modified(now - Duration::from_secs(60 * 86_400))
        .unwrap();

    let result = prune_expired_log_shards_for_profile(&store_path, &profile, now).unwrap();
    assert_eq!(result.deleted, 1);
    assert_eq!(result.bytes_deleted, 4);
    assert!(!old_path.exists());
    assert!(fresh_path.exists());
    assert!(other_path.exists());

    profile.logging.path_template = "{date}/shared.jsonl".to_string();
    assert!(validate_logging_retention(&profile).is_err());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_retention_check_registry_replaces_changes_and_reclaims_entries() {
    let root =
        std::env::temp_dir().join(format!("portmate-log-retention-cache-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let key = (store_path.clone(), "retention-session".to_string());
    let stale_key = (root.join("stale.sqlite3"), "stale-session".to_string());
    let mut profile = test_shell_profile();
    profile.id = key.1.clone();
    profile.logging.retention_days = 30;
    let checks = LOG_RETENTION_CHECKS.get_or_init(|| Mutex::new(HashMap::new()));
    checks.lock().unwrap().insert(
        stale_key.clone(),
        (
            7,
            Instant::now() - LOG_RETENTION_CHECK_INTERVAL - Duration::from_secs(1),
        ),
    );

    maybe_prune_expired_log_shards(&store_path, &profile).unwrap();
    {
        let checks = checks.lock().unwrap();
        assert_eq!(checks.get(&key).map(|(days, _)| *days), Some(30));
        assert!(!checks.contains_key(&stale_key));
    }

    profile.logging.retention_days = 31;
    maybe_prune_expired_log_shards(&store_path, &profile).unwrap();
    assert_eq!(
        checks.lock().unwrap().get(&key).map(|(days, _)| *days),
        Some(31)
    );

    profile.logging.retention_days = 0;
    maybe_prune_expired_log_shards(&store_path, &profile).unwrap();
    assert!(!checks.lock().unwrap().contains_key(&key));

    profile.logging.retention_days = 30;
    maybe_prune_expired_log_shards(&store_path, &profile).unwrap();
    clear_log_retention_check(&store_path, &profile.id);
    assert!(!checks.lock().unwrap().contains_key(&key));
    let _ = fs::remove_dir_all(root);
}

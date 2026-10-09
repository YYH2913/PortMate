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

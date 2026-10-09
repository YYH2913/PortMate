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

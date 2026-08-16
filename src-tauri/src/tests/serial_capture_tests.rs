#[test]
fn serial_capture_buffer_bounds_and_incremental_snapshots() {
    let mut capture = SerialCaptureBuffer::default();
    capture.push(EventDirection::Inbound, &[0xff, 0x00, 0x80]);
    let first = capture.snapshot_since(None);
    assert!(first.reset);
    assert_eq!(first.total_frames, 1);
    assert_eq!(first.captured_bytes, 3);
    assert_eq!(first.frames[0].bytes, [0xff, 0x00, 0x80]);
    let first_id = first.frames[0].id.clone();

    capture.push(EventDirection::Outbound, b"hello");
    let incremental = capture.snapshot_since(Some(&first_id));
    assert!(!incremental.reset);
    assert_eq!(incremental.total_frames, 2);
    assert_eq!(incremental.frames.len(), 1);
    assert_eq!(incremental.frames[0].bytes, b"hello");
    let reset = capture.snapshot_since(Some("evicted-frame"));
    assert!(reset.reset);
    assert_eq!(reset.frames.len(), 2);

    capture.clear();
    let oversized = vec![0x5a; MAX_SERIAL_CAPTURE_FRAME_BYTES + 3];
    capture.push(EventDirection::Inbound, &oversized);
    let frame = capture.frames.front().unwrap();
    assert!(frame.truncated);
    assert_eq!(frame.original_length, oversized.len());
    assert_eq!(frame.bytes.len(), MAX_SERIAL_CAPTURE_FRAME_BYTES);

    capture.clear();
    for value in 0..=MAX_SERIAL_CAPTURE_FRAMES {
        capture.push(EventDirection::Inbound, &[value as u8]);
    }
    assert_eq!(capture.frames.len(), MAX_SERIAL_CAPTURE_FRAMES);
    assert_eq!(capture.captured_bytes, MAX_SERIAL_CAPTURE_FRAMES);
}

#[test]
fn serial_capture_and_event_reject_stale_runtime_input_together() {
    let root = std::env::temp_dir().join(format!(
        "portmate-serial-capture-runtime-{}",
        Uuid::new_v4()
    ));
    let profile = test_serial_profile(portmate_core::SerialConnection {
        port: "/dev/ttyUSB0".to_string(),
        baud_rate: 115_200,
        data_bits: 8,
        stop_bits: 1,
        parity: "none".to_string(),
        flow_control: "none".to_string(),
        dtr: false,
        rts: false,
        reconnect: false,
        reconnect_delay_ms: 1_000,
        receive_idle_timeout_enabled: false,
        receive_idle_timeout_seconds: 60,
    });
    let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
    let capture = serial_capture_for_session(&state.serial_captures, &profile.id).unwrap();
    let (tap, _) = broadcast::channel(8);
    state.serial.lock().unwrap().insert(
        profile.id.clone(),
        SerialRuntime {
            runtime_id: "serial-current".to_string(),
            writer: None,
            tap,
            closed: Arc::new(AtomicBool::new(false)),
            capture: Arc::clone(&capture),
        },
    );
    let io = state.session_io();

    record_channel_bytes_with_accepted_side_effect(
        &io,
        &profile.id,
        Some("serial-stale"),
        EventStream::Stdout,
        b"stale",
        "stale".to_string(),
        || record_serial_capture(&capture, EventDirection::Inbound, b"stale"),
    );
    assert!(capture.lock().unwrap().frames.is_empty());
    assert!(!state
        .store
        .lock()
        .unwrap()
        .events
        .iter()
        .any(|event| event.text.as_deref() == Some("stale")));

    record_channel_bytes_with_accepted_side_effect(
        &io,
        &profile.id,
        Some("serial-current"),
        EventStream::Stdout,
        b"current",
        "current".to_string(),
        || record_serial_capture(&capture, EventDirection::Inbound, b"current"),
    );
    let capture_guard = capture.lock().unwrap();
    assert_eq!(capture_guard.frames.len(), 1);
    assert_eq!(capture_guard.frames[0].bytes, b"current");
    drop(capture_guard);
    assert!(state
        .store
        .lock()
        .unwrap()
        .events
        .iter()
        .any(|event| event.text.as_deref() == Some("current")));

    assert!(
        record_outbound_control_event_for_optional_runtime_with_accepted_side_effect(
            &io,
            &profile.id,
            Some("serial-stale"),
            b"stale-outbound",
            "modem-test-stale",
            false,
            || record_serial_capture(&capture, EventDirection::Outbound, b"stale-outbound"),
        )
        .is_none()
    );
    assert_eq!(capture.lock().unwrap().frames.len(), 1);
    assert!(!state.store.lock().unwrap().events.iter().any(|event| {
        event
            .annotations
            .get("origin")
            .is_some_and(|origin| origin == "modem-test-stale")
    }));

    assert!(
        record_outbound_control_event_for_optional_runtime_with_accepted_side_effect(
            &io,
            &profile.id,
            Some("serial-current"),
            b"current-outbound",
            "modem-test-current",
            false,
            || record_serial_capture(&capture, EventDirection::Outbound, b"current-outbound"),
        )
        .is_some()
    );
    let capture_guard = capture.lock().unwrap();
    assert_eq!(capture_guard.frames.len(), 2);
    assert_eq!(capture_guard.frames[1].bytes, b"current-outbound");
    assert_eq!(capture_guard.frames[1].direction, EventDirection::Outbound);
    drop(capture_guard);
    assert!(state.store.lock().unwrap().events.iter().any(|event| {
        event
            .annotations
            .get("origin")
            .is_some_and(|origin| origin == "modem-test-current")
    }));

    state.serial.lock().unwrap().remove(&profile.id);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn serial_capture_export_is_atomic_filtered_and_checksummed() {
    let root =
        std::env::temp_dir().join(format!("portmate-serial-capture-export-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let captures = Arc::new(Mutex::new(HashMap::new()));
    let capture = serial_capture_for_session(&captures, "serial-export").unwrap();
    {
        let mut capture = capture.lock().unwrap();
        capture.push(EventDirection::Inbound, &[0xff, 0x00, 0x80]);
        capture.push(EventDirection::Outbound, b"OK\r\n");
    }
    let outbound_id = capture.lock().unwrap().frames[1].id.clone();
    let result = export_serial_capture_inner(
        &root.join("portmate-store.sqlite3"),
        &captures,
        ExportSerialCaptureRequest {
            session_id: "serial-export".to_string(),
            frame_ids: vec![outbound_id],
        },
    )
    .unwrap();
    assert_eq!(result.frames, 1);
    assert_eq!(result.captured_bytes, 4);
    assert_eq!(result.truncated_frames, 0);
    assert_eq!(result.sha256, sha256_file(Path::new(&result.path)).unwrap());
    let checksum = fs::read_to_string(&result.checksum_path).unwrap();
    assert!(checksum.starts_with(&result.sha256));

    let lines = fs::read_to_string(&result.path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["format"], "portmate-serial-capture");
    assert_eq!(lines[0]["rawUnredacted"], true);
    assert_eq!(lines[1]["direction"], "outbound");
    assert_eq!(lines[1]["hex"], "4F 4B 0D 0A");
    assert_eq!(lines[1]["ascii"], "OK\\r\\n");
    assert!(!fs::read_to_string(&result.path)
        .unwrap()
        .contains("FF 00 80"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&result.checksum_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(fs::read_dir(root.join("exports"))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn serial_capture_history_reuses_verified_raw_logs_and_exports_exact_frames() {
    let root = std::env::temp_dir().join(format!("portmate-serial-history-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let mut profile = test_serial_profile(portmate_core::SerialConnection {
        port: "/dev/ttyUSB0".to_string(),
        baud_rate: 115_200,
        data_bits: 8,
        stop_bits: 1,
        parity: "none".to_string(),
        flow_control: "none".to_string(),
        dtr: false,
        rts: false,
        reconnect: true,
        reconnect_delay_ms: 1_000,
        receive_idle_timeout_enabled: false,
        receive_idle_timeout_seconds: 60,
    });
    profile.logging.enabled = true;
    profile.logging.raw = true;
    let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

    let first_bytes = vec![0xff, 0x00, 0x80];
    let oversized_bytes = vec![0x5a; MAX_SERIAL_CAPTURE_FRAME_BYTES + 3];
    let first_ref = append_log_bytes(&state.store_path, &profile, "raw", &first_bytes).unwrap();
    let parsed_first_ref = parse_log_bytes_ref(&first_ref).unwrap();
    let legacy_first_ref = format!(
        "{}:{}:{}",
        parsed_first_ref.relative, parsed_first_ref.offset, parsed_first_ref.length
    );
    let text_ref = append_log_bytes(&state.store_path, &profile, "txt", b"not raw").unwrap();
    let oversized_ref =
        append_log_bytes(&state.store_path, &profile, "raw", &oversized_bytes).unwrap();
    let (first_id, oversized_id) = {
        let mut store = state.store.lock().unwrap();
        let first = store
            .record_stream_event_with_bytes_ref(
                &profile.id,
                EventDirection::Inbound,
                EventStream::Stdout,
                "first",
                Some(first_ref),
            )
            .unwrap();
        store
            .record_stream_event_with_bytes_ref(
                &profile.id,
                EventDirection::Outbound,
                EventStream::Stdout,
                "missing",
                Some("missing.raw:0:1".to_string()),
            )
            .unwrap();
        store
            .record_stream_event_with_bytes_ref(
                &profile.id,
                EventDirection::Inbound,
                EventStream::Stdout,
                "wrong shard type",
                Some(text_ref),
            )
            .unwrap();
        store
            .record_stream_event_with_bytes_ref(
                &profile.id,
                EventDirection::Inbound,
                EventStream::Stdout,
                "unverified legacy reference",
                Some(legacy_first_ref),
            )
            .unwrap();
        let oversized = store
            .record_stream_event_with_bytes_ref(
                &profile.id,
                EventDirection::Outbound,
                EventStream::Stdout,
                "oversized",
                Some(oversized_ref),
            )
            .unwrap();
        (first.id, oversized.id)
    };

    let history =
        serial_capture_history_inner(&state.store_path, &state.store, &profile.id).unwrap();
    assert!(history.enabled);
    assert_eq!(history.total_frames, 5);
    assert_eq!(history.frames.len(), 2);
    assert_eq!(history.dropped_frames, 0);
    assert_eq!(history.unavailable_frames, 3);
    assert_eq!(
        history.captured_bytes,
        first_bytes.len() + MAX_SERIAL_CAPTURE_FRAME_BYTES
    );
    assert_eq!(history.frames[0].id, first_id);
    assert_eq!(history.frames[0].bytes, first_bytes);
    assert_eq!(history.frames[0].direction, EventDirection::Inbound);
    assert_eq!(history.frames[1].id, oversized_id);
    assert_eq!(history.frames[1].original_length, oversized_bytes.len());
    assert!(history.frames[1].truncated);

    let result = export_serial_capture_history_inner(
        &state.store_path,
        &state.store,
        ExportSerialCaptureRequest {
            session_id: profile.id.clone(),
            frame_ids: vec![first_id, oversized_id],
        },
    )
    .unwrap();
    assert_eq!(result.frames, 2);
    assert_eq!(result.captured_bytes, history.captured_bytes);
    assert_eq!(result.truncated_frames, 1);
    assert_eq!(result.sha256, sha256_file(Path::new(&result.path)).unwrap());
    let lines = fs::read_to_string(&result.path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["source"], "raw-log");
    assert_eq!(lines[1]["hex"], "FF 00 80");

    {
        let mut store = state.store.lock().unwrap();
        let mut disabled = store.profile(&profile.id).unwrap();
        disabled.logging.raw = false;
        store.upsert_profile(disabled);
    }
    let disabled =
        serial_capture_history_inner(&state.store_path, &state.store, &profile.id).unwrap();
    assert!(!disabled.enabled);
    assert!(disabled.frames.is_empty());
    let _ = fs::remove_dir_all(root);
}

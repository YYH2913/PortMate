use super::*;

#[test]
fn serial_connection_details_validate_port_and_reconnect_flag() {
    let mut profile = test_serial_profile(portmate_core::SerialConnection {
        port: " /dev/ttyUSB0 ".to_string(),
        baud_rate: 115200,
        data_bits: 8,
        stop_bits: 1,
        parity: "none".to_string(),
        flow_control: "none".to_string(),
        dtr: true,
        rts: false,
        reconnect: true,
        reconnect_delay_ms: 0,
        receive_idle_timeout_enabled: true,
        receive_idle_timeout_seconds: u64::MAX,
    });
    let (serial, port_name) = serial_connection_details(&profile).unwrap();
    assert_eq!(serial.baud_rate, 115200);
    assert_eq!(port_name, "/dev/ttyUSB0");
    assert_eq!(
        serial.reconnect_delay_ms,
        portmate_core::MIN_SERIAL_RECONNECT_DELAY_MS
    );
    assert_eq!(
        serial.receive_idle_timeout_seconds,
        portmate_core::MAX_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS
    );
    assert_eq!(
        serial_reconnect_delay(&profile),
        Duration::from_millis(portmate_core::MIN_SERIAL_RECONNECT_DELAY_MS)
    );
    assert!(serial_reconnect_enabled(&profile));

    if let ConnectionConfig::Serial(serial) = &mut profile.connection {
        serial.port = " ".to_string();
        serial.reconnect = false;
    }
    assert!(serial_connection_details(&profile)
        .unwrap_err()
        .contains("串口不能为空"));
    assert!(!serial_reconnect_enabled(&profile));
}

#[test]
fn serial_line_updates_compensate_prior_writes_after_partial_failure() {
    let mut calls = Vec::new();
    let error =
        apply_serial_line_updates_with(false, false, Some(true), Some(true), |line, value| {
            calls.push((line, value));
            if line == SerialControlLine::Rts && value {
                Err("device rejected RTS".to_string())
            } else {
                Ok(())
            }
        })
        .unwrap_err();

    assert!(error.contains("设置 RTS 失败"), "{error}");
    assert_eq!(
        calls,
        [
            (SerialControlLine::Dtr, true),
            (SerialControlLine::Rts, true),
            (SerialControlLine::Rts, false),
            (SerialControlLine::Dtr, false),
        ]
    );
}

#[test]
fn serial_break_retries_a_transient_clear_failure() {
    let clear_attempts = std::cell::Cell::new(0_u8);
    let waited = std::cell::Cell::new(false);

    let retried = pulse_serial_break_with(
        || Ok(()),
        || {
            let attempt = clear_attempts.get() + 1;
            clear_attempts.set(attempt);
            if attempt == 1 {
                Err("transient clear failure".to_string())
            } else {
                Ok(())
            }
        },
        || waited.set(true),
    )
    .unwrap();

    assert!(retried);
    assert!(waited.get());
    assert_eq!(clear_attempts.get(), 2);
}

#[test]
fn serial_line_persistence_failure_keeps_applied_device_truth_in_memory() {
    let root = std::env::temp_dir().join(format!(
        "portmate-serial-line-persistence-failure-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let blocked_parent = root.join("not-a-directory");
    fs::write(&blocked_parent, b"blocked").unwrap();
    let profile = test_serial_profile(portmate_core::SerialConnection {
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
    let mut store = SessionStore::default();
    store.upsert_profile(profile.clone());
    let request = SerialLineRequest {
        session_id: profile.id.clone(),
        dtr: Some(true),
        rts: None,
    };

    let error = record_applied_serial_line_state(
        &mut store,
        &blocked_parent.join("store.sqlite3"),
        &request,
    )
    .unwrap_err();

    assert!(error.contains("已在设备上更新"), "{error}");
    let saved = store.profile(&profile.id).unwrap();
    let ConnectionConfig::Serial(serial) = saved.connection else {
        panic!("expected Serial profile");
    };
    assert!(serial.dtr);
    assert!(store.events.iter().any(|event| {
        event
            .text
            .as_deref()
            .is_some_and(|text| text.contains("serial line state updated"))
    }));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn serial_reconnect_profile_reloads_latest_port_and_disable_state() {
    let profile = test_serial_profile(portmate_core::SerialConnection {
        port: " /dev/ttyUSB0 ".to_string(),
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
    let state = test_app_state(
        profile.clone(),
        PathBuf::from("serial-reconnect-test.sqlite3"),
    );
    assert!(serial_reconnect_attempt_matches_profile(&profile, &profile));
    assert_eq!(
        serial_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
        SerialReconnectProfileState::Current
    );

    let mut renamed = profile.clone();
    renamed.name = "Renamed Serial".to_string();
    assert!(serial_reconnect_attempt_matches_profile(&profile, &renamed));

    let mut updated = profile.clone();
    if let ConnectionConfig::Serial(serial) = &mut updated.connection {
        serial.port = " /dev/ttyUSB1 ".to_string();
        serial.baud_rate = 57_600;
        serial.reconnect_delay_ms = 250;
        serial.receive_idle_timeout_enabled = true;
        serial.receive_idle_timeout_seconds = 15;
    }
    state.store.lock().unwrap().upsert_profile(updated);
    assert_eq!(
        serial_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
        SerialReconnectProfileState::Changed
    );
    let latest = latest_serial_reconnect_profile(&state.session_io(), &profile.id)
        .unwrap()
        .unwrap();
    let (serial, port_name) = serial_connection_details(&latest).unwrap();
    assert_eq!(port_name, "/dev/ttyUSB1");
    assert_eq!(serial.baud_rate, 57_600);
    assert_eq!(serial.reconnect_delay_ms, 250);
    assert!(serial.receive_idle_timeout_enabled);
    assert_eq!(serial.receive_idle_timeout_seconds, 15);

    let mut disabled = latest;
    if let ConnectionConfig::Serial(serial) = &mut disabled.connection {
        serial.reconnect = false;
    }
    state.store.lock().unwrap().upsert_profile(disabled);
    assert_eq!(
        serial_reconnect_profile_state(&state.store.lock().unwrap(), &profile.id, &profile),
        SerialReconnectProfileState::Disabled
    );
    assert!(
        latest_serial_reconnect_profile(&state.session_io(), &profile.id)
            .unwrap()
            .is_none()
    );
}

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

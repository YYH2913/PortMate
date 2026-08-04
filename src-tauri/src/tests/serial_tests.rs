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
    assert_eq!(port_name, " /dev/ttyUSB0 ");
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
    assert_eq!(port_name, " /dev/ttyUSB1 ");
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

#[test]
fn disabling_serial_reconnect_removes_pending_runtime() {
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
    let root = std::env::temp_dir().join(format!(
        "portmate-serial-disable-pending-test-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
    let runtime_id = "pending-serial-runtime".to_string();
    let closed = Arc::new(AtomicBool::new(false));
    let (tap, _) = broadcast::channel(8);
    state.serial.lock().unwrap().insert(
        profile.id.clone(),
        SerialRuntime {
            runtime_id: runtime_id.clone(),
            writer: None,
            tap,
            closed: Arc::clone(&closed),
            capture: serial_capture_for_session(&state.serial_captures, &profile.id).unwrap(),
        },
    );
    {
        let mut store = state.store.lock().unwrap();
        store
            .set_runtime_status_with_reason(
                &profile.id,
                SessionStatus::Reconnecting,
                Some("test reconnect".to_string()),
            )
            .unwrap();
        let mut disabled = store.profile(&profile.id).unwrap();
        if let ConnectionConfig::Serial(serial) = &mut disabled.connection {
            serial.reconnect = false;
        }
        store.upsert_profile(disabled);
    }

    assert!(stop_pending_serial_reconnect_if_disabled(
        &state.session_io(),
        &profile.id,
        &runtime_id,
        "automatic reconnect disabled by test",
    ));
    assert!(closed.load(Ordering::SeqCst));
    assert!(!state.serial.lock().unwrap().contains_key(&profile.id));
    let runtime = state
        .store
        .lock()
        .unwrap()
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == profile.id)
        .unwrap()
        .runtime;
    assert_eq!(runtime.status, SessionStatus::Disconnected);
    assert_eq!(
        runtime.last_disconnect_reason.as_deref(),
        Some("automatic reconnect disabled by test")
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn serial_socat_loopback_round_trips_binary_bytes() {
    if Command::new("socat").arg("-V").output().is_err() {
        eprintln!("skipping serial integration test: socat is not installed");
        return;
    }
    let root = std::env::temp_dir().join(format!("portmate-serial-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let portmate_pty = root.join("portmate.pty ");
    let peer_pty = root.join("peer.pty");
    let child = Command::new("socat")
        .args(["-d", "-d"])
        .arg(format!("pty,raw,echo=0,link={}", portmate_pty.display()))
        .arg(format!("pty,raw,echo=0,link={}", peer_pty.display()))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut socat = ChildGuard(Some(child));

    tauri::async_runtime::block_on(async {
        tokio::time::timeout(Duration::from_secs(3), async {
            while !portmate_pty.exists() || !peer_pty.exists() {
                if let Some(status) = socat.0.as_mut().unwrap().try_wait().unwrap() {
                    panic!("socat exited before creating PTYs: {status}");
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("socat did not create PTYs");

        let profile = test_serial_profile(portmate_core::SerialConnection {
            port: portmate_pty.display().to_string(),
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".to_string(),
            flow_control: "none".to_string(),
            dtr: false,
            rts: false,
            reconnect: false,
            reconnect_delay_ms: 1_000,
            receive_idle_timeout_enabled: true,
            receive_idle_timeout_seconds: 3,
        });
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let opened = open_serial_session(&state, profile.clone()).unwrap();
        assert_eq!(opened.runtime.status, SessionStatus::Connected);
        let mut inbound = state
            .serial
            .lock()
            .unwrap()
            .get(&profile.id)
            .unwrap()
            .tap
            .subscribe();
        let mut peer = serialport::new(peer_pty.display().to_string(), 115_200)
            .timeout(Duration::from_secs(2))
            .open()
            .unwrap();

        let outbound = vec![0xff, 0x00, 0x80];
        send_bytes_inner(state.session_io(), profile.id.clone(), outbound.clone())
            .await
            .unwrap();
        let mut peer_received = [0_u8; 3];
        peer.read_exact(&mut peer_received).unwrap();
        assert_eq!(peer_received, outbound.as_slice());

        let peer_reply = [0x41, 0x00, 0xff, 0x42];
        peer.write_all(&peer_reply).unwrap();
        peer.flush().unwrap();
        let received = tokio::time::timeout(Duration::from_secs(3), inbound.recv())
            .await
            .expect("serial runtime did not receive loopback bytes")
            .expect("serial runtime tap closed");
        assert_eq!(received, peer_reply);

        peer.set_timeout(Duration::from_millis(300)).unwrap();
        let mut unsolicited = [0_u8; 1];
        let error = peer
            .read(&mut unsolicited)
            .expect_err("serial health monitoring must not write probe bytes");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);

        let capture = serial_capture_snapshot_inner(&state, &profile.id, None).unwrap();
        assert_eq!(capture.frames.len(), 2);
        assert_eq!(capture.frames[0].direction, EventDirection::Outbound);
        assert_eq!(capture.frames[0].bytes, outbound);
        assert_eq!(capture.frames[1].direction, EventDirection::Inbound);
        assert_eq!(capture.frames[1].bytes, peer_reply);

        let disconnected = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let summary = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .unwrap();
                if summary.runtime.status == SessionStatus::Disconnected {
                    break summary;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("serial receive idle timeout did not disconnect the runtime");
        let reason = disconnected.runtime.last_disconnect_reason.unwrap();
        assert!(reason.contains("serial receive idle timeout"), "{reason}");
        assert!(!state.serial.lock().unwrap().contains_key(&profile.id));
        let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
        assert!(screen.contains("serial receive idle timeout"), "{screen}");
    });

    socat.stop();
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn serial_socat_reconnects_after_pty_replacement() {
    if Command::new("socat").arg("-V").output().is_err() {
        eprintln!("skipping serial reconnect integration test: socat is not installed");
        return;
    }
    let root = std::env::temp_dir().join(format!("portmate-serial-reconnect-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let first_portmate_pty = root.join("first-portmate.pty ");
    let first_peer_pty = root.join("first-peer.pty");
    let replacement_portmate_pty = root.join("replacement-portmate.pty ");
    let replacement_peer_pty = root.join("replacement-peer.pty");
    let spawn_socat = |portmate_pty: &Path, peer_pty: &Path| {
        let child = Command::new("socat")
            .args(["-d", "-d"])
            .arg(format!("pty,raw,echo=0,link={}", portmate_pty.display()))
            .arg(format!("pty,raw,echo=0,link={}", peer_pty.display()))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        ChildGuard(Some(child))
    };
    let mut socat = spawn_socat(&first_portmate_pty, &first_peer_pty);

    tauri::async_runtime::block_on(async {
        wait_for_socat_pty_pair(&mut socat, &first_portmate_pty, &first_peer_pty).await;

        let profile = test_serial_profile(portmate_core::SerialConnection {
            port: first_portmate_pty.display().to_string(),
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".to_string(),
            flow_control: "none".to_string(),
            dtr: false,
            rts: false,
            reconnect: true,
            reconnect_delay_ms: 10_000,
            receive_idle_timeout_enabled: false,
            receive_idle_timeout_seconds: 60,
        });
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let opened = open_serial_session(&state, profile.clone()).unwrap();
        assert_eq!(opened.runtime.status, SessionStatus::Connected);
        let first_runtime_id = state
            .serial
            .lock()
            .unwrap()
            .get(&profile.id)
            .unwrap()
            .runtime_id
            .clone();
        let peer = serialport::new(first_peer_pty.display().to_string(), 115_200)
            .timeout(Duration::from_secs(2))
            .open()
            .unwrap();

        socat.stop();
        drop(peer);
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let reconnecting = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .is_some_and(|summary| summary.runtime.status == SessionStatus::Reconnecting);
                let writer_released = state
                    .serial
                    .lock()
                    .unwrap()
                    .get(&profile.id)
                    .is_some_and(|runtime| runtime.writer.is_none());
                if reconnecting && writer_released {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("serial runtime did not enter reconnecting state");
        let error = send_bytes_inner(state.session_io(), profile.id.clone(), vec![0xde, 0xad])
            .await
            .unwrap_err();
        assert!(error.contains("串口正在重连"), "{error}");

        let _ = fs::remove_file(&first_portmate_pty);
        let _ = fs::remove_file(&first_peer_pty);
        socat = spawn_socat(&replacement_portmate_pty, &replacement_peer_pty);
        wait_for_socat_pty_pair(&mut socat, &replacement_portmate_pty, &replacement_peer_pty).await;
        {
            let mut store = state.store.lock().unwrap();
            let mut updated = store.profile(&profile.id).unwrap();
            if let ConnectionConfig::Serial(serial) = &mut updated.connection {
                serial.port = replacement_portmate_pty.display().to_string();
                serial.reconnect_delay_ms = 200;
            }
            store.upsert_profile(updated);
            save_store(&state.store_path, &store).unwrap();
        }
        let reconnect_profile_updated_at = Instant::now();
        let mut peer = serialport::new(replacement_peer_pty.display().to_string(), 115_200)
            .timeout(Duration::from_secs(2))
            .open()
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let runtime_replaced =
                    state
                        .serial
                        .lock()
                        .unwrap()
                        .get(&profile.id)
                        .is_some_and(|runtime| {
                            runtime.runtime_id != first_runtime_id && runtime.writer.is_some()
                        });
                let connected = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .is_some_and(|summary| summary.runtime.status == SessionStatus::Connected);
                if runtime_replaced && connected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("serial runtime did not reconnect to the replacement PTY");
        assert!(
            reconnect_profile_updated_at.elapsed() < Duration::from_secs(5),
            "serial reconnect did not adopt the shortened delay"
        );
        let mut inbound = state
            .serial
            .lock()
            .unwrap()
            .get(&profile.id)
            .unwrap()
            .tap
            .subscribe();

        let outbound = vec![0xff, 0x00, 0x80, 0x7f];
        send_bytes_inner(state.session_io(), profile.id.clone(), outbound.clone())
            .await
            .unwrap();
        let mut peer_received = [0_u8; 4];
        peer.read_exact(&mut peer_received).unwrap();
        assert_eq!(peer_received, outbound.as_slice());

        let peer_reply = [0x41, 0x00, 0xff, 0x42];
        peer.write_all(&peer_reply).unwrap();
        peer.flush().unwrap();
        let received = tokio::time::timeout(Duration::from_secs(3), inbound.recv())
            .await
            .expect("reconnected serial runtime did not receive loopback bytes")
            .expect("reconnected serial runtime tap closed");
        assert_eq!(received, peer_reply);

        {
            let mut store = state.store.lock().unwrap();
            let mut disabled = store.profile(&profile.id).unwrap();
            if let ConnectionConfig::Serial(serial) = &mut disabled.connection {
                serial.reconnect = false;
            }
            store.upsert_profile(disabled);
            save_store(&state.store_path, &store).unwrap();
        }
        socat.stop();
        drop(peer);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let runtime_removed = !state.serial.lock().unwrap().contains_key(&profile.id);
                let status = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .unwrap()
                    .runtime
                    .status;
                assert_ne!(
                    status,
                    SessionStatus::Reconnecting,
                    "serial disconnect ignored the latest reconnect=false setting"
                );
                if runtime_removed && status == SessionStatus::Disconnected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("disabling serial reconnect did not disconnect the active runtime");
        tokio::time::sleep(Duration::from_millis(1_200)).await;
        assert!(!state.serial.lock().unwrap().contains_key(&profile.id));

        let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
        assert!(screen.contains("serial read failed"), "{screen}");
        assert!(screen.contains("serial port reconnected"));
        assert!(screen.contains(&replacement_portmate_pty.display().to_string()));
        assert_eq!(screen.matches("reconnecting in 10000ms").count(), 1);
    });

    socat.stop();
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
async fn wait_for_socat_pty_pair(socat: &mut ChildGuard, portmate_pty: &Path, peer_pty: &Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !portmate_pty.exists() || !peer_pty.exists() {
            if let Some(status) = socat.0.as_mut().unwrap().try_wait().unwrap() {
                panic!("socat exited before creating PTYs: {status}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("socat did not create PTYs");
}

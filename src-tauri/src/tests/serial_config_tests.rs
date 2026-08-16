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

#[test]
fn serial_reconnect_install_failure_requires_the_pending_runtime_owner() {
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
    let root = tempfile::tempdir().unwrap();
    let state = test_app_state(profile.clone(), root.path().join("portmate-store.sqlite3"));
    let io = state.session_io();
    let (tap, _) = broadcast::channel(8);
    let pending_closed = Arc::new(AtomicBool::new(false));
    state.serial.lock().unwrap().insert(
        profile.id.clone(),
        SerialRuntime {
            runtime_id: "pending-runtime".to_string(),
            writer: None,
            tap: tap.clone(),
            closed: Arc::clone(&pending_closed),
            capture: serial_capture_for_session(&state.serial_captures, &profile.id).unwrap(),
        },
    );
    state
        .store
        .lock()
        .unwrap()
        .set_runtime_status(&profile.id, SessionStatus::Reconnecting)
        .unwrap();

    fail_pending_serial_reconnect_install(
        &io,
        &profile.id,
        "pending-runtime",
        pending_closed.as_ref(),
        "reader unavailable",
    );

    assert!(pending_closed.load(Ordering::SeqCst));
    assert!(!state.serial.lock().unwrap().contains_key(&profile.id));
    let summary = state
        .store
        .lock()
        .unwrap()
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == profile.id)
        .unwrap();
    assert_eq!(summary.runtime.status, SessionStatus::Error);
    assert!(summary
        .runtime
        .last_disconnect_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("reader unavailable")));

    let replacement_closed = Arc::new(AtomicBool::new(false));
    state.serial.lock().unwrap().insert(
        profile.id.clone(),
        SerialRuntime {
            runtime_id: "replacement-runtime".to_string(),
            writer: None,
            tap,
            closed: Arc::clone(&replacement_closed),
            capture: serial_capture_for_session(&state.serial_captures, &profile.id).unwrap(),
        },
    );
    state
        .store
        .lock()
        .unwrap()
        .set_runtime_status(&profile.id, SessionStatus::Connected)
        .unwrap();
    let stale_closed = AtomicBool::new(false);

    fail_pending_serial_reconnect_install(
        &io,
        &profile.id,
        "pending-runtime",
        &stale_closed,
        "stale failure",
    );

    assert!(stale_closed.load(Ordering::SeqCst));
    assert!(!replacement_closed.load(Ordering::SeqCst));
    assert_eq!(
        state
            .serial
            .lock()
            .unwrap()
            .get(&profile.id)
            .map(|runtime| runtime.runtime_id.as_str()),
        Some("replacement-runtime")
    );
    let store = state.store.lock().unwrap();
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == profile.id)
        .unwrap();
    assert_eq!(summary.runtime.status, SessionStatus::Connected);
    assert!(!store.events.iter().any(|event| {
        event
            .text
            .as_deref()
            .is_some_and(|text| text.contains("stale failure"))
    }));
}

#[test]
fn serial_reconnect_store_poison_removes_the_pending_runtime() {
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
    let root = tempfile::tempdir().unwrap();
    let state = test_app_state(profile.clone(), root.path().join("portmate-store.sqlite3"));
    let io = state.session_io();
    let closed = Arc::new(AtomicBool::new(false));
    let (tap, _) = broadcast::channel(8);
    state.serial.lock().unwrap().insert(
        profile.id.clone(),
        SerialRuntime {
            runtime_id: "poisoned-store-runtime".to_string(),
            writer: None,
            tap,
            closed: Arc::clone(&closed),
            capture: serial_capture_for_session(&state.serial_captures, &profile.id).unwrap(),
        },
    );
    state
        .store
        .lock()
        .unwrap()
        .set_runtime_status(&profile.id, SessionStatus::Reconnecting)
        .unwrap();

    let store = Arc::clone(&state.store);
    let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = store.lock().unwrap();
        panic!("poison reconnect Store for test");
    }));
    assert!(poisoned.is_err());

    assert!(!serial_reconnect_pending(
        &io,
        &profile.id,
        "poisoned-store-runtime",
        closed.as_ref(),
    ));
    assert!(closed.load(Ordering::SeqCst));
    assert!(!state.serial.lock().unwrap().contains_key(&profile.id));
}

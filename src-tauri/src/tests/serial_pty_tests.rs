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

        let break_result = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "serial-break-client".to_string(),
                trusted_write: true,
                command: "serial_send_break".to_string(),
                args: serde_json::json!({ "sessionId": profile.id }),
            },
        )
        .await
        .unwrap();
        assert_eq!(break_result["sent"], true);
        assert_eq!(break_result["sessionId"], profile.id);
        {
            let store = state.store.lock().unwrap();
            assert!(store.events.iter().any(|event| {
                event.session_id == profile.id
                    && event.text.as_deref() == Some("PortMate: serial Break sent")
            }));
            let audit = store.audit.last().expect("serial Break audit");
            assert_eq!(audit.action, "serial_send_break");
            assert_eq!(audit.decision, "succeeded");
        }

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

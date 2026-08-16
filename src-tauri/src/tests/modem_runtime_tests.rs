use super::*;

#[test]
fn tcp_device_loadx_transfer_sends_command_and_file_in_one_task() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let payload = b"PortMate loadx integration".to_vec();
        let expected_packet = modem_packet_bytes(MODEM_SOH, 1, &payload, XMODEM_BLOCK_SIZE, true);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut command = Vec::new();
            loop {
                let byte = socket.read_u8().await.unwrap();
                command.push(byte);
                if byte == b'\r' {
                    break;
                }
            }
            assert_eq!(command, b"loadx 0x80000000\r");
            socket.write_all(&[MODEM_CRC_REQUEST]).await.unwrap();

            let mut packet = vec![0_u8; expected_packet.len()];
            socket.read_exact(&mut packet).await.unwrap();
            assert_eq!(packet, expected_packet);
            socket.write_all(&[MODEM_ACK]).await.unwrap();

            assert_eq!(socket.read_u8().await.unwrap(), MODEM_EOT);
            socket.write_all(&[MODEM_ACK]).await.unwrap();
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!("portmate-loadx-tcp-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("firmware.bin");
        fs::write(&source, &payload).unwrap();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let task = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Xmodem,
                source: source.display().to_string(),
                destination: "load:loadx?address=0x80000000".to_string(),
            },
        )
        .await
        .unwrap();
        let completed = wait_for_transfer_terminal_state(&state, &task.id).await;
        assert_eq!(completed.status, TransferStatus::Completed, "{completed:?}");
        assert_eq!(completed.bytes_done, payload.len() as u64);
        assert_eq!(completed.destination, "load:loadx?address=0x80000000");

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("loadx server timed out")
            .expect("loadx server failed");
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn cancelling_silent_xmodem_sends_can_and_stops_worker() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut cancel = [0_u8; 3];
            socket.read_exact(&mut cancel).await.unwrap();
            assert_eq!(cancel, [MODEM_CAN; 3]);
            let _ = release_rx.await;
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!("portmate-modem-cancel-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.bin");
        fs::write(&source, b"cancel this XModem transfer").unwrap();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let task = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Xmodem,
                source: source.display().to_string(),
                destination: "remote:/tmp/cancelled.bin".to_string(),
            },
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state
                    .store
                    .lock()
                    .unwrap()
                    .transfer_by_id(&task.id)
                    .is_some_and(|task| task.status == TransferStatus::Running)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("XModem task did not start");

        let cancelling = cancel_transfer_inner(&state, &task.id).unwrap();
        assert_eq!(cancelling.status, TransferStatus::Cancelled);
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if state
                    .store
                    .lock()
                    .unwrap()
                    .transfer_by_id(&task.id)
                    .is_some_and(|task| {
                        task.status == TransferStatus::Cancelled
                            && task.message.as_deref() == Some("cancelled")
                    })
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled XModem worker did not stop promptly");
        let cancelled = state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&task.id)
            .unwrap();
        assert_eq!(cancelled.status, TransferStatus::Cancelled);
        assert_eq!(cancelled.message.as_deref(), Some("cancelled"));

        let _ = release_tx.send(());
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("XModem cancellation bytes were not received")
            .expect("XModem cancellation server failed");
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn silent_xmodem_fails_promptly_when_transport_reconnects() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (disconnect_tx, disconnect_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = disconnect_rx.await;
            drop(socket);
            let _ = release_rx.await;
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: true,
            ..Default::default()
        }));
        let root =
            std::env::temp_dir().join(format!("portmate-modem-disconnect-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.bin");
        fs::write(&source, b"disconnect this XModem transfer").unwrap();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let task = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Xmodem,
                source: source.display().to_string(),
                destination: "remote:/tmp/disconnected.bin".to_string(),
            },
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state
                    .store
                    .lock()
                    .unwrap()
                    .transfer_by_id(&task.id)
                    .is_some_and(|task| task.status == TransferStatus::Running)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("XModem disconnect task did not start");
        let _ = disconnect_tx.send(());

        let failed = tokio::time::timeout(TEST_RUNTIME_TRANSITION_TIMEOUT, async {
            loop {
                let task = state
                    .store
                    .lock()
                    .unwrap()
                    .transfer_by_id(&task.id)
                    .unwrap();
                if task.status == TransferStatus::Failed {
                    break task;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("XModem worker did not fail promptly after transport loss");
        assert!(failed
            .message
            .as_deref()
            .is_some_and(|message| message.contains("modem session disconnected")));
        assert!(!state
            .transfer_cancellations
            .lock()
            .unwrap()
            .contains_key(&task.id));

        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        let _ = release_tx.send(());
        server.await.unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn modem_binding_rejects_replaced_tcp_runtime_without_writing_the_replacement() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_first, _) = listener.accept().await.unwrap();
            let (mut replacement, _) = listener.accept().await.unwrap();
            match tokio::time::timeout(Duration::from_millis(300), replacement.read_u8()).await {
                Err(_) => None,
                Ok(Ok(byte)) => Some(byte),
                Ok(Err(error)) => panic!("replacement TCP runtime closed unexpectedly: {error}"),
            }
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root =
            std::env::temp_dir().join(format!("portmate-modem-generation-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let binding = runtime_modem_binding(&state, &profile.id).unwrap();
        let mut protocol_receiver = binding.subscribe();
        let mut completion_receiver = binding.subscribe();
        let old_tap = state
            .tcp
            .lock()
            .unwrap()
            .get(&profile.id)
            .unwrap()
            .tap
            .clone();
        old_tap.send(b"old generation".to_vec()).unwrap();
        assert_eq!(protocol_receiver.recv().await.unwrap(), b"old generation");
        assert_eq!(completion_receiver.recv().await.unwrap(), b"old generation");
        let reader =
            binding.reader_with_receiver(protocol_receiver, Arc::new(AtomicBool::new(false)));

        open_tcp_session(&state, profile.clone()).await.unwrap();
        assert_eq!(
            state
                .store
                .lock()
                .unwrap()
                .summaries()
                .into_iter()
                .find(|summary| summary.profile.id == profile.id)
                .unwrap()
                .runtime
                .status,
            SessionStatus::Connected
        );
        assert_ne!(
            state
                .tcp
                .lock()
                .unwrap()
                .get(&profile.id)
                .unwrap()
                .runtime_id,
            binding.runtime_id()
        );

        let binding_error = binding.ensure_current().unwrap_err();
        assert!(binding_error.contains("替换"), "{binding_error}");
        let reader_error = reader.check_interrupted().unwrap_err();
        assert!(reader_error.contains("替换"), "{reader_error}");
        let write_error = binding
            .write_runtime_bytes(&state, b"must not reach replacement")
            .await
            .unwrap_err();
        assert!(write_error.contains("替换"), "{write_error}");

        let mut task = test_transfer_task(&profile.id, TransferStatus::Running);
        task.id = "stale-modem-completion".to_string();
        task.protocol = TransferProtocol::Xmodem;
        state.store.lock().unwrap().record_transfer(task);
        finish_transfer_task_for_generations(
            &state,
            "stale-modem-completion",
            &profile.id,
            TransferStatus::Completed,
            "completed".to_string(),
            Some(23),
            TransferRuntimeExpectations {
                ssh_runtime_id: None,
                modem_binding: Some(&binding),
            },
        );
        let stale_completion = state
            .store
            .lock()
            .unwrap()
            .transfer_by_id("stale-modem-completion")
            .unwrap();
        assert_eq!(stale_completion.status, TransferStatus::Failed);
        assert_eq!(stale_completion.bytes_done, 0);
        assert!(stale_completion
            .message
            .as_deref()
            .is_some_and(|message| message.contains("完成提交前已变化")));
        assert_eq!(server.await.unwrap(), None);

        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn load_baud_restore_rejects_a_replaced_serial_runtime() {
    let profile = test_serial_profile(portmate_core::SerialConnection {
        port: "test-serial".to_string(),
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
    let root = std::env::temp_dir().join(format!("portmate-load-generation-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
    let (old_tap, _) = broadcast::channel(8);
    let old_closed = Arc::new(AtomicBool::new(false));
    state.serial.lock().unwrap().insert(
        profile.id.clone(),
        SerialRuntime {
            runtime_id: "old-serial-runtime".to_string(),
            writer: None,
            tap: old_tap,
            closed: Arc::clone(&old_closed),
            capture: serial_capture_for_session(&state.serial_captures, &profile.id).unwrap(),
        },
    );
    state
        .store
        .lock()
        .unwrap()
        .set_runtime_status_with_reason(&profile.id, SessionStatus::Connected, None)
        .unwrap();
    let binding = runtime_modem_binding(&state, &profile.id).unwrap();

    old_closed.store(true, Ordering::SeqCst);
    let (new_tap, _) = broadcast::channel(8);
    state.serial.lock().unwrap().insert(
        profile.id.clone(),
        SerialRuntime {
            runtime_id: "new-serial-runtime".to_string(),
            writer: None,
            tap: new_tap,
            closed: Arc::new(AtomicBool::new(false)),
            capture: serial_capture_for_session(&state.serial_captures, &profile.id).unwrap(),
        },
    );

    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .summaries()
            .into_iter()
            .find(|summary| summary.profile.id == profile.id)
            .unwrap()
            .runtime
            .status,
        SessionStatus::Connected
    );
    let binding_error = binding.ensure_current().unwrap_err();
    assert!(binding_error.contains("替换"), "{binding_error}");
    assert!(serial_runtime_writer(&state, &profile.id, binding.runtime_id()).is_err());
    assert!(
        restore_serial_runtime_baud(&state, &profile.id, binding.runtime_id(), 115_200,).is_err()
    );

    let _ = fs::remove_dir_all(root);
}

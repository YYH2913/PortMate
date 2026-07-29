use super::*;

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

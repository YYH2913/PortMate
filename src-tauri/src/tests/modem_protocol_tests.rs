use super::*;

#[test]
fn ymodem_metadata_preserves_significant_file_name_whitespace() {
    let (name, size) = parse_ymodem_metadata(b" report.bin \x00123 0 0\0padding");
    assert_eq!(name, " report.bin ");
    assert_eq!(size, Some(123));
}

#[test]
fn remote_modem_command_uses_raw_tty_and_non_echoing_markers() {
    let token = "modem-token-1";
    let command = modem_remote_command(
        TransferProtocol::Ymodem,
        true,
        "/tmp/transfers/file.bin",
        token,
    );
    assert!(command.contains("stty raw -echo"));
    assert!(command.contains("stty sane"));
    assert!(command.contains("rb -y"));
    assert!(command.contains(token));
    assert!(!command.contains("__PORTMATE_MODEM_modem-token-1_READY__"));
    assert!(!command.contains("__PORTMATE_MODEM_modem-token-1_DONE__"));

    let root_command = modem_remote_command(TransferProtocol::Zmodem, true, "/file.bin", token);
    assert!(root_command.contains("mkdir -p '/'"), "{root_command}");
    assert!(root_command.contains("cd '/'"), "{root_command}");
    assert!(root_command.contains("rz -y"), "{root_command}");

    let finalize =
        xmodem_remote_finalize_command("/tmp/file.bin.portmate-part", "/tmp/file.bin", 37, token);
    assert!(finalize.contains("portable_path()"));
    assert!(finalize.contains("truncate -s 37"));
    assert!(finalize.contains("count=37"));
    assert!(finalize.contains("portmate_status"));
    assert!(!finalize.contains(" status="));
    assert!(!finalize.contains(" -- \"$target\""));
    assert!(!finalize.contains("mv -f --"));
    assert!(!finalize.contains("rm -f --"));
    assert!(finalize.ends_with('\r'));
    let atomic_finalize =
        remote_modem_finalize_command("/tmp/file.bin.portmate-part", "/tmp/file.bin", token);
    assert!(atomic_finalize.contains("mv -f \"$part\" \"$target\""));
    assert!(atomic_finalize.contains("__PORTMATE_MODEM_FINALIZE_%s_DONE__"));
    assert!(atomic_finalize.ends_with('\r'));
    assert!(is_modem_timeout("modem byte timeout"));
    assert!(is_modem_timeout("timed out waiting for modem ACK"));
}

#[cfg(unix)]
#[test]
fn xmodem_remote_finalize_command_handles_dash_prefixed_path() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("-received.bin");
    let part = root.path().join("-received.bin.portmate-part");
    fs::write(&part, b"abcdef").unwrap();

    let command = xmodem_remote_finalize_command(
        "-received.bin.portmate-part",
        "-received.bin",
        3,
        "modem-token",
    );
    let output = Command::new("sh")
        .arg("-c")
        .arg(command.trim_end_matches('\r'))
        .current_dir(root.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"abc");
    assert!(!part.exists());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("__PORTMATE_XMODEM_modem-token_DONE__")
    );
}

#[cfg(unix)]
#[test]
fn remote_modem_finalize_command_moves_part_atomically() {
    let root = tempfile::tempdir().unwrap();
    let part = root.path().join("-received.bin.portmate-part");
    let target = root.path().join("-received.bin");
    fs::write(&part, b"complete modem payload").unwrap();
    fs::write(&target, b"old payload").unwrap();

    let command = remote_modem_finalize_command(
        "-received.bin.portmate-part",
        "-received.bin",
        "modem-token",
    );
    let output = Command::new("sh")
        .arg("-c")
        .arg(command.trim_end_matches('\r'))
        .current_dir(root.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"complete modem payload");
    assert!(!part.exists());
    assert!(String::from_utf8_lossy(&output.stdout)
        .contains("__PORTMATE_MODEM_FINALIZE_modem-token_DONE__"));
}

#[cfg(unix)]
#[test]
fn xmodem_remote_finalize_falls_back_when_truncate_fails() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("received.bin");
    let part = root.path().join("received.bin.portmate-part");
    fs::write(&part, b"abcdef").unwrap();
    let tools = root.path().join("tools");
    fs::create_dir_all(&tools).unwrap();
    let truncate = tools.join("truncate");
    fs::write(&truncate, "#!/bin/sh\nexit 1\n").unwrap();
    let mut permissions = fs::metadata(&truncate).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&truncate, permissions).unwrap();
    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(tools).chain(std::env::split_paths(&inherited_path)))
            .unwrap();

    let output = Command::new("sh")
        .arg("-c")
        .arg(
            xmodem_remote_finalize_command(
                "received.bin.portmate-part",
                "received.bin",
                3,
                "modem-token",
            )
            .trim_end_matches('\r'),
        )
        .current_dir(root.path())
        .env("PATH", path)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"abc");
    assert!(!part.exists());
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("__PORTMATE_XMODEM_modem-token_DONE__")
    );
}

#[test]
fn modem_sender_retries_packet_and_eot_after_ack_timeout() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let payload = b"retry after a lost ACK";
        let expected_packet = modem_packet_bytes(MODEM_SOH, 1, payload, XMODEM_BLOCK_SIZE, true);
        let packet_len = expected_packet.len();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut first_packet = vec![0_u8; packet_len];
            socket.read_exact(&mut first_packet).await.unwrap();
            let mut retry_packet = vec![0_u8; packet_len];
            socket.read_exact(&mut retry_packet).await.unwrap();
            socket.write_all(&[MODEM_ACK]).await.unwrap();

            let mut first_eot = [0_u8; 1];
            socket.read_exact(&mut first_eot).await.unwrap();
            let mut retry_eot = [0_u8; 1];
            socket.read_exact(&mut retry_eot).await.unwrap();
            socket.write_all(&[MODEM_ACK]).await.unwrap();
            let _ = release_rx.await;

            assert_eq!(first_eot, [MODEM_EOT]);
            assert_eq!(retry_eot, first_eot);
            (first_packet, retry_packet)
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!("portmate-modem-retry-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();
        let receiver = runtime_tap_receiver(&state, &profile.id).unwrap();
        let mut reader = ModemByteReader::new(receiver, Arc::new(AtomicBool::new(false)))
            .watch_connection(Arc::clone(&state.store), profile.id.clone());

        modem_send_packet_bytes_with_retries(
            &state,
            &profile.id,
            &mut reader,
            1,
            &expected_packet,
            Duration::from_millis(50),
        )
        .await
        .unwrap();
        modem_finish_eot_with_timeout(&state, &profile.id, &mut reader, Duration::from_millis(50))
            .await
            .unwrap();

        let _ = release_tx.send(());
        let (first_packet, retry_packet) =
            tokio::time::timeout(TEST_RUNTIME_TRANSITION_TIMEOUT, server)
                .await
                .expect("modem retry server timed out")
                .expect("modem retry server failed");
        assert_eq!(first_packet, expected_packet);
        assert_eq!(retry_packet, first_packet);
        tokio::time::timeout(TEST_RUNTIME_TRANSITION_TIMEOUT, async {
            loop {
                if !state.tcp.lock().unwrap().contains_key(&profile.id) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("modem retry session did not close");
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn modem_ready_marker_discards_stale_bytes_and_preserves_handshake() {
    tauri::async_runtime::block_on(async {
        let (tap, receiver) = broadcast::channel(8);
        tap.send(b"old-C-old-__PORTMATE_MODEM_token_REA".to_vec())
            .unwrap();
        tap.send([b"DY__".as_slice(), &[MODEM_CRC_REQUEST]].concat())
            .unwrap();

        let mut reader = ModemByteReader::after_marker(
            receiver,
            "__PORTMATE_MODEM_token_READY__",
            Arc::new(AtomicBool::new(false)),
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            reader.next_byte(Duration::from_millis(10)).await.unwrap(),
            MODEM_CRC_REQUEST
        );
    });
}

#[test]
fn modem_marker_and_byte_waits_observe_cancellation_promptly() {
    tauri::async_runtime::block_on(async {
        let (marker_tap, marker_receiver) = broadcast::channel(8);
        let marker_cancel = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&marker_cancel);
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let result = ModemByteReader::after_marker(
            marker_receiver,
            "__PORTMATE_MODEM_never_READY__",
            marker_cancel,
            None,
        )
        .await;
        let error = match result {
            Ok(_) => panic!("cancelled modem marker wait unexpectedly succeeded"),
            Err(error) => error,
        };
        assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
        assert!(started.elapsed() < Duration::from_secs(1));
        cancel_task.await.unwrap();
        drop(marker_tap);

        let (byte_tap, byte_receiver) = broadcast::channel(8);
        let byte_cancel = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&byte_cancel);
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.store(true, Ordering::SeqCst);
        });
        let mut reader = ModemByteReader::new(byte_receiver, byte_cancel);
        let started = Instant::now();
        let error = reader.next_byte(Duration::from_secs(15)).await.unwrap_err();
        assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
        assert!(started.elapsed() < Duration::from_secs(1));
        cancel_task.await.unwrap();
        drop(byte_tap);
    });
}

#[test]
fn modem_byte_wait_fails_when_session_starts_reconnecting() {
    tauri::async_runtime::block_on(async {
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: 9,
            reconnect: true,
            ..Default::default()
        }));
        let root =
            std::env::temp_dir().join(format!("portmate-modem-disconnect-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        state
            .store
            .lock()
            .unwrap()
            .open_session(&profile.id)
            .unwrap();
        let (tap, receiver) = broadcast::channel(8);
        let mut reader = ModemByteReader::new(receiver, Arc::new(AtomicBool::new(false)))
            .watch_connection(Arc::clone(&state.store), profile.id.clone());
        state
            .store
            .lock()
            .unwrap()
            .set_runtime_status_with_reason(
                &profile.id,
                SessionStatus::Reconnecting,
                Some("test transport loss".to_string()),
            )
            .unwrap();

        let started = Instant::now();
        let error = reader.next_byte(Duration::from_secs(15)).await.unwrap_err();
        assert!(error.contains("modem session disconnected"), "{error}");
        assert!(error.contains("Reconnecting"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(tap);
        let _ = fs::remove_dir_all(root);
    });
}

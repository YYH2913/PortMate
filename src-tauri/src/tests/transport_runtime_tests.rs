use super::*;

#[test]
fn telnet_loopback_applies_binary_naws_resize_and_profile_terminal_type() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let (negotiated_tx, negotiated_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            accepted_tx.send(()).unwrap();
            release_rx.await.unwrap();
            socket
                .write_all(&[
                    TELNET_IAC,
                    TELNET_DO,
                    TELNET_OPT_BINARY,
                    TELNET_IAC,
                    TELNET_WILL,
                    TELNET_OPT_BINARY,
                    TELNET_IAC,
                    TELNET_DO,
                    TELNET_OPT_NAWS,
                    TELNET_IAC,
                    TELNET_DO,
                    TELNET_OPT_TERMINAL_TYPE,
                ])
                .await
                .unwrap();

            let initial_naws = telnet_naws_message(255, 511);
            let expected_negotiation = [
                [TELNET_IAC, TELNET_WILL, TELNET_OPT_BINARY].as_slice(),
                [TELNET_IAC, TELNET_DO, TELNET_OPT_BINARY].as_slice(),
                [TELNET_IAC, TELNET_WILL, TELNET_OPT_NAWS].as_slice(),
                initial_naws.as_slice(),
                [TELNET_IAC, TELNET_WILL, TELNET_OPT_TERMINAL_TYPE].as_slice(),
            ]
            .concat();
            let mut negotiation = vec![0_u8; expected_negotiation.len()];
            socket.read_exact(&mut negotiation).await.unwrap();
            assert_eq!(negotiation, expected_negotiation);

            socket
                .write_all(&[
                    TELNET_IAC,
                    TELNET_SB,
                    TELNET_OPT_TERMINAL_TYPE,
                    TELNET_TTYPE_SEND,
                    TELNET_IAC,
                    TELNET_SE,
                ])
                .await
                .unwrap();
            let expected_terminal = [
                [
                    TELNET_IAC,
                    TELNET_SB,
                    TELNET_OPT_TERMINAL_TYPE,
                    TELNET_TTYPE_IS,
                ]
                .as_slice(),
                b"vt100".as_slice(),
                [TELNET_IAC, TELNET_SE].as_slice(),
            ]
            .concat();
            let mut terminal = vec![0_u8; expected_terminal.len()];
            socket.read_exact(&mut terminal).await.unwrap();
            assert_eq!(terminal, expected_terminal);
            socket.write_all(&[b'\r', 0]).await.unwrap();
            negotiated_tx.send(()).unwrap();

            let mut text = [0_u8; 5];
            socket.read_exact(&mut text).await.unwrap();
            assert_eq!(&text, b"show\n");
            for expected in [telnet_naws_message(80, 24), telnet_naws_message(100, 40)] {
                let mut resize = vec![0_u8; expected.len()];
                socket.read_exact(&mut resize).await.unwrap();
                assert_eq!(resize, expected);
            }
        });

        let mut profile = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        profile.terminal.term = "vt100".to_string();
        profile.logging.enabled = true;
        profile.logging.raw = true;
        profile.logging.text = false;
        profile.logging.jsonl = false;
        let root =
            std::env::temp_dir().join(format!("portmate-telnet-binary-naws-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();
        accepted_rx.await.unwrap();

        resize_session_inner(&state, profile.id.clone(), 255, 511)
            .await
            .unwrap();
        release_tx.send(()).unwrap();
        negotiated_rx.await.unwrap();

        let text_event =
            send_text_inner(state.session_io(), profile.id.clone(), "show\n".to_string())
                .await
                .unwrap();
        assert_eq!(
            read_log_bytes_ref(&state.store_path, text_event.bytes_ref.as_deref().unwrap())
                .unwrap()
                .2,
            b"show\n"
        );
        resize_session_inner(&state, profile.id.clone(), 80, 24)
            .await
            .unwrap();
        let summary = resize_session_inner(&state, profile.id.clone(), 100, 40)
            .await
            .unwrap();
        assert_eq!(
            (summary.profile.terminal.cols, summary.profile.terminal.rows),
            (100, 40)
        );

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("Telnet BINARY/NAWS server timed out")
            .expect("Telnet BINARY/NAWS server failed");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let disconnected = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .is_some_and(|summary| summary.runtime.status == SessionStatus::Disconnected);
                if disconnected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Telnet BINARY/NAWS runtime did not close after EOF");

        let store = state.store.lock().unwrap();
        let stdout = store
            .events
            .iter()
            .filter(|event| event.session_id == profile.id && event.stream == EventStream::Stdout)
            .filter_map(|event| event.text.as_deref())
            .collect::<String>();
        assert!(stdout.contains("\r\0"));
        let control_bytes = store
            .events
            .iter()
            .filter(|event| {
                event.session_id == profile.id
                    && event.direction == EventDirection::Outbound
                    && event.stream == EventStream::Control
            })
            .map(|event| {
                read_log_bytes_ref(&state.store_path, event.bytes_ref.as_deref().unwrap())
                    .unwrap()
                    .2
            })
            .collect::<Vec<_>>();
        assert!(control_bytes.contains(&telnet_naws_message(255, 511)));
        assert!(control_bytes.contains(&telnet_naws_message(80, 24)));
        assert!(control_bytes.contains(&telnet_naws_message(100, 40)));
        drop(store);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn telnet_user_sends_bind_exact_wire_bytes_to_outbound_events() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut text = [0_u8; 6];
            socket.read_exact(&mut text).await.unwrap();
            assert_eq!(&text, b"show\r\n");
            let mut raw = [0_u8; 3];
            socket.read_exact(&mut raw).await.unwrap();
            assert_eq!(raw, [0x01, TELNET_IAC, TELNET_IAC]);
            let mut modem = [0_u8; 3];
            socket.read_exact(&mut modem).await.unwrap();
            assert_eq!(modem, [MODEM_CAN, TELNET_IAC, TELNET_IAC]);
        });

        let mut profile =
            test_tcp_profile(ConnectionConfig::Telnet(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: false,
                ..Default::default()
            }));
        profile.logging.enabled = true;
        profile.logging.raw = true;
        profile.logging.text = false;
        profile.logging.jsonl = false;
        let root = std::env::temp_dir().join(format!("portmate-telnet-send-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let text_event =
            send_text_inner(state.session_io(), profile.id.clone(), "show\n".to_string())
                .await
                .unwrap();
        let bytes_event = send_bytes_inner(
            state.session_io(),
            profile.id.clone(),
            vec![0x01, TELNET_IAC],
        )
        .await
        .unwrap();
        write_runtime_bytes(&state, &profile.id, &[MODEM_CAN, TELNET_IAC])
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("Telnet send server timed out")
            .expect("Telnet send server failed");
        assert_eq!(text_event.direction, EventDirection::Outbound);
        assert_eq!(bytes_event.direction, EventDirection::Outbound);
        assert_eq!(bytes_event.text.as_deref(), Some("Binary payload: 2 bytes"));
        assert_eq!(
            read_log_bytes_ref(&state.store_path, text_event.bytes_ref.as_deref().unwrap())
                .unwrap()
                .2,
            b"show\r\n"
        );
        assert_eq!(
            read_log_bytes_ref(&state.store_path, bytes_event.bytes_ref.as_deref().unwrap())
                .unwrap()
                .2,
            [0x01, TELNET_IAC, TELNET_IAC]
        );
        let audit_actions = state
            .store
            .lock()
            .unwrap()
            .audit
            .iter()
            .map(|record| record.action.clone())
            .collect::<Vec<_>>();
        assert_eq!(audit_actions, ["send_text", "send_bytes"]);
        let modem_event = state
            .store
            .lock()
            .unwrap()
            .events
            .iter()
            .find(|event| {
                event.direction == EventDirection::Outbound
                    && event.stream == EventStream::Control
                    && event.annotations.get("origin").map(String::as_str) == Some("modem")
            })
            .cloned()
            .unwrap();
        assert!(modem_event.text.is_none());
        assert_eq!(
            read_log_bytes_ref(&state.store_path, modem_event.bytes_ref.as_deref().unwrap())
                .unwrap()
                .2,
            [MODEM_CAN, TELNET_IAC, TELNET_IAC]
        );

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn concurrent_outbound_lane_matches_wire_raw_and_event_order() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 6];
            socket.read_exact(&mut received).await.unwrap();
            received.to_vec()
        });

        let mut profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        profile.logging.enabled = true;
        profile.logging.raw = true;
        profile.logging.text = false;
        profile.logging.jsonl = false;
        let root = std::env::temp_dir().join(format!("portmate-outbound-lane-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
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

        let lane = outbound_lane(&state.store_path, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let barrier = Arc::new(tokio::sync::Barrier::new(4));
        let first = {
            let io = state.session_io();
            let session_id = profile.id.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                send_bytes_inner(io, session_id, vec![0x11, 0xa1]).await
            })
        };
        let second = {
            let io = state.session_io();
            let session_id = profile.id.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                send_bytes_inner(io, session_id, vec![0x22, 0xa2]).await
            })
        };
        let modem = {
            let state = state.clone();
            let session_id = profile.id.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                write_runtime_bytes(&state, &session_id, &[0x33, 0xa3]).await
            })
        };
        barrier.wait().await;
        tokio::task::yield_now().await;
        assert!(!first.is_finished());
        assert!(!second.is_finished());
        assert!(!modem.is_finished());
        drop(lane_guard);
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        modem.await.unwrap().unwrap();
        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("TCP server timed out")
            .expect("TCP server failed");

        let event_bytes = state
            .store
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| {
                event.session_id == profile.id
                    && event.direction == EventDirection::Outbound
                    && event.bytes_ref.is_some()
            })
            .flat_map(|event| {
                read_log_bytes_ref(&state.store_path, event.bytes_ref.as_deref().unwrap())
                    .unwrap()
                    .2
            })
            .collect::<Vec<_>>();
        assert_eq!(event_bytes, received);
        let raw_path = log_shard_path(&state.store_path, &profile, "raw").unwrap();
        assert_eq!(fs::read(raw_path).unwrap(), received);

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn tcp_telnet_loopback_negotiates_and_round_trips_wire_bytes() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(&[
                    TELNET_IAC,
                    TELNET_WILL,
                    TELNET_OPT_ECHO,
                    b'l',
                    b'o',
                    b'g',
                    b'i',
                    b'n',
                    b':',
                    b' ',
                ])
                .await
                .unwrap();

            let mut negotiation_reply = [0_u8; 3];
            socket.read_exact(&mut negotiation_reply).await.unwrap();
            assert_eq!(negotiation_reply, [TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO]);

            let mut command = [0_u8; 6];
            socket.read_exact(&mut command).await.unwrap();
            assert_eq!(&command, b"show\r\n");

            let mut raw = [0_u8; 3];
            socket.read_exact(&mut raw).await.unwrap();
            assert_eq!(raw, [0x01, TELNET_IAC, TELNET_IAC]);
        });

        let tcp = TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            ..Default::default()
        };
        let mut client = connect_tcp_socket(&tcp, "Telnet").await.unwrap();
        let mut incoming = [0_u8; 10];
        client.read_exact(&mut incoming).await.unwrap();
        let profile = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection::default()));
        let mut negotiator =
            TelnetNegotiator::new(TelnetRuntimeState::from_profile(&profile).unwrap());
        let (text, replies) = negotiator.filter(&incoming);
        assert_eq!(text, b"login: ");
        assert_eq!(replies.len(), 1);
        client.write_all(&replies[0]).await.unwrap();
        client
            .write_all(encode_telnet_outbound_text("show\n", false).as_bytes())
            .await
            .unwrap();
        client
            .write_all(&encode_telnet_outbound_bytes(&[0x01, TELNET_IAC]))
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("Telnet loopback server timed out")
            .expect("Telnet loopback server task failed");
    });
}

#[cfg(unix)]
#[test]
fn telnet_tls_rejects_untrusted_certificate_and_connects_when_explicitly_allowed() {
    let _runtime_guard = shared_runtime_test_guard();
    tauri::async_runtime::block_on(async {
        use native_tls::{Identity, TlsAcceptor};
        use rcgen::generate_simple_self_signed;
        use tokio_native_tls::TlsAcceptor as TokioTlsAcceptor;

        let certificate = generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let identity = Identity::from_pkcs8(
            certificate.cert.pem().as_bytes(),
            certificate.signing_key.serialize_pem().as_bytes(),
        )
        .unwrap();
        let acceptor = TokioTlsAcceptor::from(TlsAcceptor::builder(identity).build().unwrap());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tauri::async_runtime::spawn(async move {
            for attempt in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                match acceptor.accept(stream).await {
                    Ok(mut stream) => {
                        stream.write_all(b"__PORTMATE_TLS_OK__\\n").await.unwrap();
                        return;
                    }
                    Err(error) if attempt == 0 => {
                        eprintln!("expected first TLS certificate failure: {error}");
                    }
                    Err(error) => panic!("TLS server handshake failed: {error}"),
                }
            }
            panic!("TLS client did not complete the allowed handshake");
        });

        let root =
            std::env::temp_dir().join(format!("portmate-telnet-tls-test-{}", Uuid::new_v4()));
        let mut rejected = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            tls_enabled: true,
            tls_server_name: Some("localhost".to_string()),
            ..Default::default()
        }));
        let rejected_state = test_app_state(rejected.clone(), root.join("rejected.sqlite3"));
        let error = open_tcp_session(&rejected_state, rejected.clone())
            .await
            .expect_err("untrusted TLS certificate should fail closed");
        assert!(
            error.contains("TLS 握手失败"),
            "unexpected TLS error: {error}"
        );

        rejected.connection = ConnectionConfig::Telnet(TcpConnection {
            tls_accept_invalid_cert: true,
            ..match rejected.connection {
                ConnectionConfig::Telnet(tcp) => tcp,
                _ => unreachable!(),
            }
        });
        let accepted_state = test_app_state(rejected.clone(), root.join("accepted.sqlite3"));
        open_tcp_session(&accepted_state, rejected.clone())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if accepted_state
                    .store
                    .lock()
                    .unwrap()
                    .screen(&rejected.id)
                    .is_some_and(|screen| screen.contains("__PORTMATE_TLS_OK__"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("TLS session did not receive server output");
        close_session_inner(&accepted_state, rejected.id.clone())
            .await
            .unwrap();
        server.await.unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn tcp_loopback_reconnects_after_remote_disconnect() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (drop_first_tx, drop_first_rx) = tokio::sync::oneshot::channel();
        let (second_connected_tx, second_connected_rx) = tokio::sync::oneshot::channel();
        let (release_server_tx, release_server_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await.unwrap();
            let _ = drop_first_rx.await;
            drop(first);
            let (mut second, _) = listener.accept().await.unwrap();
            second.write_all(b"new generation\n").await.unwrap();
            let _ = second_connected_tx.send(());
            let _ = release_server_rx.await;
            drop(second);
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: true,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!("portmate-tcp-test-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

        let opened = open_tcp_session(&state, profile.clone()).await.unwrap();
        assert_eq!(opened.runtime.status, SessionStatus::Connected);
        let first_runtime_id = state
            .tcp
            .lock()
            .unwrap()
            .get(&profile.id)
            .unwrap()
            .runtime_id
            .clone();
        let io = state.session_io();
        set_active_command(&io, &profile.id, "stale-command-id");
        let _ = drop_first_tx.send(());

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
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
                if status == SessionStatus::Reconnecting {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TCP runtime never entered reconnecting state");
        assert!(active_command_id(&io, &profile.id).is_none());

        tokio::time::timeout(Duration::from_secs(3), second_connected_rx)
            .await
            .expect("TCP runtime did not reconnect")
            .expect("TCP mock server dropped reconnect signal");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let connected = {
                    let store = state.store.lock().unwrap();
                    store
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == profile.id)
                        .is_some_and(|summary| summary.runtime.status == SessionStatus::Connected)
                };
                if connected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TCP runtime did not return to connected state");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let reconnected_output = state
                    .store
                    .lock()
                    .unwrap()
                    .events
                    .iter()
                    .find(|event| event.text.as_deref() == Some("new generation\n"))
                    .cloned();
                if let Some(event) = reconnected_output {
                    assert!(!event.annotations.contains_key("commandId"));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("reconnected TCP output was not recorded");

        let runtime = state.tcp.lock().unwrap().remove(&profile.id).unwrap();
        assert_ne!(runtime.runtime_id, first_runtime_id);
        runtime.closed.store(true, Ordering::SeqCst);
        runtime.writer.lock().await.shutdown().await.unwrap();
        let _ = release_server_tx.send(());
        server.await.unwrap();

        let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
        assert!(screen.contains("socket closed; reconnecting"));
        assert!(screen.contains("socket reconnected"));
        let _ = fs::remove_dir_all(root);
    });
}

#[cfg(target_os = "linux")]
#[test]
fn tcp_read_failure_preserves_the_socket_error_as_the_disconnect_reason() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (reset_tx, reset_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = reset_rx.await;
            SockRef::from(&socket)
                .set_linger(Some(Duration::ZERO))
                .unwrap();
            drop(socket);
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root =
            std::env::temp_dir().join(format!("portmate-tcp-read-error-test-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

        open_tcp_session(&state, profile.clone()).await.unwrap();
        let _ = reset_tx.send(());
        server.await.unwrap();
        let reason = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let runtime = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .map(|summary| summary.runtime);
                if let Some(runtime) =
                    runtime.filter(|runtime| runtime.status == SessionStatus::Disconnected)
                {
                    break runtime.last_disconnect_reason;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TCP read failure did not transition to disconnected");

        assert!(
            reason
                .as_deref()
                .is_some_and(|reason| reason.contains("TCP read failed")),
            "unexpected TCP disconnect reason: {reason:?}"
        );
        let read_error_events = state
            .store
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| {
                event
                    .text
                    .as_deref()
                    .is_some_and(|text| text.contains("TCP read failed"))
            })
            .count();
        assert_eq!(read_error_events, 1);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn tcp_reconnect_uses_latest_endpoint_and_stops_when_disabled() {
    tauri::async_runtime::block_on(async {
        let first_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let first_address = first_listener.local_addr().unwrap();
        let first_server = tokio::spawn(async move {
            let (socket, _) = first_listener.accept().await.unwrap();
            drop(first_listener);
            drop(socket);
        });

        let replacement_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let replacement_address = replacement_listener.local_addr().unwrap();
        let (replacement_connected_tx, replacement_connected_rx) = tokio::sync::oneshot::channel();
        let (drop_replacement_tx, drop_replacement_rx) = tokio::sync::oneshot::channel();
        let replacement_server = tokio::spawn(async move {
            let (socket, _) = replacement_listener.accept().await.unwrap();
            drop(replacement_listener);
            let _ = replacement_connected_tx.send(());
            let _ = drop_replacement_rx.await;
            drop(socket);
        });
        let (proxy_port, proxy_connections, proxy_task) = spawn_test_http_connect_proxy(200).await;

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: first_address.port(),
            reconnect: true,
            reconnect_delay_ms: 5_000,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!(
            "portmate-tcp-latest-reconnect-test-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();
        first_server.await.unwrap();

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let reconnecting = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                    runtime.session_id == profile.id
                        && runtime.status == SessionStatus::Reconnecting
                });
                if reconnecting {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TCP runtime never entered reconnecting state before profile update");

        {
            let mut store = state.store.lock().unwrap();
            let mut updated = store.profile(&profile.id).unwrap();
            updated.connection = ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: replacement_address.port(),
                reconnect: true,
                reconnect_delay_ms: 100,
                proxy: ProxyConfig {
                    enabled: true,
                    kind: ProxyKind::HttpConnect,
                    host: "127.0.0.1".to_string(),
                    port: proxy_port,
                    ..ProxyConfig::default()
                },
                ..Default::default()
            });
            store.upsert_profile(updated);
            save_store(&state.store_path, &store).unwrap();
        }

        tokio::time::timeout(Duration::from_millis(800), replacement_connected_rx)
            .await
            .expect("TCP reconnect did not use the updated endpoint and shorter delay")
            .expect("replacement TCP server dropped its connection signal");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let connected = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                    runtime.session_id == profile.id && runtime.status == SessionStatus::Connected
                });
                if connected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TCP runtime did not commit the updated endpoint connection");

        let _ = drop_replacement_tx.send(());
        replacement_server.await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let reconnecting = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                    runtime.session_id == profile.id
                        && runtime.status == SessionStatus::Reconnecting
                });
                if reconnecting {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("updated TCP runtime never re-entered reconnecting state");
        let second_disconnect_at = state
            .store
            .lock()
            .unwrap()
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == profile.id)
            .and_then(|runtime| runtime.last_disconnect)
            .expect("second TCP outage did not record its disconnect time");

        {
            let mut store = state.store.lock().unwrap();
            let mut disabled = store.profile(&profile.id).unwrap();
            if let ConnectionConfig::Tcp(tcp) = &mut disabled.connection {
                tcp.reconnect = false;
            }
            store.upsert_profile(disabled);
            save_store(&state.store_path, &store).unwrap();
        }

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let runtime_removed = !state.tcp.lock().unwrap().contains_key(&profile.id);
                let disconnected = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                    runtime.session_id == profile.id
                        && runtime.status == SessionStatus::Disconnected
                        && runtime
                            .last_disconnect_reason
                            .as_deref()
                            .is_some_and(|reason| reason.contains("disabled"))
                });
                if runtime_removed && disconnected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("disabling TCP reconnect did not remove the pending runtime");
        let stopped_runtime = state
            .store
            .lock()
            .unwrap()
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == profile.id)
            .cloned()
            .expect("stopped TCP runtime summary is missing");
        assert_eq!(stopped_runtime.last_disconnect, Some(second_disconnect_at));

        let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
        assert!(screen.contains("reconnecting in 5000ms"));
        assert!(screen.contains("socket reconnected"));
        assert!(screen.contains("reconnect stopped"));
        assert!(proxy_connections.load(Ordering::SeqCst) >= 1);
        proxy_task.abort();
        let _ = proxy_task.await;
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn tcp_reconnect_store_commit_failure_does_not_install_runtime() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (drop_first_tx, drop_first_rx) = tokio::sync::oneshot::channel();
        let (second_connected_tx, second_connected_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (first, _) = listener.accept().await.unwrap();
            let _ = drop_first_rx.await;
            drop(first);

            let (mut second, _) = listener.accept().await.unwrap();
            let _ = second_connected_tx.send(());
            let mut byte = [0_u8; 1];
            tokio::time::timeout(Duration::from_secs(5), second.read(&mut byte))
                .await
                .expect("failed reconnect socket was not closed")
                .unwrap()
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: true,
            reconnect_delay_ms: 100,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!(
            "portmate-tcp-reconnect-commit-failure-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        fs::remove_dir_all(&root).unwrap();
        fs::write(&root, b"blocked").unwrap();
        let _ = drop_first_tx.send(());
        tokio::time::timeout(Duration::from_secs(5), second_connected_rx)
            .await
            .expect("TCP reconnect did not reach the replacement socket")
            .unwrap();

        let failed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let summary = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .unwrap();
                if summary.runtime.status == SessionStatus::Error {
                    break summary;
                }
                assert_ne!(
                    summary.runtime.status,
                    SessionStatus::Connected,
                    "failed TCP reconnect exposed a connected runtime"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TCP reconnect Store failure did not settle to Error");

        assert!(
            failed
                .runtime
                .last_disconnect_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("TCP reconnect install failed")),
            "unexpected reconnect failure: {:?}",
            failed.runtime.last_disconnect_reason
        );
        assert!(!state.tcp.lock().unwrap().contains_key(&profile.id));
        assert_eq!(server.await.unwrap(), 0);
        assert!(state.store.lock().unwrap().events.iter().any(|event| {
            event
                .text
                .as_deref()
                .is_some_and(|text| text.contains("TCP reconnect install failed"))
        }));

        fs::remove_file(root).unwrap();
    });
}

#[test]
fn tcp_disconnect_observes_reconnect_disabled_while_connected() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = release_rx.await;
            drop(socket);
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: true,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!(
            "portmate-tcp-disable-connected-test-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        {
            let mut store = state.store.lock().unwrap();
            let mut disabled = store.profile(&profile.id).unwrap();
            if let ConnectionConfig::Tcp(tcp) = &mut disabled.connection {
                tcp.reconnect = false;
            }
            store.upsert_profile(disabled);
            save_store(&state.store_path, &store).unwrap();
        }
        let _ = release_tx.send(());
        server.await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let runtime_removed = !state.tcp.lock().unwrap().contains_key(&profile.id);
                let disconnected = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                    runtime.session_id == profile.id
                        && runtime.status == SessionStatus::Disconnected
                });
                if runtime_removed && disconnected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("TCP disconnect ignored the latest reconnect=false setting");
        let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
        assert!(!screen.contains("reconnecting in 1000ms"));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn tcp_loopback_round_trips_raw_bytes_without_telnet_escaping() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = [0_u8; 2];
            socket.read_exact(&mut raw).await.unwrap();
            assert_eq!(raw, [0x01, TELNET_IAC]);
        });

        let tcp = TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            ..Default::default()
        };
        let mut client = connect_tcp_socket(&tcp, "TCP").await.unwrap();
        client.write_all(&[0x01, TELNET_IAC]).await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("TCP loopback server timed out")
            .expect("TCP loopback server task failed");
    });
}

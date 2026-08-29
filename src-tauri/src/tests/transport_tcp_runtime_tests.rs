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

#[test]
fn tcp_close_does_not_wait_forever_for_an_occupied_writer() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (release_server_tx, release_server_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let _ = release_server_rx.await;
            drop(socket);
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!(
            "portmate-tcp-bounded-close-test-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let writer = Arc::clone(
            &state
                .tcp
                .lock()
                .unwrap()
                .get(&profile.id)
                .unwrap()
                .writer,
        );
        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let holder = tokio::spawn(async move {
            let _writer = writer.lock().await;
            let _ = locked_tx.send(());
            std::future::pending::<()>().await;
        });
        locked_rx.await.unwrap();

        let closed = tokio::time::timeout(
            crate::transport_timing::TCP_RUNTIME_SHUTDOWN_TIMEOUT + Duration::from_secs(1),
            close_session_inner(&state, profile.id.clone()),
        )
        .await
        .expect("TCP close exceeded its bounded writer shutdown deadline")
        .unwrap();
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
        assert!(!state.tcp.lock().unwrap().contains_key(&profile.id));

        holder.abort();
        let _ = holder.await;
        let _ = release_server_tx.send(());
        server.await.unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn tcp_write_deadline_includes_the_writer_lock_and_recovers_after_timeout() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut byte = [0_u8; 1];
            socket.read_exact(&mut byte).await.unwrap();
            byte[0]
        });
        let stream = TcpStream::connect(address).await.unwrap();
        let (_reader, writer) = stream.into_split();
        let writer = Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer)));
        let guard = writer.lock().await;

        let error = write_tcp_bytes_with_timeout(
            &writer,
            b"x",
            Duration::from_millis(30),
            "TCP test write",
        )
        .await
        .unwrap_err();
        assert_eq!(error, "TCP test write超时（30 ms）");
        drop(guard);

        write_tcp_bytes_with_timeout(
            &writer,
            b"y",
            Duration::from_secs(1),
            "TCP test write",
        )
        .await
        .unwrap();
        assert_eq!(server.await.unwrap(), b'y');
    });
}

#[test]
fn queued_terminal_input_bypasses_store_lock_and_preserves_control_boundaries() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (received_tx, received_rx) = tokio::sync::oneshot::channel();
        let (release_server_tx, release_server_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 4];
            socket.read_exact(&mut received).await.unwrap();
            let _ = received_tx.send(received);
            let _ = release_server_rx.await;
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!(
            "portmate-queued-input-store-lock-test-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let (store_locked_tx, store_locked_rx) = std::sync::mpsc::channel();
        let (release_store_tx, release_store_rx) = std::sync::mpsc::channel();
        let store = Arc::clone(&state.store);
        let store_holder = std::thread::spawn(move || {
            let _guard = store.lock().unwrap();
            store_locked_tx.send(()).unwrap();
            release_store_rx.recv().unwrap();
        });
        store_locked_rx.recv().unwrap();

        for (index, (text, coalesce)) in
            [("a", true), ("b", true), ("\r", false), ("c", true)]
                .into_iter()
                .enumerate()
        {
            enqueue_interactive_text(
                state.session_io(),
                profile.id.clone(),
                text.to_string(),
                coalesce,
            )
            .unwrap();
            if index < 2 {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
        }
        let received = tokio::time::timeout(Duration::from_secs(1), received_rx)
            .await
            .expect("queued input waited for the Store lock")
            .expect("queued input server dropped its response");
        assert_eq!(&received, b"ab\rc");

        release_store_tx.send(()).unwrap();
        store_holder.join().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let submitted = state
                    .store
                    .lock()
                    .unwrap()
                    .events
                    .iter()
                    .filter(|event| {
                        event.session_id == profile.id
                            && event.direction == EventDirection::Outbound
                            && event.stream == EventStream::Stdout
                    })
                    .filter_map(|event| event.text.as_deref())
                    .collect::<String>();
                if submitted == "ab\r" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("submitted terminal line was not persisted as one event");
        clear_interactive_write_queue(&state.store_path, &profile.id);
        clear_deferred_interactive_queue(&state.store_path, &profile.id);
        let persisted_input = state
            .store
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| {
                event.session_id == profile.id
                    && event.direction == EventDirection::Outbound
                    && event.stream == EventStream::Stdout
            })
            .filter_map(|event| event.text.clone())
            .collect::<Vec<_>>();
        assert_eq!(persisted_input, ["ab\r", "c"]);
        let _ = release_server_tx.send(());
        server.await.unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn paced_terminal_input_ack_waits_for_the_outbound_write_lane() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (received_tx, received_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = vec![0_u8; 5];
            socket.read_exact(&mut received).await.unwrap();
            let _ = received_tx.send(received);
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!(
            "portmate-paced-input-ack-test-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let lane = outbound_lane(&state.store_path, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let mut request = Box::pin(enqueue_interactive_text_and_wait(
            state.session_io(),
            profile.id.clone(),
            "paced".to_string(),
            false,
        ));
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut request)
            .await
            .is_err());
        drop(lane_guard);

        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), &mut request)
                .await
                .expect("paced input acknowledgement timed out")
                .expect("paced input write failed"),
            ()
        );
        let received = tokio::time::timeout(Duration::from_secs(1), received_rx)
            .await
            .expect("paced input server did not receive the payload")
            .expect("paced input server dropped its response");
        assert_eq!(received, b"paced");

        clear_interactive_write_queue(&state.store_path, &profile.id);
        clear_deferred_interactive_queue(&state.store_path, &profile.id);
        close_session_inner(&state, profile.id.clone()).await.unwrap();
        server.await.unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn timed_out_terminal_input_ack_cancels_a_request_waiting_for_the_lane() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut byte = [0_u8; 1];
            matches!(
                tokio::time::timeout(Duration::from_millis(250), socket.read_exact(&mut byte)).await,
                Ok(Ok(_))
            )
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!(
            "portmate-paced-input-timeout-test-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        let lane = outbound_lane(&state.store_path, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let error = enqueue_interactive_text_and_wait_with_timeout(
            state.session_io(),
            profile.id.clone(),
            "x".to_string(),
            false,
            Duration::from_millis(30),
        )
        .await
        .unwrap_err();
        assert!(error.contains("30 ms"));
        drop(lane_guard);

        assert!(!server.await.unwrap(), "timed-out queued input reached the transport");
        clear_interactive_write_queue(&state.store_path, &profile.id);
        clear_deferred_interactive_queue(&state.store_path, &profile.id);
        close_session_inner(&state, profile.id.clone()).await.unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn sensitive_terminal_input_reaches_transport_without_entering_logs() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 8];
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
        profile.logging.raw = true;
        profile.logging.text = true;
        profile.logging.jsonl = true;
        let root = std::env::temp_dir().join(format!(
            "portmate-private-input-test-{}",
            Uuid::new_v4()
        ));
        let store_path = root.join("portmate-store.sqlite3");
        let state = test_app_state(profile.clone(), store_path.clone());
        open_tcp_session(&state, profile.clone()).await.unwrap();

        enqueue_interactive_text_and_wait_with_sensitivity(
            state.session_io(),
            profile.id.clone(),
            "hunter2\r".to_string(),
            false,
            true,
        )
        .await
        .unwrap();
        assert_eq!(&server.await.unwrap(), b"hunter2\r");
        clear_deferred_interactive_queue(&state.store_path, &profile.id);

        {
            let store = state.store.lock().unwrap();
            let private_event = store
                .events
                .iter()
                .find(|event| {
                    event
                        .annotations
                        .get("sensitive")
                        .is_some_and(|value| value == "true")
                })
                .expect("private input did not produce its redacted control event");
            assert_eq!(private_event.text.as_deref(), Some("<private-input>"));
            assert!(private_event.bytes_ref.is_none());
            assert_eq!(
                private_event.annotations.get("wireBytes").map(String::as_str),
                Some("8")
            );
            assert!(!serde_json::to_string(&*store).unwrap().contains("hunter2"));
        }
        for extension in ["raw", "txt", "jsonl"] {
            let path = log_shard_path(&store_path, &profile, extension).unwrap();
            if path.is_file() {
                assert!(!fs::read(&path).unwrap().windows(7).any(|bytes| bytes == b"hunter2"));
            }
        }

        clear_interactive_write_queue(&state.store_path, &profile.id);
        close_session_inner(&state, profile.id.clone()).await.unwrap();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn outbound_lane_deadline_recovers_after_timeout() {
    tauri::async_runtime::block_on(async {
        let store_path = canonical_test_temp_path("portmate-outbound-lane-timeout")
            .join("portmate-store.sqlite3");
        let session_id = Uuid::new_v4().to_string();
        let lane = outbound_lane(&store_path, &session_id).unwrap();
        let guard = lane.lock().await;

        let error = acquire_outbound_lane_with_timeout(
            &store_path,
            &session_id,
            Duration::from_millis(30),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "出站队列等待超时（30 ms）");
        drop(guard);

        let recovered = acquire_outbound_lane_with_timeout(
            &store_path,
            &session_id,
            Duration::from_secs(1),
        )
        .await
        .expect("outbound lane did not recover after a timed-out waiter");
        drop(recovered);
        clear_outbound_lane(&store_path, &session_id);
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

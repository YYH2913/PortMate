#[test]
fn destroyed_window_cannot_release_pending_input_into_a_shared_session() {
    use crate::terminal_input_stream::{
        accept_text, begin_stream, clear_owner_streams, TerminalInputOrder,
    };
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (prefix_tx, prefix_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut prefix = [0_u8; 4];
            socket.read_exact(&mut prefix).await.unwrap();
            let _ = prefix_tx.send(prefix);
            let mut remaining = Vec::new();
            socket.read_to_end(&mut remaining).await.unwrap();
            remaining
        });
        let root = tempfile::tempdir().unwrap();
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".into(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();
        let io = state.session_io();
        let main = begin_stream(&io, &profile.id, "main").unwrap();
        let detached = begin_stream(&io, &profile.id, "detached").unwrap();
        accept_text(
            io.clone(), profile.id.clone(), "detached",
            TerminalInputOrder { stream_id: detached.stream_id.clone(), sequence: 1 },
            "pending-private-input".into(), true, true,
        ).unwrap();

        clear_owner_streams(&state.store_path, "detached");
        clear_owner_streams(&state.store_path, "detached");
        assert!(accept_text(
            io.clone(), profile.id.clone(), "detached",
            TerminalInputOrder { stream_id: detached.stream_id, sequence: 0 },
            "late-prefix".into(), true, true,
        ).is_err());
        accept_text(
            io, profile.id.clone(), "main",
            TerminalInputOrder { stream_id: main.stream_id, sequence: 0 },
            "kept".into(), true, false,
        ).unwrap();
        assert_eq!(tokio::time::timeout(Duration::from_secs(2), prefix_rx).await.unwrap().unwrap(), *b"kept");
        close_session_inner(&state, profile.id).await.unwrap();
        let remaining = tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
        assert!(remaining.is_empty(), "destroyed window leaked buffered bytes: {remaining:?}");
    });
}

#[test]
fn terminal_input_stream_orders_real_writes_and_fences_reconnected_runtimes() {
    use crate::terminal_input_stream::{
        accept_text, begin_stream, close_stream, TerminalInputOrder,
    };
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let (received_tx, received_rx) = tokio::sync::oneshot::channel();
        let (prefix_tx, prefix_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 5];
            socket.read_exact(&mut received[..4]).await.unwrap();
            let _ = prefix_tx.send(());
            socket.read_exact(&mut received[4..]).await.unwrap();
            let _ = received_tx.send(received);
            let _ = release_rx.await;
        });
        let root = tempfile::tempdir().unwrap();
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".into(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();
        let io = state.session_io();
        let binding = begin_stream(&io, &profile.id, "main").unwrap();
        let order = |sequence| TerminalInputOrder {
            stream_id: binding.stream_id.clone(),
            sequence,
        };
        assert!(accept_text(
            io.clone(),
            profile.id.clone(),
            "other-window",
            order(0),
            "wrong".into(),
            true,
            false
        )
        .is_err());
        for (sequence, text) in [(1, "\x7f"), (3, "\r"), (0, "a"), (2, "b")] {
            accept_text(
                io.clone(),
                profile.id.clone(),
                "main",
                order(sequence),
                text.into(),
                false,
                false,
            )
            .unwrap();
        }
        // Wait for transport completion before replacing its generation.
        tokio::time::timeout(Duration::from_secs(2), prefix_rx)
            .await
            .expect("ordered prefix did not reach the socket")
            .unwrap();
        // Simulate an automatic reconnect between IPC admission and writing.
        state
            .tcp
            .lock()
            .unwrap()
            .get_mut(&profile.id)
            .unwrap()
            .runtime_id = Uuid::new_v4().to_string();
        assert!(accept_text(
            io.clone(),
            profile.id.clone(),
            "main",
            order(4),
            "stale".into(),
            false,
            false
        )
        .unwrap_err()
        .contains("连接已变化"));
        let replacement = begin_stream(&io, &profile.id, "main").unwrap();
        close_stream(&io, &profile.id, "main", &binding.stream_id);
        assert!(accept_text(
            io.clone(),
            profile.id.clone(),
            "main",
            order(5),
            "stale".into(),
            false,
            false
        )
        .is_err());
        accept_text(
            io.clone(),
            profile.id.clone(),
            "main",
            TerminalInputOrder {
                stream_id: replacement.stream_id.clone(),
                sequence: 0,
            },
            "z".into(),
            false,
            false,
        )
        .unwrap();
        let received = tokio::time::timeout(Duration::from_secs(2), received_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&received, b"a\x7fb\rz");
        close_stream(&io, &profile.id, "main", &replacement.stream_id);
        assert!(accept_text(
            io,
            profile.id.clone(),
            "main",
            TerminalInputOrder {
                stream_id: replacement.stream_id,
                sequence: 1
            },
            "closed".into(),
            false,
            false
        )
        .is_err());
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        let _ = release_tx.send(());
        server.await.unwrap();
    });
}

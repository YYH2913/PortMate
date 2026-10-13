use super::*;
use std::pin::Pin;
use std::task::{Context, Poll};

async fn connected_input_fixture() -> (tempfile::TempDir, AppState, String, TcpStream, String) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let profile = SessionProfile {
        id: "native-input-generation".into(),
        name: "Native input generation".into(),
        kind: SessionKind::Tcp,
        group: String::new(),
        tags: Vec::new(),
        connection: ConnectionConfig::Tcp(TcpConnection {
            host: "127.0.0.1".into(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }),
        terminal: Default::default(),
        logging: Default::default(),
        triggers: Vec::new(),
        transfer: Default::default(),
    };
    let id = profile.id.clone();
    let state = crate::tests::test_app_state(
        profile.clone(),
        root.path().canonicalize().unwrap().join("store.sqlite3"),
    );
    open_tcp_session(&state, profile).await.unwrap();
    let (peer, _) = listener.accept().await.unwrap();
    let timestamp = state
        .store
        .lock()
        .unwrap()
        .runtimes
        .iter()
        .find(|runtime| runtime.session_id == id)
        .unwrap()
        .connected_since
        .unwrap()
        .to_rfc3339();
    (root, state, id, peer, timestamp)
}

fn replace_input_generation(io: &SessionIo, id: &str) -> String {
    let mut connections = io.runtimes.tcp.lock().unwrap();
    let mut store = io.store.lock().unwrap();
    connections.get_mut(id).unwrap().runtime_id = Uuid::new_v4().to_string();
    let runtime = store
        .runtimes
        .iter_mut()
        .find(|runtime| runtime.session_id == id)
        .unwrap();
    let next = runtime.connected_since.unwrap() + chrono::Duration::seconds(1);
    runtime.connected_since = Some(next);
    next.to_rfc3339()
}

fn input_options(timestamp: &str, queued: bool, await_write: bool) -> DesktopTextSendOptions {
    DesktopTextSendOptions {
        queued: Some(queued),
        await_write: Some(await_write),
        expected_connected_since: Some(timestamp.to_string()),
        ..Default::default()
    }
}

fn assert_no_input_event(io: &SessionIo, id: &str) {
    assert!(io
        .store
        .lock()
        .unwrap()
        .events
        .iter()
        .all(|event| event.session_id != id || event.direction != EventDirection::Outbound));
}

#[test]
fn desktop_late_text_and_bytes_reject_the_old_canonical_connected_since() {
    tauri::async_runtime::block_on(async {
        let (_root, state, id, mut peer, old) = connected_input_fixture().await;
        let io = state.session_io();
        let current = replace_input_generation(&io, &id);
        for queued in [false, true] {
            assert!(send_desktop_text(
                io.clone(),
                "detached",
                id.clone(),
                "late-text".into(),
                input_options(&old, queued, queued),
            )
            .await
            .unwrap_err()
            .contains("连接已变化"));
            assert!(send_desktop_bytes(
                io.clone(),
                "detached",
                id.clone(),
                b"late-bytes".to_vec(),
                Some(queued),
                None,
                Some(old.clone()),
            )
            .await
            .unwrap_err()
            .contains("连接已变化"));
        }
        assert_no_input_event(&io, &id);
        send_desktop_text(
            io.clone(),
            "detached",
            id.clone(),
            "new-text".into(),
            input_options(&current, true, true),
        )
        .await
        .unwrap();
        send_desktop_bytes(
            io.clone(),
            "detached",
            id.clone(),
            b"new-bytes".to_vec(),
            Some(false),
            None,
            Some(current),
        )
        .await
        .unwrap();
        let mut received = [0; 17];
        tokio::time::timeout(Duration::from_secs(2), peer.read_exact(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&received, b"new-textnew-bytes");
        clear_deferred_interactive_queue(&io.store_path, &id);
        crate::session_close::close_session_inner(&state, id)
            .await
            .unwrap();
        let mut remainder = Vec::new();
        peer.read_to_end(&mut remainder).await.unwrap();
        assert!(
            remainder.is_empty(),
            "late IPC input reached the new runtime"
        );
    });
}

#[test]
fn desktop_queued_text_and_bytes_do_not_rebind_after_admission() {
    tauri::async_runtime::block_on(async {
        let (_root, state, id, mut peer, old) = connected_input_fixture().await;
        let io = state.session_io();
        let lane = outbound_lane(&io.store_path, &id).unwrap();
        let guard = lane.lock().await;
        send_desktop_text(
            io.clone(),
            "detached",
            id.clone(),
            "old-queued-text".into(),
            input_options(&old, true, false),
        )
        .await
        .unwrap();
        send_desktop_bytes(
            io.clone(),
            "detached",
            id.clone(),
            b"old-queued-bytes".to_vec(),
            Some(true),
            None,
            Some(old),
        )
        .await
        .unwrap();
        let current = replace_input_generation(&io, &id);
        drop(guard);
        send_desktop_text(
            io.clone(),
            "detached",
            id.clone(),
            "fresh".into(),
            input_options(&current, true, true),
        )
        .await
        .unwrap();
        let mut received = [0; 5];
        tokio::time::timeout(Duration::from_secs(2), peer.read_exact(&mut received))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&received, b"fresh");
        clear_deferred_interactive_queue(&io.store_path, &id);
        assert!(io.store.lock().unwrap().events.iter().all(|event| {
            event.session_id != id
                || event
                    .text
                    .as_deref()
                    .is_none_or(|text| !text.contains("old-queued"))
        }));
        crate::session_close::close_session_inner(&state, id)
            .await
            .unwrap();
        let mut remainder = Vec::new();
        peer.read_to_end(&mut remainder).await.unwrap();
        assert!(remainder.is_empty());
    });
}

#[test]
fn desktop_direct_and_confirmed_input_keep_the_captured_runtime_while_waiting() {
    tauri::async_runtime::block_on(async {
        for (binary, queued) in [(false, false), (true, false), (false, true)] {
            let (_root, state, id, mut peer, old) = connected_input_fixture().await;
            let io = state.session_io();
            let lane = outbound_lane(&io.store_path, &id).unwrap();
            let guard = lane.lock().await;
            let mut request = Box::pin(async {
                if binary {
                    send_desktop_bytes(
                        io.clone(),
                        "detached",
                        id.clone(),
                        b"old".to_vec(),
                        Some(false),
                        None,
                        Some(old.clone()),
                    )
                    .await
                    .map(|_| ())
                } else {
                    send_desktop_text(
                        io.clone(),
                        "detached",
                        id.clone(),
                        "old".into(),
                        input_options(&old, queued, queued),
                    )
                    .await
                    .map(|_| ())
                }
            });
            assert!(
                tokio::time::timeout(Duration::from_millis(30), &mut request)
                    .await
                    .is_err()
            );
            let current = replace_input_generation(&io, &id);
            drop(guard);
            assert!(tokio::time::timeout(Duration::from_secs(2), request)
                .await
                .unwrap()
                .is_err());
            assert_no_input_event(&io, &id);
            send_desktop_bytes(
                io.clone(),
                "detached",
                id.clone(),
                b"new".to_vec(),
                None,
                None,
                Some(current),
            )
            .await
            .unwrap();
            let mut received = [0; 3];
            peer.read_exact(&mut received).await.unwrap();
            assert_eq!(&received, b"new");
            crate::session_close::close_session_inner(&state, id)
                .await
                .unwrap();
            let mut remainder = Vec::new();
            peer.read_to_end(&mut remainder).await.unwrap();
            assert!(remainder.is_empty());
        }
    });
}

struct ReplaceGenerationOnWrite {
    writer: TcpWriteHalf,
    replace: Option<Box<dyn FnOnce() + Send>>,
}

impl AsyncWrite for ReplaceGenerationOnWrite {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if let Some(replace) = self.replace.take() {
            replace();
        }
        Pin::new(&mut self.writer).poll_write(context, bytes)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.writer).poll_shutdown(context)
    }
}

#[test]
fn desktop_old_write_completion_is_not_a_success_after_runtime_replacement() {
    tauri::async_runtime::block_on(async {
        for (binary, queued) in [(false, false), (true, false), (false, true)] {
            let (_root, state, id, mut peer, old) = connected_input_fixture().await;
            let io = state.session_io();
            let writer = Arc::clone(&io.runtimes.tcp.lock().unwrap().get(&id).unwrap().writer);
            {
                let mut target = writer.lock().await;
                let original = std::mem::replace(&mut *target, Box::new(tokio::io::sink()));
                let replaced_io = io.clone();
                let replaced_id = id.clone();
                *target = Box::new(ReplaceGenerationOnWrite {
                    writer: original,
                    replace: Some(Box::new(move || {
                        replace_input_generation(&replaced_io, &replaced_id);
                        set_active_command(&replaced_io, &replaced_id, "new-command");
                    })),
                });
            }
            let result = if binary {
                send_desktop_bytes(
                    io.clone(),
                    "detached",
                    id.clone(),
                    b"old".to_vec(),
                    None,
                    None,
                    Some(old),
                )
                .await
                .map(|_| ())
            } else {
                send_desktop_text(
                    io.clone(),
                    "detached",
                    id.clone(),
                    "old".into(),
                    input_options(&old, queued, queued),
                )
                .await
                .map(|_| ())
            };
            assert!(result.unwrap_err().contains("未确认旧连接"));
            assert_eq!(active_command_id(&io, &id).as_deref(), Some("new-command"));
            assert_no_input_event(&io, &id);
            let mut written = [0; 3];
            peer.read_exact(&mut written).await.unwrap();
            assert_eq!(
                &written, b"old",
                "an already started write is not reversible"
            );
            crate::session_close::close_session_inner(&state, id)
                .await
                .unwrap();
        }
    });
}

#[test]
fn desktop_input_generation_rejects_malformed_disconnected_and_missing_timestamps() {
    tauri::async_runtime::block_on(async {
        let (_root, state, id, _peer, timestamp) = connected_input_fixture().await;
        let io = state.session_io();
        assert!(capture_desktop_input_runtime(&io, &id, Some("")).is_err());
        assert!(capture_desktop_input_runtime(&io, &id, Some("invalid")).is_err());
        assert_eq!(capture_desktop_input_runtime(&io, &id, None).unwrap(), None);
        let captured = capture_desktop_input_runtime(&io, &id, Some(&timestamp)).unwrap();
        assert_eq!(
            captured,
            current_session_runtime_id(&io.runtimes, &id).unwrap()
        );
        {
            let mut store = io.store.lock().unwrap();
            let runtime = store
                .runtimes
                .iter_mut()
                .find(|runtime| runtime.session_id == id)
                .unwrap();
            runtime.status = SessionStatus::Disconnected;
        }
        assert!(capture_desktop_input_runtime(&io, &id, Some(&timestamp)).is_err());
        {
            let mut store = io.store.lock().unwrap();
            let runtime = store
                .runtimes
                .iter_mut()
                .find(|runtime| runtime.session_id == id)
                .unwrap();
            runtime.status = SessionStatus::Connected;
            runtime.connected_since = None;
        }
        assert!(capture_desktop_input_runtime(&io, &id, Some(&timestamp)).is_err());
        crate::session_close::close_session_inner(&state, id)
            .await
            .unwrap();
    });
}

use super::*;

async fn fixture() -> (tempfile::TempDir, AppState, String, TcpStream) {
    let root = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
        host: "127.0.0.1".into(),
        port: address.port(),
        reconnect: false,
        ..Default::default()
    }));
    let id = profile.id.clone();
    let state = test_app_state(profile, root.path().join("store.sqlite3"));
    let stream = TcpStream::connect(address).await.unwrap();
    let (peer, _) = listener.accept().await.unwrap();
    let (_reader, writer) = stream.into_split();
    let (tap, _) = broadcast::channel(8);
    state.tcp.lock().unwrap().insert(
        id.clone(),
        TcpRuntime {
            runtime_id: Uuid::new_v4().to_string(),
            writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
            tap,
            closed: Arc::new(AtomicBool::new(false)),
            telnet: None,
        },
    );
    (root, state, id, peer)
}

#[test]
fn paced_send_pins_connection_and_owner_and_cleans_up_only_matching_jobs() {
    tauri::async_runtime::block_on(async {
        let (_root, state, id, _peer) = fixture().await;
        let io = state.session_io();
        let first = paced_send::begin_job(&io, "main", vec![id.clone()]).unwrap();
        let second = paced_send::begin_job(&io, "other", vec![id.clone()]).unwrap();
        assert!(paced_send::target(&io, "other", &first, &id).is_err());
        assert!(paced_send::target(&io, "main", &first, "unknown").is_err());
        paced_send::cancel_job(&io.store_path, "other", &first);
        assert!(paced_send::target(&io, "main", &first, &id).is_ok());
        paced_send::clear_owner(&io.store_path, "main");
        assert!(paced_send::target(&io, "main", &first, &id).is_err());
        assert!(paced_send::target(&io, "other", &second, &id).is_ok());
        state.tcp.lock().unwrap().get_mut(&id).unwrap().runtime_id = Uuid::new_v4().to_string();
        assert!(paced_send::target(&io, "other", &second, &id)
            .err()
            .unwrap()
            .contains("连接已变化"));
        paced_send::cancel_job(&io.store_path, "other", &second);
        let third = paced_send::begin_job(&io, "main", vec![id.clone()]).unwrap();
        clear_interactive_write_queue(&io.store_path, &id);
        assert!(paced_send::target(&io, "main", &third, &id).is_err());
    });
}

#[test]
fn paced_send_cancel_discards_text_and_hex_waiting_in_native_lane_or_writer() {
    tauri::async_runtime::block_on(async {
        for (payload, wait_on_writer) in [
            (b"old text".to_vec(), false),
            (vec![0, 0xff, 0x01], false),
            (b"old text".to_vec(), true),
            (vec![0, 0xff, 0x01], true),
        ] {
            let (_root, state, id, mut peer) = fixture().await;
            let io = state.session_io();
            let job_id = paced_send::begin_job(&io, "main", vec![id.clone()]).unwrap();
            let (job, runtime) = paced_send::target(&io, "main", &job_id, &id).unwrap();
            let lane = outbound_lane(&io.store_path, &id).unwrap();
            let lock = if wait_on_writer {
                None
            } else {
                Some(lane.lock().await)
            };
            let writer = Arc::clone(&state.tcp.lock().unwrap().get(&id).unwrap().writer);
            let writer_lock = if wait_on_writer {
                Some(writer.lock().await)
            } else {
                None
            };
            let pending_io = io.clone();
            let pending_id = id.clone();
            let pending = tokio::spawn(async move {
                enqueue_paced_payload_and_wait(
                    pending_io,
                    pending_id,
                    "old".into(),
                    payload,
                    job,
                    runtime,
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(1), async {
                while Arc::strong_count(&lane) < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            paced_send::cancel_job(&io.store_path, "main", &job_id);
            assert!(tokio::time::timeout(Duration::from_secs(1), pending)
                .await
                .unwrap()
                .unwrap()
                .is_err());
            drop(lock);
            drop(writer_lock);
            let next_id = paced_send::begin_job(&io, "main", vec![id.clone()]).unwrap();
            let (job, runtime) = paced_send::target(&io, "main", &next_id, &id).unwrap();
            enqueue_paced_payload_and_wait(
                io.clone(),
                id.clone(),
                "new".into(),
                b"new".to_vec(),
                job,
                runtime,
            )
            .await
            .unwrap();
            let mut received = [0; 3];
            tokio::time::timeout(Duration::from_secs(1), peer.read_exact(&mut received))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(&received, b"new", "cancelled bytes reached the socket");
            paced_send::cancel_job(&io.store_path, "main", &next_id);
            clear_interactive_write_queue(&io.store_path, &id);
        }
    });
}

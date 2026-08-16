use super::*;

#[tokio::test]
async fn delete_session_profile_persists_and_cleans_runtime_state() {
    let temp = tempfile::tempdir().unwrap();
    let store_path = temp.path().join("portmate-store.sqlite3");
    let state = test_app_state(test_shell_profile(), store_path.clone());
    let transfer_id = "completed-transfer".to_string();
    {
        let mut store = state.store.lock().unwrap();
        store
            .record_stream_event(
                "session:1",
                EventDirection::Inbound,
                EventStream::Stdout,
                "ephemeral output",
            )
            .unwrap();
        store.record_transfer(TransferTask {
            id: transfer_id.clone(),
            session_id: "session:1".to_string(),
            protocol: TransferProtocol::Sftp,
            source: "source".to_string(),
            destination: "destination".to_string(),
            bytes_total: 1,
            bytes_done: 1,
            status: TransferStatus::Completed,
            message: None,
            started_at: Some(Utc::now()),
            finished_at: Some(Utc::now()),
            average_bytes_per_second: None,
        });
    }
    state.transfer_lanes.lock().unwrap().insert(
        "session:1".to_string(),
        Arc::new(tokio::sync::Mutex::new(())),
    );
    state
        .transfer_cancellations
        .lock()
        .unwrap()
        .insert(transfer_id.clone(), Arc::new(TransferCancellation::new()));
    state
        .one_time_host_keys
        .lock()
        .unwrap()
        .insert("session:1".to_string(), Vec::new());
    let (approval_sender, approval_receiver) = tokio::sync::oneshot::channel();
    let approval_id = Uuid::new_v4().to_string();
    state.pending_mcp_approvals.lock().unwrap().insert(
        approval_id.clone(),
        PendingMcpApproval {
            request: McpApprovalRequest {
                id: approval_id,
                client_id: "test-client".to_string(),
                action: "close_session".to_string(),
                session_id: "session:1".to_string(),
                scope: "manage-sessions".to_string(),
                target: None,
                created_at: Utc::now(),
                expires_at: Utc::now() + chrono::Duration::seconds(60),
            },
            response: approval_sender,
        },
    );

    let response = delete_session_profile_inner(&state, "session:1".to_string())
        .await
        .unwrap();

    assert_eq!(response.deleted_profile_id, "session:1");
    assert!(response.sessions.is_empty());
    assert!(!approval_receiver.await.unwrap());
    assert!(!state
        .transfer_lanes
        .lock()
        .unwrap()
        .contains_key("session:1"));
    assert!(!state
        .transfer_cancellations
        .lock()
        .unwrap()
        .contains_key(&transfer_id));
    assert!(!state
        .one_time_host_keys
        .lock()
        .unwrap()
        .contains_key("session:1"));
    assert!(state.pending_mcp_approvals.lock().unwrap().is_empty());

    let stored = load_store(&store_path).unwrap();
    assert!(stored.profile("session:1").is_none());
    assert!(stored.events.is_empty());
    assert!(stored.transfers.is_empty());
    assert!(stored.audit.iter().any(|record| {
        record.action == "delete_session_profile"
            && record.session_id.as_deref() == Some("session:1")
            && record.details.get("diskLogs").map(String::as_str) == Some("retained")
    }));
}

#[test]
fn session_lifecycle_lane_serializes_open_and_close() {
    let _runtime_guard = shared_runtime_test_guard();
    tauri::async_runtime::block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));
        let lane = session_lifecycle_lane(&state, &profile.id).unwrap();
        let guard = lane.lock().await;
        let opening_state = state.clone();
        let opening_session_id = profile.id.clone();
        let mut opening = tokio::spawn(async move {
            open_session_inner(
                opening_state,
                opening_session_id,
                SessionOpenCredentials::default(),
            )
            .await
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut opening)
                .await
                .is_err(),
            "open_session crossed an occupied lifecycle lane"
        );
        assert!(state.shell.lock().unwrap().is_empty());
        assert_eq!(
            state.store.lock().unwrap().summaries()[0].runtime.status,
            SessionStatus::Disconnected
        );
        drop(guard);
        let opened = tokio::time::timeout(Duration::from_secs(15), opening)
            .await
            .expect("open_session did not resume after lifecycle lane release")
            .unwrap()
            .unwrap();
        assert_eq!(opened.runtime.status, SessionStatus::Connected);

        let guard = lane.lock().await;
        let closing_state = state.clone();
        let closing_session_id = profile.id.clone();
        let mut closing =
            tokio::spawn(
                async move { close_session_inner(&closing_state, closing_session_id).await },
            );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut closing)
                .await
                .is_err(),
            "close_session crossed an occupied lifecycle lane"
        );
        assert!(state.shell.lock().unwrap().contains_key(&profile.id));
        assert_eq!(
            state.store.lock().unwrap().summaries()[0].runtime.status,
            SessionStatus::Connected
        );
        drop(guard);
        let closed = tokio::time::timeout(Duration::from_secs(15), closing)
            .await
            .expect("close_session did not resume after lifecycle lane release")
            .unwrap()
            .unwrap();
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
        assert!(state.shell.lock().unwrap().is_empty());
    });
}

#[test]
fn session_open_registration_is_single_flight_and_releases_its_slot() {
    let temp = tempfile::tempdir().unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));

    let first = register_session_open_cancellation(&state, &profile.id).unwrap();
    assert_eq!(
        state.session_open_slots.available_permits(),
        MAX_CONCURRENT_SESSION_OPENS - 1
    );
    let duplicate = register_session_open_cancellation(&state, &profile.id)
        .err()
        .unwrap();
    assert!(duplicate.contains("already pending"), "{duplicate}");
    assert_eq!(
        state.session_open_slots.available_permits(),
        MAX_CONCURRENT_SESSION_OPENS - 1
    );
    assert_eq!(
        cancel_pending_session_opens(&state, &profile.id).unwrap(),
        1
    );
    assert!(first.is_cancelled());

    let replacement = register_session_open_cancellation(&state, &profile.id).unwrap();
    assert!(!replacement.is_cancelled());
    assert_eq!(
        state.session_open_slots.available_permits(),
        MAX_CONCURRENT_SESSION_OPENS - 2
    );
    drop(first);
    assert_eq!(
        state.session_open_slots.available_permits(),
        MAX_CONCURRENT_SESSION_OPENS - 1
    );
    drop(replacement);
    assert_eq!(
        state.session_open_slots.available_permits(),
        MAX_CONCURRENT_SESSION_OPENS
    );
}

#[test]
fn session_open_saturation_rejects_before_store_side_effects() {
    tauri::async_runtime::block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));
        let _permits = Arc::clone(&state.session_open_slots)
            .try_acquire_many_owned(MAX_CONCURRENT_SESSION_OPENS as u32)
            .unwrap();

        let error = open_session_inner(
            state.clone(),
            profile.id.clone(),
            SessionOpenCredentials::default(),
        )
        .await
        .unwrap_err();

        assert!(error.contains("connection limit"), "{error}");
        assert!(state.shell.lock().unwrap().is_empty());
        let store = state.store.lock().unwrap();
        assert!(store.events.is_empty());
        assert_eq!(
            store.summaries()[0].runtime.status,
            SessionStatus::Disconnected
        );
    });
}

#[test]
fn session_prepare_cancellation_drops_late_resources_without_installing_a_runtime() {
    #[derive(Debug)]
    struct LatePreparedResource(Arc<AtomicBool>);

    impl Drop for LatePreparedResource {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));
        let cancellation = register_session_open_cancellation(&state, &profile.id).unwrap();
        let lifecycle_lane = session_lifecycle_lane(&state, &profile.id).unwrap();
        let _lifecycle_guard = lifecycle_lane.lock().await;
        let (started_sender, started_receiver) = tokio::sync::oneshot::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let dropped = Arc::new(AtomicBool::new(false));
        let task_dropped = Arc::clone(&dropped);
        let task = spawn_session_prepare(&cancellation, move || {
            let _ = started_sender.send(());
            release_receiver.recv().unwrap();
            Ok(LatePreparedResource(task_dropped))
        });
        started_receiver.await.unwrap();

        cancellation.cancel();
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            wait_for_session_prepare(
                &state,
                &profile.id,
                &cancellation,
                task,
                "test prepare failed",
            ),
        )
        .await
        .expect("cancelled prepare waited for its blocking worker")
        .unwrap_err();

        assert!(error.contains("connection was cancelled"), "{error}");
        assert!(!dropped.load(Ordering::SeqCst));
        assert_eq!(
            state.session_open_slots.available_permits(),
            MAX_CONCURRENT_SESSION_OPENS - 1
        );
        assert!(state.shell.lock().unwrap().is_empty());
        assert_eq!(
            state.store.lock().unwrap().summaries()[0].runtime.status,
            SessionStatus::Disconnected
        );

        release_sender.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !dropped.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("late prepared resource was not dropped");
        assert_eq!(
            state.session_open_slots.available_permits(),
            MAX_CONCURRENT_SESSION_OPENS - 1
        );
        drop(cancellation);
        assert_eq!(
            state.session_open_slots.available_permits(),
            MAX_CONCURRENT_SESSION_OPENS
        );
        assert!(state.shell.lock().unwrap().is_empty());
        assert_eq!(
            state.store.lock().unwrap().summaries()[0].runtime.status,
            SessionStatus::Disconnected
        );
    });
}

#[test]
fn duplicate_session_open_preserves_the_existing_runtime() {
    tauri::async_runtime::block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));
        let opened = open_session_inner(
            state.clone(),
            profile.id.clone(),
            SessionOpenCredentials::default(),
        )
        .await
        .unwrap();
        assert_eq!(opened.runtime.status, SessionStatus::Connected);
        let runtime_id = state
            .shell
            .lock()
            .unwrap()
            .get(&profile.id)
            .unwrap()
            .runtime_id
            .clone();
        let connecting_event_count = state
            .store
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| {
                event
                    .text
                    .as_deref()
                    .is_some_and(|text| text.starts_with("PortMate: connecting to "))
            })
            .count();

        let active_error = open_session_inner(
            state.clone(),
            profile.id.clone(),
            SessionOpenCredentials::default(),
        )
        .await
        .unwrap_err();
        assert!(active_error.contains("already active"), "{active_error}");
        assert_eq!(
            state
                .shell
                .lock()
                .unwrap()
                .get(&profile.id)
                .unwrap()
                .runtime_id,
            runtime_id
        );
        assert_eq!(
            state
                .store
                .lock()
                .unwrap()
                .events
                .iter()
                .filter(|event| {
                    event
                        .text
                        .as_deref()
                        .is_some_and(|text| text.starts_with("PortMate: connecting to "))
                })
                .count(),
            connecting_event_count
        );

        state
            .store
            .lock()
            .unwrap()
            .set_runtime_status(&profile.id, SessionStatus::Disconnected)
            .unwrap();
        let residue_error = open_session_inner(
            state.clone(),
            profile.id.clone(),
            SessionOpenCredentials::default(),
        )
        .await
        .unwrap_err();
        assert!(
            residue_error.contains("transport runtime is still registered"),
            "{residue_error}"
        );
        assert_eq!(
            state
                .shell
                .lock()
                .unwrap()
                .get(&profile.id)
                .unwrap()
                .runtime_id,
            runtime_id
        );

        let closed = close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
        assert!(state.shell.lock().unwrap().is_empty());
        tokio::time::sleep(Duration::from_millis(200)).await;
    });
}

#[test]
fn session_close_cancels_a_stalled_ssh_open_before_waiting_for_the_lane() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(10)).await;
        });
        let temp = tempfile::tempdir().unwrap();
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.endpoint.host = "127.0.0.1".to_string();
        ssh.endpoint.port = address.port();
        ssh.reconnect = false;
        let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));
        let opening_state = state.clone();
        let opening_session_id = profile.id.clone();
        let opening = tokio::spawn(async move {
            open_session_inner(
                opening_state,
                opening_session_id,
                SessionOpenCredentials::default(),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let is_connecting = state.store.try_lock().ok().is_some_and(|store| {
                    store.summaries()[0].runtime.status == SessionStatus::Connecting
                });
                if is_connecting {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("SSH open did not reach Connecting");

        let closed = tokio::time::timeout(
            Duration::from_secs(2),
            close_session_inner(&state, profile.id.clone()),
        )
        .await
        .expect("close_session waited for the stalled SSH handshake")
        .unwrap();
        let open_error = tokio::time::timeout(Duration::from_secs(2), opening)
            .await
            .expect("cancelled SSH open did not finish")
            .unwrap()
            .unwrap_err();

        assert!(
            open_error.contains("connection was cancelled"),
            "{open_error}"
        );
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
        assert!(state.ssh.lock().unwrap().is_empty());
        assert_eq!(
            state.store.lock().unwrap().summaries()[0].runtime.status,
            SessionStatus::Disconnected
        );
        peer.abort();
    });
}

#[test]
fn session_close_persistence_failure_keeps_local_disconnect_truth() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-session-close-persistence-failure-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let blocked_parent = root.join("not-a-directory");
        fs::write(&blocked_parent, b"blocked").unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(
            profile.clone(),
            blocked_parent.join("portmate-store.sqlite3"),
        );
        state
            .store
            .lock()
            .unwrap()
            .open_session(&profile.id)
            .unwrap();

        let error = close_session_inner(&state, profile.id.clone())
            .await
            .unwrap_err();

        assert!(error.contains("已在本地关闭"), "{error}");
        let store = state.store.lock().unwrap();
        let summary = store
            .summaries()
            .into_iter()
            .find(|summary| summary.profile.id == profile.id)
            .unwrap();
        assert_eq!(summary.runtime.status, SessionStatus::Disconnected);
        assert!(store.events.iter().any(|event| {
            event
                .text
                .as_deref()
                .is_some_and(|text| text.contains("session disconnected"))
        }));

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn session_close_stops_tunnel_listeners_before_returning() {
    tauri::async_runtime::block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let profile = test_ssh_profile();
        let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));
        state
            .store
            .lock()
            .unwrap()
            .open_session(&profile.id)
            .unwrap();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (listener_worker, listener_completion) = TunnelListenerWorker::running();
        let listener_waiter = listener_worker.clone();
        let listener_task = tauri::async_runtime::spawn(async move {
            let _listener = listener;
            listener_waiter.wait_shutdown().await;
            drop(listener_completion);
        });
        let closed = Arc::new(AtomicBool::new(false));
        state.tunnels.lock().unwrap().insert(
            "session-close-tunnel".to_string(),
            TunnelRuntime {
                session_id: profile.id.clone(),
                ssh_runtime_id: "session-close-runtime".to_string(),
                spec: TunnelSpec {
                    id: "session-close-tunnel".to_string(),
                    label: "session close listener".to_string(),
                    egress: TunnelEgress::Ssh,
                    mode: TunnelMode::Local,
                    bind_host: address.ip().to_string(),
                    bind_port: address.port(),
                    target_host: "127.0.0.1".to_string(),
                    target_port: 22,
                    route_rules: Vec::new(),
                    enabled: true,
                },
                metrics: Arc::new(TunnelMetrics::default()),
                closed: Arc::clone(&closed),
                listener_worker: listener_worker.clone(),
            },
        );

        let summary = close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();

        assert_eq!(summary.runtime.status, SessionStatus::Disconnected);
        assert!(closed.load(Ordering::SeqCst));
        assert!(state.tunnels.lock().unwrap().is_empty());
        assert!(listener_worker.is_finished());
        listener_task.await.unwrap();
        let rebound = tokio::net::TcpListener::bind(address).await.unwrap();
        drop(rebound);
    });
}

#[test]
fn shell_open_removes_the_runtime_when_connected_state_cannot_commit() {
    let root = std::env::temp_dir().join(format!(
        "portmate-shell-open-commit-failure-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let blocked_parent = root.join("not-a-directory");
    fs::write(&blocked_parent, b"blocked").unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(
        profile.clone(),
        blocked_parent.join("portmate-store.sqlite3"),
    );

    let error = open_shell_session(&state, profile.clone()).unwrap_err();
    assert!(error.contains("无法判定 Store 提交是否生效"), "{error}");
    assert!(state.shell.lock().unwrap().is_empty());
    let store = state.store.lock().unwrap();
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == profile.id)
        .unwrap();
    assert_eq!(summary.runtime.status, SessionStatus::Disconnected);
    assert!(store.events.iter().all(|event| {
        event
            .text
            .as_deref()
            .is_none_or(|text| !text.contains("shell started") && !text.contains("connected to"))
    }));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_open_rolls_back_connecting_state_when_snapshot_cannot_commit() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-session-open-commit-failure-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let blocked_parent = root.join("not-a-directory");
        fs::write(&blocked_parent, b"blocked").unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(
            profile.clone(),
            blocked_parent.join("portmate-store.sqlite3"),
        );

        let error = open_session_inner(
            state.clone(),
            profile.id.clone(),
            SessionOpenCredentials::default(),
        )
        .await
        .unwrap_err();
        assert!(error.contains("无法判定 Store 提交是否生效"), "{error}");
        assert!(state.shell.lock().unwrap().is_empty());
        let store = state.store.lock().unwrap();
        let summary = store
            .summaries()
            .into_iter()
            .find(|summary| summary.profile.id == profile.id)
            .unwrap();
        assert_eq!(summary.runtime.status, SessionStatus::Disconnected);
        assert!(store.events.is_empty());

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn tcp_open_closes_the_socket_when_connected_state_cannot_commit() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut byte = [0_u8; 1];
            stream.read(&mut byte).await.unwrap()
        });
        let root = std::env::temp_dir().join(format!(
            "portmate-tcp-open-commit-failure-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let blocked_parent = root.join("not-a-directory");
        fs::write(&blocked_parent, b"blocked").unwrap();
        let profile = test_tcp_profile(ConnectionConfig::Tcp(TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let state = test_app_state(
            profile.clone(),
            blocked_parent.join("portmate-store.sqlite3"),
        );

        let error = open_tcp_session(&state, profile.clone()).await.unwrap_err();
        assert!(error.contains("无法判定 Store 提交是否生效"), "{error}");
        assert!(state.tcp.lock().unwrap().is_empty());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), peer)
                .await
                .expect("TCP peer did not observe cleanup")
                .unwrap(),
            0
        );
        let store = state.store.lock().unwrap();
        let summary = store
            .summaries()
            .into_iter()
            .find(|summary| summary.profile.id == profile.id)
            .unwrap();
        assert_eq!(summary.runtime.status, SessionStatus::Disconnected);
        assert!(store.events.is_empty());

        let _ = fs::remove_dir_all(root);
    });
}

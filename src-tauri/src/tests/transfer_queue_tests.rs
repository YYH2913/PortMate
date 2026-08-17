use super::*;

#[test]
fn transfer_average_bps_uses_elapsed_time() {
    let started = Utc::now();
    let finished = started + chrono::Duration::seconds(2);
    let task = TransferTask {
        id: "transfer-1".to_string(),
        session_id: "session-1".to_string(),
        protocol: TransferProtocol::Sftp,
        source: "a.bin".to_string(),
        destination: "b.bin".to_string(),
        bytes_total: 2048,
        bytes_done: 2048,
        status: TransferStatus::Completed,
        message: None,
        started_at: Some(started),
        finished_at: Some(finished),
        average_bytes_per_second: None,
    };

    assert_eq!(transfer_average_bps(&task), Some(1024.0));
}

#[test]
fn transfer_progress_keeps_applied_bytes_and_accepts_verified_commits() {
    let mut store = SessionStore::default();
    let task = test_transfer_task("session-1", TransferStatus::Running);
    let task_id = task.id.clone();
    store.record_transfer(task);

    let updated = record_applied_transfer_progress_with(
        &mut store,
        &task_id,
        512,
        1_024,
        |_| Err("post-commit version read failed".to_string()),
        |_| Ok(true),
    )
    .unwrap();
    assert_eq!(updated.bytes_done, 512);
    assert_eq!(updated.bytes_total, 1_024);

    let error = record_applied_transfer_progress_with(
        &mut store,
        &task_id,
        768,
        1_024,
        |_| Err("disk full".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "disk full");
    let retained = store.transfer_by_id(&task_id).unwrap();
    assert_eq!(retained.bytes_done, 768);
    assert_eq!(retained.bytes_total, 1_024);
    assert_eq!(retained.message.as_deref(), Some("running"));
}

#[test]
fn transfer_active_statuses_cover_queued_and_running_tasks() {
    assert!(transfer_task_is_active(&TransferStatus::Queued));
    assert!(transfer_task_is_active(&TransferStatus::Running));
    assert!(!transfer_task_is_active(&TransferStatus::Completed));
    assert!(!transfer_task_is_active(&TransferStatus::Failed));
    assert!(!transfer_task_is_active(&TransferStatus::Cancelled));
}

#[test]
fn transfer_queue_capacity_bounds_session_app_and_overflow_counts() {
    let mut store = SessionStore::default();
    for index in 0..MAX_ACTIVE_TRANSFERS_PER_SESSION {
        let mut task = test_transfer_task("session-1", TransferStatus::Queued);
        task.id = format!("queued-{index}");
        store.record_transfer(task);
    }
    assert!(ensure_transfer_queue_capacity(&store, "session-1", 1)
        .unwrap_err()
        .contains(&MAX_ACTIVE_TRANSFERS_PER_SESSION.to_string()));
    assert!(ensure_transfer_queue_capacity(&store, "session-2", 1)
        .unwrap_err()
        .contains("app limit"));
    assert!(
        ensure_transfer_queue_capacity(&SessionStore::default(), "session-1", usize::MAX)
            .unwrap_err()
            .contains(&MAX_ACTIVE_TRANSFERS_PER_SESSION.to_string())
    );
}

#[test]
fn transfer_runner_saturation_rejects_before_queue_side_effects() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-transfer-runner-limit-{}", Uuid::new_v4()));
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
        let _permits = Arc::clone(&state.transfer_task_slots)
            .try_acquire_many_owned(MAX_ACTIVE_TRANSFER_TASKS as u32)
            .unwrap();

        let error = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id,
                protocol: TransferProtocol::Sftp,
                source: "input.bin".to_string(),
                destination: "output.bin".to_string(),
            },
        )
        .await
        .unwrap_err();

        assert!(error.contains("runner limit"), "{error}");
        assert!(state.store.lock().unwrap().transfers.is_empty());
        assert!(state.transfer_cancellations.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn transfer_commit_validation_failure_has_no_queue_side_effects() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-transfer-validation-failure-{}",
            Uuid::new_v4()
        ));
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.join("store.sqlite3"));

        let error = start_transfer_inner_with_validation(
            &state,
            StartTransferRequest {
                session_id: profile.id,
                protocol: TransferProtocol::Xmodem,
                source: root.join("firmware.bin").display().to_string(),
                destination: "load:loadx".to_string(),
            },
            Some(Box::new(|| Err("stale authorization".to_string()))),
        )
        .await
        .unwrap_err();

        assert_eq!(error, "stale authorization");
        assert!(state.store.lock().unwrap().transfers.is_empty());
        assert!(state.store.lock().unwrap().events.is_empty());
        assert!(state.transfer_cancellations.lock().unwrap().is_empty());
        assert!(state
            .mcp_content_transfer_staging
            .lock()
            .unwrap()
            .is_empty());
        assert_eq!(
            state.transfer_task_slots.available_permits(),
            MAX_ACTIVE_TRANSFER_TASKS
        );
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn cancelling_queued_transfer_releases_runner_without_entering_lane() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-transfer-queued-cancel-{}",
            Uuid::new_v4()
        ));
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
        let lane = transfer_lane(&state, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let initial_permits = state.transfer_task_slots.available_permits();
        let task = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: "input.bin".to_string(),
                destination: "output.bin".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            state.transfer_task_slots.available_permits(),
            initial_permits - 1
        );

        let cancelled = cancel_transfer_inner(&state, &task.id).unwrap();
        assert_eq!(cancelled.status, TransferStatus::Cancelled);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state.transfer_task_slots.available_permits() == initial_permits
                    && !state
                        .transfer_cancellations
                        .lock()
                        .unwrap()
                        .contains_key(&task.id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("cancelled queued transfer retained its runner");
        assert!(
            lane.try_lock().is_err(),
            "queued transfer entered the occupied lane"
        );
        drop(lane_guard);

        let saved = state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&task.id)
            .unwrap();
        assert_eq!(saved.status, TransferStatus::Cancelled);
        assert_eq!(saved.message.as_deref(), Some("cancelled"));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn transfer_queue_commit_failure_rolls_back_task_event_and_cancellation() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-transfer-queue-commit-failure-{}",
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

        let error = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id,
                protocol: TransferProtocol::Sftp,
                source: "input.bin".to_string(),
                destination: "output.bin".to_string(),
            },
        )
        .await
        .unwrap_err();

        assert!(error.contains("无法判定 Store 提交是否生效"), "{error}");
        let store = state.store.lock().unwrap();
        assert!(store.transfers.is_empty());
        assert!(store.events.is_empty());
        drop(store);
        assert!(state.transfer_cancellations.lock().unwrap().is_empty());

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn transfer_running_commit_failure_restores_queued_state_and_event() {
    let root = std::env::temp_dir().join(format!(
        "portmate-transfer-running-commit-failure-{}",
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
    let task = test_transfer_task(&profile.id, TransferStatus::Queued);
    state.store.lock().unwrap().record_transfer(task.clone());
    let request = StartTransferRequest {
        session_id: profile.id,
        protocol: task.protocol.clone(),
        source: task.source.clone(),
        destination: task.destination.clone(),
    };

    let error = mark_transfer_running(&state, &task.id, &request).unwrap_err();

    assert!(error.contains("无法判定 Store 提交是否生效"), "{error}");
    let store = state.store.lock().unwrap();
    let restored = store.transfer_by_id(&task.id).unwrap();
    assert_eq!(restored.status, TransferStatus::Queued);
    assert_eq!(restored.message.as_deref(), Some("queued"));
    assert!(restored.started_at.is_none());
    assert!(store.events.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn transfer_cancel_remains_accepted_when_persistence_fails() {
    let root = std::env::temp_dir().join(format!(
        "portmate-transfer-cancel-persistence-failure-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let blocked_parent = root.join("not-a-directory");
    fs::write(&blocked_parent, b"blocked").unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), blocked_parent.join("store.sqlite3"));
    let task = test_transfer_task(&profile.id, TransferStatus::Running);
    state.store.lock().unwrap().record_transfer(task.clone());
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancellation = Arc::new(TransferCancellation::with_flag(Arc::clone(&cancelled)));
    state
        .transfer_cancellations
        .lock()
        .unwrap()
        .insert(task.id.clone(), cancellation);

    let result = cancel_transfer_inner(&state, &task.id).unwrap();

    assert_eq!(result.status, TransferStatus::Cancelled);
    assert!(cancelled.load(Ordering::SeqCst));
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&task.id)
            .unwrap()
            .status,
        TransferStatus::Cancelled
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn transfer_system_events_record_direction_without_paths() {
    assert_eq!(
        transfer_route_label("/home/operator/private", "remote:/srv/private"),
        "upload"
    );
    assert_eq!(
        transfer_route_label("ssh:/srv/private", "/home/operator/private"),
        "download"
    );
    assert_eq!(
        transfer_route_label("remote:/srv/a", "remote:/srv/b"),
        "remote-copy"
    );
    assert_eq!(transfer_route_label("local-a", "local-b"), "local-copy");
}

#[test]
fn transfer_finish_preserves_cancelled_truth_and_cleans_runtime_state() {
    let root = std::env::temp_dir().join(format!(
        "portmate-transfer-finish-persistence-failure-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let blocked_parent = root.join("not-a-directory");
    fs::write(&blocked_parent, b"blocked").unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), blocked_parent.join("store.sqlite3"));
    let mut task = test_transfer_task(&profile.id, TransferStatus::Cancelled);
    task.message = Some("cancelling".to_string());
    state.store.lock().unwrap().record_transfer(task.clone());
    let cancellation = Arc::new(TransferCancellation::with_flag(Arc::new(AtomicBool::new(
        true,
    ))));
    state
        .transfer_cancellations
        .lock()
        .unwrap()
        .insert(task.id.clone(), cancellation);

    finish_transfer_task(
        &state,
        &task.id,
        &profile.id,
        TransferStatus::Completed,
        "completed".to_string(),
        Some(512),
    );

    let store = state.store.lock().unwrap();
    let finished = store.transfer_by_id(&task.id).unwrap();
    assert_eq!(finished.status, TransferStatus::Cancelled);
    assert_eq!(finished.message.as_deref(), Some("cancelled"));
    assert_eq!(finished.bytes_done, 512);
    assert!(store.events.iter().any(|event| {
        event
            .text
            .as_deref()
            .is_some_and(|text| text.contains("transfer finished") && text.contains("Cancelled"))
    }));
    drop(store);
    assert!(state.transfer_cancellations.lock().unwrap().is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn transfer_terminal_state_is_not_visible_before_cancellation_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), temp.path().join("store.sqlite3"));
    let task = test_transfer_task(&profile.id, TransferStatus::Running);
    state.store.lock().unwrap().record_transfer(task.clone());
    state
        .transfer_cancellations
        .lock()
        .unwrap()
        .insert(task.id.clone(), Arc::new(TransferCancellation::new()));

    let cancellations = state.transfer_cancellations.lock().unwrap();
    let worker_state = state.clone();
    let worker_task_id = task.id.clone();
    let worker_session_id = profile.id.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        finish_transfer_task(
            &worker_state,
            &worker_task_id,
            &worker_session_id,
            TransferStatus::Completed,
            "completed".to_string(),
            Some(512),
        );
    });
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    std::thread::sleep(Duration::from_millis(50));

    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&task.id)
            .unwrap()
            .status,
        TransferStatus::Running
    );
    assert!(!worker.is_finished());

    drop(cancellations);
    worker.join().unwrap();
    assert_eq!(
        state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&task.id)
            .unwrap()
            .status,
        TransferStatus::Completed
    );
    assert!(state.transfer_cancellations.lock().unwrap().is_empty());
}

#[test]
fn transfer_protocol_settings_are_enforced_before_queueing() {
    tauri::async_runtime::block_on(async {
        let mut profile = test_ssh_profile();
        profile.transfer.sftp = false;
        profile.transfer.ymodem = false;
        let state = test_app_state(
            profile.clone(),
            PathBuf::from("transfer-capability-test.sqlite3"),
        );

        for (protocol, label) in [
            (TransferProtocol::Sftp, "SFTP"),
            (TransferProtocol::Ymodem, "YModem"),
        ] {
            let error = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol,
                    source: "input.bin".to_string(),
                    destination: "remote:/tmp/input.bin".to_string(),
                },
            )
            .await
            .unwrap_err();
            assert!(error.contains(label));
            assert!(error.contains("禁用"));
        }

        assert!(state.store.lock().unwrap().transfers.is_empty());
        assert!(state.transfer_cancellations.lock().unwrap().is_empty());
    });
}

#[test]
fn desktop_sftp_transfer_does_not_require_an_mcp_grant() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-desktop-sftp-without-mcp-{}",
            Uuid::new_v4()
        ));
        let profile = test_ssh_profile();
        let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
        assert!(state.store.lock().unwrap().grants.is_empty());
        assert!(state.pending_mcp_approvals.lock().unwrap().is_empty());

        let lane = transfer_lane(&state, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let task = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id,
                protocol: TransferProtocol::Sftp,
                source: root.join("desktop-upload.bin").display().to_string(),
                destination: "remote:/tmp/desktop-upload.bin".to_string(),
            },
        )
        .await
        .unwrap();

        assert_eq!(task.status, TransferStatus::Queued);
        assert!(state.pending_mcp_approvals.lock().unwrap().is_empty());
        assert_eq!(
            cancel_transfer_inner(&state, &task.id).unwrap().status,
            TransferStatus::Cancelled
        );
        drop(lane_guard);
        let finished = wait_for_transfer_terminal_state(&state, &task.id).await;
        assert_eq!(finished.status, TransferStatus::Cancelled);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn queued_transfer_rechecks_latest_protocol_settings_before_running() {
    tauri::async_runtime::block_on(async {
        let profile = test_ssh_profile();
        let root =
            std::env::temp_dir().join(format!("portmate-transfer-capability-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
        let lane = transfer_lane(&state, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let task = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: "input.bin".to_string(),
                destination: "remote:/tmp/input.bin".to_string(),
            },
        )
        .await
        .unwrap();

        let mut disabled = profile;
        disabled.transfer.sftp = false;
        state.store.lock().unwrap().upsert_profile(disabled);
        drop(lane_guard);

        let finished = wait_for_transfer_terminal_state(&state, &task.id).await;
        assert_eq!(finished.status, TransferStatus::Failed);
        assert!(finished.message.unwrap_or_default().contains("SFTP"));
        assert!(state.transfer_cancellations.lock().unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn planned_file_batch_rejects_a_missing_ssh_runtime_before_queueing() {
    tauri::async_runtime::block_on(async {
        let profile = test_ssh_profile();
        let state = test_app_state(
            profile.clone(),
            PathBuf::from("transfer-runtime-constraint-test.sqlite3"),
        );

        let error = start_transfer_inner_for_ssh_runtime(
            &state,
            StartTransferRequest {
                session_id: profile.id,
                protocol: TransferProtocol::Sftp,
                source: "input.bin".to_string(),
                destination: "remote:/tmp/input.bin".to_string(),
            },
            "planned-runtime",
        )
        .await
        .unwrap_err();

        assert!(error.contains("文件批次规划后已变化"), "{error}");
        assert!(state.store.lock().unwrap().transfers.is_empty());
        assert!(state.transfer_cancellations.lock().unwrap().is_empty());
    });
}

#[test]
fn ssh_file_transfer_rejects_non_ssh_profiles_before_queueing() {
    tauri::async_runtime::block_on(async {
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), PathBuf::from("transfer-kind-test.sqlite3"));
        let error = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id,
                protocol: TransferProtocol::Scp,
                source: "input.bin".to_string(),
                destination: "remote:/tmp/input.bin".to_string(),
            },
        )
        .await
        .unwrap_err();

        assert!(error.contains("SCP"));
        assert!(error.contains("仅支持 SSH/Tmux"));
        assert!(state.store.lock().unwrap().transfers.is_empty());
    });
}

#[test]
fn empty_remote_transfer_path_is_rejected_before_queueing() {
    tauri::async_runtime::block_on(async {
        let profile = test_ssh_profile();
        let state = test_app_state(
            profile.clone(),
            PathBuf::from("empty-remote-transfer-test.sqlite3"),
        );
        let error = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id,
                protocol: TransferProtocol::Sftp,
                source: "input.bin".to_string(),
                destination: "remote:".to_string(),
            },
        )
        .await
        .unwrap_err();

        assert!(error.contains("远端传输目标路径"), "{error}");
        assert!(state.store.lock().unwrap().transfers.is_empty());
        assert!(state.transfer_cancellations.lock().unwrap().is_empty());
    });
}

#[test]
fn local_copy_does_not_depend_on_remote_ssh_transfer_flags() {
    let mut profile = test_shell_profile();
    profile.transfer.sftp = false;
    profile.transfer.scp = false;

    for protocol in [TransferProtocol::Sftp, TransferProtocol::Scp] {
        let request = prepare_transfer_request(
            &profile,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol,
                source: "input.bin".to_string(),
                destination: "output.bin".to_string(),
            },
        )
        .unwrap();
        assert_eq!(request.source, "input.bin");
        assert_eq!(request.destination, "output.bin");
    }
}

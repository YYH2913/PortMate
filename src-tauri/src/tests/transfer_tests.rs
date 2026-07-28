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

#[test]
fn default_transfer_directory_resolves_only_relative_local_paths() {
    let mut profile = test_ssh_profile();
    let default_dir = std::env::temp_dir().join("portmate-transfer-default");
    profile.transfer.default_local_dir = Some(default_dir.to_string_lossy().into_owned());

    let upload = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: "input.bin".to_string(),
            destination: "remote:/tmp/input.bin".to_string(),
        },
    )
    .unwrap();
    assert_eq!(
        upload.source,
        default_dir.join("input.bin").to_string_lossy()
    );
    assert_eq!(upload.destination, "remote:/tmp/input.bin");

    let absolute_destination = default_dir.join("download.bin");
    let download = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
            source: "ssh:/tmp/download.bin".to_string(),
            destination: absolute_destination.to_string_lossy().into_owned(),
        },
    )
    .unwrap();
    assert_eq!(download.source, "ssh:/tmp/download.bin");
    assert_eq!(download.destination, absolute_destination.to_string_lossy());
}

#[test]
fn sftp_transfer_paths_reject_root_and_dot_components() {
    for path in ["/", "//", "~", "/tmp/../input.bin", "/tmp/./input.bin"] {
        let target_error = validate_remote_transfer_path(path, "SFTP 远端目标路径")
            .expect_err("unsafe SFTP destination was accepted");
        assert!(target_error.contains("SFTP 远端目标路径"), "{target_error}");
        let source_error = validate_remote_transfer_path(path, "SFTP 远端源路径")
            .expect_err("unsafe SFTP source was accepted");
        assert!(source_error.contains("SFTP 远端源路径"), "{source_error}");
    }
    assert!(validate_remote_transfer_path("/tmp/portmate/", "SFTP 远端目标路径").is_ok());
    assert!(
        validate_remote_transfer_path(r"C:\Users\operator\input.bin", "SFTP 远端源路径").is_ok()
    );
}

#[test]
fn scp_transfer_paths_reject_root_and_dot_components() {
    for path in ["/", "//", "~", "/tmp/../input.bin", "/tmp/./input.bin"] {
        let target_error = validate_remote_transfer_path(path, "SCP 远端目标路径")
            .expect_err("unsafe SCP destination was accepted");
        assert!(target_error.contains("SCP 远端目标路径"), "{target_error}");
        let source_error = validate_remote_transfer_path(path, "SCP 远端源路径")
            .expect_err("unsafe SCP source was accepted");
        assert!(source_error.contains("SCP 远端源路径"), "{source_error}");
    }
    assert!(validate_remote_transfer_path("/tmp/portmate/", "SCP 远端目标路径").is_ok());
}

#[test]
fn modem_transfer_paths_reject_root_and_dot_components() {
    let root = std::env::temp_dir().join(format!("portmate-modem-paths-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.bin");
    fs::write(&source, b"payload").unwrap();

    for path in ["/", "//", "~", "/tmp/../input.bin", "/tmp/./input.bin"] {
        let upload_error = match modem_direction(&StartTransferRequest {
            session_id: "session".to_string(),
            protocol: TransferProtocol::Zmodem,
            source: source.display().to_string(),
            destination: format!("remote:{path}"),
        }) {
            Err(error) => error,
            Ok(_) => panic!("unsafe Modem upload destination was accepted"),
        };
        assert!(
            upload_error.contains("Modem 远端目标路径"),
            "{upload_error}"
        );

        let download_error = match modem_direction(&StartTransferRequest {
            session_id: "session".to_string(),
            protocol: TransferProtocol::Zmodem,
            source: format!("remote:{path}"),
            destination: root.join("download.bin").display().to_string(),
        }) {
            Err(error) => error,
            Ok(_) => panic!("unsafe Modem download source was accepted"),
        };
        assert!(
            download_error.contains("Modem 远端源路径"),
            "{download_error}"
        );

        let implicit_error = match modem_direction(&StartTransferRequest {
            session_id: "session".to_string(),
            protocol: TransferProtocol::Xmodem,
            source: source.display().to_string(),
            destination: path.to_string(),
        }) {
            Err(error) => error,
            Ok(_) => panic!("unsafe implicit Modem upload destination was accepted"),
        };
        assert!(
            implicit_error.contains("Modem 远端目标路径"),
            "{implicit_error}"
        );
    }

    let accepted = modem_direction(&StartTransferRequest {
        session_id: "session".to_string(),
        protocol: TransferProtocol::Ymodem,
        source: source.display().to_string(),
        destination: "remote:/tmp/portmate/".to_string(),
    })
    .unwrap();
    match accepted {
        ModemDirection::Upload {
            remote_destination, ..
        } => {
            assert_eq!(remote_destination, "/tmp/portmate/")
        }
        _ => panic!("expected Modem upload direction"),
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_transfer_path_classification_covers_unix_windows_and_unc_forms() {
    for path in ["/tmp/input.bin", "//server/share/input.bin"] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Unix),
            LocalTransferPathKind::Absolute
        );
    }
    for path in ["input.bin", "nested/input.bin", r"nested\input.bin"] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Unix),
            LocalTransferPathKind::Relative
        );
    }
    for path in [
        r"C:\Users\operator\input.bin",
        "D:/data/input.bin",
        r"C:input.bin",
        r"\input.bin",
        r"\\server\share\input.bin",
    ] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Unix),
            LocalTransferPathKind::ForeignAnchored
        );
    }

    for path in [
        r"C:\Users\operator\input.bin",
        "D:/data/input.bin",
        r"\\server\share\input.bin",
        "//server/share/input.bin",
    ] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Windows),
            LocalTransferPathKind::Absolute
        );
    }
    for path in [r"\input.bin", "/input.bin"] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Windows),
            LocalTransferPathKind::RootedWithoutDrive
        );
    }
    assert_eq!(
        classify_local_transfer_path(r"C:input.bin", LocalTransferPathPlatform::Windows),
        LocalTransferPathKind::DriveRelative
    );
    assert_eq!(
        classify_local_transfer_path("nested/input.bin", LocalTransferPathPlatform::Windows),
        LocalTransferPathKind::Relative
    );
}

#[test]
fn transfer_paths_reject_non_native_or_ambiguous_local_roots() {
    let mut profile = test_ssh_profile();
    profile.transfer.default_local_dir = Some("relative/default".to_string());
    let error = validate_transfer_default_local_dir(&profile).unwrap_err();
    assert!(error.contains("完整绝对路径"), "{error}");

    profile.transfer.default_local_dir = None;
    let foreign_local_path = if cfg!(windows) {
        "C:input.bin"
    } else {
        r"C:\Users\operator\input.bin"
    };
    let error = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: foreign_local_path.to_string(),
            destination: "remote:/tmp/input.bin".to_string(),
        },
    )
    .unwrap_err();
    assert!(
        error.contains("不兼容") || error.contains("drive-relative"),
        "{error}"
    );

    let remote_windows_path = r"remote:C:\Users\operator\input.bin";
    let request = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: remote_windows_path.to_string(),
            destination: std::env::temp_dir()
                .join("input.bin")
                .to_string_lossy()
                .into_owned(),
        },
    )
    .unwrap();
    assert_eq!(request.source, remote_windows_path);
}

#[test]
fn file_manager_local_paths_reject_foreign_roots_and_filesystem_roots() {
    let foreign = if cfg!(windows) {
        "/tmp/portmate-foreign"
    } else {
        r"C:\Users\operator\foreign"
    };
    assert!(validate_native_local_path(foreign).is_err());
    assert!(validate_local_mutating_path(foreign).is_err());
    assert!(validate_local_drop_destination(foreign).is_err());
    assert!(list_local_files(foreign).is_err());
    assert!(local_file_properties(foreign).is_err());

    let filesystem_root = if cfg!(windows) { r"C:\" } else { "/" };
    assert!(validate_local_mutating_path(filesystem_root).is_err());
    assert!(validate_local_mutating_path("~").is_err());
    assert_eq!(
        validate_native_local_path("nested/child").unwrap(),
        expand_identity_path("nested/child")
    );
    assert!(validate_local_mutating_path("nested/../child").is_err());
    assert!(validate_local_mutating_path("nested/./child").is_err());
}

#[test]
fn file_manager_remote_mutating_paths_reject_parent_components() {
    for path in [
        "/tmp/..",
        "/tmp/./file",
        "nested/../outside",
        "../outside",
        "/",
        "//",
        "~",
    ] {
        assert!(
            validate_remote_mutating_path(path).is_err(),
            "unsafe remote path was accepted: {path}"
        );
        assert!(normalize_remote_batch_source(path).is_err());
        assert!(validate_remote_drop_destination(path).is_err());
    }
    assert!(validate_remote_drop_destination("/tmp/portmate/").is_ok());
    assert_eq!(
        validate_remote_mutating_path("/tmp/portmate/file").unwrap(),
        "/tmp/portmate/file"
    );
}

#[test]
fn file_manager_local_file_creation_is_exclusive() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("new-file.txt");
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    tauri::async_runtime::block_on(async {
        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: None,
                path: file.display().to_string(),
                remote: false,
            },
            FileOperation::CreateFile,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"");

        fs::write(&file, b"existing contents").unwrap();
        let error = file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: None,
                path: file.display().to_string(),
                remote: false,
            },
            FileOperation::CreateFile,
        )
        .await
        .unwrap_err();
        assert!(error.contains("新建本地文件失败"), "{error}");
    });

    assert_eq!(fs::read(&file).unwrap(), b"existing contents");
}

#[test]
fn file_manager_local_batch_delete_removes_files_and_directories() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("remove.txt");
    let directory = root.path().join("remove-tree");
    fs::create_dir_all(directory.join("nested")).unwrap();
    fs::write(&file, b"remove").unwrap();
    fs::write(directory.join("nested/value.txt"), b"remove nested").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    tauri::async_runtime::block_on(delete_paths_inner(
        &state,
        DeletePathsRequest {
            session_id: None,
            paths: vec![file.display().to_string(), directory.display().to_string()],
            remote: false,
        },
    ))
    .unwrap();

    assert!(!file.exists());
    assert!(!directory.exists());
}

#[test]
fn file_manager_local_batch_delete_preflights_directory_children() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("remove-tree");
    let child = directory.join("value.txt");
    fs::create_dir(&directory).unwrap();
    fs::write(&child, b"keep").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(delete_paths_inner(
        &state,
        DeletePathsRequest {
            session_id: None,
            paths: vec![directory.display().to_string(), child.display().to_string()],
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("目录及其子项"), "{error}");
    assert!(directory.is_dir());
    assert_eq!(fs::read(&child).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn file_manager_local_batch_delete_removes_a_final_symlink_only() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("protected.txt");
    let link = root.path().join("remove-link");
    fs::write(&target, b"protected").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    tauri::async_runtime::block_on(delete_paths_inner(
        &state,
        DeletePathsRequest {
            session_id: None,
            paths: vec![link.display().to_string()],
            remote: false,
        },
    ))
    .unwrap();

    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"protected");
}

#[test]
fn file_manager_local_move_moves_multiple_selected_paths() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    let file = source.join("report.txt");
    let directory = source.join("nested");
    fs::write(&file, b"report").unwrap();
    fs::write(directory.join("detail.txt"), b"detail").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    tauri::async_runtime::block_on(move_paths_inner(
        &state,
        MovePathsRequest {
            session_id: None,
            paths: vec![file.display().to_string(), directory.display().to_string()],
            destination: destination.display().to_string(),
            remote: false,
        },
    ))
    .unwrap();

    assert!(!file.exists());
    assert!(!directory.exists());
    assert_eq!(fs::read(destination.join("report.txt")).unwrap(), b"report");
    assert_eq!(
        fs::read(destination.join("nested/detail.txt")).unwrap(),
        b"detail"
    );
}

#[test]
fn file_manager_local_move_rejects_collisions_before_any_mutation() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let first = source.join("first.txt");
    let second = source.join("second.txt");
    fs::write(&first, b"first source").unwrap();
    fs::write(&second, b"second source").unwrap();
    fs::write(destination.join("second.txt"), b"existing target").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(move_paths_inner(
        &state,
        MovePathsRequest {
            session_id: None,
            paths: vec![first.display().to_string(), second.display().to_string()],
            destination: destination.display().to_string(),
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("已存在"), "{error}");
    assert_eq!(fs::read(&first).unwrap(), b"first source");
    assert_eq!(fs::read(&second).unwrap(), b"second source");
    assert!(!destination.join("first.txt").exists());
    assert_eq!(
        fs::read(destination.join("second.txt")).unwrap(),
        b"existing target"
    );
}

#[test]
fn file_manager_local_move_rejects_a_directory_destination_inside_the_source() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let directory = source.join("tree");
    let nested_destination = directory.join("nested");
    fs::create_dir_all(&nested_destination).unwrap();
    fs::write(directory.join("detail.txt"), b"detail").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(move_paths_inner(
        &state,
        MovePathsRequest {
            session_id: None,
            paths: vec![directory.display().to_string()],
            destination: nested_destination.display().to_string(),
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("自身内部"), "{error}");
    assert_eq!(fs::read(directory.join("detail.txt")).unwrap(), b"detail");
}

#[test]
fn file_manager_local_move_rejects_a_selected_directory_and_its_child() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let directory = source.join("tree");
    let child = directory.join("detail.txt");
    fs::create_dir_all(&directory).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(&child, b"detail").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(move_paths_inner(
        &state,
        MovePathsRequest {
            session_id: None,
            paths: vec![directory.display().to_string(), child.display().to_string()],
            destination: destination.display().to_string(),
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("目录及其子项"), "{error}");
    assert!(directory.is_dir());
    assert_eq!(fs::read(&child).unwrap(), b"detail");
    assert!(!destination.join("tree").exists());
    assert!(!destination.join("detail.txt").exists());
}

#[test]
fn file_manager_local_rename_refuses_to_replace_an_existing_target() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"source contents").unwrap();
    fs::write(&target, b"target contents").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(rename_path_inner(
        &state,
        RenamePathRequest {
            session_id: None,
            old_path: source.display().to_string(),
            new_path: target.display().to_string(),
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("已存在"), "{error}");
    assert_eq!(fs::read(&source).unwrap(), b"source contents");
    assert_eq!(fs::read(&target).unwrap(), b"target contents");
}

#[cfg(unix)]
#[test]
fn local_directory_creation_rejects_symlink_components() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join("link");
    let renamed_link = root.path().join("renamed-link");
    fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let nested = link.join("nested");
    let error = reject_local_symlink_components(&nested, false, "test path").unwrap_err();

    assert!(error.contains("符号链接"), "{error}");
    assert!(!target.join("nested").exists());
    assert!(reject_local_symlink_components(&link, true, "final link").is_ok());

    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );
    let file_error = tauri::async_runtime::block_on(file_operation_inner(
        &state,
        FileOperationRequest {
            session_id: None,
            path: link.join("new-file.txt").display().to_string(),
            remote: false,
        },
        FileOperation::CreateFile,
    ))
    .unwrap_err();
    assert!(file_error.contains("符号链接"), "{file_error}");
    assert!(!target.join("new-file.txt").exists());
    tauri::async_runtime::block_on(async {
        rename_path_inner(
            &state,
            RenamePathRequest {
                session_id: None,
                old_path: link.display().to_string(),
                new_path: renamed_link.display().to_string(),
                remote: false,
            },
        )
        .await
        .unwrap();
        assert!(fs::symlink_metadata(&renamed_link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(target.is_dir());

        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: None,
                path: renamed_link.display().to_string(),
                remote: false,
            },
            FileOperation::Delete,
        )
        .await
        .unwrap();
    });
    assert!(fs::symlink_metadata(&renamed_link).is_err());
    assert!(target.is_dir());
}

#[test]
fn modem_file_names_normalize_windows_and_unix_separators() {
    assert_eq!(
        portable_file_name(r"C:\Users\operator\report.bin"),
        Some("report.bin".to_string())
    );
    assert_eq!(
        portable_file_name(r"\\server\share\report.bin"),
        Some("report.bin".to_string())
    );
    assert_eq!(
        portable_file_name("/var/tmp/report.bin"),
        Some("report.bin".to_string())
    );
    assert_eq!(portable_file_name("../"), None);
    assert_eq!(
        local_file_name(r"C:\Users\operator\report.bin"),
        "report.bin"
    );
    assert_eq!(
        remote_parent_and_file_name(r"C:\Users\operator\report.bin"),
        (r"C:\Users\operator".to_string(), "report.bin".to_string())
    );
    assert_eq!(
        remote_parent_and_file_name("/report.bin"),
        ("/".to_string(), "report.bin".to_string())
    );

    let root = std::env::temp_dir().join(format!("portmate-modem-name-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target =
        zmodem_local_target_path(root.to_str().unwrap(), r"C:\Users\operator\report.bin", 0)
            .unwrap();
    assert_eq!(target, root.join("report.bin"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn remote_file_names_are_safe_for_local_directory_targets() {
    assert_eq!(remote_file_name("/var/tmp/report.bin"), "report.bin");
    assert_eq!(
        remote_file_name(r"C:\Users\operator\report.bin"),
        "report.bin"
    );
    assert_eq!(remote_file_name("../"), "portmate-file.bin");
    assert_eq!(remote_file_name("."), "portmate-file.bin");

    let root = std::env::temp_dir().join(format!("portmate-remote-name-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target =
        local_destination_file_path(&format!("{}/", root.display()), "../outside.bin").unwrap();
    assert_eq!(target, root.join("outside.bin"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn local_file_listing_and_chmod_do_not_follow_symbolic_links() {
    let root = std::env::temp_dir().join(format!("portmate-file-links-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let protected = root.join("protected.txt");
    fs::write(&protected, b"protected").unwrap();
    let link = root.join("linked.txt");
    std::os::unix::fs::symlink(&protected, &link).unwrap();

    let entries = list_local_files(root.to_str().unwrap()).unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry.name == "linked.txt")
        .unwrap();
    assert!(!entry.is_dir);
    assert_eq!(entry.size, 0);

    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let error = tauri::async_runtime::block_on(chmod_path_inner(
        &state,
        ChmodPathRequest {
            session_id: None,
            path: link.display().to_string(),
            mode: 0o600,
            remote: false,
        },
    ))
    .unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn transfer_throttle_delay_respects_rate_limit() {
    assert!(transfer_throttle_delay(None, 1024, Duration::ZERO).is_none());
    assert!(transfer_throttle_delay(Some(0), 1024, Duration::ZERO).is_none());
    assert!(transfer_throttle_delay(Some(1024), 0, Duration::ZERO).is_none());

    assert_eq!(
        transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(1)),
        Some(Duration::from_secs(1))
    );
    assert!(transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(2)).is_none());
    assert!(transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(3)).is_none());
}

#[test]
fn transfer_throttle_wait_is_async_and_cancellable() {
    tauri::async_runtime::block_on(async {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = TransferProgressContext {
            state: test_app_state(
                test_shell_profile(),
                PathBuf::from("transfer-throttle-test.sqlite3"),
            ),
            task_id: "unused-transfer".to_string(),
            cancel: Arc::clone(&cancel),
            last_emit: Arc::new(Mutex::new(Instant::now())),
            started: Instant::now(),
            rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
            rate_limit_bytes_per_second: Some(1024),
        };
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.store(true, Ordering::SeqCst);
        });

        let started = Instant::now();
        let error = progress.throttle(1024).await.unwrap_err();
        assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
        assert!(started.elapsed() < Duration::from_millis(500));
        cancel_task.await.unwrap();
    });
}

#[test]
fn local_resume_part_helpers_keep_stable_offsets() {
    let root = std::env::temp_dir().join(format!("portmate-resume-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("image.bin");
    let part = local_resume_part_path(&target);
    assert_eq!(
        part.file_name().unwrap().to_string_lossy(),
        "image.bin.portmate-part"
    );

    fs::write(&part, b"abc").unwrap();
    assert_eq!(local_resume_offset(&part, 10).unwrap(), 3);
    assert!(part.exists());

    fs::write(&part, b"too-long").unwrap();
    assert_eq!(local_resume_offset(&part, 3).unwrap(), 0);
    assert!(!part.exists());

    fs::write(&part, b"complete").unwrap();
    fs::write(&target, b"old").unwrap();
    finalize_local_resume_file(&part, &target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"complete");
    assert!(!part.exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_resume_requires_a_matching_source_prefix() {
    let root = tempfile::tempdir().unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    state
        .store
        .lock()
        .unwrap()
        .transfers
        .push(test_transfer_task(&profile.id, TransferStatus::Running));
    let progress = test_transfer_progress_context(
        &state,
        "transfer-commit-test",
        Arc::new(AtomicBool::new(false)),
    );
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    let part = local_resume_part_path(&target);
    fs::write(&source, b"abcdef").unwrap();

    fs::write(&part, b"abc").unwrap();
    let mut source_file = open_local_transfer_source(&source, "source").unwrap().0;
    assert_eq!(
        local_resume_offset_matching_local_source(&mut source_file, &part, 6, &progress,).unwrap(),
        3
    );
    let mut suffix = Vec::new();
    source_file.read_to_end(&mut suffix).unwrap();
    assert_eq!(suffix, b"def");

    fs::write(&part, b"xyz").unwrap();
    let mut source_file = open_local_transfer_source(&source, "source").unwrap().0;
    assert_eq!(
        local_resume_offset_matching_local_source(&mut source_file, &part, 6, &progress,).unwrap(),
        0
    );
    let mut full = Vec::new();
    source_file.read_to_end(&mut full).unwrap();
    assert_eq!(full, b"abcdef");
}

#[test]
fn sftp_prefix_read_errors_distinguish_short_file_and_io_failure() {
    assert!(!prefix_read_mismatch_or_error(
        std::io::Error::from(std::io::ErrorKind::UnexpectedEof),
        "SFTP 远端断点文件",
    )
    .unwrap());

    let error = prefix_read_mismatch_or_error(
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        "SFTP 远端断点文件",
    )
    .unwrap_err();
    assert!(error.contains("读取SFTP 远端断点文件前缀失败"), "{error}");
}

#[test]
fn local_transfer_creates_empty_files_and_rejects_source_shrink() {
    let root = tempfile::tempdir().unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    state
        .store
        .lock()
        .unwrap()
        .transfers
        .push(test_transfer_task(&profile.id, TransferStatus::Running));

    tauri::async_runtime::block_on(async {
        let empty_source = root.path().join("empty-source.bin");
        let empty_target = root.path().join("empty-target.bin");
        fs::write(&empty_source, []).unwrap();
        let empty_progress = test_transfer_progress_context(
            &state,
            "transfer-commit-test",
            Arc::new(AtomicBool::new(false)),
        );
        let copied = copy_local_file_for_transfer(
            empty_source.to_str().unwrap(),
            empty_target.to_str().unwrap(),
            &empty_progress,
        )
        .await
        .unwrap();
        assert_eq!(copied, 0);
        assert_eq!(fs::metadata(&empty_target).unwrap().len(), 0);

        let source = root.path().join("shrinking-source.bin");
        let target = root.path().join("shrinking-target.bin");
        fs::write(&source, vec![b'x'; 256 * 1024]).unwrap();
        let part = local_resume_part_path(&target);
        let progress = TransferProgressContext {
            state: state.clone(),
            task_id: "transfer-commit-test".to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            last_emit: Arc::new(Mutex::new(Instant::now())),
            started: Instant::now(),
            rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
            rate_limit_bytes_per_second: Some(512 * 1024),
        };
        let source_path = source.display().to_string();
        let target_path = target.display().to_string();
        let copy = tokio::spawn(async move {
            copy_local_file_for_transfer(&source_path, &target_path, &progress).await
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if fs::metadata(&part).is_ok_and(|metadata| metadata.len() >= 64 * 1024) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("local transfer did not write its first block");
        OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap()
            .set_len(64 * 1024)
            .unwrap();

        let error = copy.await.unwrap().unwrap_err();
        assert_eq!(
            error,
            "local transfer size mismatch: copied 65536, expected 262144"
        );
        assert!(!target.exists());
        assert_eq!(fs::metadata(&part).unwrap().len(), 64 * 1024);
    });
}

#[cfg(unix)]
#[test]
fn local_transfer_writes_reject_symlink_targets() {
    let root = std::env::temp_dir().join(format!("portmate-transfer-links-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let protected = root.join("protected.bin");
    fs::write(&protected, b"protected").unwrap();

    let destination = root.join("destination.bin");
    std::os::unix::fs::symlink(&protected, &destination).unwrap();
    let error =
        write_local_transfer_file(destination.to_str().unwrap(), b"replacement").unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let part = root.join("destination.bin.portmate-part");
    std::os::unix::fs::symlink(&protected, &part).unwrap();
    let error = local_resume_offset(&part, 128).unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let temp = root.join("new.part");
    fs::write(&temp, b"new payload").unwrap();
    let final_link = root.join("final.bin");
    std::os::unix::fs::symlink(&protected, &final_link).unwrap();
    let error = finalize_local_resume_file(&temp, &final_link).unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");
    assert!(temp.exists());

    let protected_dir = root.join("protected-dir");
    fs::create_dir(&protected_dir).unwrap();
    let linked_dir = root.join("linked-dir");
    std::os::unix::fs::symlink(&protected_dir, &linked_dir).unwrap();
    let nested_target = linked_dir.join("nested.bin");
    let error = write_local_transfer_file(nested_target.to_str().unwrap(), b"nested").unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert!(!protected_dir.join("nested.bin").exists());

    let copy_source = root.join("copy-source.bin");
    fs::write(&copy_source, b"copy payload").unwrap();
    let copied_target = linked_dir.join("copied.bin");
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
    state
        .store
        .lock()
        .unwrap()
        .transfers
        .push(test_transfer_task(&profile.id, TransferStatus::Running));
    let progress = test_transfer_progress_context(
        &state,
        "transfer-commit-test",
        Arc::new(AtomicBool::new(false)),
    );
    let error = tauri::async_runtime::block_on(copy_local_file_for_transfer(
        copy_source.to_str().unwrap(),
        copied_target.to_str().unwrap(),
        &progress,
    ))
    .unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert!(!protected_dir.join("copied.bin").exists());

    let scp_target = linked_dir.join("scp.bin");
    let error = prepare_local_transfer_target_path(&scp_target, "SCP 本地目标路径").unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert!(!protected_dir.join("scp.bin").exists());

    let missing_parent = root.join("missing-parent").join("payload.bin");
    prepare_local_transfer_target_path(&missing_parent, "本地传输目标路径").unwrap();
    assert!(root.join("missing-parent").is_dir());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn streamed_modem_output_preserves_internal_eof_and_discards_final_padding() {
    let root = std::env::temp_dir().join(format!("portmate-modem-output-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("received.bin");
    let mut output =
        PendingLocalTransferOutput::create(&target, "测试 Modem 本地目标文件").unwrap();
    let mut trailing_padding = 0_u64;
    let mut bytes_written = 0_u64;

    append_modem_data_without_trailing_padding(
        &mut output,
        b"before\x1a\x1a",
        &mut trailing_padding,
        &mut bytes_written,
    )
    .unwrap();
    append_modem_data_without_trailing_padding(
        &mut output,
        b"\x1aafter\x1a\x1a",
        &mut trailing_padding,
        &mut bytes_written,
    )
    .unwrap();
    output.finish().unwrap();

    let expected = b"before\x1a\x1a\x1aafter";
    assert_eq!(bytes_written, expected.len() as u64);
    assert_eq!(fs::read(&target).unwrap(), expected);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn streamed_modem_output_respects_ymodem_declared_length() {
    let root = std::env::temp_dir().join(format!("portmate-ymodem-output-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("received.bin");
    let mut output =
        PendingLocalTransferOutput::create(&target, "测试 Modem 本地目标文件").unwrap();
    let mut bytes_written = 0_u64;

    append_modem_data_with_size_limit(&mut output, b"abcdef", 4, &mut bytes_written).unwrap();
    append_modem_data_with_size_limit(&mut output, b"more", 4, &mut bytes_written).unwrap();
    output.finish().unwrap();

    assert_eq!(bytes_written, 4);
    assert_eq!(fs::read(&target).unwrap(), b"abcd");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn streamed_modem_output_removes_temp_file_when_not_finalized() {
    let root = std::env::temp_dir().join(format!("portmate-modem-temp-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("received.bin");
    let mut output =
        PendingLocalTransferOutput::create(&target, "测试 Modem 本地目标文件").unwrap();
    let temp = output.temp.clone();
    output.file_mut().unwrap().write_all(b"partial").unwrap();
    drop(output);

    assert!(!target.exists());
    assert!(!temp.exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn local_transfer_sources_reject_symlinks() {
    let root = std::env::temp_dir().join(format!("portmate-transfer-sources-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let protected = root.join("protected.bin");
    fs::write(&protected, b"protected").unwrap();
    let source = root.join("source.bin");
    std::os::unix::fs::symlink(&protected, &source).unwrap();

    assert!(open_local_transfer_source(&source, "source").is_err());
    let error = match modem_direction(&StartTransferRequest {
        session_id: "session".to_string(),
        protocol: TransferProtocol::Xmodem,
        source: source.display().to_string(),
        destination: "remote:/tmp/source.bin".to_string(),
    }) {
        Err(error) => error,
        Ok(_) => panic!("symbolic link source was accepted"),
    };
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_file_properties_reports_file_metadata() {
    let root = std::env::temp_dir().join(format!("portmate-file-props-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("payload.bin");
    fs::write(&target, b"payload").unwrap();

    let properties = local_file_properties(target.to_str().unwrap()).unwrap();
    assert_eq!(properties.name, "payload.bin");
    assert_eq!(properties.path, target.display().to_string());
    assert!(!properties.remote);
    assert_eq!(properties.kind, "file");
    assert!(properties.is_file);
    assert!(!properties.is_dir);
    assert_eq!(properties.size, 7);
    assert!(properties.modified.is_some());
    #[cfg(unix)]
    assert!(properties.permissions.is_some());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_drop_plan_preserves_directories_and_skips_unsafe_entries() {
    let root = std::env::temp_dir().join(format!("portmate-drop-plan-{}", Uuid::new_v4()));
    let source = root.join("source-tree");
    let nested = source.join("nested");
    let empty = source.join("empty");
    let destination = root.join("destination");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(&empty).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("top.txt"), b"top").unwrap();
    fs::write(nested.join("payload.bin"), b"payload").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&nested, source.join("nested-link")).unwrap();

    let paths = vec![
        source.display().to_string(),
        nested.join("payload.bin").display().to_string(),
    ];
    let destination = destination.canonicalize().unwrap();
    let plan = plan_external_drop(&paths, Some(&destination)).unwrap();

    assert_eq!(
        plan.directories,
        vec![
            PathBuf::from("source-tree"),
            PathBuf::from("source-tree/empty"),
            PathBuf::from("source-tree/nested"),
        ]
    );
    assert_eq!(
        plan.files
            .iter()
            .map(|file| file.relative.clone())
            .collect::<Vec<_>>(),
        vec![
            PathBuf::from("source-tree/nested/payload.bin"),
            PathBuf::from("source-tree/top.txt"),
        ]
    );
    assert_eq!(plan.total_bytes, 10);
    assert!(plan
        .skipped
        .iter()
        .any(|item| item.contains("already included")));
    #[cfg(unix)]
    assert!(plan
        .skipped
        .iter()
        .any(|item| item.contains("symbolic link")));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_drop_plan_rejects_self_descendants_and_target_conflicts() {
    let root = std::env::temp_dir().join(format!("portmate-drop-guards-{}", Uuid::new_v4()));
    let first = root.join("first/shared");
    let second = root.join("second/shared");
    let destination = root.join("destination");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(first.join("one.txt"), b"one").unwrap();

    let root_destination = root.canonicalize().unwrap();
    let self_error = plan_external_drop(
        &[first.display().to_string()],
        Some(&root.join("first").canonicalize().unwrap()),
    )
    .unwrap_err();
    assert!(self_error.contains("复制到自身"), "{self_error}");

    let descendant = first.join("child-target");
    fs::create_dir_all(&descendant).unwrap();
    let descendant_error = plan_external_drop(
        &[first.display().to_string()],
        Some(&descendant.canonicalize().unwrap()),
    )
    .unwrap_err();
    assert!(descendant_error.contains("子目录"), "{descendant_error}");

    let conflict_error = plan_external_drop(
        &[first.display().to_string(), second.display().to_string()],
        Some(&destination.canonicalize().unwrap()),
    )
    .unwrap_err();
    assert!(
        conflict_error.contains("冲突的目标目录"),
        "{conflict_error}"
    );
    assert!(root_destination.is_dir());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_batch_conflict_policies_fail_skip_overwrite_and_rename() {
    let root = std::env::temp_dir().join(format!("portmate-conflicts-{}", Uuid::new_v4()));
    let source = root.join("source/report.txt");
    let destination = root.join("destination");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(&source, b"new report").unwrap();
    fs::write(destination.join("report.txt"), b"old report").unwrap();
    let destination = destination.canonicalize().unwrap();
    let paths = vec![source.display().to_string()];

    tauri::async_runtime::block_on(async {
        let mut fail = plan_external_drop(&paths, Some(&destination)).unwrap();
        let error = apply_external_drop_conflicts(
            &mut fail,
            None,
            destination.to_str().unwrap(),
            Some(&destination),
            false,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap_err();
        assert!(error.contains("目标文件已存在"), "{error}");

        let mut skip = plan_external_drop(&paths, Some(&destination)).unwrap();
        apply_external_drop_conflicts(
            &mut skip,
            None,
            destination.to_str().unwrap(),
            Some(&destination),
            false,
            TransferConflictPolicy::Skip,
        )
        .await
        .unwrap();
        assert!(skip.files.is_empty());
        assert_eq!(skip.skipped.len(), 1);
        assert_eq!(skip.total_bytes, 0);

        let mut overwrite = plan_external_drop(&paths, Some(&destination)).unwrap();
        apply_external_drop_conflicts(
            &mut overwrite,
            None,
            destination.to_str().unwrap(),
            Some(&destination),
            false,
            TransferConflictPolicy::Overwrite,
        )
        .await
        .unwrap();
        assert_eq!(overwrite.files[0].relative, PathBuf::from("report.txt"));
        assert_eq!(overwrite.total_bytes, 10);

        let mut rename = plan_external_drop(&paths, Some(&destination)).unwrap();
        apply_external_drop_conflicts(
            &mut rename,
            None,
            destination.to_str().unwrap(),
            Some(&destination),
            false,
            TransferConflictPolicy::Rename,
        )
        .await
        .unwrap();
        assert_eq!(rename.files[0].relative, PathBuf::from("report (1).txt"));
        assert_eq!(rename.total_bytes, 10);
    });

    assert_eq!(
        numbered_batch_relative_path("nested/archive.tar.gz", 2).unwrap(),
        "nested/archive.tar (2).gz"
    );
    assert!(validate_batch_relative_path("../escape").is_err());
    assert!(validate_batch_relative_path("folder\\escape").is_err());
    assert!(validate_batch_relative_path("C:/escape").is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_drop_local_batch_copies_nested_files_through_transfer_queue() {
    let root = std::env::temp_dir().join(format!("portmate-drop-local-{}", Uuid::new_v4()));
    let source = root.join("incoming");
    let nested = source.join("nested");
    let empty = source.join("empty");
    let destination = root.join("destination");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(&empty).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("alpha.txt"), b"alpha").unwrap();
    fs::write(nested.join("beta.bin"), b"beta").unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

    tauri::async_runtime::block_on(async {
        let result = start_external_drop_inner(
            &state,
            StartExternalDropRequest {
                session_id: profile.id.clone(),
                paths: vec![source.display().to_string()],
                destination: destination.display().to_string(),
                remote: false,
                conflict_policy: TransferConflictPolicy::Fail,
            },
        )
        .await
        .unwrap();
        assert_eq!(result.tasks.len(), 2);
        assert_eq!(result.directories_prepared, 3);
        assert_eq!(result.total_bytes, 9);
        assert!(result.skipped.is_empty());
        for task in result.tasks {
            let task = wait_for_transfer_terminal_state(&state, &task.id).await;
            assert_eq!(
                task.status,
                TransferStatus::Completed,
                "local recursive drop failed: {:?}",
                task.message
            );
        }
    });

    let copied = destination.join("incoming");
    assert_eq!(fs::read(copied.join("alpha.txt")).unwrap(), b"alpha");
    assert_eq!(fs::read(copied.join("nested/beta.bin")).unwrap(), b"beta");
    assert!(copied.join("empty").is_dir());

    let _ = fs::remove_dir_all(root);
}

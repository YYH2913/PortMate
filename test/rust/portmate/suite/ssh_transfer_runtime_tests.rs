#[cfg(unix)]
#[test]
fn sftp_in_flight_request_observes_transfer_cancellation() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping SFTP cancellation test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-silent-sftp-user";
        let secret = "PortMate silent SFTP secret";
        let (port, counters, server_task) =
            spawn_silent_sftp_test_server(&host_key, username, secret, Duration::from_secs(1))
                .await;
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(handle
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let handle = Arc::new(tokio::sync::Mutex::new(handle));
        let sftp = open_sftp_session_with_timeout(Arc::clone(&handle), Duration::from_secs(1))
            .await
            .unwrap();
        let state = test_app_state(
            test_shell_profile(),
            root.path().join("portmate-store.sqlite3"),
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = test_transfer_progress_context(
            &state,
            "unused-silent-sftp-cancel",
            Arc::clone(&cancel),
        );
        let request_counters = Arc::clone(&counters);
        let cancel_task = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(1), async {
                while request_counters.lstat_attempts.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("silent SFTP server did not receive LSTAT");
            cancel.store(true, Ordering::SeqCst);
        });

        let started = Instant::now();
        let local_target = root.path().join("silent-target.bin");
        let transfer = sftp_download(
            &sftp,
            "/silent-source.bin",
            local_target.to_str().unwrap(),
            &progress,
        );
        let error = await_sftp_transfer_with_cancellation(transfer, &progress)
            .await
            .unwrap_err();
        assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
        assert!(started.elapsed() < Duration::from_millis(500));
        cancel_task.await.unwrap();
        sftp.close().await.unwrap();

        let channel = open_shared_russh_exec_channel(
            &handle,
            "true",
            Duration::from_secs(1),
            "SFTP cancellation follow-up exec",
        )
        .await
        .unwrap();
        close_russh_channel_bounded(&channel).await;
        assert_eq!(counters.subsystem_requests.load(Ordering::SeqCst), 1);
        assert_eq!(counters.lstat_attempts.load(Ordering::SeqCst), 1);
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(
            counters.session_channel_completions.load(Ordering::SeqCst),
            2
        );

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn scp_upload_closes_success_and_rejects_status_after_eof() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping SCP upload completion test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);
    let source = root.path().join("empty.bin");
    fs::write(&source, []).unwrap();

    tauri::async_runtime::block_on(async {
        let username = "portmate-scp-upload-completion-user";
        let secret = "PortMate SCP upload completion secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(handle
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let handle = Arc::new(tokio::sync::Mutex::new(handle));
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

        let uploaded = scp_upload(
            Arc::clone(&handle),
            source.to_str().unwrap(),
            "/__PORTMATE_TEST_SCP_UPLOAD_SUCCESS__",
            &progress,
        )
        .await
        .unwrap();
        assert_eq!(uploaded, 0);

        let error = scp_upload(
            Arc::clone(&handle),
            source.to_str().unwrap(),
            "/__PORTMATE_TEST_SCP_UPLOAD_EOF_BEFORE_NONZERO__",
            &progress,
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            "SCP upload remote returned non-zero 12: late SCP upload failure"
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.channel_closes.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("SCP upload channels were not closed on success and failure");
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn scp_download_validates_completion_and_protocol_streams() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping SCP download completion test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-scp-download-completion-user";
        let secret = "PortMate SCP download completion secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(handle
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let handle = Arc::new(tokio::sync::Mutex::new(handle));
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

        let target = root.path().join("download-success.bin");
        let part = local_resume_part_path(&target);
        fs::write(&part, b"zz").unwrap();
        let downloaded = scp_download_with_idle_timeout(
            Arc::clone(&handle),
            "/__PORTMATE_TEST_SCP_DOWNLOAD_SUCCESS__",
            target.to_str().unwrap(),
            &progress,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(downloaded, 4);
        assert_eq!(fs::read(&target).unwrap(), b"data");
        assert!(!part.exists());

        let failed_target = root.path().join("download-failed.bin");
        let failed_part = local_resume_part_path(&failed_target);
        let error = scp_download_with_idle_timeout(
            Arc::clone(&handle),
            "/__PORTMATE_TEST_SCP_DOWNLOAD_EOF_BEFORE_NONZERO__",
            failed_target.to_str().unwrap(),
            &progress,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            "SCP download remote returned non-zero 13: late SCP download failure"
        );
        assert!(!failed_target.exists());
        assert_eq!(fs::read(&failed_part).unwrap(), b"data");

        let stderr_target = root.path().join("download-with-stderr.bin");
        let downloaded = scp_download_with_idle_timeout(
            Arc::clone(&handle),
            "/__PORTMATE_TEST_SCP_DOWNLOAD_STDERR_BEFORE_DATA__",
            stderr_target.to_str().unwrap(),
            &progress,
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(downloaded, 4);
        assert_eq!(fs::read(&stderr_target).unwrap(), b"data");

        let oversized_target = root.path().join("download-oversized-header.bin");
        let error = scp_download_with_idle_timeout(
            Arc::clone(&handle),
            "/__PORTMATE_TEST_SCP_DOWNLOAD_OVERSIZED_HEADER__",
            oversized_target.to_str().unwrap(),
            &progress,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            format!("SCP 读取文件头 超过协议行上限（{MAX_SCP_PROTOCOL_LINE_BYTES} bytes）")
        );
        assert!(!oversized_target.exists());

        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.channel_closes.load(Ordering::SeqCst) < 4 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("SCP download channels were not closed across protocol outcomes");
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 4);

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn scp_download_silent_peer_observes_cancellation_and_idle_timeout() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping silent SCP test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-silent-scp-user";
        let secret = "PortMate silent SCP secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server_with_delays(&host_key, username, secret, None, None, None)
                .await;
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(handle
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let handle = Arc::new(tokio::sync::Mutex::new(handle));
        let state = test_app_state(
            test_shell_profile(),
            root.path().join("portmate-store.sqlite3"),
        );

        let cancel = Arc::new(AtomicBool::new(false));
        let cancellation_progress = TransferProgressContext {
            state: state.clone(),
            task_id: "unused-silent-scp-cancel".to_string(),
            cancel: Arc::clone(&cancel),
            last_emit: Arc::new(Mutex::new(Instant::now())),
            started: Instant::now(),
            rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
            rate_limit_bytes_per_second: None,
        };
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let error = scp_download_with_idle_timeout(
            Arc::clone(&handle),
            "/silent-cancel.bin",
            root.path().join("cancel.bin").to_str().unwrap(),
            &cancellation_progress,
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
        assert!(started.elapsed() < Duration::from_millis(500));
        cancel_task.await.unwrap();

        let idle_progress = TransferProgressContext {
            state,
            task_id: "unused-silent-scp-idle".to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            last_emit: Arc::new(Mutex::new(Instant::now())),
            started: Instant::now(),
            rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
            rate_limit_bytes_per_second: None,
        };
        let started = Instant::now();
        let error = scp_download_with_idle_timeout(
            Arc::clone(&handle),
            "/silent-idle.bin",
            root.path().join("idle.bin").to_str().unwrap(),
            &idle_progress,
            Duration::from_millis(30),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "SCP 等待文件头 空闲超时（30 ms）");
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);
        assert_eq!(
            counters.session_channel_completions.load(Ordering::SeqCst),
            2
        );

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn remote_copy_silent_peer_observes_cancellation_and_idle_timeout() {
    let _runtime_guard = shared_runtime_test_guard();
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping silent remote-copy test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-silent-remote-copy-user";
        let secret = "PortMate silent remote-copy secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server_with_delays(&host_key, username, secret, None, None, None)
                .await;
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(handle
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let handle = Arc::new(tokio::sync::Mutex::new(handle));
        let state = test_app_state(
            test_shell_profile(),
            root.path().join("portmate-store.sqlite3"),
        );

        let cancel = Arc::new(AtomicBool::new(false));
        let cancellation_progress = test_transfer_progress_context(
            &state,
            "unused-silent-remote-copy-cancel",
            Arc::clone(&cancel),
        );
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.store(true, Ordering::SeqCst);
        });
        let started = Instant::now();
        let error = remote_copy_with_timeouts(
            Arc::clone(&handle),
            "/silent-source.bin",
            "/silent-destination.bin",
            &cancellation_progress,
            Duration::from_secs(1),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
        assert!(started.elapsed() < Duration::from_millis(500));
        cancel_task.await.unwrap();

        let idle_progress = test_transfer_progress_context(
            &state,
            "unused-silent-remote-copy-idle",
            Arc::new(AtomicBool::new(false)),
        );
        let started = Instant::now();
        let error = remote_copy_with_timeouts(
            Arc::clone(&handle),
            "/silent-source.bin",
            "/silent-destination.bin",
            &idle_progress,
            Duration::from_millis(30),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "SSH remote copy 空闲超时（30 ms）");
        assert!(started.elapsed() < Duration::from_millis(500));
        state
            .store
            .lock()
            .unwrap()
            .transfers
            .push(test_transfer_task(
                &test_shell_profile().id,
                TransferStatus::Running,
            ));
        let late_status_progress = test_transfer_progress_context(
            &state,
            "transfer-commit-test",
            Arc::new(AtomicBool::new(false)),
        );
        let error = remote_copy_with_timeouts(
            Arc::clone(&handle),
            "/__PORTMATE_TEST_REMOTE_COPY_EOF_BEFORE_NONZERO__",
            "/destination.bin",
            &late_status_progress,
            Duration::from_secs(1),
            Duration::from_secs(15),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            "SSH remote copy 返回非零状态 11: late remote-copy failure"
        );
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 3);
        assert_eq!(
            counters.session_channel_completions.load(Ordering::SeqCst),
            3
        );

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

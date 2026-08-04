#[cfg(unix)]
#[test]
fn ssh_exec_capture_handles_eof_status_order_and_closes_every_exit_path() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping SSH exec cleanup test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-exec-cleanup-user";
        let secret = "PortMate exec cleanup secret";
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
        let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_russh(
            handle,
        )));

        let output = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_SUCCESS__",
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(output, "captured");

        let nonzero_error = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_NONZERO__",
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(nonzero_error, "SSH exec 返回非零状态 7: remote failure");

        let late_status_error = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_EOF_BEFORE_NONZERO__",
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(
            late_status_error,
            "SSH exec 返回非零状态 9: late status failure"
        );

        let timeout_error = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_TIMEOUT__",
            Duration::from_millis(500),
        )
        .await
        .unwrap_err();
        assert_eq!(timeout_error, "SSH exec 超时");

        let overflow_error = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_OVERFLOW__",
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert!(overflow_error.contains("stderr"), "{overflow_error}");
        assert!(
            overflow_error.contains(&MAX_SSH_EXEC_STDERR_BYTES.to_string()),
            "{overflow_error}"
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.channel_closes.load(Ordering::SeqCst) < 5 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("SSH exec channels were not closed on every exit path");
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 5);

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

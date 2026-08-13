#[cfg(unix)]
fn connect_libssh_password_test_session(
    port: u16,
    username: &str,
    secret: &str,
) -> libssh_rs::Session {
    let session = libssh_rs::Session::new().unwrap();
    // libssh 0.10's default cipher list has a trailing comma that russh rejects.
    session
        .set_option(libssh_rs::SshOption::CiphersCS("aes256-ctr".to_string()))
        .unwrap();
    session
        .set_option(libssh_rs::SshOption::CiphersSC("aes256-ctr".to_string()))
        .unwrap();
    session
        .set_option(libssh_rs::SshOption::ProcessConfig(false))
        .unwrap();
    session
        .set_option(libssh_rs::SshOption::Hostname("127.0.0.1".to_string()))
        .unwrap();
    session
        .set_option(libssh_rs::SshOption::Port(port))
        .unwrap();
    session
        .set_option(libssh_rs::SshOption::User(Some(username.to_string())))
        .unwrap();
    session.connect().unwrap();
    assert_eq!(
        authenticate_libssh_with_order(
            &session,
            &[AuthMethod::GssapiWithMic, AuthMethod::Password],
            Some(secret),
            &[],
            None,
            false,
            false,
        )
        .unwrap(),
        AuthMethod::Password
    );
    session
}

#[cfg(unix)]
#[test]
fn libssh_backend_exec_normalizes_output_and_exit_status() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh backend test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-backend-user";
        let secret = "PortMate libssh backend secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;

        let session = connect_libssh_password_test_session(port, username, secret);

        let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::<
            AcceptAnyTestSshClient,
        >::from_libssh(session)));
        let output = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_SUCCESS__",
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(output, "captured");

        let error = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_NONZERO__",
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "SSH exec 返回非零状态 7: remote failure");
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);

        handle
            .lock()
            .await
            .disconnect("PortMate libssh backend test complete")
            .await
            .unwrap();
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn libssh_backend_supports_scp_and_remote_copy_channels() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh transfer test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);
    let upload_source = root.path().join("upload.bin");
    fs::write(&upload_source, b"data").unwrap();
    let empty_source = root.path().join("empty.bin");
    fs::write(&empty_source, []).unwrap();

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-transfer-user";
        let secret = "PortMate libssh transfer secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;
        let session = connect_libssh_password_test_session(port, username, secret);
        let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::<
            AcceptAnyTestSshClient,
        >::from_libssh(session)));

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

        assert_eq!(
            scp_upload(
                Arc::clone(&handle),
                upload_source.to_str().unwrap(),
                "/__PORTMATE_TEST_SCP_UPLOAD_DATA_SUCCESS__",
                &progress,
            )
            .await
            .unwrap(),
            4
        );
        assert_eq!(counters.scp_upload_bytes.load(Ordering::SeqCst), 4);
        assert_eq!(
            scp_upload(
                Arc::clone(&handle),
                empty_source.to_str().unwrap(),
                "/__PORTMATE_TEST_SCP_UPLOAD_EOF_BEFORE_NONZERO__",
                &progress,
            )
            .await
            .unwrap_err(),
            "SCP upload remote returned non-zero 12: late SCP upload failure"
        );

        let download_target = root.path().join("download.bin");
        assert_eq!(
            scp_download_with_idle_timeout(
                Arc::clone(&handle),
                "/__PORTMATE_TEST_SCP_DOWNLOAD_SUCCESS__",
                download_target.to_str().unwrap(),
                &progress,
                Duration::from_secs(2),
            )
            .await
            .unwrap(),
            4
        );
        assert_eq!(fs::read(&download_target).unwrap(), b"data");

        assert_eq!(
            remote_copy_with_timeouts(
                Arc::clone(&handle),
                "/__PORTMATE_TEST_REMOTE_COPY_SUCCESS__",
                "/destination.bin",
                &progress,
                Duration::from_secs(2),
                Duration::from_secs(4),
            )
            .await
            .unwrap(),
            4
        );
        assert_eq!(
            remote_copy_with_timeouts(
                Arc::clone(&handle),
                "/__PORTMATE_TEST_REMOTE_COPY_EOF_BEFORE_NONZERO__",
                "/destination.bin",
                &progress,
                Duration::from_secs(2),
                Duration::from_secs(4),
            )
            .await
            .unwrap_err(),
            "SSH remote copy 返回非零状态 11: late remote-copy failure"
        );
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 5);

        handle
            .lock()
            .await
            .disconnect("PortMate libssh transfer test complete")
            .await
            .unwrap();
        server_task.abort();
        let _ = server_task.await;
    });
}

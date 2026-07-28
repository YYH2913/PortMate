use super::*;

#[cfg(unix)]
#[test]
fn libssh_backend_exec_normalizes_output_and_exit_status() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh backend test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-backend-user";
        let secret = "PortMate libssh backend secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;

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
        let ConnectionConfig::Ssh(mut ssh) = test_ssh_profile().connection else {
            panic!("expected SSH profile");
        };
        ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::Password];
        assert_eq!(
            authenticate_libssh_with_order(&session, &ordered_auth_methods(&ssh), Some(secret),)
                .unwrap(),
            AuthMethod::Password
        );

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

#[test]
fn libssh_backend_selection_accepts_supported_gssapi_mixed_auth_order() {
    let ConnectionConfig::Ssh(mut ssh) = test_ssh_profile().connection else {
        panic!("expected SSH profile");
    };
    ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic];
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.auth_order = vec![
        AuthMethod::GssapiWithMic,
        AuthMethod::KeyboardInteractive,
        AuthMethod::Password,
    ];
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.auth_order = vec![AuthMethod::Password, AuthMethod::GssapiWithMic];
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::PublicKey];
    assert!(!ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.auth_order = vec![AuthMethod::Password];
    assert!(!ssh_uses_libssh_gssapi_backend(&ssh));
}

#[cfg(unix)]
#[test]
fn libssh_mixed_auth_falls_back_to_keyboard_interactive() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh keyboard-interactive test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-keyboard-user";
        let secret = "PortMate libssh keyboard secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;

        let session = libssh_rs::Session::new().unwrap();
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
                &[AuthMethod::GssapiWithMic, AuthMethod::KeyboardInteractive,],
                Some(secret),
            )
            .unwrap(),
            AuthMethod::KeyboardInteractive
        );
        assert_eq!(
            counters
                .keyboard_interactive_successes
                .load(Ordering::SeqCst),
            1
        );

        session.disconnect();
        server_task.abort();
        let _ = server_task.await;
    });
}

#[test]
fn tunnel_label_reflects_assigned_local_port() {
    assert_eq!(
        tunnel_label(TunnelMode::Local, "127.0.0.1", 4567, "10.0.0.5", 22),
        "127.0.0.1:4567 -> 10.0.0.5:22"
    );
    assert_eq!(
        tunnel_label(TunnelMode::Dynamic, "127.0.0.1", 1080, "", 0),
        "SOCKS5 127.0.0.1:1080"
    );
}

#[test]
fn bounded_connection_step_preserves_results_and_stops_pending_operations() {
    tauri::async_runtime::block_on(async {
        let success = bounded_connection_step(
            async { Ok::<_, &'static str>("connected") },
            Duration::from_secs(1),
        )
        .await
        .unwrap();
        assert_eq!(success, "connected");

        let failed = bounded_connection_step(
            async { Err::<(), _>("connection refused") },
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(
            failed,
            BoundedConnectionStepError::Failed("connection refused".to_string())
        );

        let timed_out = bounded_connection_step(
            std::future::pending::<Result<(), &'static str>>(),
            Duration::from_millis(20),
        )
        .await
        .unwrap_err();
        assert_eq!(timed_out, BoundedConnectionStepError::TimedOut);
    });
}

#[cfg(unix)]
#[test]
fn shared_ssh_disconnect_times_out_waiting_for_a_busy_backend_handle() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping shared SSH disconnect timeout test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-shared-disconnect-user";
        let secret = "PortMate shared disconnect secret";
        let (port, _, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;
        let mut session = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(session
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_russh(
            session,
        )));
        let guard = handle.lock().await;
        let disconnect_handle = Arc::clone(&handle);
        let warning = tokio::time::timeout(Duration::from_secs(2), async move {
            request_shared_backend_disconnect_with_timeout(
                &disconnect_handle,
                "PortMate shared disconnect timeout test",
            )
            .await
        })
        .await
        .expect("shared SSH disconnect did not honor its lock deadline")
        .expect("shared SSH disconnect unexpectedly acquired a held backend handle");
        assert!(warning.contains("handle lock timed out"), "{warning}");
        drop(guard);
        assert_eq!(
            request_shared_backend_disconnect_with_timeout(
                &handle,
                "PortMate shared disconnect cleanup",
            )
            .await,
            None
        );

        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn direct_tcpip_open_timeout_disconnects_a_stalled_russh_session() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping direct-tcpip timeout test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-direct-tcpip-user";
        let secret = "PortMate direct-tcpip secret";
        let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
            &host_key,
            username,
            secret,
            None,
            None,
            Some(Duration::from_millis(200)),
        )
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

        let error = open_direct_tcpip_with_timeout(
            &handle,
            "127.0.0.1".to_string(),
            9,
            "127.0.0.1".to_string(),
            0,
            Duration::from_millis(30),
            "PortMate direct-tcpip timeout test",
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            DirectTcpipOpenError::TimedOut {
                timeout_ms: 30,
                cleanup_warning: None,
            }
        );
        assert_eq!(counters.direct_tcpip_attempts.load(Ordering::SeqCst), 1);
        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.direct_tcpip_completions.load(Ordering::SeqCst) != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("delayed direct-tcpip callback did not finish");

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn ssh_authentication_timeout_disconnects_a_stalled_russh_session() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping SSH authentication timeout test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-auth-timeout-user";
        let secret = "PortMate authentication timeout secret";
        let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
            &host_key,
            username,
            secret,
            Some(Duration::from_millis(200)),
            None,
            None,
        )
        .await;
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        let mut ssh = match test_ssh_profile().connection {
            ConnectionConfig::Ssh(ssh) => ssh,
            _ => unreachable!("test SSH profile changed transport"),
        };
        ssh.identity_policy.auth_order = vec![AuthMethod::Password];
        ssh.identity_policy.last_successful = None;
        ssh.identity_refs.clear();
        ssh.agent_policy.enabled = false;

        let error = authenticate_ssh_with_timeout(
            &mut handle,
            SshAuthenticationRequest {
                ssh,
                username: username.to_string(),
                password: Some(secret.to_string()),
                passphrase: None,
                agent_socket_path: None,
                timeout: Duration::from_millis(30),
                disconnect_description: "PortMate authentication timeout test",
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            SshAuthenticationError::TimedOut {
                timeout_ms: 30,
                cleanup_warning: None,
            }
        );
        assert_eq!(counters.password_attempts.load(Ordering::SeqCst), 1);
        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.password_completions.load(Ordering::SeqCst) != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("delayed password authentication callback did not finish");
        assert_eq!(counters.password_successes.load(Ordering::SeqCst), 1);

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn ssh_terminal_setup_timeout_disconnects_a_stalled_russh_session() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping SSH terminal setup timeout test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-terminal-setup-user";
        let secret = "PortMate terminal setup secret";
        let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
            &host_key,
            username,
            secret,
            None,
            Some(Duration::from_millis(200)),
            None,
        )
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
        let profile = test_ssh_profile();
        let ssh = match &profile.connection {
            ConnectionConfig::Ssh(ssh) => ssh,
            _ => unreachable!("test SSH profile changed transport"),
        };

        let error = open_ssh_terminal_channel_with_timeout(
            &handle,
            &profile,
            ssh,
            Duration::from_millis(30),
            "PortMate terminal setup timeout test",
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            SshTerminalSetupError::TimedOut {
                timeout_ms: 30,
                cleanup_warning: None,
            }
        );
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 1);
        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.session_channel_completions.load(Ordering::SeqCst) != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("delayed SSH session-channel callback did not finish");

        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn ssh_auxiliary_setups_timeout_and_disconnect_stalled_russh_sessions() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping SSH auxiliary setup timeout test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-auxiliary-setup-user";
        let secret = "PortMate auxiliary setup secret";
        let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
            &host_key,
            username,
            secret,
            None,
            Some(Duration::from_millis(200)),
            None,
        )
        .await;

        let mut exec_handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(exec_handle
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let exec_handle = Arc::new(tokio::sync::Mutex::new(exec_handle));
        let error = open_shared_russh_exec_channel(
            &exec_handle,
            "true",
            Duration::from_millis(30),
            "SSH auxiliary exec test",
        )
        .await
        .unwrap_err();
        assert_eq!(error, "SSH auxiliary exec test setup 超时（30 ms）");
        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.session_channel_completions.load(Ordering::SeqCst) != 1 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("delayed auxiliary exec channel callback did not finish");
        drop(exec_handle);

        let mut sftp_handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            AcceptAnyTestSshClient,
        )
        .await
        .unwrap();
        assert!(sftp_handle
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let sftp_handle = Arc::new(tokio::sync::Mutex::new(sftp_handle));
        let error = open_sftp_session_with_timeout(sftp_handle, Duration::from_millis(30))
            .await
            .err()
            .expect("stalled SFTP setup unexpectedly succeeded");
        assert_eq!(error, "SFTP setup 超时（30 ms）");
        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.session_channel_completions.load(Ordering::SeqCst) != 2 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("delayed SFTP channel callback did not finish");
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);

        server_task.abort();
        let _ = server_task.await;
    });
}

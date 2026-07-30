use super::*;

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

    let root = tempfile::tempdir().unwrap();
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

    let root = tempfile::tempdir().unwrap();
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
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.agent_policy.enabled = true;
    ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::BeforeProfileKeys;
    ssh.identity_policy.identities_only = false;
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.identities_only = true;
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_refs.push(IdentityRef {
        id: "agent-key".to_string(),
        label: "Agent key".to_string(),
        source: IdentitySource::Agent,
        fingerprint_sha256: Some("SHA256:test".to_string()),
        path: None,
        secret_ref: None,
    });
    assert_eq!(ssh_uses_libssh_gssapi_backend(&ssh), cfg!(unix));
    assert_eq!(libssh_agent_offer_positions(&ssh, true), (false, true));

    ssh.identity_policy.identities_only = false;
    assert_eq!(libssh_agent_offer_positions(&ssh, true), (true, false));
    assert_eq!(libssh_agent_offer_positions(&ssh, false), (false, false));

    ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
    assert_eq!(libssh_agent_offer_positions(&ssh, true), (false, false));

    ssh.agent_policy.enabled = false;
    ssh.identity_policy.auth_order = vec![AuthMethod::Password];
    assert!(!ssh_uses_libssh_gssapi_backend(&ssh));
}

#[cfg(target_os = "linux")]
#[test]
fn libssh_gssapi_file_cache_validation_rejects_corruption_without_restricting_other_backends() {
    use std::ffi::OsString;

    let root = std::env::temp_dir().join(format!(
        "portmate-gssapi-cache-validation-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let cache_path = root.join("ticket.ccache");
    let cache_name = OsString::from(format!("FILE:{}", cache_path.display()));

    assert!(validate_libssh_gssapi_credential_cache(Some(cache_name.as_os_str())).is_ok());
    assert!(
        validate_libssh_gssapi_credential_cache(Some(std::ffi::OsStr::new(
            "KEYRING:persistent:1000"
        )))
        .is_ok()
    );

    fs::write(&cache_path, []).unwrap();
    let truncated =
        validate_libssh_gssapi_credential_cache(Some(cache_name.as_os_str())).unwrap_err();
    assert!(truncated.contains("truncated"), "{truncated}");

    fs::write(&cache_path, b"not-a-valid-cache").unwrap();
    let invalid =
        validate_libssh_gssapi_credential_cache(Some(cache_name.as_os_str())).unwrap_err();
    assert!(invalid.contains("invalid"), "{invalid}");

    fs::write(&cache_path, [5, 4]).unwrap();
    let short_header =
        validate_libssh_gssapi_credential_cache(Some(cache_name.as_os_str())).unwrap_err();
    assert!(short_header.contains("truncated"), "{short_header}");

    fs::remove_dir_all(root).unwrap();
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
                &[],
                None,
                false,
                false,
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

#[cfg(unix)]
async fn assert_libssh_local_and_dynamic_tunnels(state: &AppState, session_id: &str) {
    let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let echo_address = echo_listener.local_addr().unwrap();
    drop(echo_listener);
    let tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: session_id.to_string(),
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: "127.0.0.1".to_string(),
            target_port: echo_address.port(),
            label: None,
        },
    )
    .await
    .unwrap();

    let mut failed_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
        .await
        .unwrap();
    failed_client.write_all(b"ping").await.unwrap();
    let mut closed_byte = [0_u8; 1];
    let read = tokio::time::timeout(Duration::from_secs(3), failed_client.read(&mut closed_byte))
        .await
        .expect("failed libssh local tunnel client did not close");
    assert_tunnel_client_closed(read, "failed libssh local tunnel client");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 1 {
                assert!(status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("direct-tcpip open failed")));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh local tunnel failure metrics did not settle");

    let echo_listener = TcpListener::bind(echo_address).await.unwrap();
    let echo = tokio::spawn(async move {
        let (mut socket, _) = echo_listener.accept().await.unwrap();
        let mut request = [0_u8; 4];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut tunnel_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
        .await
        .unwrap();
    tunnel_client.write_all(b"ping").await.unwrap();
    let mut response = [0_u8; 4];
    tunnel_client.read_exact(&mut response).await.unwrap();
    assert_eq!(&response, b"pong");
    drop(tunnel_client);
    echo.await.unwrap();

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 2 {
                assert_eq!(status.tcp_to_ssh_bytes, 4);
                assert_eq!(status.ssh_to_tcp_bytes, 4);
                assert!(status.last_error.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh local tunnel metrics did not settle");
    let stopped = stop_tunnel_inner(state, &tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);

    let dynamic_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let dynamic_echo_address = dynamic_echo_listener.local_addr().unwrap();
    drop(dynamic_echo_listener);
    let dynamic_tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: session_id.to_string(),
            mode: TunnelMode::Dynamic,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: String::new(),
            target_port: 0,
            label: None,
        },
    )
    .await
    .unwrap();
    let [port_high, port_low] = dynamic_echo_address.port().to_be_bytes();

    let mut failed_socks_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
        .await
        .unwrap();
    failed_socks_client.write_all(&[5, 1, 0]).await.unwrap();
    let mut failed_method = [0_u8; 2];
    failed_socks_client
        .read_exact(&mut failed_method)
        .await
        .unwrap();
    assert_eq!(failed_method, [5, 0]);
    failed_socks_client
        .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
        .await
        .unwrap();
    let mut failed_socks_reply = [0_u8; 10];
    failed_socks_client
        .read_exact(&mut failed_socks_reply)
        .await
        .unwrap();
    assert_eq!(failed_socks_reply, super::socks5_reply(5));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == dynamic_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 1 {
                assert!(status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("dynamic direct-tcpip open failed")));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh dynamic tunnel failure metrics did not settle");

    let dynamic_echo_listener = TcpListener::bind(dynamic_echo_address).await.unwrap();
    let dynamic_echo = tokio::spawn(async move {
        let (mut socket, _) = dynamic_echo_listener.accept().await.unwrap();
        let mut request = [0_u8; 4];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut socks_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
        .await
        .unwrap();
    socks_client.write_all(&[5, 1, 0]).await.unwrap();
    let mut method = [0_u8; 2];
    socks_client.read_exact(&mut method).await.unwrap();
    assert_eq!(method, [5, 0]);
    socks_client
        .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
        .await
        .unwrap();
    let mut socks_reply = [0_u8; 10];
    socks_client.read_exact(&mut socks_reply).await.unwrap();
    assert_eq!(socks_reply, super::socks5_reply(0));
    socks_client.write_all(b"ping").await.unwrap();
    let mut socks_response = [0_u8; 4];
    socks_client.read_exact(&mut socks_response).await.unwrap();
    assert_eq!(&socks_response, b"pong");
    drop(socks_client);
    dynamic_echo.await.unwrap();

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == dynamic_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 2 {
                assert_eq!(status.tcp_to_ssh_bytes, 4);
                assert_eq!(status.ssh_to_tcp_bytes, 4);
                assert!(status.last_error.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh dynamic tunnel metrics did not settle");
    let stopped = stop_tunnel_inner(state, &dynamic_tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);

    let remote_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let remote_echo_address = remote_echo_listener.local_addr().unwrap();
    drop(remote_echo_listener);
    let remote_tunnel = create_tunnel_inner(
        state,
        CreateTunnelRequest {
            session_id: session_id.to_string(),
            mode: TunnelMode::Remote,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: "127.0.0.1".to_string(),
            target_port: remote_echo_address.port(),
            label: None,
        },
    )
    .await
    .unwrap();
    assert_ne!(remote_tunnel.bind_port, 0);

    let mut failed_remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
        .await
        .unwrap();
    failed_remote_client.write_all(b"ping").await.unwrap();
    let mut closed_byte = [0_u8; 1];
    let read = tokio::time::timeout(
        Duration::from_secs(3),
        failed_remote_client.read(&mut closed_byte),
    )
    .await
    .expect("failed libssh remote tunnel client did not close");
    assert_tunnel_client_closed(read, "failed libssh remote tunnel client");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == remote_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 1 {
                assert!(status
                    .last_error
                    .as_deref()
                    .is_some_and(|error| error.contains("target connect failed")));
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh remote tunnel failure metrics did not settle");

    let remote_echo_listener = TcpListener::bind(remote_echo_address).await.unwrap();
    let remote_echo = tokio::spawn(async move {
        let (mut socket, _) = remote_echo_listener.accept().await.unwrap();
        let mut request = [0_u8; 4];
        socket.read_exact(&mut request).await.unwrap();
        assert_eq!(&request, b"ping");
        socket.write_all(b"pong").await.unwrap();
    });
    let mut remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
        .await
        .unwrap();
    remote_client.write_all(b"ping").await.unwrap();
    let mut remote_response = [0_u8; 4];
    remote_client
        .read_exact(&mut remote_response)
        .await
        .unwrap();
    assert_eq!(&remote_response, b"pong");
    drop(remote_client);
    remote_echo.await.unwrap();

    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let status = list_tunnels_inner(state, Some(session_id))
                .unwrap()
                .into_iter()
                .find(|status| status.spec.id == remote_tunnel.id)
                .unwrap();
            if status.active_connections == 0 && status.total_connections == 2 {
                assert_eq!(status.tcp_to_ssh_bytes, 4);
                assert_eq!(status.ssh_to_tcp_bytes, 4);
                assert!(status.last_error.is_none());
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh remote tunnel metrics did not settle");
    let stopped = stop_tunnel_inner(state, &remote_tunnel.id).await.unwrap();
    assert!(!stopped.spec.enabled);
    tokio::time::timeout(Duration::from_secs(3), async {
        while let Ok(stream) = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port)).await {
            drop(stream);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("libssh remote tunnel listener remained reachable after stop");
}

#[cfg(unix)]
#[test]
fn libssh_gssapi_falls_back_to_ordered_explicit_public_keys() {
    let _runtime_guard = shared_runtime_test_guard();
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping libssh public-key test: sshd is not installed");
        return;
    };
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh public-key test: ssh-keygen is not installed");
        return;
    }

    let root = std::env::temp_dir().join(format!("portmate-libssh-public-key-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let host_key = root.join("ssh_host_ed25519_key");
    let rejected_key = root.join("id_rejected");
    let accepted_key = root.join("id_accepted");
    generate_ed25519_test_key(&host_key);
    generate_ed25519_test_key(&rejected_key);
    let passphrase = "PortMate libssh encrypted key passphrase";
    let keygen = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", passphrase, "-f"])
        .arg(&accepted_key)
        .status()
        .unwrap();
    assert!(keygen.success(), "ssh-keygen failed for encrypted key");
    let authorized_keys = root.join("authorized_keys");
    fs::copy(accepted_key.with_extension("pub"), &authorized_keys).unwrap();
    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let config_path = root.join("sshd_config");
    write_openssh_test_config(
        &config_path,
        &host_key,
        &root.join("sshd.pid"),
        &authorized_keys,
        port,
    );
    let mut sshd = spawn_openssh_test_server(sshd_path, &config_path);

    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(&mut sshd, port, "libssh public-key sshd").await;
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.endpoint.host = "127.0.0.1".to_string();
        ssh.endpoint.port = port;
        ssh.username = openssh_test_username();
        ssh.reconnect = false;
        ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
        ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::PublicKey];
        ssh.identity_policy.identities_only = true;
        ssh.identity_refs = [(&rejected_key, "rejected"), (&accepted_key, "accepted")]
            .into_iter()
            .map(|(path, id)| IdentityRef {
                id: id.to_string(),
                label: format!("{id} key"),
                source: IdentitySource::SystemFile,
                fingerprint_sha256: None,
                path: Some(path.display().to_string()),
                secret_ref: None,
            })
            .collect();
        ssh.agent_policy.enabled = false;
        ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;

        let failed_state =
            test_app_state(profile.clone(), root.join("failed-portmate-store.sqlite3"));
        let error = open_ssh_session(
            &failed_state,
            profile.clone(),
            None,
            Some("wrong passphrase".to_string()),
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("libssh SSH authentication failed"),
            "{error}"
        );
        assert!(error.contains("private key 解析失败"), "{error}");
        assert!(!failed_state.ssh.lock().unwrap().contains_key(&profile.id));

        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_ssh_session(&state, profile.clone(), None, Some(passphrase.to_string()))
            .await
            .unwrap();
        let health = ssh_health::check_ssh_health_inner(&state, &profile.id, true)
            .await
            .unwrap();
        assert_eq!(health.status, ssh_health::SshHealthStatus::Healthy);
        assert_eq!(health.backend, SshBackendKind::Libssh);
        assert_eq!(health.authentication_method, AuthMethod::PublicKey);
        assert!(health.terminal_channel_open);
        assert!(health.terminal_error.is_none());
        assert!(health.sftp_probed);
        assert!(health.sftp_round_trip_ms.is_some());
        assert!(health.sftp_error.is_none());

        let terminal_channel_open = state
            .ssh
            .lock()
            .unwrap()
            .get(&profile.id)
            .unwrap()
            .terminal_channel_open
            .clone();
        terminal_channel_open.store(false, Ordering::SeqCst);
        let terminal_closed_health = ssh_health::check_ssh_health_inner(&state, &profile.id, false)
            .await
            .unwrap();
        assert_eq!(
            terminal_closed_health.status,
            ssh_health::SshHealthStatus::Degraded
        );
        assert!(!terminal_closed_health.terminal_channel_open);
        assert!(terminal_closed_health.transport_round_trip_ms.is_some());
        assert!(terminal_closed_health.channel_round_trip_ms.is_some());
        assert!(terminal_closed_health
            .terminal_error
            .as_deref()
            .is_some_and(|error| error.contains("交互输入不可用")));
        terminal_channel_open.store(true, Ordering::SeqCst);

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
        let source = root.join("libssh-sftp-source.bin");
        let downloaded = root.join("libssh-sftp-downloaded.bin");
        let payload = b"PortMate libssh native SFTP payload";
        fs::write(&source, payload).unwrap();
        let remote_root = format!("/tmp/portmate-libssh-sftp-{}", Uuid::new_v4());
        let uploaded = remote_join_path(&remote_root, "uploaded.bin");
        let copied = remote_join_path(&remote_root, "copied.bin");
        let renamed = remote_join_path(&remote_root, "renamed.bin");
        let exclusive = remote_join_path(&remote_root, "exclusive.bin");

        let auxiliary = ssh_auxiliary_lease(&state, &profile.id).unwrap();
        let sftp = auxiliary.sftp().await.unwrap();
        sftp_create_dir_all(&sftp, &remote_root).await.unwrap();
        let mut exclusive_file = sftp
            .open_with_flags(
                exclusive.clone(),
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .unwrap();
        exclusive_file.shutdown().await.unwrap();
        assert!(sftp
            .open_with_flags(
                exclusive.clone(),
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .is_err());
        sftp.remove_file(exclusive).await.unwrap();
        assert_eq!(
            sftp_upload(&sftp, source.to_str().unwrap(), &uploaded, &progress,)
                .await
                .unwrap(),
            payload.len() as u64
        );
        assert_eq!(
            sftp_download(&sftp, &uploaded, downloaded.to_str().unwrap(), &progress,)
                .await
                .unwrap(),
            payload.len() as u64
        );
        assert_eq!(fs::read(&downloaded).unwrap(), payload);
        assert_eq!(
            sftp_remote_copy(&sftp, &uploaded, &copied, &progress)
                .await
                .unwrap(),
            payload.len() as u64
        );

        let entries = list_remote_files(&sftp, &remote_root).await.unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from(["uploaded.bin", "copied.bin"])
        );
        let properties = remote_file_properties(&sftp, &copied).await.unwrap();
        assert!(properties.is_file);
        assert_eq!(properties.size, payload.len() as u64);

        let mut metadata = sftp.symlink_metadata(copied.clone()).await.unwrap();
        let file_type_bits = metadata.permissions.unwrap_or(0) & 0o170000;
        metadata.permissions = Some(file_type_bits | 0o600);
        sftp.set_metadata(copied.clone(), metadata).await.unwrap();
        assert_eq!(
            sftp.symlink_metadata(copied.clone())
                .await
                .unwrap()
                .permissions
                .unwrap_or(0)
                & 0o777,
            0o600
        );
        sftp.rename(copied.clone(), renamed.clone()).await.unwrap();
        assert!(!sftp.try_exists(copied).await.unwrap());
        assert!(sftp.try_exists(renamed).await.unwrap());
        sftp_remove_recursive(&sftp, &remote_root).await.unwrap();
        assert!(!sftp.try_exists(remote_root).await.unwrap());
        drop(sftp);
        drop(auxiliary);

        assert_libssh_local_and_dynamic_tunnels(&state, &profile.id).await;

        let stored = state
            .store
            .lock()
            .unwrap()
            .profiles
            .iter()
            .find(|candidate| candidate.id == profile.id)
            .cloned()
            .unwrap();
        let ConnectionConfig::Ssh(stored_ssh) = stored.connection else {
            panic!("stored profile changed transport");
        };
        assert_eq!(
            stored_ssh.identity_policy.last_successful,
            Some(AuthMethod::PublicKey)
        );
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
    });

    sshd.stop();
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn libssh_profile_vault_key_loader_uses_secret_ref_and_passphrase() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh vault key test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let key_path = root.path().join("vault_ed25519");
    let passphrase = "PortMate vault identity passphrase";
    let keygen = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", passphrase, "-f"])
        .arg(&key_path)
        .status()
        .unwrap();
    assert!(keygen.success(), "ssh-keygen failed for vault identity");
    let key_material = fs::read_to_string(&key_path).unwrap();
    let identity = IdentityRef {
        id: "vault-key".to_string(),
        label: "Vault key".to_string(),
        source: IdentitySource::ProfileVault,
        fingerprint_sha256: None,
        path: None,
        secret_ref: Some("stronghold:vault-key".to_string()),
    };

    let key = load_libssh_private_key_with(&identity, Some(passphrase), |secret_ref| {
        assert_eq!(secret_ref, "stronghold:vault-key");
        Ok(key_material.clone())
    })
    .unwrap()
    .unwrap();
    assert_eq!(key.key_type_name().unwrap(), "ssh-ed25519");

    let error = load_libssh_private_key_with(&identity, Some("wrong passphrase"), |_| {
        Ok(key_material.clone())
    })
    .err()
    .expect("wrong vault passphrase unexpectedly parsed");
    assert!(error.contains("private key 解析失败"), "{error}");

    let error = load_libssh_private_key_with(&identity, Some(passphrase), |secret_ref| {
        Err(format!("missing test secret: {secret_ref}"))
    })
    .err()
    .expect("missing vault secret unexpectedly loaded");
    assert!(
        error.contains("profile-vault stronghold:vault-key"),
        "{error}"
    );
    assert!(!error.contains(&key_material));

    let mut missing_ref = identity;
    missing_ref.secret_ref = None;
    let error = load_libssh_private_key_with(&missing_ref, Some(passphrase), |_| {
        panic!("missing secretRef must not invoke the provider")
    })
    .err()
    .expect("missing vault secretRef unexpectedly loaded");
    assert_eq!(error, "profile-vault identity 缺少 secretRef");
}

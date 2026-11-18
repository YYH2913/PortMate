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
fn libssh_direct_tcpip_worker_releases_the_session_after_its_outer_deadline() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh deadline worker test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-worker-deadline-user";
        let secret = "PortMate libssh worker deadline secret";
        let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
            &host_key,
            username,
            secret,
            None,
            None,
            Some(Duration::from_secs(4)),
        )
        .await;
        let target = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let target_port = target.local_addr().unwrap().port();
        let session = connect_libssh_password_test_session(port, username, secret);
        session
            .set_option(libssh_rs::SshOption::Timeout(Duration::from_secs(4)))
            .unwrap();
        let session_probe = session.clone();
        let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::<
            AcceptAnyTestSshClient,
        >::from_libssh(session)));

        let started = Instant::now();
        let error = open_tunnel_direct_tcpip_with_timeout(
            &handle,
            "127.0.0.1".to_string(),
            target_port,
            "127.0.0.1:40000".parse().unwrap(),
            "bounded libssh direct-tcpip",
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert!(error.contains("direct-tcpip"), "unexpected error: {error}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "libssh direct-tcpip wrapper exceeded its cleanup budget"
        );
        tokio::time::timeout(Duration::from_secs(1), async {
            while counters.direct_tcpip_attempts.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("delayed direct-tcpip request never reached the server");

        tokio::time::timeout(
            Duration::from_millis(700),
            tokio::task::spawn_blocking(move || {
                session_probe.set_option(libssh_rs::SshOption::Timeout(Duration::from_secs(4)))
            }),
        )
        .await
        .expect("timed-out libssh worker kept the session mutex")
        .expect("libssh session probe worker failed")
        .expect("libssh session probe failed");

        drop(target);
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn libssh_terminal_typing_and_backspace_echo_latency() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh terminal echo test: ssh-keygen is not installed");
        return;
    }
    let root = canonical_test_tempdir();
    let host_key = root.path().join("host_key");
    generate_ed25519_test_key(&host_key);
    tauri::async_runtime::block_on(async {
        let user = "echo-fixture";
        let secret = "local fixture only";
        let (port, counters, server) = spawn_mixed_auth_test_server(&host_key, user, secret).await;
        counters.echo_terminal_edits.store(true, Ordering::Relaxed);
        let session = connect_libssh_password_test_session(port, user, secret);
        let channel = session.new_channel().unwrap();
        channel.open_session().unwrap();
        channel.request_shell().unwrap();
        let (mut reader, writer) = SshBackendChannel::from_libssh(channel).split_terminal();
        let writer = Arc::new(tokio::sync::Mutex::new(writer));
        let closed = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&closed);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let reader = tokio::spawn(async move {
            while let Some(message) = reader.wait_until_closed(&stop).await {
                if let SshBackendMessage::Data(bytes) = message {
                    for byte in bytes {
                        tx.send(byte).await.unwrap();
                    }
                }
            }
        });
        let mut samples = [Vec::new(), Vec::new()];
        let extended = std::env::var("PORTMATE_EXTENDED_INPUT_BENCH").as_deref() == Ok("1");
        let count = if extended { 1024 } else { 128 };
        for i in 0..count {
            // Alternate bursts and vary cadence relative to the reader's clock.
            tokio::time::sleep(Duration::from_millis(3 + (i % 7) * 2)).await;
            let deleting = i % 16 >= 8;
            let key = if deleting { 0x7f } else { b'a' + (i % 8) as u8 };
            let expected = if deleting {
                &b"\x08 \x08"[..]
            } else {
                std::slice::from_ref(&key)
            };
            let at = Instant::now();
            write_ssh_channel_bytes_with_cancellation(
                &writer,
                &[key],
                Duration::from_secs(2),
                "echo test",
                None,
            )
            .await
            .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                for &byte in expected {
                    assert_eq!(rx.recv().await, Some(byte));
                }
            })
            .await
            .expect("terminal echo stalled");
            samples[usize::from(deleting)].push(at.elapsed());
        }
        closed.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(2), reader)
            .await
            .unwrap()
            .unwrap();
        drop(writer);
        session.disconnect();
        server.abort();
        let _ = server.await;
        for (name, values) in ["type", "backspace"].into_iter().zip(&mut samples) {
            values.sort_unstable();
            let percentile = |p: usize| values[(values.len() * p).div_ceil(100) - 1];
            eprintln!(
                "libssh loopback {name}: n={} p50={:?} p95={:?} p99={:?} max={:?}",
                values.len(),
                percentile(50),
                percentile(95),
                percentile(99),
                values.last().unwrap()
            );
            // Leave scheduling headroom on shared CI hosts. Gate P99 only
            // in the extended run, which takes 512 samples per kind of edit.
            assert!(
                percentile(95) < Duration::from_millis(40),
                "{name} waited for an idle poll"
            );
            if extended {
                assert!(
                    percentile(99) < Duration::from_millis(40),
                    "{name} echo P99 stalled"
                );
            }
        }
    });
}

#[cfg(unix)]
#[test]
fn libssh_terminal_writer_lock_deadline_recovers_without_an_orphaned_waiter() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh writer deadline test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-writer-deadline-user";
        let secret = "PortMate libssh writer deadline secret";
        let (port, _, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;
        let session = connect_libssh_password_test_session(port, username, secret);
        session
            .set_option(libssh_rs::SshOption::Timeout(Duration::from_secs(2)))
            .unwrap();
        let channel = session.new_channel().unwrap();
        channel.open_session().unwrap();
        channel
            .request_exec("__PORTMATE_TEST_TERMINAL_INPUT__")
            .unwrap();
        let (reader, writer) = SshBackendChannel::from_libssh(channel).split_terminal();
        let writer = Arc::new(tokio::sync::Mutex::new(writer));
        let backend = SshBackendSession::<AcceptAnyTestSshClient>::from_libssh(session.clone());
        let cleanup_warning = request_backend_disconnect_with_timeout(
            &backend,
            "PortMate libssh live-channel cleanup test",
        )
        .await
        .expect("libssh forced disconnect skip was not reported");
        assert!(
            cleanup_warning.contains("active channels"),
            "unexpected cleanup warning: {cleanup_warning}"
        );
        drop(backend);
        let guard = writer.lock().await;

        let error = write_ssh_channel_bytes_with_timeout(
            &writer,
            b"blocked",
            Duration::from_millis(30),
            "SSH test write",
        )
        .await
        .unwrap_err();
        assert_eq!(error, "SSH test write writer lock 超时（30 ms）");
        drop(guard);

        write_ssh_channel_bytes_with_timeout(
            &writer,
            b"released",
            Duration::from_secs(1),
            "SSH test write",
        )
        .await
        .expect("SSH writer did not recover after its timed-out waiter");

        drop(writer);
        drop(reader);
        session.disconnect();
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn libssh_uninstalled_runtime_cleanup_drops_channel_before_session_disconnect() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh cleanup order test: ssh-keygen is not installed");
        return;
    }

    let root = canonical_test_tempdir();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-cleanup-order-user";
        let secret = "PortMate libssh cleanup order secret";
        let (port, _, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;
        let session = connect_libssh_password_test_session(port, username, secret);
        let channel = session.new_channel().unwrap();
        channel.open_session().unwrap();
        channel
            .request_exec("__PORTMATE_TEST_TERMINAL_INPUT__")
            .unwrap();
        let (read_half, writer) = SshBackendChannel::from_libssh(channel).split_terminal();
        let (tap, _) = broadcast::channel(4);
        let (reader_finished, reader_stopped) = tokio::sync::oneshot::channel();
        let closed = Arc::new(AtomicBool::new(false));
        let terminal_channel_open = Arc::new(AtomicBool::new(true));
        let runtime = SshRuntime {
            runtime_id: "libssh-uninstalled-cleanup-test".to_string(),
            profile_snapshot: "{}".to_string(),
            backend: SshBackendKind::Libssh,
            auth_method: AuthMethod::Password,
            handle: Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_libssh(
                session,
            ))),
            sftp: Arc::new(tokio::sync::Mutex::new(None)),
            jump_handles: Vec::new(),
            writer: Arc::new(tokio::sync::Mutex::new(writer)),
            tap,
            remote_forwards: Arc::new(Mutex::new(HashMap::new())),
            remote_forward_acceptor_started: Arc::new(AtomicBool::new(false)),
            agent_forwarder_finished: None,
            transport_bridge_finished: None,
            closed,
            terminal_channel_open,
            reader_finished: reader_stopped,
        };

        disconnect_ssh_runtime(
            runtime,
            read_half,
            reader_finished,
            "PortMate libssh cleanup order test",
        )
        .await;

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

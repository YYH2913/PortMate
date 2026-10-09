use super::*;

#[test]
fn failed_sftp_transfer_discards_closed_cache_and_retries_on_the_same_ssh_runtime() {
    let _guard = shared_runtime_test_guard();
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping SFTP retry regression: sshd is not installed");
        return;
    };
    let root = canonical_test_tempdir();
    let key = root.path().join("client-key");
    let host_key = root.path().join("host-key");
    generate_ed25519_test_key(&key);
    generate_ed25519_test_key(&host_key);
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = root.path().join("sshd_config");
    write_openssh_test_config(
        &config,
        &host_key,
        &root.path().join("sshd.pid"),
        &key.with_extension("pub"),
        port,
    );
    let mut sshd = spawn_openssh_test_server(sshd_path, &config);
    let profile = super::sftp_component_benchmarks::profile_for_backend(port, &key, "russh");
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    let source = root.path().join("remote-source.bin");
    let target = root.path().join("download.bin");
    let payload = vec![0x5a; 300 * 1024];
    fs::write(&source, &payload).unwrap();
    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(&mut sshd, port, "SFTP retry regression").await;
        open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap();
        let (runtime_id, cache) = {
            let ssh = state.ssh.lock().unwrap();
            let runtime = &ssh[&profile.id];
            (runtime.runtime_id.clone(), Arc::clone(&runtime.sftp))
        };
        {
            let auxiliary = ssh_auxiliary_lease(&state, &profile.id).unwrap();
            let sftp = auxiliary.sftp().await.unwrap();
            let SftpBackendSession::Russh(session) = &*sftp else {
                unreachable!()
            };
            session.close().await.unwrap();
        }
        state
            .store
            .lock()
            .unwrap()
            .record_transfer(test_transfer_task(&profile.id, TransferStatus::Running));
        let progress = test_transfer_progress_context(
            &state,
            "transfer-commit-test",
            Arc::new(AtomicBool::new(false)),
        );
        let request = StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: format!("remote:{}", source.display()),
            destination: target.display().to_string(),
        };
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            transfer_file_via_sftp(&state, &request, &progress, Some(&runtime_id)),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(!error.is_empty());
        assert!(cache.lock().await.is_none());
        assert_eq!(
            transfer_file_via_sftp(&state, &request, &progress, Some(&runtime_id))
                .await
                .unwrap(),
            payload.len() as u64
        );
        assert_eq!(fs::read(&target).unwrap(), payload);
        assert_eq!(
            state.ssh.lock().unwrap()[&profile.id].runtime_id,
            runtime_id
        );
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
    });
    sshd.stop();
}

#[test]
fn libssh_idle_terminal_does_not_serialize_sftp_writes_on_poll_timeouts() {
    let _guard = shared_runtime_test_guard();
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping libssh upload regression: sshd is not installed");
        return;
    };
    let root = canonical_test_tempdir();
    let key = root.path().join("client-key");
    let host_key = root.path().join("host-key");
    generate_ed25519_test_key(&key);
    generate_ed25519_test_key(&host_key);
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = root.path().join("sshd_config");
    write_openssh_test_config(
        &config,
        &host_key,
        &root.path().join("sshd.pid"),
        &key.with_extension("pub"),
        port,
    );
    let mut sshd = spawn_openssh_test_server(sshd_path, &config);
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
    ssh.identity_refs = vec![IdentityRef {
        id: "upload-key".to_string(),
        label: "upload key".to_string(),
        source: IdentitySource::SystemFile,
        fingerprint_sha256: None,
        path: Some(key.display().to_string()),
        secret_ref: None,
    }];
    ssh.agent_policy.enabled = false;
    ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));

    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(&mut sshd, port, "libssh upload regression").await;
        let EstablishedSshRuntime {
            runtime,
            mut read_half,
            closed,
            reader_finished,
            ..
        } = establish_ssh_runtime(&state, &profile, None, None)
            .await
            .unwrap();
        let reader_closed = Arc::clone(&closed);
        let reader = tokio::spawn(async move {
            while read_half.wait_until_closed(&reader_closed).await.is_some() {}
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let sftp = open_sftp_session(Arc::clone(&runtime.handle))
            .await
            .unwrap();
        assert!(matches!(sftp, SftpBackendSession::Libssh(_)));
        let target = root.path().join("uploaded.bin");
        let payload: Vec<u8> = (0..4 * 1024 * 1024).map(|index| index as u8).collect();
        // Sixty-four writes would incur over three seconds of idle waits if
        // the terminal reader held the shared session mutex for each 50 ms poll.
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let mut file = sftp
                .open_with_flags(
                    target.display().to_string(),
                    OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                )
                .await
                .unwrap();
            for chunk in payload.chunks(64 * 1024) {
                file.write_all(chunk).await.unwrap();
            }
            file.flush().await.unwrap();
            file.shutdown().await.unwrap();
        })
        .await;
        closed.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(3), reader)
            .await
            .unwrap()
            .unwrap();
        drop(reader_finished);
        result.expect("idle terminal polling blocked the SFTP upload");
        assert_eq!(fs::read(&target).unwrap(), payload);
    });
    sshd.stop();
}

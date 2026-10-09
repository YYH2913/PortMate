use super::*;

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

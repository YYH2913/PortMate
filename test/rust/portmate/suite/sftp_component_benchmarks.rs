use super::*;

fn print_metric(label: String, bytes: usize, elapsed: Duration, extra: serde_json::Value) {
    println!(
        "SFTP_COMPONENT {}",
        serde_json::json!({
            "label": label, "bytes": bytes, "seconds": elapsed.as_secs_f64(),
            "mib_per_second": bytes as f64 / 1048576.0 / elapsed.as_secs_f64(), "extra": extra,
        })
    );
}

pub(super) fn profile_for_backend(port: u16, key: &Path, backend: &str) -> SessionProfile {
    let mut profile = test_ssh_profile();
    let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
        unreachable!()
    };
    ssh.endpoint.host = "127.0.0.1".into();
    ssh.endpoint.port = port;
    ssh.username = openssh_test_username();
    ssh.reconnect = false;
    ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
    ssh.identity_policy.auth_order = if backend == "libssh" {
        vec![AuthMethod::GssapiWithMic, AuthMethod::PublicKey]
    } else {
        vec![AuthMethod::PublicKey]
    };
    ssh.identity_refs = vec![IdentityRef {
        id: "benchmark-key".into(),
        label: "benchmark key".into(),
        source: IdentitySource::SystemFile,
        fingerprint_sha256: None,
        path: Some(key.display().to_string()),
        secret_ref: None,
    }];
    ssh.agent_policy.enabled = false;
    ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
    profile
}

#[test]
#[ignore = "opt-in localhost SFTP component throughput benchmark"]
fn sftp_component_throughput_matrix() {
    let _guard = shared_runtime_test_guard();
    let root = canonical_test_tempdir();
    let sshd_path = openssh_test_server_path().expect("sshd required for throughput benchmark");
    let key = root.path().join("client-key");
    let host_key = root.path().join("host-key");
    generate_ed25519_test_key(&key);
    generate_ed25519_test_key(&host_key);
    let config = root.path().join("sshd_config");
    let port = std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    write_openssh_test_config_with_extra(
        &config,
        &host_key,
        &root.path().join("sshd.pid"),
        &key.with_extension("pub"),
        port,
        "RekeyLimit 1G 1h\n",
    );
    let mut sshd = spawn_openssh_test_server(sshd_path, &config);
    let size_mib = std::env::var("PORTMATE_PERF_SIZE_MIB")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(32)
        .clamp(1, 128);
    let mut rng = 0x91ab30d579e24618_u64;
    let payload: Vec<u8> = (0..size_mib * 1024 * 1024)
        .map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng as u8
        })
        .collect();
    let source = root.path().join("source.bin");
    fs::write(&source, &payload).unwrap();
    let listing_path = root.path().join("listing");
    fs::create_dir(&listing_path).unwrap();
    for index in 0..1_000 {
        fs::write(listing_path.join(format!("entry-{index:04}")), [0x5a]).unwrap();
    }

    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(&mut sshd, port, "SFTP benchmark").await;
        for backend in ["russh", "libssh"] {
            let profile = profile_for_backend(port, &key, backend);
            let mut state = test_app_state(
                profile.clone(),
                root.path().join(format!("{backend}.sqlite3")),
            );
            state.synchronous_inbound_logs = false;
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
            let sftp = open_sftp_session(Arc::clone(&runtime.handle))
                .await
                .unwrap();
            state
                .store
                .lock()
                .unwrap()
                .record_transfer(test_transfer_task(&profile.id, TransferStatus::Running));
            for iteration in 0..3 {
                let chunks = if iteration % 2 == 0 {
                    [32, 64, 128, 256]
                } else {
                    [256, 128, 64, 32]
                };
                for chunk_kib in chunks {
                    let path = root
                        .path()
                        .join(format!("{backend}-upload-{iteration}-{chunk_kib}.bin"));
                    let mut remote = sftp
                        .open_with_flags(
                            path.display().to_string(),
                            OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                        )
                        .await
                        .unwrap();
                    let started = Instant::now();
                    for chunk in payload.chunks(chunk_kib * 1024) {
                        remote.write_all(chunk).await.unwrap();
                    }
                    let data_seconds = started.elapsed().as_secs_f64();
                    remote.flush().await.unwrap();
                    remote.shutdown().await.unwrap();
                    print_metric(
                        format!("{backend}-upload-{chunk_kib}k-{iteration}"),
                        payload.len(),
                        started.elapsed(),
                        serde_json::json!({"chunk_kib":chunk_kib,"data_seconds":data_seconds,"terminal_reader":true}),
                    );
                    assert_eq!(fs::read(&path).unwrap(), payload);
                    fs::remove_file(&path).unwrap();
                }
                let aligned_path = root
                    .path()
                    .join(format!("aligned-{backend}-{iteration}.bin"));
                let mut remote = sftp
                    .open_with_flags(
                        aligned_path.display().to_string(),
                        OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                    )
                    .await
                    .unwrap();
                let chunk_size = remote.write_buffer_size(SFTP_TRANSFER_BUFFER_SIZE);
                let started = Instant::now();
                for chunk in payload.chunks(chunk_size) {
                    remote.write_all(chunk).await.unwrap();
                }
                let data_seconds = started.elapsed().as_secs_f64();
                remote.flush().await.unwrap();
                remote.shutdown().await.unwrap();
                print_metric(
                    format!("{backend}-upload-aligned-{iteration}"),
                    payload.len(),
                    started.elapsed(),
                    serde_json::json!({"chunk_bytes":chunk_size,"data_seconds":data_seconds,"terminal_reader":true}),
                );
                assert_eq!(fs::read(&aligned_path).unwrap(), payload);
                fs::remove_file(aligned_path).unwrap();
                for chunk_kib in chunks {
                    let mut remote = sftp.open(source.display().to_string()).await.unwrap();
                    let mut buffer = vec![0_u8; chunk_kib * 1024];
                    let mut downloaded = Vec::with_capacity(payload.len());
                    let started = Instant::now();
                    loop {
                        let read = match &mut remote {
                            SftpBackendFile::Russh(file) => {
                                AsyncReadExt::read(file, &mut buffer).await.unwrap()
                            }
                            _ => remote.read(&mut buffer).await.unwrap(),
                        };
                        if read == 0 {
                            break;
                        }
                        downloaded.extend_from_slice(&buffer[..read]);
                    }
                    remote.shutdown().await.unwrap();
                    print_metric(
                        format!("{backend}-download-serial-{chunk_kib}k-{iteration}"),
                        payload.len(),
                        started.elapsed(),
                        serde_json::json!({"chunk_kib":chunk_kib,"terminal_reader":true}),
                    );
                    assert_eq!(downloaded, payload);
                }
                if backend == "russh" {
                    let windows = if iteration % 2 == 0 {
                        [1, 4, 8, 16]
                    } else {
                        [16, 8, 4, 1]
                    };
                    for window in windows {
                        let mut remote = sftp.open(source.display().to_string()).await.unwrap();
                        let SftpBackendFile::Russh(file) = &mut remote else {
                            unreachable!()
                        };
                        let mut buffer = vec![0_u8; 32 * 1024 * window];
                        let mut downloaded = Vec::with_capacity(payload.len());
                        let started = Instant::now();
                        loop {
                            let read = file.read_pipelined(&mut buffer, window).await.unwrap();
                            if read == 0 {
                                break;
                            }
                            downloaded.extend_from_slice(&buffer[..read]);
                        }
                        remote.shutdown().await.unwrap();
                        print_metric(
                            format!("{backend}-download-pipeline-{window}-{iteration}"),
                            payload.len(),
                            started.elapsed(),
                            serde_json::json!({"request_max_kib":32,"window":window,"terminal_reader":true}),
                        );
                        assert_eq!(downloaded, payload);
                    }
                }
                let path = root
                    .path()
                    .join(format!("production-{backend}-{iteration}.bin"));
                let progress = test_transfer_progress_context(
                    &state,
                    "transfer-commit-test",
                    Arc::new(AtomicBool::new(false)),
                );
                let started = Instant::now();
                let bytes = sftp_download(
                    &sftp,
                    source.to_str().unwrap(),
                    path.to_str().unwrap(),
                    &progress,
                )
                .await
                .unwrap();
                print_metric(
                    format!("{backend}-production-download-{iteration}"),
                    bytes as usize,
                    started.elapsed(),
                    serde_json::json!({"terminal_reader":true}),
                );
                assert_eq!(fs::read(&path).unwrap(), payload);
                let path = root.path().join(format!("copy-{backend}-{iteration}.bin"));
                let started = Instant::now();
                let bytes = sftp_remote_copy(
                    &sftp,
                    source.to_str().unwrap(),
                    path.to_str().unwrap(),
                    &progress,
                )
                .await
                .unwrap();
                print_metric(
                    format!("{backend}-production-remote-copy-{iteration}"),
                    bytes as usize,
                    started.elapsed(),
                    serde_json::json!({"terminal_reader":true}),
                );
                assert_eq!(fs::read(&path).unwrap(), payload);
                let started = Instant::now();
                let entries: Vec<_> = sftp
                    .read_dir(listing_path.display().to_string())
                    .await
                    .unwrap()
                    .collect();
                let elapsed = started.elapsed();
                assert_eq!(entries.len(), 1_000);
                println!(
                    "SFTP_COMPONENT {}",
                    serde_json::json!({
                        "label":format!("{backend}-list-1000-{iteration}"), "entries":entries.len(),
                        "seconds":elapsed.as_secs_f64(), "terminal_reader":true,
                    })
                );
                let started = Instant::now();
                for index in 0..100 {
                    assert_eq!(
                        sftp.symlink_metadata(
                            listing_path
                                .join(format!("entry-{index:04}"))
                                .display()
                                .to_string()
                        )
                        .await
                        .unwrap()
                        .len(),
                        1
                    );
                }
                println!(
                    "SFTP_COMPONENT {}",
                    serde_json::json!({
                        "label":format!("{backend}-lstat-100-{iteration}"), "requests":100,
                        "seconds":started.elapsed().as_secs_f64(), "terminal_reader":true,
                    })
                );
                let path = root
                    .path()
                    .join(format!("production-upload-{backend}-{iteration}.bin"));
                let started = Instant::now();
                let bytes = sftp_upload(
                    &sftp,
                    source.to_str().unwrap(),
                    path.to_str().unwrap(),
                    &progress,
                )
                .await
                .unwrap();
                print_metric(
                    format!("{backend}-production-upload-{iteration}"),
                    bytes as usize,
                    started.elapsed(),
                    serde_json::json!({"terminal_reader":true}),
                );
                assert_eq!(fs::read(path).unwrap(), payload);
            }
            drop(sftp);
            closed.store(true, Ordering::SeqCst);
            if backend == "russh" {
                let handle = runtime.handle.lock().await;
                if let SshBackendSession::Russh(handle) = &*handle {
                    handle
                        .disconnect(Disconnect::ByApplication, "benchmark complete", "en")
                        .await
                        .unwrap();
                }
            }
            tokio::time::timeout(Duration::from_secs(3), reader)
                .await
                .unwrap()
                .unwrap();
            drop(reader_finished);
        }
    });
    sshd.stop();
}

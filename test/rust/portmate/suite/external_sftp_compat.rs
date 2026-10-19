use super::*;

#[cfg(unix)]
#[test]
fn external_sftp_server_compatibility() {
    let _runtime_guard = shared_runtime_test_guard();
    let Ok(label) = std::env::var("PORTMATE_COMPAT_SSH_LABEL") else {
        eprintln!("skipping external SFTP compatibility test: matrix environment is not set");
        return;
    };
    let host = std::env::var("PORTMATE_COMPAT_SSH_HOST").unwrap();
    let port = std::env::var("PORTMATE_COMPAT_SSH_PORT")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let username = std::env::var("PORTMATE_COMPAT_SSH_USERNAME").unwrap();
    let password = std::env::var("PORTMATE_COMPAT_SSH_PASSWORD").unwrap();
    let verify_extended_status_codes =
        std::env::var("PORTMATE_COMPAT_SFTP_EXTENDED_STATUS_CODES").as_deref() == Ok("1");
    let safe_commit = match std::env::var("PORTMATE_COMPAT_SFTP_NO_REPLACE").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => panic!("{label} matrix must pin the safe SFTP commit capability"),
    };
    let safe_commit_error =
        std::env::var("PORTMATE_COMPAT_SFTP_NO_REPLACE_ERROR").unwrap_or_default();
    assert_eq!(safe_commit, safe_commit_error.is_empty());
    let root = std::env::temp_dir().join(format!(
        "portmate-external-sftp-{}-{}",
        label,
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();

    tauri::async_runtime::block_on(async {
        let mut profile = test_ssh_profile();
        {
            let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
                panic!("expected SSH profile");
            };
            ssh.endpoint.host = host.clone();
            ssh.endpoint.port = port;
            ssh.username = username.clone();
            ssh.reconnect = false;
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.identity_policy.auth_order = vec![AuthMethod::Password];
            ssh.identity_refs.clear();
            ssh.agent_policy.enabled = false;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
        }
        let ConnectionConfig::Ssh(ssh) = &profile.connection else {
            panic!("expected SSH profile");
        };

        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let ssh_config = Arc::new(ssh_client_config(ssh));
        let observed_key = Arc::new(Mutex::new(None));
        let host_key_error = Arc::new(Mutex::new(None));
        let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
        let store = Arc::clone(&state.store);
        let store_path = state.store_path.clone();
        let host_keys = store.lock().unwrap().host_keys.clone();
        let connected_target = connect_ssh_target(
            SshConnectRequest {
                config: ssh_config,
                store,
                store_path,
                profile: &profile,
                ssh,
                host_keys,
                one_time_host_keys: Vec::new(),
                observed_key,
                host_key_error,
                remote_forwards,
                password: Some(&password),
                passphrase: None,
                enforce_profile_snapshot: false,
                host_key_verification: SshHostKeyVerification::Standard,
            },
            SSH_CONNECT_TIMEOUT,
            None,
            SshTargetTransportMode::Russh,
        )
        .await
        .unwrap_or_else(|error| panic!("{label} SFTP SSH connect failed: {error}"));
        let ConnectedSshTarget::Russh {
            mut session,
            jump_sessions,
        } = connected_target
        else {
            panic!("{label} SFTP SSH connect returned a Jump Host transport");
        };
        assert!(jump_sessions.is_empty());
        authenticate_ssh_with_timeout(
            &mut session,
            SshAuthenticationRequest {
                ssh: ssh.clone(),
                username: username.clone(),
                password: Some(password),
                passphrase: None,
                agent_socket_path: None,
                timeout: SSH_CONNECT_TIMEOUT,
                disconnect_description: "PortMate SFTP compatibility authentication timeout",
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{label} SFTP authentication failed: {error}"));

        let channel = session
            .channel_open_session()
            .await
            .unwrap_or_else(|error| panic!("{label} SFTP channel failed: {error}"));
        channel
            .request_subsystem(true, "sftp")
            .await
            .unwrap_or_else(|error| panic!("{label} SFTP subsystem failed: {error}"));
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .unwrap_or_else(|error| panic!("{label} SFTP init failed: {error}"));
        sftp.set_timeout(SFTP_REQUEST_TIMEOUT_SECONDS);
        let sftp = SftpBackendSession::from_russh(sftp);

        let remote_root = format!("/home/{username}/portmate-compat-{}", Uuid::new_v4());
        sftp_create_dir_all(&sftp, &remote_root)
            .await
            .unwrap_or_else(|error| panic!("{label} SFTP mkdir failed: {error}"));
        if verify_extended_status_codes {
            let started = Instant::now();
            let error = sftp
                .create_dir(remote_root.clone())
                .await
                .expect_err("Apache MINA duplicate SFTP mkdir unexpectedly succeeded");
            assert!(
                error.contains("File already exists"),
                "{label} returned the wrong duplicate mkdir status: {error}"
            );
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{label} duplicate mkdir status exceeded the compatibility deadline"
            );
        }
        let source = root.join("source.bin");
        let payload = format!("PortMate {label} SFTP compatibility\n").repeat(64);
        fs::write(&source, payload.as_bytes()).unwrap();
        let task_id = Uuid::new_v4().to_string();
        {
            let task = TransferTask {
                id: task_id.clone(),
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: source.display().to_string(),
                destination: format!("remote:{remote_root}/"),
                bytes_total: 0,
                bytes_done: 0,
                status: TransferStatus::Running,
                message: Some("running".to_string()),
                started_at: Some(Utc::now()),
                finished_at: None,
                average_bytes_per_second: None,
            };
            let mut store = state.store.lock().unwrap();
            store.record_transfer(task);
            save_store(&state.store_path, &store).unwrap();
        }
        let progress = TransferProgressContext {
            state: state.clone(),
            task_id,
            cancel: Arc::new(AtomicBool::new(false)),
            last_emit: Arc::new(Mutex::new(Instant::now())),
            started: Instant::now(),
            rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
            rate_limit_bytes_per_second: None,
        };
        let uploaded = sftp_upload(
            &sftp,
            &source.display().to_string(),
            &format!("{remote_root}/"),
            &progress,
        )
        .await;
        let remote_source = format!("{remote_root}/source.bin");
        let remote_source = if safe_commit {
            assert_eq!(
                uploaded.unwrap_or_else(|error| panic!("{label} SFTP upload failed: {error}")),
                payload.len() as u64
            );
            remote_source
        } else {
            let error =
                uploaded.expect_err("unsupported safe upload commit unexpectedly succeeded");
            assert!(error.contains(&safe_commit_error), "{error}");
            assert!(!sftp.try_exists(remote_source.clone()).await.unwrap());
            let partial = remote_resume_part_path(&remote_source);
            assert!(sftp.try_exists(partial.clone()).await.unwrap());
            partial
        };

        let downloaded = root.join("download.bin");
        let downloaded_bytes = sftp_download(
            &sftp,
            &remote_source,
            &downloaded.display().to_string(),
            &progress,
        )
        .await
        .unwrap_or_else(|error| panic!("{label} SFTP download failed: {error}"));
        assert_eq!(downloaded_bytes, payload.len() as u64);
        assert_eq!(fs::read(&downloaded).unwrap(), payload.as_bytes());

        let copied = format!("{remote_root}/copied.bin");
        let copied_bytes = sftp_remote_copy(&sftp, &remote_source, &copied, &progress).await;
        let copied = if safe_commit {
            assert_eq!(
                copied_bytes
                    .unwrap_or_else(|error| panic!("{label} SFTP remote copy failed: {error}")),
                payload.len() as u64
            );
            copied
        } else {
            let error =
                copied_bytes.expect_err("unsupported safe copy commit unexpectedly succeeded");
            assert!(error.contains(&safe_commit_error), "{error}");
            assert!(!sftp.try_exists(copied.clone()).await.unwrap());
            let partial = remote_resume_part_path(&copied);
            assert!(sftp.try_exists(partial.clone()).await.unwrap());
            partial
        };
        let copied_download = root.join("copied-download.bin");
        sftp_download(
            &sftp,
            &copied,
            &copied_download.display().to_string(),
            &progress,
        )
        .await
        .unwrap_or_else(|error| panic!("{label} preserved copy download failed: {error}"));
        assert_eq!(fs::read(copied_download).unwrap(), payload.as_bytes());
        if verify_extended_status_codes {
            let started = Instant::now();
            let error = sftp
                .remove_dir(remote_root.clone())
                .await
                .expect_err("Apache MINA non-empty SFTP rmdir unexpectedly succeeded");
            assert!(
                error.contains("Directory not empty"),
                "{label} returned the wrong non-empty rmdir status: {error}"
            );
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{label} non-empty rmdir status exceeded the compatibility deadline"
            );
        }
        sftp.remove_file(remote_source)
            .await
            .unwrap_or_else(|error| panic!("{label} SFTP source cleanup failed: {error}"));
        sftp.remove_file(copied)
            .await
            .unwrap_or_else(|error| panic!("{label} SFTP copy cleanup failed: {error}"));
        sftp.remove_dir(remote_root.clone())
            .await
            .unwrap_or_else(|error| panic!("{label} SFTP directory cleanup failed: {error}"));
        let _ = session
            .disconnect(
                Disconnect::ByApplication,
                "PortMate SFTP compatibility complete",
                "en",
            )
            .await;
    });

    let _ = fs::remove_dir_all(root);
}

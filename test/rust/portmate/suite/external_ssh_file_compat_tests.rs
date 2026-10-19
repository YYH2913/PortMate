#[cfg(unix)]
#[test]
fn external_ssh_server_sftp_scp_compatibility() {
    let _runtime_guard = shared_runtime_test_guard();
    let Ok(label) = std::env::var("PORTMATE_COMPAT_SSH_LABEL") else {
        eprintln!("skipping external SSH compatibility test: matrix environment is not set");
        return;
    };
    let host = std::env::var("PORTMATE_COMPAT_SSH_HOST").unwrap();
    let port = std::env::var("PORTMATE_COMPAT_SSH_PORT")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let username = std::env::var("PORTMATE_COMPAT_SSH_USERNAME").unwrap();
    let password = std::env::var("PORTMATE_COMPAT_SSH_PASSWORD").unwrap();
    let safe_commit = match std::env::var("PORTMATE_COMPAT_SFTP_NO_REPLACE").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => panic!("{label} matrix must pin the safe SFTP commit capability"),
    };
    let safe_commit_error =
        std::env::var("PORTMATE_COMPAT_SFTP_NO_REPLACE_ERROR").unwrap_or_default();
    assert_eq!(safe_commit, safe_commit_error.is_empty());
    let local_root = std::env::temp_dir().join(format!(
        "portmate-external-ssh-compat-{}-{}",
        label,
        Uuid::new_v4()
    ));
    fs::create_dir_all(&local_root).unwrap();

    tauri::async_runtime::block_on(async {
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.endpoint.host = host;
        ssh.endpoint.port = port;
        ssh.username = username.clone();
        ssh.reconnect = false;
        ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
        ssh.identity_policy.auth_order = vec![AuthMethod::Password];
        ssh.identity_refs.clear();
        ssh.agent_policy.enabled = false;
        ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;

        let state = test_app_state(profile.clone(), local_root.join("portmate-store.sqlite3"));
        let connected = open_ssh_session(&state, profile.clone(), Some(password.clone()), None)
            .await
            .unwrap_or_else(|error| panic!("{label} SSH open failed: {error}"));
        assert_eq!(connected.runtime.status, SessionStatus::Connected);
        assert_eq!(
            state.ssh.lock().unwrap().get(&profile.id).unwrap().backend,
            SshBackendKind::Russh
        );
        let mut health_attempts = Vec::new();
        for attempt in 0..3 {
            let health = ssh_health::check_ssh_health_inner(&state, &profile.id, true)
                .await
                .unwrap_or_else(|error| panic!("{label} SSH health check failed: {error}"));
            let healthy = health.status == ssh_health::SshHealthStatus::Healthy;
            health_attempts.push(health);
            if healthy {
                break;
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        let health = health_attempts.last().unwrap();
        assert_eq!(
            health.status,
            ssh_health::SshHealthStatus::Healthy,
            "{label} SSH health did not stabilize: {health_attempts:?}"
        );
        assert_eq!(health.backend, SshBackendKind::Russh);
        assert_eq!(health.authentication_method, AuthMethod::Password);
        assert!(health.terminal_channel_open);
        assert!(health.terminal_error.is_none());
        assert!(health.transport_round_trip_ms.is_some());
        assert!(health.channel_round_trip_ms.is_some());
        assert!(health.sftp_round_trip_ms.is_some());

        send_text_inner(
            state.session_io(),
            profile.id.clone(),
            "printf '__PORTMATE_COMPAT_PTY__\\n'\n".to_string(),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if state
                    .store
                    .lock()
                    .unwrap()
                    .screen(&profile.id)
                    .is_some_and(|screen| screen.contains("__PORTMATE_COMPAT_PTY__"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{label} PTY output timed out"));

        let remote_root = format!("/home/{username}/compat/portmate-{}", Uuid::new_v4());
        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                expected_connected_since: None,
                path: remote_root.clone(),
                remote: true,
            },
            FileOperation::CreateDirectory,
        )
        .await
        .unwrap_or_else(|error| panic!("{label} SFTP mkdir failed: {error}"));

        let entries = list_files_inner(
            &state,
            ListFilesRequest {
                session_id: Some(profile.id.clone()),
                path: format!("/home/{username}/compat"),
                remote: true,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{label} SFTP list failed: {error}"));
        assert!(entries.iter().any(|entry| entry.path == remote_root));

        for (name, protocol) in [
            ("sftp", TransferProtocol::Sftp),
            ("scp", TransferProtocol::Scp),
        ] {
            let source = local_root.join(format!("{name}-source.bin"));
            let download = local_root.join(format!("{name}-download.bin"));
            let payload = format!("PortMate {label} {name} compatibility\n").repeat(32);
            fs::write(&source, payload.as_bytes()).unwrap();
            let mut remote_file = format!("{remote_root}/{name}-source.bin");

            let upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: protocol.clone(),
                    source: source.display().to_string(),
                    destination: format!("remote:{remote_root}/"),
                },
            )
            .await
            .unwrap();
            let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
            if name == "sftp" && !safe_commit {
                assert_eq!(upload.status, TransferStatus::Failed);
                assert!(
                    upload
                        .message
                        .as_deref()
                        .is_some_and(|message| message.contains(&safe_commit_error)),
                    "{label} failed for an unexpected reason: {:?}",
                    upload.message
                );
                let partial = remote_resume_part_path(&remote_file);
                assert_compat_remote_paths(&state, &profile, &remote_root, &partial, &remote_file)
                    .await;
                remote_file = partial;
            } else {
                assert_eq!(
                    upload.status,
                    TransferStatus::Completed,
                    "{label} {name} upload failed: {:?}",
                    upload.message
                );
            }

            let downloaded = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol,
                    source: format!("remote:{remote_file}"),
                    destination: download.display().to_string(),
                },
            )
            .await
            .unwrap();
            let downloaded = wait_for_transfer_terminal_state(&state, &downloaded.id).await;
            assert_eq!(
                downloaded.status,
                TransferStatus::Completed,
                "{label} {name} download failed: {:?}",
                downloaded.message
            );
            assert_eq!(fs::read(&download).unwrap(), payload.as_bytes());
        }

        let sftp_source = format!("{remote_root}/sftp-source.bin");
        let sftp_source = if safe_commit {
            sftp_source
        } else {
            remote_resume_part_path(&sftp_source)
        };
        let mut copied = format!("{remote_root}/sftp-copied.bin");
        let copy = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: format!("remote:{sftp_source}"),
                destination: format!("remote:{copied}"),
            },
        )
        .await
        .unwrap();
        let copy = wait_for_transfer_terminal_state(&state, &copy.id).await;
        if safe_commit {
            assert_eq!(
                copy.status,
                TransferStatus::Completed,
                "{label} SFTP remote copy failed: {:?}",
                copy.message
            );
        } else {
            assert_eq!(copy.status, TransferStatus::Failed);
            assert!(
                copy.message
                    .as_deref()
                    .is_some_and(|message| message.contains(&safe_commit_error)),
                "{label} remote copy failed for an unexpected reason: {:?}",
                copy.message
            );
            let partial = remote_resume_part_path(&copied);
            assert_compat_remote_paths(&state, &profile, &remote_root, &partial, &copied).await;
            copied = partial;
        }

        let renamed = format!("{remote_root}/sftp-renamed.bin");
        let rename = rename_path_inner(
            &state,
            RenamePathRequest {
                session_id: Some(profile.id.clone()),
                expected_connected_since: None,
                old_path: copied.clone(),
                new_path: renamed.clone(),
                remote: true,
            },
        )
        .await;
        let checked_path = if safe_commit {
            rename.unwrap_or_else(|error| panic!("{label} SFTP rename failed: {error}"));
            renamed
        } else {
            let error = rename.expect_err("unsupported safe rename unexpectedly succeeded");
            assert!(error.contains(&safe_commit_error), "{error}");
            assert_compat_remote_paths(&state, &profile, &remote_root, &copied, &renamed).await;
            copied
        };
        let copy_download = local_root.join("copied-download.bin");
        let downloaded = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: format!("remote:{checked_path}"),
                destination: copy_download.display().to_string(),
            },
        )
        .await
        .unwrap();
        let downloaded = wait_for_transfer_terminal_state(&state, &downloaded.id).await;
        assert_eq!(downloaded.status, TransferStatus::Completed);
        assert_eq!(
            fs::read(&copy_download).unwrap(),
            format!("PortMate {label} sftp compatibility\n")
                .repeat(32)
                .as_bytes()
        );
        chmod_path_inner(
            &state,
            ChmodPathRequest {
                session_id: Some(profile.id.clone()),
                expected_connected_since: None,
                path: checked_path.clone(),
                mode: 0o640,
                remote: true,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{label} SFTP chmod failed: {error}"));
        let properties = file_properties_inner(
            &state,
            FilePropertiesRequest {
                session_id: Some(profile.id.clone()),
                path: checked_path,
                remote: true,
            },
        )
        .await
        .unwrap();
        assert!(properties.is_file);
        assert_eq!(properties.permissions.unwrap() & 0o777, 0o640);

        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                expected_connected_since: None,
                path: remote_root,
                remote: true,
            },
            FileOperation::Delete,
        )
        .await
        .unwrap_or_else(|error| panic!("{label} SFTP cleanup failed: {error}"));
        let closed = close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
    });

    let _ = fs::remove_dir_all(local_root);
}

#[cfg(unix)]
async fn assert_compat_remote_paths(
    state: &AppState,
    profile: &SessionProfile,
    directory: &str,
    present: &str,
    absent: &str,
) {
    let entries = list_files_inner(
        state,
        ListFilesRequest {
            session_id: Some(profile.id.clone()),
            path: directory.to_string(),
            remote: true,
        },
    )
    .await
    .unwrap();
    assert!(
        entries.iter().any(|entry| entry.path == present),
        "missing preserved {present}"
    );
    assert!(
        !entries.iter().any(|entry| entry.path == absent),
        "unexpected committed {absent}"
    );
}

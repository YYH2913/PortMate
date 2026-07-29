use super::*;

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
            let remote_file = format!("{remote_root}/{name}-source.bin");

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
            assert_eq!(
                upload.status,
                TransferStatus::Completed,
                "{label} {name} upload failed: {:?}",
                upload.message
            );

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

        let copied = format!("{remote_root}/sftp-copied.bin");
        let copy = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: format!("remote:{remote_root}/sftp-source.bin"),
                destination: format!("remote:{copied}"),
            },
        )
        .await
        .unwrap();
        let copy = wait_for_transfer_terminal_state(&state, &copy.id).await;
        assert_eq!(
            copy.status,
            TransferStatus::Completed,
            "{label} SFTP remote copy failed: {:?}",
            copy.message
        );

        let renamed = format!("{remote_root}/sftp-renamed.bin");
        rename_path_inner(
            &state,
            RenamePathRequest {
                session_id: Some(profile.id.clone()),
                old_path: copied,
                new_path: renamed.clone(),
                remote: true,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{label} SFTP rename failed: {error}"));
        chmod_path_inner(
            &state,
            ChmodPathRequest {
                session_id: Some(profile.id.clone()),
                path: renamed.clone(),
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
                path: renamed,
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
#[test]
fn external_ssh_server_active_transfer_disconnect() {
    let _runtime_guard = shared_runtime_test_guard();
    let Ok(label) = std::env::var("PORTMATE_COMPAT_SSH_LABEL") else {
        eprintln!("skipping external SSH disconnect test: matrix environment is not set");
        return;
    };
    let host = std::env::var("PORTMATE_COMPAT_SSH_HOST").unwrap();
    let port = std::env::var("PORTMATE_COMPAT_SSH_PORT")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let username = std::env::var("PORTMATE_COMPAT_SSH_USERNAME").unwrap();
    let password = std::env::var("PORTMATE_COMPAT_SSH_PASSWORD").unwrap();
    let container = std::env::var("PORTMATE_COMPAT_SSH_CONTAINER").unwrap();
    let protocol_name = std::env::var("PORTMATE_COMPAT_SSH_DISCONNECT_PROTOCOL").unwrap();
    let protocol = match protocol_name.as_str() {
        "sftp" => TransferProtocol::Sftp,
        "scp" => TransferProtocol::Scp,
        value => panic!("unsupported disconnect protocol: {value}"),
    };
    let modem_protocol_name =
        std::env::var("PORTMATE_COMPAT_SSH_MODEM_DISCONNECT_PROTOCOL").unwrap();
    let modem_protocol = match modem_protocol_name.as_str() {
        "xmodem" => TransferProtocol::Xmodem,
        "ymodem" => TransferProtocol::Ymodem,
        "zmodem" => TransferProtocol::Zmodem,
        value => panic!("unsupported modem disconnect protocol: {value}"),
    };
    let local_root = std::env::temp_dir().join(format!(
        "portmate-external-ssh-disconnect-{}-{}",
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
        profile.transfer.rate_limit_bytes_per_second = Some(64 * 1024);

        let state = test_app_state(profile.clone(), local_root.join("portmate-store.sqlite3"));
        let connected = open_ssh_session(&state, profile.clone(), Some(password.clone()), None)
            .await
            .unwrap_or_else(|error| panic!("{label} SSH disconnect setup failed: {error}"));
        assert_eq!(connected.runtime.status, SessionStatus::Connected);

        let modem_store_path = local_root.join("portmate-modem-store.sqlite3");
        let modem_state = test_app_state(profile.clone(), modem_store_path);
        let modem_connected =
            open_ssh_session(&modem_state, profile.clone(), Some(password.clone()), None)
                .await
                .unwrap_or_else(|error| {
                    panic!("{label} modem SSH disconnect setup failed: {error}")
                });
        assert_eq!(modem_connected.runtime.status, SessionStatus::Connected);

        let remote_directory = format!("portmate-disconnect-{}", Uuid::new_v4());
        let remote_root = format!("/home/{username}/compat/{remote_directory}");
        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: remote_root.clone(),
                remote: true,
            },
            FileOperation::CreateDirectory,
        )
        .await
        .unwrap_or_else(|error| panic!("{label} disconnect mkdir failed: {error}"));

        let payload = (0..2 * 1024 * 1024)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let source = local_root.join("active-transfer-source.bin");
        fs::write(&source, &payload).unwrap();
        let remote_file = format!("{remote_root}/{protocol_name}-disconnect.bin");
        let remote_part = remote_resume_part_path(&remote_file);
        let transfer = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: protocol.clone(),
                source: source.display().to_string(),
                destination: format!("remote:{remote_file}"),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{label} {protocol_name} disconnect start failed: {error}"));
        let modem_payload = (0..2 * 1024 * 1024)
            .map(|index| ((index * 17) % 251) as u8)
            .collect::<Vec<_>>();
        let modem_source = local_root.join("active-modem-source.bin");
        fs::write(&modem_source, &modem_payload).unwrap();
        let modem_file = format!("{remote_root}/{modem_protocol_name}-disconnect.bin");
        let modem_transfer = start_transfer_inner(
            &modem_state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: modem_protocol.clone(),
                source: modem_source.display().to_string(),
                destination: format!("remote:{modem_file}"),
            },
        )
        .await
        .unwrap_or_else(|error| {
            panic!("{label} {modem_protocol_name} disconnect start failed: {error}")
        });
        let transfer_progress_label = format!("{label} limited {protocol_name} upload");
        let modem_progress_label = format!("{label} limited {modem_protocol_name} upload");
        let (_, _) = tokio::join!(
            wait_for_transfer_progress(&state, &transfer.id, &transfer_progress_label,),
            wait_for_transfer_progress(&modem_state, &modem_transfer.id, &modem_progress_label,),
        );

        let killed = Command::new("docker")
            .args(["kill", "--signal", "KILL", &container])
            .status()
            .unwrap();
        assert!(killed.success(), "failed to kill {container}");

        let (interrupted, modem_interrupted) = tokio::join!(
            wait_for_transfer_terminal_state(&state, &transfer.id),
            wait_for_transfer_terminal_state(&modem_state, &modem_transfer.id),
        );
        assert_eq!(
            interrupted.status,
            TransferStatus::Failed,
            "{label} {protocol:?} server loss was not reported: {:?}",
            interrupted.message
        );
        assert!(
            interrupted.bytes_done > 0 && interrupted.bytes_done < payload.len() as u64,
            "{label} {protocol:?} interruption reported invalid progress: {interrupted:?}"
        );
        assert!(
            interrupted
                .message
                .as_deref()
                .is_some_and(|message| message.contains(protocol_name.to_uppercase().as_str())),
            "{label} {protocol:?} interruption lacked protocol context: {interrupted:?}"
        );
        assert!(
            !state
                .transfer_cancellations
                .lock()
                .unwrap()
                .contains_key(&transfer.id),
            "{label} {protocol:?} interruption retained its cancellation handle"
        );
        assert_eq!(
            modem_interrupted.status,
            TransferStatus::Failed,
            "{label} {modem_protocol:?} server loss was not reported: {:?}",
            modem_interrupted.message
        );
        assert!(
            modem_interrupted.bytes_done > 0
                && modem_interrupted.bytes_done < modem_payload.len() as u64,
            "{label} {modem_protocol:?} interruption reported invalid progress: {modem_interrupted:?}"
        );
        assert!(
            modem_interrupted
                .message
                .as_deref()
                .is_some_and(|message| message.to_ascii_lowercase().contains("modem")),
            "{label} {modem_protocol:?} interruption lacked protocol context: {modem_interrupted:?}"
        );
        assert!(
            !modem_state
                .transfer_cancellations
                .lock()
                .unwrap()
                .contains_key(&modem_transfer.id),
            "{label} {modem_protocol:?} interruption retained its cancellation handle"
        );
        let closed = close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
        let modem_closed = close_session_inner(&modem_state, profile.id.clone())
            .await
            .unwrap();
        assert_eq!(modem_closed.runtime.status, SessionStatus::Disconnected);

        let copied_remote = local_root.join("remote-after-kill");
        fs::create_dir_all(&copied_remote).unwrap();
        let remote_copy_source = format!("{container}:{remote_root}/.");
        let copied = Command::new("docker")
            .args([
                "cp",
                remote_copy_source.as_str(),
                copied_remote.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert!(
            copied.success(),
            "failed to copy interrupted transfer evidence from {container}"
        );
        let host_final = copied_remote.join(Path::new(&remote_file).file_name().unwrap());
        let host_part = copied_remote.join(Path::new(&remote_part).file_name().unwrap());
        assert!(
            !host_final.exists(),
            "{label} {protocol:?} committed a final file after server loss"
        );
        let part_metadata = fs::metadata(&host_part).unwrap_or_else(|error| {
            panic!(
                "{label} {protocol:?} partial file was not recoverable at {}: {error}",
                host_part.display()
            )
        });
        assert!(part_metadata.is_file());
        assert!(
            part_metadata.len() > 0 && part_metadata.len() < payload.len() as u64,
            "{label} {protocol:?} partial file had invalid size: {}",
            part_metadata.len()
        );
        let modem_part = remote_resume_part_path(&modem_file);
        let host_modem_final = copied_remote.join(Path::new(&modem_file).file_name().unwrap());
        assert!(
            !host_modem_final.exists(),
            "{label} {modem_protocol:?} committed a final file after server loss"
        );
        let host_modem_part = copied_remote.join(Path::new(&modem_part).file_name().unwrap());
        if host_modem_part.exists() {
            let metadata = fs::metadata(&host_modem_part).unwrap();
            assert!(
                metadata.len() < modem_payload.len() as u64,
                "{label} {modem_protocol:?} partial file had invalid size: {}",
                metadata.len()
            );
        }
    });

    let _ = fs::remove_dir_all(local_root);
}

#[cfg(unix)]
#[test]
fn external_ssh_health_fault_matrix_case() {
    let _runtime_guard = shared_runtime_test_guard();
    let Ok(fault) = std::env::var("PORTMATE_COMPAT_SSH_HEALTH_FAULT") else {
        eprintln!("skipping external SSH health fault test: matrix environment is not set");
        return;
    };
    let host = std::env::var("PORTMATE_COMPAT_SSH_HOST").unwrap();
    let port = std::env::var("PORTMATE_COMPAT_SSH_PORT")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let username = std::env::var("PORTMATE_COMPAT_SSH_USERNAME").unwrap();
    let password = std::env::var("PORTMATE_COMPAT_SSH_PASSWORD").unwrap();
    let probe_sftp = std::env::var("PORTMATE_COMPAT_SSH_PROBE_SFTP")
        .unwrap()
        .parse::<bool>()
        .unwrap();
    let expected_status = std::env::var("PORTMATE_COMPAT_SSH_EXPECTED_STATUS").unwrap();
    let expected_error_field = std::env::var("PORTMATE_COMPAT_SSH_EXPECTED_ERROR_FIELD").unwrap();
    let expected_error_contains =
        std::env::var("PORTMATE_COMPAT_SSH_EXPECTED_ERROR_CONTAINS").unwrap();
    let expect_sftp_recovery = std::env::var("PORTMATE_COMPAT_SSH_EXPECT_SFTP_RECOVERY")
        .unwrap_or_else(|_| "false".to_string())
        .parse::<bool>()
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "portmate-external-ssh-health-fault-{}-{}",
        fault,
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();

    tauri::async_runtime::block_on(async {
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.endpoint.host = host;
        ssh.endpoint.port = port;
        ssh.username = username;
        ssh.reconnect = fault == "transport-closed";
        if ssh.reconnect {
            ssh.reconnect_delay_ms = 60_000;
        }
        ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
        ssh.identity_policy.auth_order = vec![AuthMethod::Password];
        ssh.identity_refs.clear();
        ssh.agent_policy.enabled = false;
        ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;

        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let connected = open_ssh_session(&state, profile.clone(), Some(password), None)
            .await
            .unwrap_or_else(|error| panic!("{fault} SSH open failed: {error}"));
        assert_eq!(connected.runtime.status, SessionStatus::Connected);

        if fault == "runtime-replaced" {
            let mut check = Box::pin(ssh_health::check_ssh_health_inner(
                &state,
                &profile.id,
                probe_sftp,
            ));
            tokio::select! {
                result = &mut check => panic!("runtime replacement health check completed before injection: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(200)) => {}
            }
            state
                .ssh
                .lock()
                .unwrap()
                .get_mut(&profile.id)
                .unwrap()
                .runtime_id = format!("replacement-{}", Uuid::new_v4());
            let error = check.await.unwrap_err();
            assert!(
                error.contains(&expected_error_contains),
                "{fault} returned an unexpected generation error: {error}"
            );
        } else {
            let paused_container = if fault == "ping-unresponsive" {
                let container = std::env::var("PORTMATE_COMPAT_SSH_CONTAINER").unwrap();
                let paused = Command::new("docker")
                    .args(["pause", &container])
                    .status()
                    .unwrap();
                assert!(paused.success(), "failed to pause {container}");
                Some(container)
            } else {
                None
            };
            if fault == "transport-closed" {
                let container = std::env::var("PORTMATE_COMPAT_SSH_CONTAINER").unwrap();
                let killed = Command::new("docker")
                    .args(["kill", "--signal", "KILL", &container])
                    .status()
                    .unwrap();
                assert!(killed.success(), "failed to kill {container}");
            }

            let result = tokio::time::timeout(
                Duration::from_secs(12),
                ssh_health::check_ssh_health_inner(&state, &profile.id, probe_sftp),
            )
            .await
            .unwrap_or_else(|_| panic!("{fault} SSH health check exceeded the test deadline"));
            if let Some(container) = paused_container {
                let unpaused = Command::new("docker")
                    .args(["unpause", &container])
                    .status()
                    .unwrap();
                assert!(unpaused.success(), "failed to unpause {container}");
            }
            let health =
                result.unwrap_or_else(|error| panic!("{fault} SSH health command failed: {error}"));
            let expected_health_status = match expected_status.as_str() {
                "degraded" => ssh_health::SshHealthStatus::Degraded,
                "unresponsive" => ssh_health::SshHealthStatus::Unresponsive,
                value => panic!("unsupported expected SSH health status: {value}"),
            };
            assert_eq!(health.status, expected_health_status, "{fault}: {health:?}");
            let field_error = match expected_error_field.as_str() {
                "transportError" => health.transport_error.as_deref(),
                "channelError" => health.channel_error.as_deref(),
                "sftpError" => health.sftp_error.as_deref(),
                value => panic!("unsupported SSH health error field: {value}"),
            }
            .unwrap_or_else(|| panic!("{fault} omitted {expected_error_field}: {health:?}"));
            if !expected_error_contains.is_empty() {
                assert!(
                    field_error.contains(&expected_error_contains),
                    "{fault} returned an unexpected {expected_error_field}: {field_error}"
                );
            }
            if expected_health_status != ssh_health::SshHealthStatus::Unresponsive {
                assert!(health.transport_round_trip_ms.is_some(), "{health:?}");
            }
            if expected_error_field == "sftpError" {
                assert!(health.channel_round_trip_ms.is_some(), "{health:?}");
                assert!(health.sftp_probed, "{health:?}");
            }
            if expect_sftp_recovery {
                let recovered = tokio::time::timeout(
                    Duration::from_secs(8),
                    ssh_health::check_ssh_health_inner(&state, &profile.id, true),
                )
                .await
                .unwrap_or_else(|_| panic!("{fault} SFTP recovery check timed out"))
                .unwrap_or_else(|error| panic!("{fault} SFTP recovery check failed: {error}"));
                assert_eq!(
                    recovered.status,
                    ssh_health::SshHealthStatus::Healthy,
                    "{fault} did not recover with a fresh SFTP channel: {recovered:?}"
                );
                assert!(recovered.transport_round_trip_ms.is_some(), "{recovered:?}");
                assert!(recovered.channel_round_trip_ms.is_some(), "{recovered:?}");
                assert!(recovered.sftp_round_trip_ms.is_some(), "{recovered:?}");
            }
        }

        let _ = close_session_inner(&state, profile.id.clone()).await;
    });

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn external_ssh_transfer_fault_matrix_case() {
    let _runtime_guard = shared_runtime_test_guard();
    let Ok(fault) = std::env::var("PORTMATE_COMPAT_SSH_TRANSFER_FAULT") else {
        eprintln!("skipping external SSH transfer fault test: matrix environment is not set");
        return;
    };
    let host = std::env::var("PORTMATE_COMPAT_SSH_HOST").unwrap();
    let port = std::env::var("PORTMATE_COMPAT_SSH_PORT")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let username = std::env::var("PORTMATE_COMPAT_SSH_USERNAME").unwrap();
    let password = std::env::var("PORTMATE_COMPAT_SSH_PASSWORD").unwrap();
    let protocol_name = std::env::var("PORTMATE_COMPAT_SSH_TRANSFER_PROTOCOL").unwrap();
    let expected_error = std::env::var("PORTMATE_COMPAT_SSH_TRANSFER_EXPECTED_ERROR").unwrap();
    let protocol = match protocol_name.as_str() {
        "sftp" => TransferProtocol::Sftp,
        "scp" => TransferProtocol::Scp,
        value => panic!("unsupported SSH transfer fault protocol: {value}"),
    };
    let root = std::env::temp_dir().join(format!(
        "portmate-external-ssh-transfer-fault-{}-{}",
        fault,
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();

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

        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let connected = open_ssh_session(&state, profile.clone(), Some(password), None)
            .await
            .unwrap_or_else(|error| panic!("{fault} SSH open failed: {error}"));
        assert_eq!(connected.runtime.status, SessionStatus::Connected);

        let source = root.join("fault-source.bin");
        fs::write(&source, b"PortMate transfer fault matrix\n".repeat(32)).unwrap();
        let (source_path, destination_path) = match fault.as_str() {
            "sftp-no-such-file" | "sftp-unknown-status" => (
                format!("remote:/home/{username}/portmate-missing-source.bin"),
                root.join("missing-source-download.bin")
                    .display()
                    .to_string(),
            ),
            "sftp-permission-denied" => (
                source.display().to_string(),
                format!("remote:/home/{username}/portmate-readonly/compat-fault.bin"),
            ),
            _ => (
                source.display().to_string(),
                format!("remote:/home/{username}/compat-fault.bin"),
            ),
        };
        let task = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol,
                source: source_path,
                destination: destination_path,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{fault} transfer queue rejected unexpectedly: {error}"));
        let wait_started = Instant::now();
        let task = wait_for_transfer_terminal_state(&state, &task.id).await;
        if matches!(
            fault.as_str(),
            "sftp-unknown-status" | "sftp-no-space" | "sftp-quota-exceeded"
        ) || fault.starts_with("sftp-status-")
            || matches!(
                fault.as_str(),
                "sftp-malformed-packet" | "sftp-wrong-request-id"
            )
        {
            assert!(
                wait_started.elapsed() < Duration::from_secs(5),
                "{fault} did not fail promptly: {task:?}"
            );
        }
        assert_eq!(
            task.status,
            TransferStatus::Failed,
            "{fault} transfer unexpectedly succeeded: {task:?}"
        );
        assert!(
            task.message
                .as_deref()
                .is_some_and(|message| message.contains(&expected_error)),
            "{fault} transfer error lacked {expected_error:?}: {task:?}"
        );

        let _ = close_session_inner(&state, profile.id.clone()).await;
    });

    let _ = fs::remove_dir_all(root);
}

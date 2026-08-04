use super::openssh_modem_integration::exercise_openssh_modem_transfers;
use super::openssh_sftp_integration::exercise_openssh_sftp_operations;
use super::openssh_tunnel_integration::{
    exercise_openssh_local_and_dynamic_tunnels, exercise_openssh_remote_tunnel,
    exercise_openssh_tunnel_reconnect,
};
use super::*;

#[cfg(unix)]
#[test]
fn openssh_sftp_scp_and_tunnels_end_to_end() {
    let _runtime_guard = shared_runtime_test_guard();

    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping OpenSSH integration test: sshd is not installed");
        return;
    };
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping OpenSSH integration test: ssh-keygen is not installed");
        return;
    }
    let modem_tools_available = ["rx", "sx", "rb", "sb", "rz", "sz"]
        .into_iter()
        .all(|command| Command::new(command).arg("--version").output().is_ok());

    let root = std::env::temp_dir().join(format!("portmate-sshd-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let host_key = root.join("ssh_host_ed25519_key");
    let replacement_host_key = root.join("ssh_host_ed25519_key_replacement");
    let client_key = root.join("id_ed25519");
    for key_path in [&host_key, &replacement_host_key, &client_key] {
        generate_ed25519_test_key(key_path);
    }
    let authorized_keys = root.join("authorized_keys");
    fs::copy(client_key.with_extension("pub"), &authorized_keys).unwrap();

    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let username = openssh_test_username();
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
        wait_for_openssh_test_server(&mut sshd, port, "sshd").await;

        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.endpoint.host = "127.0.0.1".to_string();
            ssh.endpoint.port = port;
            ssh.username = username.clone();
            ssh.reconnect = true;
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
            ssh.identity_refs = vec![IdentityRef {
                id: "integration-client-key".to_string(),
                label: "integration client key".to_string(),
                source: IdentitySource::SystemFile,
                fingerprint_sha256: None,
                path: Some(client_key.display().to_string()),
                secret_ref: None,
            }];
            ssh.agent_policy.enabled = false;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
        }
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let summary = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(summary.runtime.status, SessionStatus::Connected);
        assert_eq!(summary.profile.connection.kind(), SessionKind::Ssh);
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

        send_text_inner(
            state.session_io(),
            profile.id.clone(),
            "printf '__PORTMATE_SSH_OK__\\n'\n".to_string(),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if state
                    .store
                    .lock()
                    .unwrap()
                    .screen(&profile.id)
                    .is_some_and(|screen| screen.contains("__PORTMATE_SSH_OK__"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("SSH PTY command output was not recorded");

        exercise_openssh_sftp_operations(&state, &profile, &root).await;

        let upload_source = root.join("scp-upload-source.bin");
        let remote_file = root.join("scp-remote.bin");
        let download_target = root.join("scp-download-target.bin");
        let payload = b"PortMate OpenSSH SCP integration payload\n";
        fs::write(&upload_source, payload).unwrap();
        let remote_part = PathBuf::from(remote_resume_part_path(remote_file.to_str().unwrap()));
        fs::write(&remote_part, b"wrong-prefix").unwrap();
        let upload = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Scp,
                source: upload_source.display().to_string(),
                destination: format!("remote:{}", remote_file.display()),
            },
        )
        .await
        .unwrap();
        let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
        assert_eq!(
            upload.status,
            TransferStatus::Completed,
            "SCP upload failed: {:?}",
            upload.message
        );
        assert_eq!(upload.bytes_done, payload.len() as u64);
        assert_eq!(fs::read(&remote_file).unwrap(), payload);
        assert!(!remote_part.exists());

        let download_part = local_resume_part_path(&download_target);
        fs::write(&download_part, &payload[..15]).unwrap();
        let download = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Scp,
                source: format!("remote:{}", remote_file.display()),
                destination: download_target.display().to_string(),
            },
        )
        .await
        .unwrap();
        let download = wait_for_transfer_terminal_state(&state, &download.id).await;
        assert_eq!(
            download.status,
            TransferStatus::Completed,
            "SCP download failed: {:?}",
            download.message
        );
        assert_eq!(download.bytes_done, payload.len() as u64);
        assert_eq!(fs::read(&download_target).unwrap(), payload);
        assert!(!download_part.exists());

        let denied_target = format!("/proc/portmate-transfer-denied-{}.bin", Uuid::new_v4());
        for protocol in [TransferProtocol::Sftp, TransferProtocol::Scp] {
            let failed_upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: protocol.clone(),
                    source: upload_source.display().to_string(),
                    destination: format!("remote:{denied_target}"),
                },
            )
            .await
            .unwrap();
            let failed_upload = wait_for_transfer_terminal_state(&state, &failed_upload.id).await;
            assert_eq!(
                failed_upload.status,
                TransferStatus::Failed,
                "{protocol:?} server-side write failure was not reported: {:?}",
                failed_upload.message
            );
            let message = failed_upload.message.unwrap_or_default();
            assert!(
                message.contains("SFTP") || message.contains("SCP"),
                "{protocol:?} failure lacked protocol context: {message}"
            );
            assert!(
                !state
                    .transfer_cancellations
                    .lock()
                    .unwrap()
                    .contains_key(&failed_upload.id),
                "{protocol:?} failed transfer retained its cancellation handle"
            );
        }

        {
            let mut store = state.store.lock().unwrap();
            let mut limited = store.profile(&profile.id).unwrap();
            limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
            store.upsert_profile(limited);
        }
        let cancel_source = root.join("sftp-cancel-source.bin");
        let cancel_remote = root.join("sftp-cancel-remote.bin");
        let cancel_remote_part =
            PathBuf::from(remote_resume_part_path(cancel_remote.to_str().unwrap()));
        // Keep enough limited payload remaining that a heavily loaded parallel test
        // runner cannot finish the transfer before the cancellation poll is scheduled.
        let cancel_payload = (0..2 * 1024 * 1024)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&cancel_source, &cancel_payload).unwrap();
        let cancelled_upload = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: cancel_source.display().to_string(),
                destination: format!("remote:{}", cancel_remote.display()),
            },
        )
        .await
        .unwrap();
        wait_for_transfer_progress(&state, &cancelled_upload.id, "limited SFTP upload").await;
        let cancelling = cancel_transfer_inner(&state, &cancelled_upload.id).unwrap();
        assert_eq!(cancelling.status, TransferStatus::Cancelled);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !state
                    .transfer_cancellations
                    .lock()
                    .unwrap()
                    .contains_key(&cancelled_upload.id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cancelled SFTP worker did not stop");
        let cancelled = state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&cancelled_upload.id)
            .unwrap();
        assert_eq!(cancelled.status, TransferStatus::Cancelled);
        assert!(!cancel_remote.exists());
        let partial_size = fs::metadata(&cancel_remote_part).unwrap().len();
        assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

        {
            let mut store = state.store.lock().unwrap();
            let mut unlimited = store.profile(&profile.id).unwrap();
            unlimited.transfer.rate_limit_bytes_per_second = None;
            store.upsert_profile(unlimited);
        }
        let retried = retry_transfer_inner(&state, &cancelled_upload.id)
            .await
            .unwrap();
        let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
        assert_eq!(
            retried.status,
            TransferStatus::Completed,
            "SFTP retry failed: {:?}",
            retried.message
        );
        assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
        assert_eq!(fs::read(&cancel_remote).unwrap(), cancel_payload);
        assert!(!cancel_remote_part.exists());

        {
            let mut store = state.store.lock().unwrap();
            let mut limited = store.profile(&profile.id).unwrap();
            limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
            store.upsert_profile(limited);
        }
        let scp_cancel_source = root.join("scp-cancel-source.bin");
        let scp_cancel_remote = root.join("scp-cancel-remote.bin");
        let scp_cancel_remote_part =
            PathBuf::from(remote_resume_part_path(scp_cancel_remote.to_str().unwrap()));
        fs::write(&scp_cancel_source, &cancel_payload).unwrap();
        let cancelled_scp_upload = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Scp,
                source: scp_cancel_source.display().to_string(),
                destination: format!("remote:{}", scp_cancel_remote.display()),
            },
        )
        .await
        .unwrap();
        wait_for_transfer_progress(&state, &cancelled_scp_upload.id, "limited SCP upload").await;
        let cancelling = cancel_transfer_inner(&state, &cancelled_scp_upload.id).unwrap();
        assert_eq!(cancelling.status, TransferStatus::Cancelled);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !state
                    .transfer_cancellations
                    .lock()
                    .unwrap()
                    .contains_key(&cancelled_scp_upload.id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cancelled SCP worker did not stop");
        let cancelled = state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&cancelled_scp_upload.id)
            .unwrap();
        assert_eq!(cancelled.status, TransferStatus::Cancelled);
        assert!(!scp_cancel_remote.exists());
        let partial_size = fs::metadata(&scp_cancel_remote_part).unwrap().len();
        assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

        {
            let mut store = state.store.lock().unwrap();
            let mut unlimited = store.profile(&profile.id).unwrap();
            unlimited.transfer.rate_limit_bytes_per_second = None;
            store.upsert_profile(unlimited);
        }
        let retried = retry_transfer_inner(&state, &cancelled_scp_upload.id)
            .await
            .unwrap();
        let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
        assert_eq!(
            retried.status,
            TransferStatus::Completed,
            "SCP retry failed: {:?}",
            retried.message
        );
        assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
        assert_eq!(fs::read(&scp_cancel_remote).unwrap(), cancel_payload);
        assert!(!scp_cancel_remote_part.exists());

        for (label, protocol) in [
            ("sftp", TransferProtocol::Sftp),
            ("scp", TransferProtocol::Scp),
        ] {
            {
                let mut store = state.store.lock().unwrap();
                let mut limited = store.profile(&profile.id).unwrap();
                limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
                store.upsert_profile(limited);
            }
            let disconnect_remote = root.join(format!("{label}-disconnect-remote.bin"));
            let disconnect_remote_part =
                PathBuf::from(remote_resume_part_path(disconnect_remote.to_str().unwrap()));
            let interrupted_upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: protocol.clone(),
                    source: cancel_source.display().to_string(),
                    destination: format!("remote:{}", disconnect_remote.display()),
                },
            )
            .await
            .unwrap();
            wait_for_transfer_progress(
                &state,
                &interrupted_upload.id,
                &format!("limited {label} upload"),
            )
            .await;

            let disconnected = close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
            assert_eq!(disconnected.runtime.status, SessionStatus::Disconnected);
            let interrupted =
                wait_for_transfer_terminal_state(&state, &interrupted_upload.id).await;
            assert_eq!(
                interrupted.status,
                TransferStatus::Failed,
                "{protocol:?} SSH disconnect was not reported as a failure: {:?}",
                interrupted.message
            );
            assert!(
                !state
                    .transfer_cancellations
                    .lock()
                    .unwrap()
                    .contains_key(&interrupted.id),
                "{protocol:?} disconnected transfer retained its cancellation handle"
            );
            assert!(!disconnect_remote.exists());
            let partial_size = fs::metadata(&disconnect_remote_part).unwrap().len();
            assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

            let reopened = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap();
            assert_eq!(reopened.runtime.status, SessionStatus::Connected);
            {
                let mut store = state.store.lock().unwrap();
                let mut unlimited = store.profile(&profile.id).unwrap();
                unlimited.transfer.rate_limit_bytes_per_second = None;
                store.upsert_profile(unlimited);
            }
            let retried = retry_transfer_inner(&state, &interrupted_upload.id)
                .await
                .unwrap();
            let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
            assert_eq!(
                retried.status,
                TransferStatus::Completed,
                "{protocol:?} retry after reconnect failed: {:?}",
                retried.message
            );
            assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
            assert_eq!(fs::read(&disconnect_remote).unwrap(), cancel_payload);
            assert!(!disconnect_remote_part.exists());
        }

        exercise_openssh_modem_transfers(&state, &profile, &root, modem_tools_available).await;

        exercise_openssh_local_and_dynamic_tunnels(&state, &profile).await;

        exercise_openssh_remote_tunnel(&state, &profile).await;

        exercise_openssh_tunnel_reconnect(&state, &profile, port).await;

        let closed = close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
        tokio::time::sleep(Duration::from_millis(200)).await;

        sshd.stop();
        write_openssh_test_config(
            &config_path,
            &replacement_host_key,
            &root.join("sshd.pid"),
            &authorized_keys,
            port,
        );
        sshd = spawn_openssh_test_server(sshd_path, &config_path);
        wait_for_openssh_test_server(&mut sshd, port, "replacement sshd").await;

        let trusted_before = state.store.lock().unwrap().host_keys.keys.clone();
        let mismatch = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap_err();
        assert!(mismatch.contains("alias=bench-device"), "{mismatch}");
        assert!(mismatch.contains("observed="), "{mismatch}");
        assert!(mismatch.contains("expected=["), "{mismatch}");
        assert_eq!(state.store.lock().unwrap().host_keys.keys, trusted_before);

        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.host_key_policy.allow_rotation = true;
        }
        state.store.lock().unwrap().upsert_profile(profile.clone());
        let rotated = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(rotated.runtime.status, SessionStatus::Connected);
        let trusted_after_rotation = state.store.lock().unwrap().host_keys.keys.clone();
        assert_eq!(trusted_after_rotation.len(), 2);
        assert!(trusted_after_rotation
            .iter()
            .all(|key| key.alias == "bench-device" && key.port == port));
        assert_ne!(
            trusted_after_rotation[0].fingerprint_sha256,
            trusted_after_rotation[1].fingerprint_sha256
        );
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
    });

    sshd.stop();
    let _ = fs::remove_dir_all(root);
}

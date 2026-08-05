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

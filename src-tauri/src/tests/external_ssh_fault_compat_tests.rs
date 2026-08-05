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
                "sftp-malformed-packet"
                    | "sftp-malformed-status-payload"
                    | "sftp-oversized-packet"
                    | "sftp-truncated-packet"
                    | "sftp-wrong-request-id"
                    | "sftp-zero-length-packet"
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

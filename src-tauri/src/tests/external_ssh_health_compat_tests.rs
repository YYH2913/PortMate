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

        if fault == "terminal-channel-closed" {
            state
                .ssh
                .lock()
                .unwrap()
                .get(&profile.id)
                .unwrap()
                .terminal_channel_open
                .store(false, Ordering::SeqCst);
        }

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
            assert_eq!(health.backend, SshBackendKind::Russh);
            assert_eq!(health.authentication_method, AuthMethod::Password);
            let field_error = match expected_error_field.as_str() {
                "transportError" => health.transport_error.as_deref(),
                "terminalError" => health.terminal_error.as_deref(),
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

use super::*;

#[test]
fn ssh_exec_capture_buffer_accepts_exact_limit_and_rejects_whole_overflow_chunk() {
    let mut buffer = vec![1_u8, 2];
    append_bounded_ssh_exec_data(&mut buffer, &[3, 4], 4, "stdout").unwrap();
    assert_eq!(buffer, [1, 2, 3, 4]);

    let before_overflow = buffer.clone();
    let error = append_bounded_ssh_exec_data(&mut buffer, &[5], 4, "stdout").unwrap_err();
    assert!(error.contains("stdout"));
    assert!(error.contains("4"));
    assert_eq!(buffer, before_overflow);
}

#[test]
fn sysmon_history_query_limit_defaults_and_rejects_out_of_range_values() {
    assert_eq!(
        validate_sysmon_history_query_limit(None).unwrap(),
        DEFAULT_SYSMON_HISTORY_QUERY_LIMIT
    );
    assert_eq!(validate_sysmon_history_query_limit(Some(1)).unwrap(), 1);
    assert_eq!(
        validate_sysmon_history_query_limit(Some(MAX_SYSMON_HISTORY_QUERY_LIMIT)).unwrap(),
        MAX_SYSMON_HISTORY_QUERY_LIMIT
    );
    assert!(validate_sysmon_history_query_limit(Some(0)).is_err());
    assert!(validate_sysmon_history_query_limit(Some(MAX_SYSMON_HISTORY_QUERY_LIMIT + 1)).is_err());
}

#[test]
fn sysmon_refresh_saturation_rejects_before_collection_side_effects() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-sysmon-refresh-limit-{}", Uuid::new_v4()));
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
        let _permits = Arc::clone(&state.sysmon_slots)
            .try_acquire_many_owned(MAX_CONCURRENT_SYSMON_REFRESHES as u32)
            .unwrap();

        let error = refresh_sysmon_inner(&state, &profile.id).await.unwrap_err();

        assert!(error.contains("refresh limit"), "{error}");
        let store = state.store.lock().unwrap();
        assert!(store.sysmon.is_empty());
        assert!(store.events.is_empty());
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn ssh_auxiliary_capacity_rejects_before_lookup_and_recovers_permits() {
    let temp = tempfile::tempdir().unwrap();
    let state = test_app_state(test_ssh_profile(), temp.path().join("store.sqlite3"));
    let permits = Arc::clone(&state.ssh_auxiliary_slots)
        .try_acquire_many_owned(MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS as u32)
        .unwrap();

    let saturated = ssh_auxiliary_lease(&state, "missing-session")
        .err()
        .expect("saturated auxiliary capacity unexpectedly allowed a lease");
    assert_eq!(
        saturated,
        format!(
            "SSH auxiliary operation limit reached ({MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS})"
        )
    );
    assert_eq!(state.ssh_auxiliary_slots.available_permits(), 0);

    drop(permits);
    assert_eq!(
        state.ssh_auxiliary_slots.available_permits(),
        MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS
    );
    let missing_runtime = ssh_auxiliary_lease(&state, "missing-session")
        .err()
        .expect("missing SSH runtime unexpectedly produced a lease");
    assert_eq!(missing_runtime, "需要先连接 SSH/Tmux 会话才能执行远端操作");
    assert_eq!(
        state.ssh_auxiliary_slots.available_permits(),
        MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS
    );
}

#[test]
fn ssh_auxiliary_saturation_blocks_remote_entry_points_without_side_effects() {
    tauri::async_runtime::block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let profile = test_ssh_profile();
        let state = test_app_state(profile.clone(), temp.path().join("store.sqlite3"));
        let _permits = Arc::clone(&state.ssh_auxiliary_slots)
            .try_acquire_many_owned(MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS as u32)
            .unwrap();

        let file_error = list_files_inner(
            &state,
            ListFilesRequest {
                session_id: Some(profile.id.clone()),
                path: "/tmp".to_string(),
                remote: true,
            },
        )
        .await
        .unwrap_err();
        let tmux_list_error = list_tmux_state_inner(&state, &profile.id)
            .await
            .unwrap_err();
        let tmux_mutation_error = mutate_tmux_inner(
            &state,
            TmuxMutationRequest {
                session_id: profile.id.clone(),
                action: TmuxMutationAction::KillPane,
                target: "%1".to_string(),
                name: None,
                destination: None,
                layout: None,
                amount: None,
            },
        )
        .await
        .unwrap_err();
        let tmux_control_error = start_tmux_control_inner(&state, &profile.id, "lab")
            .await
            .unwrap_err();
        let sysmon_error = refresh_sysmon_inner(&state, &profile.id).await.unwrap_err();
        let tunnel_error = probe_remote_tunnel_health(
            &state,
            &TunnelRuntime {
                session_id: profile.id.clone(),
                ssh_runtime_id: "missing-runtime".to_string(),
                spec: TunnelSpec {
                    id: "saturated-health-check".to_string(),
                    label: "saturated health check".to_string(),
                    mode: TunnelMode::Remote,
                    bind_host: "127.0.0.1".to_string(),
                    bind_port: 10_022,
                    target_host: "127.0.0.1".to_string(),
                    target_port: 22,
                    enabled: true,
                },
                metrics: Arc::new(TunnelMetrics::default()),
                closed: Arc::new(AtomicBool::new(false)),
            },
        )
        .await
        .unwrap_err();

        for error in [
            file_error,
            tmux_list_error,
            tmux_mutation_error,
            tmux_control_error,
            sysmon_error,
            tunnel_error,
        ] {
            assert!(error.contains("auxiliary operation limit"), "{error}");
        }
        assert!(state.ssh.lock().unwrap().is_empty());
        assert!(state.tmux_controls.lock().unwrap().is_empty());
        assert_eq!(
            state.tmux_control_slots.available_permits(),
            MAX_ACTIVE_TMUX_CONTROLS
        );
        assert_eq!(
            state.sysmon_slots.available_permits(),
            MAX_CONCURRENT_SYSMON_REFRESHES
        );
        let store = state.store.lock().unwrap();
        assert!(store.sysmon.is_empty());
        assert!(store.events.is_empty());
    });
}

#[cfg(unix)]
#[test]
fn tmux_mutation_reuses_one_ssh_auxiliary_lease_for_state_refresh() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping tmux auxiliary lease test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let profile = test_ssh_profile();
        let mut state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        state.ssh_auxiliary_slots = Arc::new(tokio::sync::Semaphore::new(1));
        let username = "portmate-tmux-lease-user";
        let secret = "PortMate tmux lease secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;
        let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
        let handler = PortMateSshHandler {
            profile_id: profile.id.clone(),
            host: "127.0.0.1".to_string(),
            port,
            alias: None,
            policy: portmate_core::HostKeyPolicy {
                mode: HostKeyMode::TrustOnFirstUse,
                alias: None,
                trust_scope: HostKeyScope::Profile,
                allow_rotation: false,
                check_ip: false,
            },
            host_keys: state.store.lock().unwrap().host_keys.clone(),
            one_time_host_key_ids: Vec::new(),
            observed_key: Arc::new(Mutex::new(None)),
            host_key_error: Arc::new(Mutex::new(None)),
            remote_forwards: Arc::clone(&remote_forwards),
        };
        let mut handle = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            handler,
        )
        .await
        .unwrap();
        assert!(handle
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let terminal = SshBackendChannel::from_russh(handle.channel_open_session().await.unwrap());
        let (_reader, writer) = terminal.split();
        let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_russh(
            handle,
        )));
        let (tap, _) = broadcast::channel(1);
        let (_reader_finished_sender, reader_finished) = tokio::sync::oneshot::channel();
        state.ssh.lock().unwrap().insert(
            profile.id.clone(),
            SshRuntime {
                runtime_id: "tmux-lease-runtime".to_string(),
                handle: Arc::clone(&handle),
                sftp: Arc::new(tokio::sync::Mutex::new(None)),
                jump_handles: Vec::new(),
                writer: Arc::new(tokio::sync::Mutex::new(writer)),
                tap,
                remote_forwards,
                closed: Arc::new(AtomicBool::new(false)),
                reader_finished,
            },
        );

        let state_after_mutation = mutate_tmux_inner(
            &state,
            TmuxMutationRequest {
                session_id: profile.id.clone(),
                action: TmuxMutationAction::KillPane,
                target: "%1".to_string(),
                name: None,
                destination: None,
                layout: None,
                amount: None,
            },
        )
        .await
        .unwrap();

        assert!(state_after_mutation.sessions.is_empty());
        assert!(state_after_mutation.windows.is_empty());
        assert!(state_after_mutation.panes.is_empty());
        assert_eq!(state.ssh_auxiliary_slots.available_permits(), 1);
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 5);

        state.ssh.lock().unwrap().remove(&profile.id);
        let handle = handle.lock().await;
        let _ = handle.disconnect("PortMate tmux lease test complete").await;
        drop(handle);
        server_task.abort();
        let _ = server_task.await;
    });
}

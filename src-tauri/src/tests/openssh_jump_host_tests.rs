use super::*;

#[cfg(unix)]
#[test]
fn openssh_multi_hop_chain_and_key_mismatch_end_to_end() {
    let _runtime_guard = shared_runtime_test_guard();
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping OpenSSH Jump Host test: sshd is not installed");
        return;
    };
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping OpenSSH Jump Host test: ssh-keygen is not installed");
        return;
    }

    let root = std::env::temp_dir().join(format!("portmate-jump-sshd-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let jump_one_host_key = root.join("jump_one_host_ed25519_key");
    let jump_two_host_key = root.join("jump_two_host_ed25519_key");
    let replacement_jump_two_host_key = root.join("jump_two_host_ed25519_key_replacement");
    let target_host_key = root.join("target_host_ed25519_key");
    let jump_one_client_key = root.join("jump_one_id_ed25519");
    let jump_two_client_key = root.join("jump_two_id_ed25519");
    let target_client_key = root.join("target_id_ed25519");
    for key_path in [
        &jump_one_host_key,
        &jump_two_host_key,
        &replacement_jump_two_host_key,
        &target_host_key,
        &jump_one_client_key,
        &jump_two_client_key,
        &target_client_key,
    ] {
        generate_ed25519_test_key(key_path);
    }
    let jump_one_authorized_keys = root.join("jump_one_authorized_keys");
    let jump_two_authorized_keys = root.join("jump_two_authorized_keys");
    let target_authorized_keys = root.join("target_authorized_keys");
    fs::copy(
        jump_one_client_key.with_extension("pub"),
        &jump_one_authorized_keys,
    )
    .unwrap();
    fs::copy(
        jump_two_client_key.with_extension("pub"),
        &jump_two_authorized_keys,
    )
    .unwrap();
    fs::copy(
        target_client_key.with_extension("pub"),
        &target_authorized_keys,
    )
    .unwrap();

    let jump_one_reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let jump_two_reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let target_reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let jump_one_port = jump_one_reservation.local_addr().unwrap().port();
    let jump_two_port = jump_two_reservation.local_addr().unwrap().port();
    let target_port = target_reservation.local_addr().unwrap().port();
    drop(jump_one_reservation);
    drop(jump_two_reservation);
    drop(target_reservation);

    let jump_one_config = root.join("jump_one_sshd_config");
    let jump_two_config = root.join("jump_two_sshd_config");
    let target_config = root.join("target_sshd_config");
    write_openssh_test_config(
        &jump_one_config,
        &jump_one_host_key,
        &root.join("jump_one_sshd.pid"),
        &jump_one_authorized_keys,
        jump_one_port,
    );
    write_openssh_test_config(
        &jump_two_config,
        &jump_two_host_key,
        &root.join("jump_two_sshd.pid"),
        &jump_two_authorized_keys,
        jump_two_port,
    );
    write_openssh_test_config(
        &target_config,
        &target_host_key,
        &root.join("target_sshd.pid"),
        &target_authorized_keys,
        target_port,
    );
    let mut jump_one_sshd = spawn_openssh_test_server(sshd_path, &jump_one_config);
    let mut jump_two_sshd = spawn_openssh_test_server(sshd_path, &jump_two_config);
    let mut target_sshd = spawn_openssh_test_server(sshd_path, &target_config);

    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(&mut jump_one_sshd, jump_one_port, "jump one sshd").await;
        wait_for_openssh_test_server(&mut jump_two_sshd, jump_two_port, "jump two sshd").await;
        wait_for_openssh_test_server(&mut target_sshd, target_port, "target sshd").await;

        let username = openssh_test_username();
        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.endpoint.host = "127.0.0.1".to_string();
            ssh.endpoint.port = target_port;
            ssh.username = username.clone();
            ssh.reconnect = false;
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.host_key_policy.alias = Some("integration-target".to_string());
            ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
            ssh.identity_refs = vec![
                IdentityRef {
                    id: "target-client-key".to_string(),
                    label: "target client key".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(target_client_key.display().to_string()),
                    secret_ref: None,
                },
                IdentityRef {
                    id: "jump-one-client-key".to_string(),
                    label: "jump one client key".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(jump_one_client_key.display().to_string()),
                    secret_ref: None,
                },
                IdentityRef {
                    id: "jump-two-client-key".to_string(),
                    label: "jump two client key".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(jump_two_client_key.display().to_string()),
                    secret_ref: None,
                },
            ];
            ssh.agent_policy.enabled = false;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
            let mut jump_one_policy =
                portmate_core::HostKeyPolicy::profile_alias("integration-jump-1");
            jump_one_policy.mode = HostKeyMode::TrustOnFirstUse;
            let mut jump_two_policy =
                portmate_core::HostKeyPolicy::profile_alias("integration-jump-2");
            jump_two_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.jumps = vec![
                portmate_core::JumpHop {
                    host: "127.0.0.1".to_string(),
                    port: jump_one_port,
                    username: username.clone(),
                    password_secret_ref: None,
                    passphrase_secret_ref: None,
                    identity_ref: Some("jump-one-client-key".to_string()),
                    host_key_policy: Some(jump_one_policy),
                },
                portmate_core::JumpHop {
                    host: "127.0.0.1".to_string(),
                    port: jump_two_port,
                    username: username.clone(),
                    password_secret_ref: None,
                    passphrase_secret_ref: None,
                    identity_ref: Some("jump-two-client-key".to_string()),
                    host_key_policy: Some(jump_two_policy),
                },
            ];
        }

        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

        let (stalled_first_port, stalled_first) = spawn_stalled_ssh_endpoint().await;
        let mut timed_out_first = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut timed_out_first.connection {
            ssh.jumps[0].port = stalled_first_port;
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &timed_out_first,
            None,
            None,
            Duration::from_millis(200),
            None,
        )
        .await
        .err()
        .expect("stalled first Jump Host unexpectedly connected");
        stalled_first.abort();
        let _ = stalled_first.await;
        assert!(error.contains("Jump Host 第 1 跳连接超时"), "{error}");
        assert!(
            error.contains(&format!("127.0.0.1:{stalled_first_port}")),
            "{error}"
        );
        assert!(state.store.lock().unwrap().host_keys.keys.is_empty());

        let refused_first_port = {
            let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            reservation.local_addr().unwrap().port()
        };
        let mut refused_first = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut refused_first.connection {
            ssh.jumps[0].port = refused_first_port;
        }
        let error = open_ssh_session(&state, refused_first, None, None)
            .await
            .unwrap_err();
        assert!(error.contains("Jump Host 第 1 跳"), "{error}");
        assert!(
            error.contains(&format!("127.0.0.1:{refused_first_port}")),
            "{error}"
        );
        assert!(state.store.lock().unwrap().host_keys.keys.is_empty());

        let (stalled_second_port, stalled_second) = spawn_stalled_ssh_endpoint().await;
        let mut timed_out_second = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut timed_out_second.connection {
            ssh.jumps[1].port = stalled_second_port;
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &timed_out_second,
            None,
            None,
            Duration::from_millis(200),
            None,
        )
        .await
        .err()
        .expect("stalled second Jump Host unexpectedly connected");
        stalled_second.abort();
        let _ = stalled_second.await;
        assert!(error.contains("Jump Host 第 2 跳连接超时"), "{error}");
        assert!(
            error.contains(&format!("127.0.0.1:{stalled_second_port}")),
            "{error}"
        );
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

        let refused_second_port = {
            let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
            reservation.local_addr().unwrap().port()
        };
        let mut refused_second = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut refused_second.connection {
            ssh.jumps[1].port = refused_second_port;
        }
        let error = open_ssh_session(&state, refused_second, None, None)
            .await
            .unwrap_err();
        assert!(
            error.contains("Jump Host 第 2 跳打开 direct-tcpip"),
            "{error}"
        );
        assert!(
            error.contains(&format!("127.0.0.1:{refused_second_port}")),
            "{error}"
        );
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

        let mut rejected_second_identity = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut rejected_second_identity.connection {
            ssh.jumps[1].identity_ref = Some("target-client-key".to_string());
        }
        let error = open_ssh_session(&state, rejected_second_identity, None, None)
            .await
            .unwrap_err();
        assert!(error.contains("Jump Host 第 2 跳认证失败"), "{error}");
        assert!(
            error.contains(&format!("127.0.0.1:{jump_two_port}")),
            "{error}"
        );
        assert!(error.contains("target client key"), "{error}");
        assert!(error.contains("被服务器拒绝"), "{error}");
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

        let (stalled_target_port, stalled_target) = spawn_stalled_ssh_endpoint().await;
        let mut timed_out_target = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut timed_out_target.connection {
            ssh.endpoint.port = stalled_target_port;
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &timed_out_target,
            None,
            None,
            Duration::from_millis(200),
            None,
        )
        .await
        .err()
        .expect("stalled Jump Host target unexpectedly connected");
        stalled_target.abort();
        let _ = stalled_target.await;
        assert!(error.contains("SSH 经 Jump Host 连接超时"), "{error}");
        assert!(
            error.contains(&format!("127.0.0.1:{stalled_target_port}")),
            "{error}"
        );
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 2);

        let mut rejected_target_identities = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut rejected_target_identities.connection {
            ssh.identity_refs
                .retain(|identity| identity.id != "target-client-key");
        }
        let error = open_ssh_session(&state, rejected_target_identities, None, None)
            .await
            .unwrap_err();
        assert!(error.contains("SSH 目标认证失败"), "{error}");
        assert!(
            error.contains(&format!("127.0.0.1:{target_port}")),
            "{error}"
        );
        assert!(error.contains("jump one client key"), "{error}");
        assert!(error.contains("jump two client key"), "{error}");
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 2);

        let summary = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(summary.runtime.status, SessionStatus::Connected);
        assert_eq!(
            state
                .ssh
                .lock()
                .unwrap()
                .get(&profile.id)
                .unwrap()
                .jump_handles
                .len(),
            2
        );
        let trusted = state.store.lock().unwrap().host_keys.keys.clone();
        assert_eq!(trusted.len(), 3);
        assert!(trusted
            .iter()
            .any(|key| key.alias == "integration-jump-1" && key.port == jump_one_port));
        assert!(trusted
            .iter()
            .any(|key| key.alias == "integration-jump-2" && key.port == jump_two_port));
        assert!(trusted
            .iter()
            .any(|key| key.alias == "integration-target" && key.port == target_port));

        send_text_inner(
            state.session_io(),
            profile.id.clone(),
            "printf '__PORTMATE_JUMP_OK__\\n'\n".to_string(),
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
                    .is_some_and(|screen| screen.contains("__PORTMATE_JUMP_OK__"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("Jump Host PTY command output was not recorded");

        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();

        let mut libssh_jump_profile = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut libssh_jump_profile.connection {
            ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::PublicKey];
        }
        let jumped = establish_ssh_runtime_with_timeout(
            &state,
            &libssh_jump_profile,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            None,
        )
        .await
        .unwrap();
        assert_eq!(jumped.auth_method, AuthMethod::PublicKey);
        assert!(jumped.runtime.handle.lock().await.is_libssh());
        assert_eq!(jumped.runtime.jump_handles.len(), 2);
        let EstablishedSshRuntime {
            runtime,
            mut read_half,
            ..
        } = jumped;
        let SshRuntime {
            handle,
            jump_handles,
            writer,
            closed,
            transport_bridge_finished,
            ..
        } = runtime;
        writer
            .lock()
            .await
            .data(b"printf '__PORTMATE_%s__\\n' LIBSSH_JUMP_OK\r")
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            let mut output = Vec::new();
            loop {
                match read_half.wait().await {
                    Some(SshBackendMessage::Data(data)) => {
                        output.extend_from_slice(&data);
                        if output
                            .windows(b"__PORTMATE_LIBSSH_JUMP_OK__".len())
                            .any(|window| window == b"__PORTMATE_LIBSSH_JUMP_OK__")
                        {
                            break;
                        }
                    }
                    Some(_) => {}
                    None => panic!("libssh Jump Host terminal closed before marker"),
                }
            }
        })
        .await
        .expect("libssh Jump Host PTY command output was not recorded");
        closed.store(true, Ordering::SeqCst);
        drop(read_half);
        drop(writer);
        if let Some(finished) = transport_bridge_finished {
            tokio::time::timeout(Duration::from_secs(2), finished)
                .await
                .expect("libssh Jump Host bridge did not stop")
                .expect("libssh Jump Host bridge completion sender dropped");
        }
        handle
            .lock()
            .await
            .disconnect("PortMate libssh Jump Host test")
            .await
            .unwrap();
        drop(handle);
        for jump_handle in jump_handles {
            jump_handle
                .lock()
                .await
                .disconnect(
                    Disconnect::ByApplication,
                    "PortMate libssh Jump Host test",
                    "en",
                )
                .await
                .unwrap();
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        jump_two_sshd.stop();
        write_openssh_test_config(
            &jump_two_config,
            &replacement_jump_two_host_key,
            &root.join("jump_two_sshd.pid"),
            &jump_two_authorized_keys,
            jump_two_port,
        );
        jump_two_sshd = spawn_openssh_test_server(sshd_path, &jump_two_config);
        wait_for_openssh_test_server(
            &mut jump_two_sshd,
            jump_two_port,
            "replacement jump two sshd",
        )
        .await;

        let trusted_before = state.store.lock().unwrap().host_keys.keys.clone();
        let mismatch = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap_err();
        assert!(mismatch.contains("alias=integration-jump-2"), "{mismatch}");
        assert!(mismatch.contains("observed="), "{mismatch}");
        assert!(mismatch.contains("expected=["), "{mismatch}");
        let store = state.store.lock().unwrap();
        let trusted_after = &store.host_keys.keys;
        assert_eq!(trusted_after.len(), trusted_before.len());
        for before in &trusted_before {
            let after = trusted_after
                .iter()
                .find(|key| key.id == before.id)
                .expect("host key mismatch must not replace trusted keys");
            if before.alias == "integration-jump-1" {
                assert!(after.last_seen > before.last_seen);
                let mut expected = before.clone();
                expected.last_seen = after.last_seen;
                assert_eq!(after, &expected);
            } else {
                assert_eq!(after, before);
            }
        }
        let profile_keys = store
            .profiles
            .iter()
            .find(|stored| stored.id == profile.id)
            .and_then(|stored| match &stored.connection {
                ConnectionConfig::Ssh(ssh) => Some(&ssh.trusted_host_keys),
                _ => None,
            })
            .unwrap();
        assert_eq!(profile_keys, trusted_after);
    });

    jump_one_sshd.stop();
    jump_two_sshd.stop();
    target_sshd.stop();
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn jump_host_password_and_keyboard_interactive_mix_with_public_keys() {
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping mixed-auth Jump Host test: sshd is not installed");
        return;
    };
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping mixed-auth Jump Host test: ssh-keygen is not installed");
        return;
    }

    let root = std::env::temp_dir().join(format!("portmate-mixed-auth-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let password_jump_host_key = root.join("password_jump_host_ed25519_key");
    let public_key_jump_host_key = root.join("public_key_jump_host_ed25519_key");
    let target_host_key = root.join("target_host_ed25519_key");
    let public_key_jump_client_key = root.join("public_key_jump_id_ed25519");
    let target_client_key = root.join("target_id_ed25519");
    for key_path in [
        &password_jump_host_key,
        &public_key_jump_host_key,
        &target_host_key,
        &public_key_jump_client_key,
        &target_client_key,
    ] {
        generate_ed25519_test_key(key_path);
    }
    let public_key_jump_authorized_keys = root.join("public_key_jump_authorized_keys");
    let target_authorized_keys = root.join("target_authorized_keys");
    fs::copy(
        public_key_jump_client_key.with_extension("pub"),
        &public_key_jump_authorized_keys,
    )
    .unwrap();
    fs::copy(
        target_client_key.with_extension("pub"),
        &target_authorized_keys,
    )
    .unwrap();

    let public_key_jump_port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let target_port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let public_key_jump_config = root.join("public_key_jump_sshd_config");
    let target_config = root.join("target_sshd_config");
    write_openssh_test_config(
        &public_key_jump_config,
        &public_key_jump_host_key,
        &root.join("public_key_jump_sshd.pid"),
        &public_key_jump_authorized_keys,
        public_key_jump_port,
    );
    write_openssh_test_config(
        &target_config,
        &target_host_key,
        &root.join("target_sshd.pid"),
        &target_authorized_keys,
        target_port,
    );
    let mut public_key_jump_sshd = spawn_openssh_test_server(sshd_path, &public_key_jump_config);
    let mut target_sshd = spawn_openssh_test_server(sshd_path, &target_config);

    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(
            &mut public_key_jump_sshd,
            public_key_jump_port,
            "mixed-auth public-key jump sshd",
        )
        .await;
        wait_for_openssh_test_server(&mut target_sshd, target_port, "mixed-auth target sshd").await;
        let mixed_username = "portmate-mixed-user";
        let mixed_secret = "PortMate mixed auth secret";
        let (password_jump_port, counters, password_jump_task) =
            spawn_mixed_auth_test_server(&password_jump_host_key, mixed_username, mixed_secret)
                .await;
        let (proxy_port, proxy_connections, proxy_task) = spawn_test_http_connect_proxy(200).await;

        let openssh_username = openssh_test_username();
        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.endpoint.host = "127.0.0.1".to_string();
            ssh.endpoint.port = target_port;
            ssh.username = openssh_username.clone();
            ssh.reconnect = false;
            ssh.proxy = ProxyConfig {
                enabled: true,
                kind: ProxyKind::HttpConnect,
                host: "127.0.0.1".to_string(),
                port: proxy_port,
                ..ProxyConfig::default()
            };
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.host_key_policy.alias = Some("mixed-auth-target".to_string());
            ssh.identity_refs = vec![
                IdentityRef {
                    id: "mixed-target-key".to_string(),
                    label: "mixed target key".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(target_client_key.display().to_string()),
                    secret_ref: None,
                },
                IdentityRef {
                    id: "mixed-public-jump-key".to_string(),
                    label: "mixed public jump key".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(public_key_jump_client_key.display().to_string()),
                    secret_ref: None,
                },
            ];
            ssh.agent_policy.enabled = false;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
            let mut password_jump_policy =
                portmate_core::HostKeyPolicy::profile_alias("mixed-auth-password-jump");
            password_jump_policy.mode = HostKeyMode::TrustOnFirstUse;
            let mut public_key_jump_policy =
                portmate_core::HostKeyPolicy::profile_alias("mixed-auth-public-key-jump");
            public_key_jump_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.jumps = vec![
                portmate_core::JumpHop {
                    host: "127.0.0.1".to_string(),
                    port: password_jump_port,
                    username: mixed_username.to_string(),
                    password_secret_ref: None,
                    passphrase_secret_ref: None,
                    identity_ref: Some("no-profile-key-for-password-jump".to_string()),
                    host_key_policy: Some(password_jump_policy),
                },
                portmate_core::JumpHop {
                    host: "127.0.0.1".to_string(),
                    port: public_key_jump_port,
                    username: openssh_username,
                    password_secret_ref: None,
                    passphrase_secret_ref: None,
                    identity_ref: Some("mixed-public-jump-key".to_string()),
                    host_key_policy: Some(public_key_jump_policy),
                },
            ];
        }
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

        let scan = scan_ssh_host_key_inner(&state, profile.clone(), Some(mixed_secret), None)
            .await
            .unwrap();
        assert_eq!(scan.label.as_deref(), Some("目标 SSH"));
        counters.password_successes.store(0, Ordering::SeqCst);
        counters
            .keyboard_interactive_successes
            .store(0, Ordering::SeqCst);

        let mut password_profile = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut password_profile.connection {
            ssh.identity_policy.auth_order = vec![AuthMethod::Password, AuthMethod::PublicKey];
        }
        let error = open_ssh_session(
            &state,
            password_profile.clone(),
            Some("wrong mixed auth secret".to_string()),
            None,
        )
        .await
        .unwrap_err();
        assert!(error.contains("Jump Host 第 1 跳认证失败"), "{error}");
        assert!(error.contains("password"), "{error}");
        assert!(state.store.lock().unwrap().host_keys.keys.is_empty());

        let connected = open_ssh_session(
            &state,
            password_profile,
            Some(mixed_secret.to_string()),
            None,
        )
        .await
        .unwrap();
        assert_eq!(connected.runtime.status, SessionStatus::Connected);
        assert_eq!(counters.password_successes.load(Ordering::SeqCst), 1);
        assert_eq!(
            counters
                .keyboard_interactive_successes
                .load(Ordering::SeqCst),
            0
        );
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 3);
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();

        let mut keyboard_profile = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut keyboard_profile.connection {
            ssh.identity_policy.auth_order =
                vec![AuthMethod::KeyboardInteractive, AuthMethod::PublicKey];
        }
        let connected = open_ssh_session(
            &state,
            keyboard_profile,
            Some(mixed_secret.to_string()),
            None,
        )
        .await
        .unwrap();
        assert_eq!(connected.runtime.status, SessionStatus::Connected);
        assert_eq!(counters.password_successes.load(Ordering::SeqCst), 1);
        assert_eq!(
            counters
                .keyboard_interactive_successes
                .load(Ordering::SeqCst),
            1
        );
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();

        assert_eq!(proxy_connections.load(Ordering::SeqCst), 4);
        proxy_task.abort();
        let _ = proxy_task.await;
        password_jump_task.abort();
        let _ = password_jump_task.await;
    });

    public_key_jump_sshd.stop();
    target_sshd.stop();
    let _ = fs::remove_dir_all(root);
}

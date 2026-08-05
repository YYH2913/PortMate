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

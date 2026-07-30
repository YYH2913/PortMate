use super::*;

#[cfg(unix)]
#[test]
fn openssh_identity_order_respects_max_auth_tries() {
    let _runtime_guard = shared_runtime_test_guard();
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping OpenSSH identity-order test: sshd is not installed");
        return;
    };
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping OpenSSH identity-order test: ssh-keygen is not installed");
        return;
    }

    let root = std::env::temp_dir().join(format!("portmate-auth-order-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let host_key = root.join("ssh_host_ed25519_key");
    let accepted_key = root.join("accepted_ed25519_key");
    let rejected_key_one = root.join("rejected_one_ed25519_key");
    let rejected_key_two = root.join("rejected_two_ed25519_key");
    for key_path in [
        &host_key,
        &accepted_key,
        &rejected_key_one,
        &rejected_key_two,
    ] {
        generate_ed25519_test_key(key_path);
    }
    let authorized_keys = root.join("authorized_keys");
    fs::copy(accepted_key.with_extension("pub"), &authorized_keys).unwrap();

    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let config_path = root.join("sshd_config");
    write_openssh_test_config_with_extra(
        &config_path,
        &host_key,
        &root.join("sshd.pid"),
        &authorized_keys,
        port,
        "MaxAuthTries 2\n",
    );
    let mut sshd = spawn_openssh_test_server(sshd_path, &config_path);

    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(&mut sshd, port, "identity-order sshd").await;

        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.endpoint.host = "127.0.0.1".to_string();
            ssh.endpoint.port = port;
            ssh.username = openssh_test_username();
            ssh.reconnect = false;
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.host_key_policy.alias = Some("identity-order-target".to_string());
            ssh.identity_policy.identities_only = true;
            ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
            ssh.identity_refs = vec![
                IdentityRef {
                    id: "rejected-key-one".to_string(),
                    label: "rejected key one".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(rejected_key_one.display().to_string()),
                    secret_ref: None,
                },
                IdentityRef {
                    id: "rejected-key-two".to_string(),
                    label: "rejected key two".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(rejected_key_two.display().to_string()),
                    secret_ref: None,
                },
                IdentityRef {
                    id: "accepted-key".to_string(),
                    label: "accepted key".to_string(),
                    source: IdentitySource::SystemFile,
                    fingerprint_sha256: None,
                    path: Some(accepted_key.display().to_string()),
                    secret_ref: None,
                },
            ];
            ssh.agent_policy.enabled = false;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
        }
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

        let exhausted = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap_err();
        assert!(
            exhausted.contains("认证失败") || exhausted.contains("authentication"),
            "{exhausted}"
        );
        assert!(exhausted.contains("rejected key one"), "{exhausted}");
        assert!(exhausted.contains("rejected key two"), "{exhausted}");
        assert!(exhausted.contains("accepted key"), "{exhausted}");
        assert!(state.store.lock().unwrap().host_keys.keys.is_empty());

        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.identity_refs.rotate_right(1);
        }
        state.store.lock().unwrap().upsert_profile(profile.clone());
        let connected = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(connected.runtime.status, SessionStatus::Connected);
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
    });

    sshd.stop();
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn openssh_agent_policy_and_identity_filtering_end_to_end() {
    let _runtime_guard = shared_runtime_test_guard();
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping OpenSSH agent test: sshd is not installed");
        return;
    };
    let client_tools_available = Command::new("sh")
        .args([
            "-c",
            "command -v ssh-agent >/dev/null 2>&1 && command -v ssh-add >/dev/null 2>&1",
        ])
        .status()
        .is_ok_and(|status| status.success());
    if Command::new("ssh-keygen").arg("-V").output().is_err() || !client_tools_available {
        eprintln!("skipping OpenSSH agent test: OpenSSH client tools are not installed");
        return;
    }

    let root = std::env::temp_dir().join(format!("portmate-agent-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let host_key = root.join("ssh_host_ed25519_key");
    let accepted_key = root.join("accepted_agent_ed25519_key");
    let rejected_key = root.join("rejected_agent_ed25519_key");
    for key_path in [&host_key, &accepted_key, &rejected_key] {
        generate_ed25519_test_key(key_path);
    }
    let authorized_keys = root.join("authorized_keys");
    fs::copy(accepted_key.with_extension("pub"), &authorized_keys).unwrap();

    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let config_path = root.join("sshd_config");
    write_openssh_test_config(
        &config_path,
        &host_key,
        &root.join("sshd.pid"),
        &authorized_keys,
        port,
    );
    let mut sshd = spawn_openssh_test_server(sshd_path, &config_path);
    let agent_socket = root.join("agent.sock");
    let mut agent = spawn_openssh_test_agent(&agent_socket);

    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(&mut sshd, port, "agent-policy sshd").await;
        wait_for_openssh_test_agent(&mut agent, &agent_socket, "agent-policy ssh-agent").await;
        for key_path in [&rejected_key, &accepted_key] {
            let status = Command::new("ssh-add")
                .arg(key_path)
                .env("SSH_AUTH_SOCK", &agent_socket)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(
                status.success(),
                "ssh-add failed for {}",
                key_path.display()
            );
        }

        let identities = list_ssh_agent_identities_on_thread(Some(agent_socket.clone()))
            .await
            .unwrap();
        assert_eq!(identities.len(), 2);
        let accepted_public_key = fs::read_to_string(accepted_key.with_extension("pub"))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();
        let accepted_fingerprint = compute_ssh_sha256_fingerprint(&accepted_public_key).unwrap();
        let accepted_comment = identities
            .iter()
            .find(|identity| {
                compute_ssh_sha256_fingerprint(&identity.public_key().public_key_base64())
                    .ok()
                    .as_deref()
                    == Some(accepted_fingerprint.as_str())
            })
            .unwrap()
            .comment()
            .to_string();

        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.endpoint.host = "127.0.0.1".to_string();
            ssh.endpoint.port = port;
            ssh.username = openssh_test_username();
            ssh.reconnect = false;
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.host_key_policy.alias = Some("agent-policy-target".to_string());
            ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
            ssh.identity_policy.identities_only = false;
            ssh.identity_refs.clear();
            ssh.agent_policy.enabled = true;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::AfterProfileKeys;
        }
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let missing_socket = root.join("missing-agent.sock");

        let mut disabled = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut disabled.connection {
            ssh.agent_policy.enabled = false;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &disabled,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(missing_socket.clone()),
        )
        .await
        .err()
        .expect("disabled ssh-agent policy unexpectedly authenticated");
        assert!(error.contains("没有可尝试的认证方式"), "{error}");
        assert!(!error.contains("无法连接 SSH agent socket"), "{error}");

        let mut identities_only_without_refs = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut identities_only_without_refs.connection {
            ssh.identity_policy.identities_only = true;
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &identities_only_without_refs,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(missing_socket),
        )
        .await
        .err()
        .expect("IdentitiesOnly without agent refs unexpectedly authenticated");
        assert!(error.contains("IdentitiesOnly"), "{error}");
        assert!(!error.contains("无法连接 SSH agent socket"), "{error}");

        let unfiltered = establish_ssh_runtime_with_timeout(
            &state,
            &profile,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .unwrap();
        assert_eq!(unfiltered.auth_method, AuthMethod::PublicKey);
        disconnect_ssh_runtime(unfiltered.runtime, "PortMate agent test").await;

        let mut libssh_agent = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut libssh_agent.connection {
            ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::PublicKey];
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::BeforeProfileKeys;
        }
        let libssh_runtime = establish_ssh_runtime_with_timeout(
            &state,
            &libssh_agent,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .unwrap();
        assert_eq!(libssh_runtime.auth_method, AuthMethod::PublicKey);
        assert!(libssh_runtime.runtime.handle.lock().await.is_libssh());
        let EstablishedSshRuntime {
            runtime, read_half, ..
        } = libssh_runtime;
        let SshRuntime {
            handle,
            writer,
            closed,
            ..
        } = runtime;
        closed.store(true, Ordering::SeqCst);
        drop(read_half);
        drop(writer);
        handle
            .lock()
            .await
            .disconnect("PortMate libssh agent fallback test")
            .await
            .unwrap();

        let mut libssh_forwarding = libssh_agent.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut libssh_forwarding.connection {
            ssh.agent_policy.forwarding = true;
        }
        let forwarded = establish_ssh_runtime_with_timeout(
            &state,
            &libssh_forwarding,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .unwrap();
        assert!(forwarded.runtime.handle.lock().await.is_libssh());
        let EstablishedSshRuntime {
            runtime,
            mut read_half,
            ..
        } = forwarded;
        let SshRuntime {
            handle,
            writer,
            closed,
            agent_forwarder_finished,
            ..
        } = runtime;
        writer
            .lock()
            .await
            .data(b"ssh-add -L 2>&1; printf '\\n__PORTMATE_%s__\\n' AGENT_FORWARD_DONE\r")
            .await
            .unwrap();
        let output = tokio::time::timeout(Duration::from_secs(3), async {
            let mut output = Vec::new();
            loop {
                match read_half.wait().await {
                    Some(SshBackendMessage::Data(data)) => {
                        output.extend_from_slice(&data);
                        if output
                            .windows(b"__PORTMATE_AGENT_FORWARD_DONE__".len())
                            .any(|window| window == b"__PORTMATE_AGENT_FORWARD_DONE__")
                        {
                            return output;
                        }
                    }
                    Some(SshBackendMessage::ExtendedData { data, .. }) => {
                        output.extend_from_slice(&data);
                    }
                    Some(_) => {}
                    None => panic!("libssh agent forwarding terminal closed before marker"),
                }
            }
        })
        .await
        .expect("libssh agent forwarding command did not finish");
        assert!(
            String::from_utf8_lossy(&output).contains(&accepted_public_key),
            "remote ssh-add did not return the forwarded agent identity: {}",
            String::from_utf8_lossy(&output)
        );
        closed.store(true, Ordering::SeqCst);
        drop(read_half);
        drop(writer);
        if let Some(finished) = agent_forwarder_finished {
            tokio::time::timeout(Duration::from_secs(2), finished)
                .await
                .expect("libssh agent forwarder did not stop")
                .expect("libssh agent forwarder completion sender dropped");
        }
        handle
            .lock()
            .await
            .disconnect("PortMate libssh agent forwarding test")
            .await
            .unwrap();

        let (http_proxy_port, http_proxy_connections, http_proxy_task) =
            spawn_test_http_connect_proxy(200).await;
        let mut libssh_http_proxy = libssh_agent.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut libssh_http_proxy.connection {
            ssh.proxy = ProxyConfig {
                enabled: true,
                kind: ProxyKind::HttpConnect,
                host: "127.0.0.1".to_string(),
                port: http_proxy_port,
                ..ProxyConfig::default()
            };
        }
        let proxied = establish_ssh_runtime_with_timeout(
            &state,
            &libssh_http_proxy,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .unwrap();
        assert_eq!(proxied.auth_method, AuthMethod::PublicKey);
        assert!(proxied.runtime.handle.lock().await.is_libssh());
        let EstablishedSshRuntime {
            runtime, read_half, ..
        } = proxied;
        let SshRuntime {
            handle,
            writer,
            closed,
            ..
        } = runtime;
        closed.store(true, Ordering::SeqCst);
        drop(read_half);
        drop(writer);
        handle
            .lock()
            .await
            .disconnect("PortMate libssh HTTP proxy test")
            .await
            .unwrap();
        drop(handle);
        assert_eq!(http_proxy_connections.load(Ordering::SeqCst), 1);
        http_proxy_task.abort();
        let _ = http_proxy_task.await;

        let (socks_proxy_port, socks_proxy_connections, socks_proxy_task) =
            spawn_test_socks5_proxy(0).await;
        let mut libssh_socks_proxy = libssh_agent.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut libssh_socks_proxy.connection {
            ssh.proxy = ProxyConfig {
                enabled: true,
                kind: ProxyKind::Socks5,
                host: "127.0.0.1".to_string(),
                port: socks_proxy_port,
                ..ProxyConfig::default()
            };
        }
        let proxied = establish_ssh_runtime_with_timeout(
            &state,
            &libssh_socks_proxy,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .unwrap();
        assert_eq!(proxied.auth_method, AuthMethod::PublicKey);
        assert!(proxied.runtime.handle.lock().await.is_libssh());
        let EstablishedSshRuntime {
            runtime, read_half, ..
        } = proxied;
        let SshRuntime {
            handle,
            writer,
            closed,
            ..
        } = runtime;
        closed.store(true, Ordering::SeqCst);
        drop(read_half);
        drop(writer);
        handle
            .lock()
            .await
            .disconnect("PortMate libssh SOCKS5 proxy test")
            .await
            .unwrap();
        drop(handle);
        assert_eq!(socks_proxy_connections.load(Ordering::SeqCst), 1);
        socks_proxy_task.abort();
        let _ = socks_proxy_task.await;

        let (rejected_http_port, _, rejected_http_task) = spawn_test_http_connect_proxy(407).await;
        if let ConnectionConfig::Ssh(ssh) = &mut libssh_http_proxy.connection {
            ssh.proxy.port = rejected_http_port;
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &libssh_http_proxy,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .err()
        .expect("rejected HTTP CONNECT proxy unexpectedly established libssh");
        assert!(error.contains("HTTP CONNECT 被代理拒绝"), "{error}");
        rejected_http_task.abort();
        let _ = rejected_http_task.await;

        let (rejected_socks_port, _, rejected_socks_task) = spawn_test_socks5_proxy(0x05).await;
        if let ConnectionConfig::Ssh(ssh) = &mut libssh_socks_proxy.connection {
            ssh.proxy.port = rejected_socks_port;
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &libssh_socks_proxy,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .err()
        .expect("rejected SOCKS5 proxy unexpectedly established libssh");
        assert!(error.contains("SOCKS5 CONNECT 被拒绝"), "{error}");
        rejected_socks_task.abort();
        let _ = rejected_socks_task.await;

        let matching_ref = IdentityRef {
            id: "accepted-agent-key".to_string(),
            label: accepted_comment.clone(),
            source: IdentitySource::Agent,
            fingerprint_sha256: Some(accepted_fingerprint),
            path: Some(accepted_comment.clone()),
            secret_ref: None,
        };
        let mut filtered_libssh = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut filtered_libssh.connection {
            ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::PublicKey];
            ssh.identity_policy.identities_only = true;
            ssh.identity_refs = vec![matching_ref.clone()];
        }
        let filtered_libssh_runtime = establish_ssh_runtime_with_timeout(
            &state,
            &filtered_libssh,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .unwrap();
        assert_eq!(filtered_libssh_runtime.auth_method, AuthMethod::PublicKey);
        assert!(filtered_libssh_runtime
            .runtime
            .handle
            .lock()
            .await
            .is_libssh());
        let EstablishedSshRuntime {
            runtime, read_half, ..
        } = filtered_libssh_runtime;
        let SshRuntime {
            handle,
            writer,
            closed,
            ..
        } = runtime;
        closed.store(true, Ordering::SeqCst);
        drop(read_half);
        drop(writer);
        handle
            .lock()
            .await
            .disconnect("PortMate filtered libssh agent test")
            .await
            .unwrap();

        let mut mismatched_libssh = filtered_libssh;
        if let ConnectionConfig::Ssh(ssh) = &mut mismatched_libssh.connection {
            ssh.identity_refs[0].fingerprint_sha256 =
                Some("SHA256:deliberately-wrong-fingerprint".to_string());
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &mismatched_libssh,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .err()
        .expect("libssh bypassed a mismatched agent fingerprint");
        assert!(
            error.contains("libssh SSH authentication failed"),
            "{error}"
        );
        assert!(error.contains("SSH agent"), "{error}");

        let mut filtered = profile.clone();
        if let ConnectionConfig::Ssh(ssh) = &mut filtered.connection {
            ssh.identity_policy.identities_only = true;
            ssh.identity_refs = vec![matching_ref.clone()];
        }
        let filtered_runtime = establish_ssh_runtime_with_timeout(
            &state,
            &filtered,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .unwrap();
        assert_eq!(filtered_runtime.auth_method, AuthMethod::PublicKey);
        disconnect_ssh_runtime(filtered_runtime.runtime, "PortMate agent filter test").await;

        let mut mismatched = filtered;
        if let ConnectionConfig::Ssh(ssh) = &mut mismatched.connection {
            ssh.identity_refs[0].fingerprint_sha256 =
                Some("SHA256:deliberately-wrong-fingerprint".to_string());
        }
        let error = establish_ssh_runtime_with_timeout(
            &state,
            &mismatched,
            None,
            None,
            SSH_CONNECT_TIMEOUT,
            Some(agent_socket.clone()),
        )
        .await
        .err()
        .expect("mismatched agent fingerprint was bypassed by its comment");
        assert!(error.contains("IdentitiesOnly"), "{error}");
        assert!(error.contains("agent(after-profile-keys)"), "{error}");
    });

    agent.stop();
    sshd.stop();
    let _ = fs::remove_dir_all(root);
}

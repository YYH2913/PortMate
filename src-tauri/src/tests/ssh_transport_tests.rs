use super::*;

#[cfg(unix)]
#[test]
fn libssh_backend_exec_normalizes_output_and_exit_status() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh backend test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-backend-user";
        let secret = "PortMate libssh backend secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;

        let session = libssh_rs::Session::new().unwrap();
        // libssh 0.10's default cipher list has a trailing comma that russh rejects.
        session
            .set_option(libssh_rs::SshOption::CiphersCS("aes256-ctr".to_string()))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::CiphersSC("aes256-ctr".to_string()))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::ProcessConfig(false))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::Hostname("127.0.0.1".to_string()))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::Port(port))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::User(Some(username.to_string())))
            .unwrap();
        session.connect().unwrap();
        let ConnectionConfig::Ssh(mut ssh) = test_ssh_profile().connection else {
            panic!("expected SSH profile");
        };
        ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::Password];
        assert_eq!(
            authenticate_libssh_with_order(
                &session,
                &ordered_auth_methods(&ssh),
                Some(secret),
                &[],
                None,
            )
            .unwrap(),
            AuthMethod::Password
        );

        let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::<
            AcceptAnyTestSshClient,
        >::from_libssh(session)));
        let output = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_SUCCESS__",
            Duration::from_secs(2),
        )
        .await
        .unwrap();
        assert_eq!(output, "captured");

        let error = exec_ssh_command_capture(
            Arc::clone(&handle),
            "__PORTMATE_TEST_EXEC_NONZERO__",
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "SSH exec 返回非零状态 7: remote failure");
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 2);

        handle
            .lock()
            .await
            .disconnect("PortMate libssh backend test complete")
            .await
            .unwrap();
        server_task.abort();
        let _ = server_task.await;
    });
}

#[test]
fn libssh_backend_selection_accepts_supported_gssapi_mixed_auth_order() {
    let ConnectionConfig::Ssh(mut ssh) = test_ssh_profile().connection else {
        panic!("expected SSH profile");
    };
    ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic];
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.auth_order = vec![
        AuthMethod::GssapiWithMic,
        AuthMethod::KeyboardInteractive,
        AuthMethod::Password,
    ];
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.auth_order = vec![AuthMethod::Password, AuthMethod::GssapiWithMic];
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::PublicKey];
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.agent_policy.enabled = true;
    ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::BeforeProfileKeys;
    ssh.identity_policy.identities_only = false;
    assert!(!ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_policy.identities_only = true;
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.identity_refs.push(IdentityRef {
        id: "agent-key".to_string(),
        label: "Agent key".to_string(),
        source: IdentitySource::Agent,
        fingerprint_sha256: Some("SHA256:test".to_string()),
        path: None,
        secret_ref: None,
    });
    assert!(!ssh_uses_libssh_gssapi_backend(&ssh));

    ssh.agent_policy.enabled = false;
    ssh.identity_policy.auth_order = vec![AuthMethod::Password];
    assert!(!ssh_uses_libssh_gssapi_backend(&ssh));
}

#[cfg(unix)]
#[test]
fn libssh_mixed_auth_falls_back_to_keyboard_interactive() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh keyboard-interactive test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-keyboard-user";
        let secret = "PortMate libssh keyboard secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;

        let session = libssh_rs::Session::new().unwrap();
        session
            .set_option(libssh_rs::SshOption::CiphersCS("aes256-ctr".to_string()))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::CiphersSC("aes256-ctr".to_string()))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::ProcessConfig(false))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::Hostname("127.0.0.1".to_string()))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::Port(port))
            .unwrap();
        session
            .set_option(libssh_rs::SshOption::User(Some(username.to_string())))
            .unwrap();
        session.connect().unwrap();

        assert_eq!(
            authenticate_libssh_with_order(
                &session,
                &[AuthMethod::GssapiWithMic, AuthMethod::KeyboardInteractive,],
                Some(secret),
                &[],
                None,
            )
            .unwrap(),
            AuthMethod::KeyboardInteractive
        );
        assert_eq!(
            counters
                .keyboard_interactive_successes
                .load(Ordering::SeqCst),
            1
        );

        session.disconnect();
        server_task.abort();
        let _ = server_task.await;
    });
}

#[cfg(unix)]
#[test]
fn libssh_gssapi_falls_back_to_ordered_explicit_public_keys() {
    let _runtime_guard = shared_runtime_test_guard();
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping libssh public-key test: sshd is not installed");
        return;
    };
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh public-key test: ssh-keygen is not installed");
        return;
    }

    let root = std::env::temp_dir().join(format!("portmate-libssh-public-key-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let host_key = root.join("ssh_host_ed25519_key");
    let rejected_key = root.join("id_rejected");
    let accepted_key = root.join("id_accepted");
    generate_ed25519_test_key(&host_key);
    generate_ed25519_test_key(&rejected_key);
    let passphrase = "PortMate libssh encrypted key passphrase";
    let keygen = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", passphrase, "-f"])
        .arg(&accepted_key)
        .status()
        .unwrap();
    assert!(keygen.success(), "ssh-keygen failed for encrypted key");
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

    tauri::async_runtime::block_on(async {
        wait_for_openssh_test_server(&mut sshd, port, "libssh public-key sshd").await;
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.endpoint.host = "127.0.0.1".to_string();
        ssh.endpoint.port = port;
        ssh.username = openssh_test_username();
        ssh.reconnect = false;
        ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
        ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic, AuthMethod::PublicKey];
        ssh.identity_policy.identities_only = true;
        ssh.identity_refs = [(&rejected_key, "rejected"), (&accepted_key, "accepted")]
            .into_iter()
            .map(|(path, id)| IdentityRef {
                id: id.to_string(),
                label: format!("{id} key"),
                source: IdentitySource::SystemFile,
                fingerprint_sha256: None,
                path: Some(path.display().to_string()),
                secret_ref: None,
            })
            .collect();
        ssh.agent_policy.enabled = false;
        ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;

        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_ssh_session(&state, profile.clone(), None, Some(passphrase.to_string()))
            .await
            .unwrap();
        let stored = state
            .store
            .lock()
            .unwrap()
            .profiles
            .iter()
            .find(|candidate| candidate.id == profile.id)
            .cloned()
            .unwrap();
        let ConnectionConfig::Ssh(stored_ssh) = stored.connection else {
            panic!("stored profile changed transport");
        };
        assert_eq!(
            stored_ssh.identity_policy.last_successful,
            Some(AuthMethod::PublicKey)
        );
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
    });

    sshd.stop();
    let _ = fs::remove_dir_all(root);
}

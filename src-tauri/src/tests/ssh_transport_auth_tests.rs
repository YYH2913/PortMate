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
    assert!(ssh_uses_libssh_gssapi_backend(&ssh));

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
    assert_eq!(ssh_uses_libssh_gssapi_backend(&ssh), cfg!(unix));
    assert_eq!(libssh_agent_offer_positions(&ssh, true), (false, true));

    ssh.identity_policy.identities_only = false;
    assert_eq!(libssh_agent_offer_positions(&ssh, true), (true, false));
    assert_eq!(libssh_agent_offer_positions(&ssh, false), (false, false));

    ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
    assert_eq!(libssh_agent_offer_positions(&ssh, true), (false, false));

    ssh.agent_policy.enabled = false;
    ssh.identity_policy.auth_order = vec![AuthMethod::Password];
    assert!(!ssh_uses_libssh_gssapi_backend(&ssh));
}

#[cfg(target_os = "linux")]
#[test]
fn libssh_gssapi_file_cache_validation_rejects_corruption_without_restricting_other_backends() {
    use std::ffi::OsString;

    let root = std::env::temp_dir().join(format!(
        "portmate-gssapi-cache-validation-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let cache_path = root.join("ticket.ccache");
    let cache_name = OsString::from(format!("FILE:{}", cache_path.display()));

    assert!(validate_libssh_gssapi_credential_cache(Some(cache_name.as_os_str())).is_ok());
    assert!(
        validate_libssh_gssapi_credential_cache(Some(std::ffi::OsStr::new(
            "KEYRING:persistent:1000"
        )))
        .is_ok()
    );

    fs::write(&cache_path, []).unwrap();
    let truncated =
        validate_libssh_gssapi_credential_cache(Some(cache_name.as_os_str())).unwrap_err();
    assert!(truncated.contains("truncated"), "{truncated}");

    fs::write(&cache_path, b"not-a-valid-cache").unwrap();
    let invalid =
        validate_libssh_gssapi_credential_cache(Some(cache_name.as_os_str())).unwrap_err();
    assert!(invalid.contains("invalid"), "{invalid}");

    fs::write(&cache_path, [5, 4]).unwrap();
    let short_header =
        validate_libssh_gssapi_credential_cache(Some(cache_name.as_os_str())).unwrap_err();
    assert!(short_header.contains("truncated"), "{short_header}");

    fs::remove_dir_all(root).unwrap();
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
                false,
                false,
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
fn libssh_connection_stages_share_one_total_deadline() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh setup deadline test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let username = "portmate-libssh-deadline-user";
        let secret = "PortMate libssh deadline secret";
        let (port, counters, server_task) = spawn_mixed_auth_test_server_with_delays(
            &host_key,
            username,
            secret,
            Some(Duration::from_millis(300)),
            Some(Duration::from_millis(1700)),
            None,
        )
        .await;
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.endpoint.host = "127.0.0.1".to_string();
        ssh.endpoint.port = port;
        ssh.username = username.to_string();
        ssh.reconnect = false;
        ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
        ssh.identity_policy.auth_order = vec![AuthMethod::Password, AuthMethod::GssapiWithMic];
        ssh.identity_policy.identities_only = true;
        ssh.identity_refs.clear();
        ssh.agent_policy.enabled = false;
        ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
        let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));

        let error = match establish_ssh_runtime_with_timeout(
            &state,
            &profile,
            Some(secret.to_string()),
            None,
            Duration::from_millis(2000),
            None,
        )
        .await
        {
            Ok(_) => panic!("libssh setup ignored its shared connection deadline"),
            Err(error) => error,
        };
        let normalized = error.to_ascii_lowercase();
        assert!(
            error.contains("超时") || normalized.contains("timeout"),
            "libssh setup did not exhaust the shared deadline: {error}"
        );
        assert_eq!(counters.password_completions.load(Ordering::SeqCst), 1);
        assert_eq!(counters.session_channel_attempts.load(Ordering::SeqCst), 1);
        assert!(!state.ssh.lock().unwrap().contains_key(&profile.id));

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

    let root = canonical_test_temp_path("portmate-libssh-public-key");
    fs::create_dir_all(&root).unwrap();
    let host_key = root.join("ssh_host_ed25519_key");
    let rejected_key = root.join("id_rejected");
    let accepted_key = root.join("id_accepted ");
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

        let failed_state =
            test_app_state(profile.clone(), root.join("failed-portmate-store.sqlite3"));
        let error = open_ssh_session(
            &failed_state,
            profile.clone(),
            None,
            Some("wrong passphrase".to_string()),
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("libssh SSH authentication failed"),
            "{error}"
        );
        assert!(error.contains("private key 解析失败"), "{error}");
        assert!(!failed_state.ssh.lock().unwrap().contains_key(&profile.id));

        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_ssh_session(&state, profile.clone(), None, Some(passphrase.to_string()))
            .await
            .unwrap();
        let health = ssh_health::check_ssh_health_inner(&state, &profile.id, true)
            .await
            .unwrap();
        assert_eq!(health.status, ssh_health::SshHealthStatus::Healthy);
        assert_eq!(health.backend, SshBackendKind::Libssh);
        assert_eq!(health.authentication_method, AuthMethod::PublicKey);
        assert!(health.terminal_channel_open);
        assert!(health.terminal_error.is_none());
        assert!(health.sftp_probed);
        assert!(health.sftp_round_trip_ms.is_some());
        assert!(health.sftp_error.is_none());

        let terminal_channel_open = state
            .ssh
            .lock()
            .unwrap()
            .get(&profile.id)
            .unwrap()
            .terminal_channel_open
            .clone();
        terminal_channel_open.store(false, Ordering::SeqCst);
        let terminal_closed_health = ssh_health::check_ssh_health_inner(&state, &profile.id, false)
            .await
            .unwrap();
        assert_eq!(
            terminal_closed_health.status,
            ssh_health::SshHealthStatus::Degraded
        );
        assert!(!terminal_closed_health.terminal_channel_open);
        assert!(terminal_closed_health.transport_round_trip_ms.is_some());
        assert!(terminal_closed_health.channel_round_trip_ms.is_some());
        assert!(terminal_closed_health
            .terminal_error
            .as_deref()
            .is_some_and(|error| error.contains("交互输入不可用")));
        terminal_channel_open.store(true, Ordering::SeqCst);

        state
            .store
            .lock()
            .unwrap()
            .transfers
            .push(test_transfer_task(&profile.id, TransferStatus::Running));
        let progress = test_transfer_progress_context(
            &state,
            "transfer-commit-test",
            Arc::new(AtomicBool::new(false)),
        );
        let source = root.join("libssh-sftp-source.bin");
        let downloaded = root.join("libssh-sftp-downloaded.bin");
        let payload = b"PortMate libssh native SFTP payload";
        fs::write(&source, payload).unwrap();
        let remote_root = root.join("libssh-sftp").display().to_string();
        let uploaded = remote_join_path(&remote_root, "uploaded.bin");
        let copied = remote_join_path(&remote_root, "copied.bin");
        let renamed = remote_join_path(&remote_root, "renamed.bin");
        let exclusive = remote_join_path(&remote_root, "exclusive.bin");

        let auxiliary = ssh_auxiliary_lease(&state, &profile.id).unwrap();
        let sftp = auxiliary.sftp().await.unwrap();
        sftp_create_dir_all(&sftp, &remote_root).await.unwrap();
        let mut exclusive_file = sftp
            .open_with_flags(
                exclusive.clone(),
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .unwrap();
        exclusive_file.shutdown().await.unwrap();
        assert!(sftp
            .open_with_flags(
                exclusive.clone(),
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .is_err());
        sftp.remove_file(exclusive).await.unwrap();
        assert_eq!(
            sftp_upload(&sftp, source.to_str().unwrap(), &uploaded, &progress,)
                .await
                .unwrap(),
            payload.len() as u64
        );
        let mut uploaded_file = sftp.open(uploaded.clone()).await.unwrap();
        let end = tokio::time::timeout(
            Duration::from_secs(2),
            uploaded_file.seek(std::io::SeekFrom::End(0)),
        )
        .await
        .expect("libssh SFTP seek from end deadlocked")
        .unwrap();
        assert_eq!(end, payload.len() as u64);
        assert_eq!(
            uploaded_file
                .seek(std::io::SeekFrom::End(-((payload.len() as i64) + 1)))
                .await
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        uploaded_file.shutdown().await.unwrap();
        assert_eq!(
            sftp_download(&sftp, &uploaded, downloaded.to_str().unwrap(), &progress,)
                .await
                .unwrap(),
            payload.len() as u64
        );
        assert_eq!(fs::read(&downloaded).unwrap(), payload);
        assert_eq!(
            sftp_remote_copy(&sftp, &uploaded, &copied, &progress)
                .await
                .unwrap(),
            payload.len() as u64
        );

        let entries = list_remote_files(&sftp, &remote_root).await.unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from(["uploaded.bin", "copied.bin"])
        );
        let SftpBackendSession::Libssh(session) = &*sftp else {
            panic!("expected libssh SFTP backend");
        };
        let session = Arc::clone(session);
        let bounded_root = remote_root.clone();
        let error = tokio::task::spawn_blocking(move || {
            match session.blocking_lock().read_dir_bounded(&bounded_root, 1) {
                Ok(_) => panic!("libssh SFTP returned a silently truncated directory"),
                Err(error) => error,
            }
        })
        .await
        .unwrap();
        assert!(error
            .to_string()
            .contains("SFTP directory entry count exceeds 1"));
        let properties = remote_file_properties(&sftp, &copied).await.unwrap();
        assert!(properties.is_file);
        assert_eq!(properties.size, payload.len() as u64);

        let mut metadata = sftp.symlink_metadata(copied.clone()).await.unwrap();
        let file_type_bits = metadata.permissions.unwrap_or(0) & 0o170000;
        metadata.permissions = Some(file_type_bits | 0o600);
        sftp.set_metadata(copied.clone(), metadata).await.unwrap();
        assert_eq!(
            sftp.symlink_metadata(copied.clone())
                .await
                .unwrap()
                .permissions
                .unwrap_or(0)
                & 0o777,
            0o600
        );
        sftp.rename(copied.clone(), renamed.clone()).await.unwrap();
        assert!(!sftp.try_exists(copied).await.unwrap());
        assert!(sftp.try_exists(renamed).await.unwrap());
        sftp_remove_recursive(&sftp, &remote_root).await.unwrap();
        assert!(!sftp.try_exists(remote_root).await.unwrap());
        drop(sftp);
        drop(auxiliary);

        assert_libssh_local_and_dynamic_tunnels(&state, &profile.id).await;

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

#[cfg(unix)]
#[test]
fn libssh_profile_vault_key_loader_uses_secret_ref_and_passphrase() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping libssh vault key test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let key_path = root.path().join("vault_ed25519");
    let passphrase = "PortMate vault identity passphrase";
    let keygen = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", passphrase, "-f"])
        .arg(&key_path)
        .status()
        .unwrap();
    assert!(keygen.success(), "ssh-keygen failed for vault identity");
    let key_material = fs::read_to_string(&key_path).unwrap();
    let identity = IdentityRef {
        id: "vault-key".to_string(),
        label: "Vault key".to_string(),
        source: IdentitySource::ProfileVault,
        fingerprint_sha256: None,
        path: None,
        secret_ref: Some("stronghold:vault-key".to_string()),
    };

    let key = load_libssh_private_key_with(&identity, Some(passphrase), |secret_ref| {
        assert_eq!(secret_ref, "stronghold:vault-key");
        Ok(key_material.clone())
    })
    .unwrap()
    .unwrap();
    assert_eq!(key.key_type_name().unwrap(), "ssh-ed25519");

    let error = load_libssh_private_key_with(&identity, Some("wrong passphrase"), |_| {
        Ok(key_material.clone())
    })
    .err()
    .expect("wrong vault passphrase unexpectedly parsed");
    assert!(error.contains("private key 解析失败"), "{error}");

    let error = load_libssh_private_key_with(&identity, Some(passphrase), |secret_ref| {
        Err(format!("missing test secret: {secret_ref}"))
    })
    .err()
    .expect("missing vault secret unexpectedly loaded");
    assert!(
        error.contains("profile-vault stronghold:vault-key"),
        "{error}"
    );
    assert!(!error.contains(&key_material));

    let mut missing_ref = identity;
    missing_ref.secret_ref = None;
    let error = load_libssh_private_key_with(&missing_ref, Some(passphrase), |_| {
        panic!("missing secretRef must not invoke the provider")
    })
    .err()
    .expect("missing vault secretRef unexpectedly loaded");
    assert_eq!(error, "profile-vault identity 缺少 secretRef");
}

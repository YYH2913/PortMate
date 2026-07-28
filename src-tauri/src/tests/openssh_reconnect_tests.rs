use super::*;

#[cfg(unix)]
#[test]
fn openssh_reconnect_store_commit_failure_does_not_install_runtime() {
    let _runtime_guard = shared_runtime_test_guard();
    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping OpenSSH reconnect Store test: sshd is not installed");
        return;
    };
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping OpenSSH reconnect Store test: ssh-keygen is not installed");
        return;
    }

    let root = std::env::temp_dir().join(format!(
        "portmate-ssh-reconnect-commit-failure-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    let host_key = root.join("ssh_host_ed25519_key");
    let client_key = root.join("id_ed25519");
    generate_ed25519_test_key(&host_key);
    generate_ed25519_test_key(&client_key);
    let authorized_keys = root.join("authorized_keys");
    fs::copy(client_key.with_extension("pub"), &authorized_keys).unwrap();
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
        wait_for_openssh_test_server(&mut sshd, port, "reconnect Store sshd").await;
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.endpoint.host = "127.0.0.1".to_string();
        ssh.endpoint.port = port;
        ssh.username = openssh_test_username();
        ssh.reconnect = true;
        ssh.reconnect_delay_ms = 100;
        ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
        ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
        ssh.identity_refs = vec![IdentityRef {
            id: "reconnect-store-client-key".to_string(),
            label: "reconnect Store client key".to_string(),
            source: IdentitySource::SystemFile,
            fingerprint_sha256: None,
            path: Some(client_key.display().to_string()),
            secret_ref: None,
        }];
        ssh.agent_policy.enabled = false;
        ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;

        let store_dir = root.join("store");
        fs::create_dir_all(&store_dir).unwrap();
        let state = test_app_state(profile.clone(), store_dir.join("portmate-store.sqlite3"));
        open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap();
        let reconnect_handle = {
            let connections = state.ssh.lock().unwrap();
            Arc::clone(&connections.get(&profile.id).unwrap().handle)
        };

        *state.ssh_reconnect_install_error.lock().unwrap() =
            Some("injected SSH reconnect install commit failure".to_string());
        {
            let handle = reconnect_handle.lock().await;
            handle
                .disconnect("PortMate reconnect Store failure test")
                .await
                .unwrap();
        }

        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let status = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .unwrap()
                    .runtime
                    .status;
                if status != SessionStatus::Connected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("SSH disconnect did not leave the connected state");

        let failed = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let summary = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .unwrap();
                if summary.runtime.status == SessionStatus::Error {
                    break summary;
                }
                assert_ne!(
                    summary.runtime.status,
                    SessionStatus::Connected,
                    "failed SSH reconnect exposed a connected runtime"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("SSH reconnect Store failure did not settle to Error");

        assert!(
            failed
                .runtime
                .last_disconnect_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("SSH reconnect install failed")),
            "unexpected SSH reconnect failure: {:?}",
            failed.runtime.last_disconnect_reason
        );
        assert!(!state.ssh.lock().unwrap().contains_key(&profile.id));
        assert!(state.store.lock().unwrap().events.iter().any(|event| {
            event
                .text
                .as_deref()
                .is_some_and(|text| text.contains("SSH reconnect install failed"))
        }));
    });

    sshd.stop();
    let _ = fs::remove_dir_all(root);
}

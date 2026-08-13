use super::openssh_modem_integration::exercise_openssh_modem_transfers;
use super::openssh_sftp_integration::exercise_openssh_sftp_operations;
use super::openssh_transfer_recovery_integration::exercise_openssh_scp_and_transfer_recovery;
use super::openssh_tunnel_integration::{
    exercise_openssh_local_and_dynamic_tunnels, exercise_openssh_remote_tunnel,
    exercise_openssh_tunnel_reconnect,
};
use super::*;

#[cfg(unix)]
#[test]
fn openssh_sftp_scp_and_tunnels_end_to_end() {
    let _runtime_guard = shared_runtime_test_guard();

    let Some(sshd_path) = openssh_test_server_path() else {
        eprintln!("skipping OpenSSH integration test: sshd is not installed");
        return;
    };
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping OpenSSH integration test: ssh-keygen is not installed");
        return;
    }
    let modem_tools_available = ["rx", "sx", "rb", "sb", "rz", "sz"]
        .into_iter()
        .all(|command| Command::new(command).arg("--version").output().is_ok());

    let root = canonical_test_temp_path("portmate-sshd-test");
    fs::create_dir_all(&root).unwrap();
    let host_key = root.join("ssh_host_ed25519_key");
    let replacement_host_key = root.join("ssh_host_ed25519_key_replacement");
    let client_key = root.join("id_ed25519");
    for key_path in [&host_key, &replacement_host_key, &client_key] {
        generate_ed25519_test_key(key_path);
    }
    let authorized_keys = root.join("authorized_keys");
    fs::copy(client_key.with_extension("pub"), &authorized_keys).unwrap();

    let port = {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        listener.local_addr().unwrap().port()
    };
    let username = openssh_test_username();
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
        wait_for_openssh_test_server(&mut sshd, port, "sshd").await;

        let mut profile = test_ssh_profile();
        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.endpoint.host = "127.0.0.1".to_string();
            ssh.endpoint.port = port;
            ssh.username = username.clone();
            ssh.reconnect = true;
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.identity_policy.auth_order = vec![AuthMethod::PublicKey];
            ssh.identity_refs = vec![IdentityRef {
                id: "integration-client-key".to_string(),
                label: "integration client key".to_string(),
                source: IdentitySource::SystemFile,
                fingerprint_sha256: None,
                path: Some(client_key.display().to_string()),
                secret_ref: None,
            }];
            ssh.agent_policy.enabled = false;
            ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;
        }
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let summary = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(summary.runtime.status, SessionStatus::Connected);
        assert_eq!(summary.profile.connection.kind(), SessionKind::Ssh);
        assert_eq!(state.store.lock().unwrap().host_keys.keys.len(), 1);

        send_text_inner(
            state.session_io(),
            profile.id.clone(),
            "printf '__PORTMATE_SSH_OK__\\n'\n".to_string(),
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
                    .is_some_and(|screen| screen.contains("__PORTMATE_SSH_OK__"))
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("SSH PTY command output was not recorded");

        exercise_openssh_sftp_operations(&state, &profile, &root).await;

        exercise_openssh_scp_and_transfer_recovery(&state, &profile, &root).await;

        exercise_openssh_modem_transfers(&state, &profile, &root, modem_tools_available).await;

        exercise_openssh_local_and_dynamic_tunnels(&state, &profile).await;

        exercise_openssh_remote_tunnel(&state, &profile).await;

        exercise_openssh_tunnel_reconnect(&state, &profile, port).await;

        let closed = close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        assert_eq!(closed.runtime.status, SessionStatus::Disconnected);
        tokio::time::sleep(Duration::from_millis(200)).await;

        sshd.stop();
        write_openssh_test_config(
            &config_path,
            &replacement_host_key,
            &root.join("sshd.pid"),
            &authorized_keys,
            port,
        );
        sshd = spawn_openssh_test_server(sshd_path, &config_path);
        wait_for_openssh_test_server(&mut sshd, port, "replacement sshd").await;

        let trusted_before = state.store.lock().unwrap().host_keys.keys.clone();
        let mismatch = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap_err();
        assert!(mismatch.contains("alias=bench-device"), "{mismatch}");
        assert!(mismatch.contains("observed="), "{mismatch}");
        assert!(mismatch.contains("expected=["), "{mismatch}");
        assert_eq!(state.store.lock().unwrap().host_keys.keys, trusted_before);

        if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
            ssh.host_key_policy.allow_rotation = true;
        }
        state.store.lock().unwrap().upsert_profile(profile.clone());
        let rotated = open_ssh_session(&state, profile.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(rotated.runtime.status, SessionStatus::Connected);
        let trusted_after_rotation = state.store.lock().unwrap().host_keys.keys.clone();
        assert_eq!(trusted_after_rotation.len(), 2);
        assert!(trusted_after_rotation
            .iter()
            .all(|key| key.alias == "bench-device" && key.port == port));
        assert_ne!(
            trusted_after_rotation[0].fingerprint_sha256,
            trusted_after_rotation[1].fingerprint_sha256
        );
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
    });

    sshd.stop();
    let _ = fs::remove_dir_all(root);
}

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

#[cfg(unix)]
#[test]
fn openssh_sftp_scp_and_tunnels_end_to_end() {
    let _runtime_guard = shared_runtime_test_guard();
    use std::os::unix::fs::PermissionsExt;

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

    let root = std::env::temp_dir().join(format!("portmate-sshd-test-{}", Uuid::new_v4()));
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

        let entries = list_files_inner(
            &state,
            ListFilesRequest {
                session_id: Some(profile.id.clone()),
                path: ".".to_string(),
                remote: true,
            },
        )
        .await
        .unwrap();
        assert!(entries.iter().all(|entry| !entry.name.is_empty()));

        let sftp_root = root.join("sftp-workspace");
        let sftp_nested = sftp_root.join("nested");
        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: sftp_nested.display().to_string(),
                remote: true,
            },
            FileOperation::CreateDirectory,
        )
        .await
        .unwrap();
        assert!(sftp_nested.is_dir());

        let sftp_new_file = root.join("sftp-created-file.txt");
        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: sftp_new_file.display().to_string(),
                remote: true,
            },
            FileOperation::CreateFile,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&sftp_new_file).unwrap(), b"");
        fs::write(&sftp_new_file, b"existing remote contents").unwrap();
        let error = file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: sftp_new_file.display().to_string(),
                remote: true,
            },
            FileOperation::CreateFile,
        )
        .await
        .unwrap_err();
        assert!(error.contains("新建远端文件"), "{error}");
        assert_eq!(
            fs::read(&sftp_new_file).unwrap(),
            b"existing remote contents"
        );

        let sftp_move_source = root.join("sftp-move-source");
        let sftp_move_destination = root.join("sftp-move-destination");
        let sftp_move_file = sftp_move_source.join("report.txt");
        let sftp_move_directory = sftp_move_source.join("nested");
        fs::create_dir_all(&sftp_move_directory).unwrap();
        fs::create_dir(&sftp_move_destination).unwrap();
        fs::write(&sftp_move_file, b"remote report").unwrap();
        fs::write(sftp_move_directory.join("detail.txt"), b"remote detail").unwrap();
        move_paths_inner(
            &state,
            MovePathsRequest {
                session_id: Some(profile.id.clone()),
                paths: vec![
                    sftp_move_file.display().to_string(),
                    sftp_move_directory.display().to_string(),
                ],
                destination: sftp_move_destination.display().to_string(),
                remote: true,
            },
        )
        .await
        .unwrap();
        assert!(!sftp_move_file.exists());
        assert!(!sftp_move_directory.exists());
        assert_eq!(
            fs::read(sftp_move_destination.join("report.txt")).unwrap(),
            b"remote report"
        );
        assert_eq!(
            fs::read(sftp_move_destination.join("nested/detail.txt")).unwrap(),
            b"remote detail"
        );

        let sftp_move_first = sftp_move_source.join("first.txt");
        let sftp_move_collision = sftp_move_source.join("collision.txt");
        fs::write(&sftp_move_first, b"first source").unwrap();
        fs::write(&sftp_move_collision, b"collision source").unwrap();
        fs::write(
            sftp_move_destination.join("collision.txt"),
            b"existing remote target",
        )
        .unwrap();
        let error = move_paths_inner(
            &state,
            MovePathsRequest {
                session_id: Some(profile.id.clone()),
                paths: vec![
                    sftp_move_first.display().to_string(),
                    sftp_move_collision.display().to_string(),
                ],
                destination: sftp_move_destination.display().to_string(),
                remote: true,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("已存在"), "{error}");
        assert_eq!(fs::read(&sftp_move_first).unwrap(), b"first source");
        assert_eq!(fs::read(&sftp_move_collision).unwrap(), b"collision source");
        assert!(!sftp_move_destination.join("first.txt").exists());
        assert_eq!(
            fs::read(sftp_move_destination.join("collision.txt")).unwrap(),
            b"existing remote target"
        );

        let sftp_delete_root = root.join("sftp-delete-root");
        let sftp_delete_file = sftp_delete_root.join("single.txt");
        let sftp_delete_directory = sftp_delete_root.join("nested");
        fs::create_dir_all(&sftp_delete_directory).unwrap();
        fs::write(&sftp_delete_file, b"delete remote file").unwrap();
        fs::write(
            sftp_delete_directory.join("value.txt"),
            b"delete remote nested",
        )
        .unwrap();
        delete_paths_inner(
            &state,
            DeletePathsRequest {
                session_id: Some(profile.id.clone()),
                paths: vec![
                    sftp_delete_file.display().to_string(),
                    sftp_delete_directory.display().to_string(),
                ],
                remote: true,
            },
        )
        .await
        .unwrap();
        assert!(!sftp_delete_file.exists());
        assert!(!sftp_delete_directory.exists());
        fs::remove_dir(&sftp_delete_root).unwrap();

        let sftp_link_target = root.join("sftp-link-target");
        let sftp_directory_link = root.join("sftp-directory-link");
        fs::create_dir(&sftp_link_target).unwrap();
        std::os::unix::fs::symlink(&sftp_link_target, &sftp_directory_link).unwrap();
        let error = file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: sftp_directory_link
                    .join("new-file.txt")
                    .display()
                    .to_string(),
                remote: true,
            },
            FileOperation::CreateFile,
        )
        .await
        .unwrap_err();
        assert!(error.contains("符号链接"), "{error}");
        assert!(!sftp_link_target.join("new-file.txt").exists());
        let error = file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: sftp_directory_link.join("nested").display().to_string(),
                remote: true,
            },
            FileOperation::CreateDirectory,
        )
        .await
        .unwrap_err();
        assert!(error.contains("符号链接"), "{error}");
        assert!(!sftp_link_target.join("nested").exists());
        let linked_file = sftp_link_target.join("protected.bin");
        fs::write(&linked_file, b"protected").unwrap();
        let linked_path = sftp_directory_link.join("protected.bin");
        let original_mode = fs::metadata(&linked_file).unwrap().permissions().mode() & 0o777;
        let error = chmod_path_inner(
            &state,
            ChmodPathRequest {
                session_id: Some(profile.id.clone()),
                path: linked_path.display().to_string(),
                mode: 0o600,
                remote: true,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("符号链接"), "{error}");
        assert_eq!(
            fs::metadata(&linked_file).unwrap().permissions().mode() & 0o777,
            original_mode
        );
        let error = rename_path_inner(
            &state,
            RenamePathRequest {
                session_id: Some(profile.id.clone()),
                old_path: linked_path.display().to_string(),
                new_path: sftp_directory_link
                    .join("renamed.bin")
                    .display()
                    .to_string(),
                remote: true,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("符号链接"), "{error}");
        let error = file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: linked_path.display().to_string(),
                remote: true,
            },
            FileOperation::Delete,
        )
        .await
        .unwrap_err();
        assert!(error.contains("符号链接"), "{error}");
        assert_eq!(fs::read(&linked_file).unwrap(), b"protected");

        let renamed_directory_link = root.join("sftp-directory-link-renamed");
        rename_path_inner(
            &state,
            RenamePathRequest {
                session_id: Some(profile.id.clone()),
                old_path: sftp_directory_link.display().to_string(),
                new_path: renamed_directory_link.display().to_string(),
                remote: true,
            },
        )
        .await
        .unwrap();
        assert!(fs::symlink_metadata(&renamed_directory_link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&linked_file).unwrap(), b"protected");
        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: renamed_directory_link.display().to_string(),
                remote: true,
            },
            FileOperation::Delete,
        )
        .await
        .unwrap();
        assert!(fs::symlink_metadata(&renamed_directory_link).is_err());
        assert_eq!(fs::read(&linked_file).unwrap(), b"protected");
        fs::remove_dir_all(&sftp_link_target).unwrap();

        let drop_source = root.join("external-drop-source");
        let drop_source_nested = drop_source.join("nested");
        fs::create_dir_all(drop_source.join("empty")).unwrap();
        fs::create_dir_all(&drop_source_nested).unwrap();
        fs::write(drop_source.join("alpha.txt"), b"external-alpha").unwrap();
        fs::write(drop_source_nested.join("beta.bin"), b"external-beta").unwrap();
        let drop_remote_target = root.join("external-drop-remote");
        let drop_result = start_external_drop_inner(
            &state,
            StartExternalDropRequest {
                session_id: profile.id.clone(),
                paths: vec![drop_source.display().to_string()],
                destination: drop_remote_target.display().to_string(),
                remote: true,
                conflict_policy: TransferConflictPolicy::Fail,
            },
        )
        .await
        .unwrap();
        assert_eq!(drop_result.tasks.len(), 2);
        assert_eq!(drop_result.directories_prepared, 3);
        assert_eq!(drop_result.total_bytes, 27);
        assert!(drop_result.skipped.is_empty());
        for task in drop_result.tasks {
            let task = wait_for_transfer_terminal_state(&state, &task.id).await;
            assert_eq!(
                task.status,
                TransferStatus::Completed,
                "recursive external SFTP drop failed: {:?}",
                task.message
            );
        }
        let dropped_remote_tree = drop_remote_target.join("external-drop-source");
        assert_eq!(
            fs::read(dropped_remote_tree.join("alpha.txt")).unwrap(),
            b"external-alpha"
        );
        assert_eq!(
            fs::read(dropped_remote_tree.join("nested/beta.bin")).unwrap(),
            b"external-beta"
        );
        assert!(dropped_remote_tree.join("empty").is_dir());

        let sftp_source = root.join("sftp-upload-source.bin");
        let sftp_payload = b"PortMate OpenSSH SFTP integration payload\n";
        fs::write(&sftp_source, sftp_payload).unwrap();
        let uploaded_sftp_file = sftp_nested.join("sftp-upload-source.bin");
        let uploaded_sftp_part = PathBuf::from(remote_resume_part_path(
            uploaded_sftp_file.to_str().unwrap(),
        ));
        fs::write(&uploaded_sftp_part, b"wrong-prefix").unwrap();
        let sftp_upload = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: sftp_source.display().to_string(),
                destination: format!("remote:{}/", sftp_nested.display()),
            },
        )
        .await
        .unwrap();
        let sftp_upload = wait_for_transfer_terminal_state(&state, &sftp_upload.id).await;
        assert_eq!(
            sftp_upload.status,
            TransferStatus::Completed,
            "SFTP upload failed: {:?}",
            sftp_upload.message
        );
        assert_eq!(sftp_upload.bytes_done, sftp_payload.len() as u64);
        assert!(!uploaded_sftp_part.exists());

        let existing_rename_target = sftp_nested.join("existing-rename-target.bin");
        fs::write(&existing_rename_target, b"existing rename target").unwrap();
        let error = rename_path_inner(
            &state,
            RenamePathRequest {
                session_id: Some(profile.id.clone()),
                old_path: uploaded_sftp_file.display().to_string(),
                new_path: existing_rename_target.display().to_string(),
                remote: true,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("已存在"), "{error}");
        assert_eq!(fs::read(&uploaded_sftp_file).unwrap(), sftp_payload);
        assert_eq!(
            fs::read(&existing_rename_target).unwrap(),
            b"existing rename target"
        );
        fs::remove_file(&existing_rename_target).unwrap();

        let renamed_sftp_file = sftp_nested.join("renamed.bin");
        rename_path_inner(
            &state,
            RenamePathRequest {
                session_id: Some(profile.id.clone()),
                old_path: uploaded_sftp_file.display().to_string(),
                new_path: renamed_sftp_file.display().to_string(),
                remote: true,
            },
        )
        .await
        .unwrap();
        chmod_path_inner(
            &state,
            ChmodPathRequest {
                session_id: Some(profile.id.clone()),
                path: renamed_sftp_file.display().to_string(),
                mode: 0o640,
                remote: true,
            },
        )
        .await
        .unwrap();
        let properties = file_properties_inner(
            &state,
            FilePropertiesRequest {
                session_id: Some(profile.id.clone()),
                path: renamed_sftp_file.display().to_string(),
                remote: true,
            },
        )
        .await
        .unwrap();
        assert!(properties.is_file);
        assert_eq!(properties.size, sftp_payload.len() as u64);
        assert_eq!(properties.permissions.unwrap() & 0o777, 0o640);

        let chmod_link = sftp_nested.join("chmod-link.bin");
        std::os::unix::fs::symlink(&renamed_sftp_file, &chmod_link).unwrap();
        let original_mode = fs::metadata(&renamed_sftp_file)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let error = chmod_path_inner(
            &state,
            ChmodPathRequest {
                session_id: Some(profile.id.clone()),
                path: chmod_link.display().to_string(),
                mode: 0o600,
                remote: true,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("符号链接"), "{error}");
        assert_eq!(
            fs::metadata(&renamed_sftp_file)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            original_mode
        );
        fs::remove_file(&chmod_link).unwrap();

        let copied_sftp_file = sftp_root.join("copied.bin");
        let copied_sftp_part =
            PathBuf::from(remote_resume_part_path(copied_sftp_file.to_str().unwrap()));
        fs::write(&copied_sftp_part, b"wrong-prefix").unwrap();
        let sftp_copy = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: format!("remote:{}", renamed_sftp_file.display()),
                destination: format!("remote:{}", copied_sftp_file.display()),
            },
        )
        .await
        .unwrap();
        let sftp_copy = wait_for_transfer_terminal_state(&state, &sftp_copy.id).await;
        assert_eq!(
            sftp_copy.status,
            TransferStatus::Completed,
            "SFTP remote copy failed: {:?}",
            sftp_copy.message
        );
        assert_eq!(sftp_copy.bytes_done, sftp_payload.len() as u64);
        assert_eq!(fs::read(&copied_sftp_file).unwrap(), sftp_payload);
        assert!(!copied_sftp_part.exists());

        let sftp_download_target = root.join("sftp-download-target.bin");
        let sftp_download_part = local_resume_part_path(&sftp_download_target);
        fs::write(&sftp_download_part, b"wrong-prefix").unwrap();
        let sftp_download = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: format!("remote:{}", renamed_sftp_file.display()),
                destination: sftp_download_target.display().to_string(),
            },
        )
        .await
        .unwrap();
        let sftp_download = wait_for_transfer_terminal_state(&state, &sftp_download.id).await;
        assert_eq!(
            sftp_download.status,
            TransferStatus::Completed,
            "SFTP download failed: {:?}",
            sftp_download.message
        );
        assert_eq!(sftp_download.bytes_done, sftp_payload.len() as u64);
        assert_eq!(fs::read(&sftp_download_target).unwrap(), sftp_payload);
        assert!(!sftp_download_part.exists());

        let sftp_empty = sftp_root.join("empty");
        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: sftp_empty.display().to_string(),
                remote: true,
            },
            FileOperation::CreateDirectory,
        )
        .await
        .unwrap();
        let recursive_download_root = root.join("recursive-download");
        fs::create_dir_all(&recursive_download_root).unwrap();
        let recursive_download = start_file_batch_inner(
            &state,
            StartFileBatchRequest {
                session_id: profile.id.clone(),
                paths: vec![sftp_root.display().to_string()],
                source_remote: true,
                destination: recursive_download_root.display().to_string(),
                destination_remote: false,
                conflict_policy: TransferConflictPolicy::Fail,
            },
        )
        .await
        .unwrap();
        assert_eq!(recursive_download.tasks.len(), 2);
        assert_eq!(recursive_download.directories_prepared, 3);
        assert_eq!(
            recursive_download.total_bytes,
            (sftp_payload.len() * 2) as u64
        );
        assert!(recursive_download.skipped.is_empty());
        for task in recursive_download.tasks {
            let task = wait_for_transfer_terminal_state(&state, &task.id).await;
            assert_eq!(
                task.status,
                TransferStatus::Completed,
                "recursive SFTP download failed: {:?}",
                task.message
            );
        }
        let downloaded_tree = recursive_download_root.join("sftp-workspace");
        assert_eq!(
            fs::read(downloaded_tree.join("copied.bin")).unwrap(),
            sftp_payload
        );
        assert_eq!(
            fs::read(downloaded_tree.join("nested/renamed.bin")).unwrap(),
            sftp_payload
        );
        assert!(downloaded_tree.join("empty").is_dir());

        fs::write(recursive_download_root.join("copied.bin"), b"existing").unwrap();
        let renamed_download = start_file_batch_inner(
            &state,
            StartFileBatchRequest {
                session_id: profile.id.clone(),
                paths: vec![copied_sftp_file.display().to_string()],
                source_remote: true,
                destination: recursive_download_root.display().to_string(),
                destination_remote: false,
                conflict_policy: TransferConflictPolicy::Rename,
            },
        )
        .await
        .unwrap();
        assert_eq!(renamed_download.tasks.len(), 1);
        let renamed_task =
            wait_for_transfer_terminal_state(&state, &renamed_download.tasks[0].id).await;
        assert_eq!(renamed_task.status, TransferStatus::Completed);
        assert_eq!(
            fs::read(recursive_download_root.join("copied (1).bin")).unwrap(),
            sftp_payload
        );

        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: Some(profile.id.clone()),
                path: sftp_root.display().to_string(),
                remote: true,
            },
            FileOperation::Delete,
        )
        .await
        .unwrap();
        assert!(!sftp_root.exists());

        let empty_sftp_source = root.join("empty-sftp-source.bin");
        let empty_sftp_target = root.join("empty-sftp-target.bin");
        fs::write(&empty_sftp_source, []).unwrap();
        let empty_sftp_upload = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: empty_sftp_source.display().to_string(),
                destination: format!("remote:{}", empty_sftp_target.display()),
            },
        )
        .await
        .unwrap();
        let empty_sftp_upload =
            wait_for_transfer_terminal_state(&state, &empty_sftp_upload.id).await;
        assert_eq!(
            empty_sftp_upload.status,
            TransferStatus::Completed,
            "empty SFTP upload failed: {:?}",
            empty_sftp_upload.message
        );
        assert_eq!(fs::metadata(&empty_sftp_target).unwrap().len(), 0);

        let upload_source = root.join("scp-upload-source.bin");
        let remote_file = root.join("scp-remote.bin");
        let download_target = root.join("scp-download-target.bin");
        let payload = b"PortMate OpenSSH SCP integration payload\n";
        fs::write(&upload_source, payload).unwrap();
        let remote_part = PathBuf::from(remote_resume_part_path(remote_file.to_str().unwrap()));
        fs::write(&remote_part, b"wrong-prefix").unwrap();
        let upload = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Scp,
                source: upload_source.display().to_string(),
                destination: format!("remote:{}", remote_file.display()),
            },
        )
        .await
        .unwrap();
        let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
        assert_eq!(
            upload.status,
            TransferStatus::Completed,
            "SCP upload failed: {:?}",
            upload.message
        );
        assert_eq!(upload.bytes_done, payload.len() as u64);
        assert_eq!(fs::read(&remote_file).unwrap(), payload);
        assert!(!remote_part.exists());

        let download_part = local_resume_part_path(&download_target);
        fs::write(&download_part, &payload[..15]).unwrap();
        let download = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Scp,
                source: format!("remote:{}", remote_file.display()),
                destination: download_target.display().to_string(),
            },
        )
        .await
        .unwrap();
        let download = wait_for_transfer_terminal_state(&state, &download.id).await;
        assert_eq!(
            download.status,
            TransferStatus::Completed,
            "SCP download failed: {:?}",
            download.message
        );
        assert_eq!(download.bytes_done, payload.len() as u64);
        assert_eq!(fs::read(&download_target).unwrap(), payload);
        assert!(!download_part.exists());

        let denied_target = format!("/proc/portmate-transfer-denied-{}.bin", Uuid::new_v4());
        for protocol in [TransferProtocol::Sftp, TransferProtocol::Scp] {
            let failed_upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: protocol.clone(),
                    source: upload_source.display().to_string(),
                    destination: format!("remote:{denied_target}"),
                },
            )
            .await
            .unwrap();
            let failed_upload = wait_for_transfer_terminal_state(&state, &failed_upload.id).await;
            assert_eq!(
                failed_upload.status,
                TransferStatus::Failed,
                "{protocol:?} server-side write failure was not reported: {:?}",
                failed_upload.message
            );
            let message = failed_upload.message.unwrap_or_default();
            assert!(
                message.contains("SFTP") || message.contains("SCP"),
                "{protocol:?} failure lacked protocol context: {message}"
            );
            assert!(
                !state
                    .transfer_cancellations
                    .lock()
                    .unwrap()
                    .contains_key(&failed_upload.id),
                "{protocol:?} failed transfer retained its cancellation handle"
            );
        }

        {
            let mut store = state.store.lock().unwrap();
            let mut limited = store.profile(&profile.id).unwrap();
            limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
            store.upsert_profile(limited);
        }
        let cancel_source = root.join("sftp-cancel-source.bin");
        let cancel_remote = root.join("sftp-cancel-remote.bin");
        let cancel_remote_part =
            PathBuf::from(remote_resume_part_path(cancel_remote.to_str().unwrap()));
        // Keep enough limited payload remaining that a heavily loaded parallel test
        // runner cannot finish the transfer before the cancellation poll is scheduled.
        let cancel_payload = (0..2 * 1024 * 1024)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        fs::write(&cancel_source, &cancel_payload).unwrap();
        let cancelled_upload = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: cancel_source.display().to_string(),
                destination: format!("remote:{}", cancel_remote.display()),
            },
        )
        .await
        .unwrap();
        wait_for_transfer_progress(&state, &cancelled_upload.id, "limited SFTP upload").await;
        let cancelling = cancel_transfer_inner(&state, &cancelled_upload.id).unwrap();
        assert_eq!(cancelling.status, TransferStatus::Cancelled);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !state
                    .transfer_cancellations
                    .lock()
                    .unwrap()
                    .contains_key(&cancelled_upload.id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cancelled SFTP worker did not stop");
        let cancelled = state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&cancelled_upload.id)
            .unwrap();
        assert_eq!(cancelled.status, TransferStatus::Cancelled);
        assert!(!cancel_remote.exists());
        let partial_size = fs::metadata(&cancel_remote_part).unwrap().len();
        assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

        {
            let mut store = state.store.lock().unwrap();
            let mut unlimited = store.profile(&profile.id).unwrap();
            unlimited.transfer.rate_limit_bytes_per_second = None;
            store.upsert_profile(unlimited);
        }
        let retried = retry_transfer_inner(&state, &cancelled_upload.id)
            .await
            .unwrap();
        let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
        assert_eq!(
            retried.status,
            TransferStatus::Completed,
            "SFTP retry failed: {:?}",
            retried.message
        );
        assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
        assert_eq!(fs::read(&cancel_remote).unwrap(), cancel_payload);
        assert!(!cancel_remote_part.exists());

        {
            let mut store = state.store.lock().unwrap();
            let mut limited = store.profile(&profile.id).unwrap();
            limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
            store.upsert_profile(limited);
        }
        let scp_cancel_source = root.join("scp-cancel-source.bin");
        let scp_cancel_remote = root.join("scp-cancel-remote.bin");
        let scp_cancel_remote_part =
            PathBuf::from(remote_resume_part_path(scp_cancel_remote.to_str().unwrap()));
        fs::write(&scp_cancel_source, &cancel_payload).unwrap();
        let cancelled_scp_upload = start_transfer_inner(
            &state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Scp,
                source: scp_cancel_source.display().to_string(),
                destination: format!("remote:{}", scp_cancel_remote.display()),
            },
        )
        .await
        .unwrap();
        wait_for_transfer_progress(&state, &cancelled_scp_upload.id, "limited SCP upload").await;
        let cancelling = cancel_transfer_inner(&state, &cancelled_scp_upload.id).unwrap();
        assert_eq!(cancelling.status, TransferStatus::Cancelled);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !state
                    .transfer_cancellations
                    .lock()
                    .unwrap()
                    .contains_key(&cancelled_scp_upload.id)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("cancelled SCP worker did not stop");
        let cancelled = state
            .store
            .lock()
            .unwrap()
            .transfer_by_id(&cancelled_scp_upload.id)
            .unwrap();
        assert_eq!(cancelled.status, TransferStatus::Cancelled);
        assert!(!scp_cancel_remote.exists());
        let partial_size = fs::metadata(&scp_cancel_remote_part).unwrap().len();
        assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

        {
            let mut store = state.store.lock().unwrap();
            let mut unlimited = store.profile(&profile.id).unwrap();
            unlimited.transfer.rate_limit_bytes_per_second = None;
            store.upsert_profile(unlimited);
        }
        let retried = retry_transfer_inner(&state, &cancelled_scp_upload.id)
            .await
            .unwrap();
        let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
        assert_eq!(
            retried.status,
            TransferStatus::Completed,
            "SCP retry failed: {:?}",
            retried.message
        );
        assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
        assert_eq!(fs::read(&scp_cancel_remote).unwrap(), cancel_payload);
        assert!(!scp_cancel_remote_part.exists());

        for (label, protocol) in [
            ("sftp", TransferProtocol::Sftp),
            ("scp", TransferProtocol::Scp),
        ] {
            {
                let mut store = state.store.lock().unwrap();
                let mut limited = store.profile(&profile.id).unwrap();
                limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
                store.upsert_profile(limited);
            }
            let disconnect_remote = root.join(format!("{label}-disconnect-remote.bin"));
            let disconnect_remote_part =
                PathBuf::from(remote_resume_part_path(disconnect_remote.to_str().unwrap()));
            let interrupted_upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: protocol.clone(),
                    source: cancel_source.display().to_string(),
                    destination: format!("remote:{}", disconnect_remote.display()),
                },
            )
            .await
            .unwrap();
            wait_for_transfer_progress(
                &state,
                &interrupted_upload.id,
                &format!("limited {label} upload"),
            )
            .await;

            let disconnected = close_session_inner(&state, profile.id.clone())
                .await
                .unwrap();
            assert_eq!(disconnected.runtime.status, SessionStatus::Disconnected);
            let interrupted =
                wait_for_transfer_terminal_state(&state, &interrupted_upload.id).await;
            assert_eq!(
                interrupted.status,
                TransferStatus::Failed,
                "{protocol:?} SSH disconnect was not reported as a failure: {:?}",
                interrupted.message
            );
            assert!(
                !state
                    .transfer_cancellations
                    .lock()
                    .unwrap()
                    .contains_key(&interrupted.id),
                "{protocol:?} disconnected transfer retained its cancellation handle"
            );
            assert!(!disconnect_remote.exists());
            let partial_size = fs::metadata(&disconnect_remote_part).unwrap().len();
            assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

            let reopened = open_ssh_session(&state, profile.clone(), None, None)
                .await
                .unwrap();
            assert_eq!(reopened.runtime.status, SessionStatus::Connected);
            {
                let mut store = state.store.lock().unwrap();
                let mut unlimited = store.profile(&profile.id).unwrap();
                unlimited.transfer.rate_limit_bytes_per_second = None;
                store.upsert_profile(unlimited);
            }
            let retried = retry_transfer_inner(&state, &interrupted_upload.id)
                .await
                .unwrap();
            let retried = wait_for_transfer_terminal_state(&state, &retried.id).await;
            assert_eq!(
                retried.status,
                TransferStatus::Completed,
                "{protocol:?} retry after reconnect failed: {:?}",
                retried.message
            );
            assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
            assert_eq!(fs::read(&disconnect_remote).unwrap(), cancel_payload);
            assert!(!disconnect_remote_part.exists());
        }

        if modem_tools_available {
            let zmodem_source = root.join("zmodem-upload-source.bin");
            let zmodem_remote = root.join("zmodem-remote.bin");
            let zmodem_download = root.join("zmodem-download-target.bin");
            let zmodem_payload = b"PortMate ZModem\x00binary\xffpayload\n";
            fs::write(&zmodem_source, zmodem_payload).unwrap();

            let upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Zmodem,
                    source: zmodem_source.display().to_string(),
                    destination: format!("remote:{}", zmodem_remote.display()),
                },
            )
            .await
            .unwrap();
            let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
            assert_eq!(
                upload.status,
                TransferStatus::Completed,
                "ZModem upload failed: {:?}",
                upload.message
            );
            assert_eq!(upload.bytes_done, zmodem_payload.len() as u64);
            assert_eq!(fs::read(&zmodem_remote).unwrap(), zmodem_payload);
            assert!(
                !PathBuf::from(remote_resume_part_path(zmodem_remote.to_str().unwrap())).exists()
            );

            let download = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Zmodem,
                    source: format!("remote:{}", zmodem_remote.display()),
                    destination: zmodem_download.display().to_string(),
                },
            )
            .await
            .unwrap();
            let download = wait_for_transfer_terminal_state(&state, &download.id).await;
            assert_eq!(
                download.status,
                TransferStatus::Completed,
                "ZModem download failed: {:?}",
                download.message
            );
            assert_eq!(download.bytes_done, zmodem_payload.len() as u64);
            assert_eq!(fs::read(&zmodem_download).unwrap(), zmodem_payload);

            let xmodem_source = root.join("xmodem-upload-source.bin");
            let xmodem_remote = root.join("xmodem-remote.bin");
            let xmodem_download = root.join("xmodem-download-target.bin");
            let xmodem_payload = b"PortMate XModem integration payload\n".repeat(8);
            assert!(xmodem_payload.len() > XMODEM_BLOCK_SIZE);
            fs::write(&xmodem_source, &xmodem_payload).unwrap();
            let upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Xmodem,
                    source: xmodem_source.display().to_string(),
                    destination: format!("remote:{}", xmodem_remote.display()),
                },
            )
            .await
            .unwrap();
            let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
            let xmodem_screen = state
                .store
                .lock()
                .unwrap()
                .screen(&profile.id)
                .unwrap_or_default();
            assert_eq!(
                upload.status,
                TransferStatus::Completed,
                "XModem upload failed: {:?}; screen={xmodem_screen:?}",
                upload.message,
            );
            assert_eq!(upload.bytes_done, xmodem_payload.len() as u64);
            assert_eq!(fs::read(&xmodem_remote).unwrap(), xmodem_payload);
            assert!(
                !PathBuf::from(remote_resume_part_path(xmodem_remote.to_str().unwrap())).exists()
            );

            let download = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Xmodem,
                    source: format!("remote:{}", xmodem_remote.display()),
                    destination: xmodem_download.display().to_string(),
                },
            )
            .await
            .unwrap();
            let download = wait_for_transfer_terminal_state(&state, &download.id).await;
            assert_eq!(
                download.status,
                TransferStatus::Completed,
                "XModem download failed: {:?}",
                download.message
            );
            assert_eq!(download.bytes_done, xmodem_payload.len() as u64);
            assert_eq!(fs::read(&xmodem_download).unwrap(), xmodem_payload);

            let ymodem_source = root.join("ymodem-upload-source.bin");
            let ymodem_remote = root.join("ymodem-remote.bin");
            let ymodem_download = root.join("ymodem-download-target.bin");
            let ymodem_payload = b"PortMate YModem\x00binary\xffpayload\n".repeat(40);
            assert!(ymodem_payload.len() > YMODEM_BLOCK_SIZE);
            fs::write(&ymodem_source, &ymodem_payload).unwrap();
            let upload = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Ymodem,
                    source: ymodem_source.display().to_string(),
                    destination: format!("remote:{}", ymodem_remote.display()),
                },
            )
            .await
            .unwrap();
            let upload = wait_for_transfer_terminal_state(&state, &upload.id).await;
            let ymodem_screen = state
                .store
                .lock()
                .unwrap()
                .screen(&profile.id)
                .unwrap_or_default();
            assert_eq!(
                upload.status,
                TransferStatus::Completed,
                "YModem upload failed: {:?}; screen={ymodem_screen:?}",
                upload.message,
            );
            assert_eq!(upload.bytes_done, ymodem_payload.len() as u64);
            assert_eq!(fs::read(&ymodem_remote).unwrap(), ymodem_payload);
            assert!(
                !PathBuf::from(remote_resume_part_path(ymodem_remote.to_str().unwrap())).exists()
            );

            let download = start_transfer_inner(
                &state,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol: TransferProtocol::Ymodem,
                    source: format!("remote:{}", ymodem_remote.display()),
                    destination: ymodem_download.display().to_string(),
                },
            )
            .await
            .unwrap();
            let download = wait_for_transfer_terminal_state(&state, &download.id).await;
            assert_eq!(
                download.status,
                TransferStatus::Completed,
                "YModem download failed: {:?}",
                download.message
            );
            assert_eq!(download.bytes_done, ymodem_payload.len() as u64);
            assert_eq!(fs::read(&ymodem_download).unwrap(), ymodem_payload);
        } else {
            eprintln!("skipping modem OpenSSH coverage: lrzsz tools are not installed");
        }

        let echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let echo_address = echo_listener.local_addr().unwrap();
        drop(echo_listener);
        let tunnel = create_tunnel_inner(
            &state,
            CreateTunnelRequest {
                session_id: profile.id.clone(),
                mode: TunnelMode::Local,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
                target_host: "127.0.0.1".to_string(),
                target_port: echo_address.port(),
                label: None,
            },
        )
        .await
        .unwrap();
        assert_ne!(tunnel.bind_port, 0);

        let mut failed_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
            .await
            .unwrap();
        failed_client.write_all(b"ping").await.unwrap();
        let mut closed_byte = [0_u8; 1];
        let read =
            tokio::time::timeout(Duration::from_secs(2), failed_client.read(&mut closed_byte))
                .await
                .expect("failed local tunnel client did not close");
        assert_tunnel_client_closed(read, "failed local tunnel client");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = list_tunnels_inner(&state, Some(&profile.id))
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == tunnel.id)
                    .unwrap();
                if status.active_connections == 0 && status.total_connections == 1 {
                    assert!(status
                        .last_error
                        .as_deref()
                        .is_some_and(|error| error.contains("direct-tcpip open failed")));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("local tunnel failure metrics did not settle");

        let echo_listener = TcpListener::bind(echo_address).await.unwrap();
        let echo = tokio::spawn(async move {
            let (mut socket, _) = echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            socket.write_all(b"pong").await.unwrap();
        });
        let mut tunnel_client = TcpStream::connect(("127.0.0.1", tunnel.bind_port))
            .await
            .unwrap();
        tunnel_client.write_all(b"ping").await.unwrap();
        let mut response = [0_u8; 4];
        tunnel_client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"pong");
        drop(tunnel_client);
        echo.await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = list_tunnels_inner(&state, Some(&profile.id))
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == tunnel.id)
                    .unwrap();
                if status.active_connections == 0 && status.total_connections == 2 {
                    assert_eq!(status.tcp_to_ssh_bytes, 4);
                    assert_eq!(status.ssh_to_tcp_bytes, 4);
                    assert!(status.last_error.is_none());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("local tunnel metrics did not settle");
        let stopped = stop_tunnel_inner(&state, &tunnel.id).await.unwrap();
        assert!(!stopped.spec.enabled);

        let dynamic_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let dynamic_echo_address = dynamic_echo_listener.local_addr().unwrap();
        drop(dynamic_echo_listener);
        let dynamic_tunnel = create_tunnel_inner(
            &state,
            CreateTunnelRequest {
                session_id: profile.id.clone(),
                mode: TunnelMode::Dynamic,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
                target_host: String::new(),
                target_port: 0,
                label: None,
            },
        )
        .await
        .unwrap();
        assert_ne!(dynamic_tunnel.bind_port, 0);

        let [port_high, port_low] = dynamic_echo_address.port().to_be_bytes();
        let mut failed_socks_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
            .await
            .unwrap();
        failed_socks_client.write_all(&[5, 1, 0]).await.unwrap();
        let mut failed_method = [0_u8; 2];
        failed_socks_client
            .read_exact(&mut failed_method)
            .await
            .unwrap();
        assert_eq!(failed_method, [5, 0]);
        failed_socks_client
            .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
            .await
            .unwrap();
        let mut failed_socks_reply = [0_u8; 10];
        failed_socks_client
            .read_exact(&mut failed_socks_reply)
            .await
            .unwrap();
        assert_eq!(failed_socks_reply, super::socks5_reply(5));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = list_tunnels_inner(&state, Some(&profile.id))
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == dynamic_tunnel.id)
                    .unwrap();
                if status.active_connections == 0 && status.total_connections == 1 {
                    assert!(status
                        .last_error
                        .as_deref()
                        .is_some_and(|error| error.contains("dynamic direct-tcpip open failed")));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("dynamic tunnel failure metrics did not settle");

        let dynamic_echo_listener = TcpListener::bind(dynamic_echo_address).await.unwrap();
        let dynamic_echo = tokio::spawn(async move {
            let (mut socket, _) = dynamic_echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            socket.write_all(b"pong").await.unwrap();
        });
        let mut socks_client = TcpStream::connect(("127.0.0.1", dynamic_tunnel.bind_port))
            .await
            .unwrap();
        socks_client.write_all(&[5, 1, 0]).await.unwrap();
        let mut method = [0_u8; 2];
        socks_client.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);
        socks_client
            .write_all(&[5, 1, 0, 1, 127, 0, 0, 1, port_high, port_low])
            .await
            .unwrap();
        let mut socks_reply = [0_u8; 10];
        socks_client.read_exact(&mut socks_reply).await.unwrap();
        assert_eq!(socks_reply, super::socks5_reply(0));
        socks_client.write_all(b"ping").await.unwrap();
        let mut socks_response = [0_u8; 4];
        socks_client.read_exact(&mut socks_response).await.unwrap();
        assert_eq!(&socks_response, b"pong");
        drop(socks_client);
        dynamic_echo.await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = list_tunnels_inner(&state, Some(&profile.id))
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == dynamic_tunnel.id)
                    .unwrap();
                if status.active_connections == 0 && status.total_connections == 2 {
                    assert_eq!(status.tcp_to_ssh_bytes, 4);
                    assert_eq!(status.ssh_to_tcp_bytes, 4);
                    assert!(status.last_error.is_none());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("dynamic tunnel metrics did not settle");
        let stopped = stop_tunnel_inner(&state, &dynamic_tunnel.id).await.unwrap();
        assert!(!stopped.spec.enabled);

        let remote_echo_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let remote_echo_address = remote_echo_listener.local_addr().unwrap();
        drop(remote_echo_listener);
        let remote_tunnel = create_tunnel_inner(
            &state,
            CreateTunnelRequest {
                session_id: profile.id.clone(),
                mode: TunnelMode::Remote,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
                target_host: "127.0.0.1".to_string(),
                target_port: remote_echo_address.port(),
                label: None,
            },
        )
        .await
        .unwrap();
        assert_ne!(remote_tunnel.bind_port, 0);
        assert!(remote_tunnel
            .label
            .contains(&remote_tunnel.bind_port.to_string()));

        let mut failed_remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
            .await
            .unwrap();
        failed_remote_client.write_all(b"ping").await.unwrap();
        let mut closed_byte = [0_u8; 1];
        let read = tokio::time::timeout(
            Duration::from_secs(2),
            failed_remote_client.read(&mut closed_byte),
        )
        .await
        .expect("failed remote tunnel client did not close");
        assert_tunnel_client_closed(read, "failed remote tunnel client");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = list_tunnels_inner(&state, Some(&profile.id))
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == remote_tunnel.id)
                    .unwrap();
                if status.active_connections == 0 && status.total_connections == 1 {
                    assert!(status
                        .last_error
                        .as_deref()
                        .is_some_and(|error| error.contains("target connect failed")));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("remote tunnel failure metrics did not settle");

        let remote_echo_listener = TcpListener::bind(remote_echo_address).await.unwrap();
        let remote_echo = tokio::spawn(async move {
            let (mut socket, _) = remote_echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            socket.write_all(b"pong").await.unwrap();
        });
        let mut remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
            .await
            .unwrap();
        remote_client.write_all(b"ping").await.unwrap();
        let mut remote_response = [0_u8; 4];
        remote_client
            .read_exact(&mut remote_response)
            .await
            .unwrap();
        assert_eq!(&remote_response, b"pong");
        drop(remote_client);
        remote_echo.await.unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = list_tunnels_inner(&state, Some(&profile.id))
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == remote_tunnel.id)
                    .unwrap();
                if status.active_connections == 0 && status.total_connections == 2 {
                    assert_eq!(status.tcp_to_ssh_bytes, 4);
                    assert_eq!(status.ssh_to_tcp_bytes, 4);
                    assert!(status.last_error.is_none());
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("remote tunnel metrics did not settle");

        let (remote_health_handle, remote_forward_routes) = {
            let connections = state.ssh.lock().unwrap();
            let runtime = connections.get(&profile.id).unwrap();
            (
                Arc::clone(&runtime.handle),
                Arc::clone(&runtime.remote_forwards),
            )
        };
        {
            let handle = remote_health_handle.lock().await;
            handle
                .russh_compat()
                .unwrap()
                .cancel_tcpip_forward(
                    remote_tunnel.bind_host.clone(),
                    u32::from(remote_tunnel.bind_port),
                )
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port)).await {
                    Err(_) => break,
                    Ok(stream) => drop(stream),
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("server-side remote forward cancellation did not close the listener");

        assert_eq!(
            check_remote_tunnel_health(&state, &remote_tunnel.id)
                .await
                .unwrap(),
            RemoteTunnelHealth::Restored
        );
        assert!(state
            .store
            .lock()
            .unwrap()
            .tail_log(&profile.id, 100)
            .iter()
            .any(|event| event.text.as_deref().is_some_and(|text| {
                text.contains(&remote_tunnel.id)
                    && text.contains("listener was missing and has been restored")
            })));

        let restored_remote_echo_listener = TcpListener::bind(remote_echo_address).await.unwrap();
        let restored_remote_echo = tokio::spawn(async move {
            let (mut socket, _) = restored_remote_echo_listener.accept().await.unwrap();
            let mut request = [0_u8; 4];
            socket.read_exact(&mut request).await.unwrap();
            assert_eq!(&request, b"ping");
            socket.write_all(b"pong").await.unwrap();
        });
        let mut restored_remote_client = TcpStream::connect(("127.0.0.1", remote_tunnel.bind_port))
            .await
            .unwrap();
        restored_remote_client.write_all(b"ping").await.unwrap();
        let mut restored_remote_response = [0_u8; 4];
        restored_remote_client
            .read_exact(&mut restored_remote_response)
            .await
            .unwrap();
        assert_eq!(&restored_remote_response, b"pong");
        drop(restored_remote_client);
        restored_remote_echo.await.unwrap();

        {
            let handle = remote_health_handle.lock().await;
            handle
                .russh_compat()
                .unwrap()
                .cancel_tcpip_forward(
                    remote_tunnel.bind_host.clone(),
                    u32::from(remote_tunnel.bind_port),
                )
                .await
                .unwrap();
        }
        let stopped = stop_tunnel_inner(&state, &remote_tunnel.id).await.unwrap();
        assert!(!stopped.spec.enabled);
        assert!(stopped
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("remote SSH tunnel cancel failed")));
        assert!(list_tunnels_inner(&state, Some(&profile.id))
            .unwrap()
            .iter()
            .all(|status| status.spec.id != remote_tunnel.id));
        {
            let routes = remote_forward_routes.lock().unwrap();
            assert!(!routes.contains_key(&remote_forward_key(
                &remote_tunnel.bind_host,
                remote_tunnel.bind_port,
            )));
            assert!(!routes.contains_key(&remote_forward_port_key(remote_tunnel.bind_port)));
        }
        let saved_profile = state.store.lock().unwrap().profile(&profile.id).unwrap();
        let saved_remote_tunnel = match saved_profile.connection {
            ConnectionConfig::Ssh(ssh) => ssh
                .tunnels
                .into_iter()
                .find(|tunnel| tunnel.id == remote_tunnel.id)
                .unwrap(),
            _ => panic!("expected SSH profile"),
        };
        assert!(!saved_remote_tunnel.enabled);

        let reconnect_tunnel = create_tunnel_inner(
            &state,
            CreateTunnelRequest {
                session_id: profile.id.clone(),
                mode: TunnelMode::Local,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
                target_host: "127.0.0.1".to_string(),
                target_port: port,
                label: Some("reconnect tunnel".to_string()),
            },
        )
        .await
        .unwrap();
        let reconnect_remote_tunnel = create_tunnel_inner(
            &state,
            CreateTunnelRequest {
                session_id: profile.id.clone(),
                mode: TunnelMode::Remote,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
                target_host: "127.0.0.1".to_string(),
                target_port: port,
                label: Some("reconnect remote tunnel".to_string()),
            },
        )
        .await
        .unwrap();
        let conflict_listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let conflict_port = conflict_listener.local_addr().unwrap().port();
        let conflict_tunnel = TunnelSpec {
            id: "reconnect-conflict".to_string(),
            label: "occupied reconnect tunnel".to_string(),
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: conflict_port,
            target_host: "127.0.0.1".to_string(),
            target_port: port,
            enabled: true,
        };
        {
            let mut store = state.store.lock().unwrap();
            let mut saved_profile = store.profile(&profile.id).unwrap();
            match &mut saved_profile.connection {
                ConnectionConfig::Ssh(ssh) => {
                    ssh.tunnels.push(conflict_tunnel.clone());
                    ssh.reconnect_delay_ms = 5_000;
                }
                _ => panic!("expected SSH profile"),
            }
            store.upsert_profile(saved_profile);
            save_store(&state.store_path, &store).unwrap();
        }
        let (previous_runtime_id, reconnect_handle) = {
            let connections = state.ssh.lock().unwrap();
            let runtime = connections.get(&profile.id).unwrap();
            (runtime.runtime_id.clone(), Arc::clone(&runtime.handle))
        };
        {
            let handle = reconnect_handle.lock().await;
            handle
                .disconnect("PortMate tunnel reconnect integration test")
                .await
                .unwrap();
        }

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let reconnecting = state.store.lock().unwrap().runtimes.iter().any(|runtime| {
                    runtime.session_id == profile.id
                        && runtime.status == SessionStatus::Reconnecting
                });
                if reconnecting {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("SSH runtime did not enter reconnecting state");
        {
            let mut store = state.store.lock().unwrap();
            let mut updated = store.profile(&profile.id).unwrap();
            match &mut updated.connection {
                ConnectionConfig::Ssh(ssh) => ssh.reconnect_delay_ms = 100,
                _ => panic!("expected SSH profile"),
            }
            store.upsert_profile(updated);
            save_store(&state.store_path, &store).unwrap();
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let runtime_replaced = state
                    .ssh
                    .lock()
                    .unwrap()
                    .get(&profile.id)
                    .is_some_and(|runtime| runtime.runtime_id != previous_runtime_id);
                if runtime_replaced {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("SSH reconnect did not adopt the shortened profile delay");

        let restored = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let runtime_replaced = state
                    .ssh
                    .lock()
                    .unwrap()
                    .get(&profile.id)
                    .is_some_and(|runtime| runtime.runtime_id != previous_runtime_id);
                let restored = list_tunnels_inner(&state, Some(&profile.id))
                    .unwrap()
                    .into_iter()
                    .find(|status| status.spec.id == reconnect_tunnel.id);
                let remote_restored = list_tunnels_inner(&state, Some(&profile.id))
                    .unwrap()
                    .iter()
                    .any(|status| status.spec.id == reconnect_remote_tunnel.id);
                let conflict_reported = state
                    .store
                    .lock()
                    .unwrap()
                    .tail_log(&profile.id, 200)
                    .iter()
                    .any(|event| {
                        event.text.as_deref().is_some_and(|text| {
                            text.contains("failed to restore SSH tunnel reconnect-conflict")
                                && text.contains("SSH tunnel bind failed")
                        })
                    });
                if runtime_replaced && remote_restored && conflict_reported {
                    if let Some(restored) = restored {
                        break restored;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| {
            let statuses = list_tunnels_inner(&state, Some(&profile.id)).unwrap();
            let events = state.store.lock().unwrap().tail_log(&profile.id, 20);
            panic!(
                "SSH reconnect did not restore the tunnel runtime; statuses={statuses:?}; recent events={events:?}"
            )
        });
        assert_eq!(restored.spec.id, reconnect_tunnel.id);
        assert_eq!(restored.spec.label, reconnect_tunnel.label);
        assert_eq!(restored.spec.bind_port, reconnect_tunnel.bind_port);
        let restored_tunnels = list_tunnels_inner(&state, Some(&profile.id)).unwrap();
        let restored_remote = restored_tunnels
            .iter()
            .find(|status| status.spec.id == reconnect_remote_tunnel.id)
            .unwrap();
        assert_eq!(restored_remote.spec.label, reconnect_remote_tunnel.label);
        assert_eq!(
            restored_remote.spec.bind_port,
            reconnect_remote_tunnel.bind_port
        );
        assert!(restored_tunnels
            .iter()
            .all(|status| status.spec.id != conflict_tunnel.id));

        let saved_profile = state.store.lock().unwrap().profile(&profile.id).unwrap();
        let saved_tunnels = match saved_profile.connection {
            ConnectionConfig::Ssh(ssh) => ssh.tunnels,
            _ => panic!("expected SSH profile"),
        };
        assert!(saved_tunnels
            .iter()
            .any(|tunnel| tunnel.id == conflict_tunnel.id && tunnel.enabled));
        assert!(state
            .store
            .lock()
            .unwrap()
            .tail_log(&profile.id, 200)
            .iter()
            .any(|event| event.text.as_deref().is_some_and(|text| {
                text.contains("failed to restore SSH tunnel reconnect-conflict")
                    && text.contains("SSH tunnel bind failed")
            })));
        let screen = state.store.lock().unwrap().screen(&profile.id).unwrap();
        assert!(screen.contains("reconnecting in 5000ms"), "{screen}");

        let mut restored_client = TcpStream::connect(("127.0.0.1", reconnect_tunnel.bind_port))
            .await
            .unwrap();
        let mut ssh_banner = [0_u8; 4];
        tokio::time::timeout(
            Duration::from_secs(2),
            restored_client.read_exact(&mut ssh_banner),
        )
        .await
        .expect("restored tunnel did not receive an SSH banner")
        .unwrap();
        assert_eq!(&ssh_banner, b"SSH-");
        drop(restored_client);

        let mut restored_remote_client =
            TcpStream::connect(("127.0.0.1", reconnect_remote_tunnel.bind_port))
                .await
                .unwrap();
        let mut remote_ssh_banner = [0_u8; 4];
        tokio::time::timeout(
            Duration::from_secs(2),
            restored_remote_client.read_exact(&mut remote_ssh_banner),
        )
        .await
        .expect("restored remote tunnel did not receive an SSH banner")
        .unwrap();
        assert_eq!(&remote_ssh_banner, b"SSH-");
        drop(restored_remote_client);
        drop(conflict_listener);
        let stopped = stop_tunnel_inner(&state, &reconnect_tunnel.id)
            .await
            .unwrap();
        assert!(!stopped.spec.enabled);
        let stopped = stop_tunnel_inner(&state, &reconnect_remote_tunnel.id)
            .await
            .unwrap();
        assert!(!stopped.spec.enabled);

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

        let matching_ref = IdentityRef {
            id: "accepted-agent-key".to_string(),
            label: accepted_comment.clone(),
            source: IdentitySource::Agent,
            fingerprint_sha256: Some(accepted_fingerprint),
            path: Some(accepted_comment.clone()),
            secret_ref: None,
        };
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

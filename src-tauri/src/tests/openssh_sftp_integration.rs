use super::*;

#[cfg(unix)]
pub(super) async fn exercise_openssh_sftp_operations(
    state: &AppState,
    profile: &SessionProfile,
    root: &std::path::Path,
) {
    use std::os::unix::fs::PermissionsExt;

    let entries = list_files_inner(
        state,
        ListFilesRequest {
            session_id: Some(profile.id.clone()),
            path: ".".to_string(),
            remote: true,
        },
    )
    .await
    .unwrap();
    assert!(entries.iter().all(|entry| !entry.name.is_empty()));

    let default_drop_name = format!("portmate-default-drop-{}.txt", Uuid::new_v4());
    let default_drop_source = root.join(&default_drop_name);
    fs::write(&default_drop_source, b"default remote directory").unwrap();
    let auxiliary = ssh_auxiliary_lease(state, &profile.id).unwrap();
    let sftp = auxiliary.sftp().await.unwrap();
    let default_remote_directory = resolve_remote_drop_destination(&sftp, ".").await.unwrap();
    assert_eq!(
        default_remote_directory,
        sftp.canonicalize(".").await.unwrap(),
        "the default SFTP target must resolve to the server current directory"
    );
    drop(sftp);
    drop(auxiliary);
    let default_drop = start_external_drop_inner(
        state,
        StartExternalDropRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
            paths: vec![default_drop_source.display().to_string()],
            destination: ".".to_string(),
            remote: true,
            conflict_policy: TransferConflictPolicy::Fail,
        },
    )
    .await
    .unwrap();
    assert_eq!(default_drop.tasks.len(), 1);
    let default_drop_task =
        wait_for_transfer_terminal_state(state, &default_drop.tasks[0].id).await;
    assert_eq!(
        default_drop_task.status,
        TransferStatus::Completed,
        "default-directory SCP drop failed: {:?}",
        default_drop_task.message
    );
    let default_remote_path = remote_join_path(&default_remote_directory, &default_drop_name);
    let default_entries = list_files_inner(
        state,
        ListFilesRequest {
            session_id: Some(profile.id.clone()),
            path: default_remote_directory,
            remote: true,
        },
    )
    .await
    .unwrap();
    assert!(default_entries
        .iter()
        .any(|entry| entry.path == default_remote_path));
    file_operation_inner(
        state,
        FileOperationRequest {
            session_id: Some(profile.id.clone()),
            path: default_remote_path,
            remote: true,
        },
        FileOperation::Delete,
    )
    .await
    .unwrap();

    let sftp_root = root.join("sftp-workspace");
    let sftp_nested = sftp_root.join("nested");
    file_operation_inner(
        state,
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
    fs::write(sftp_nested.join("limit-alpha"), b"alpha").unwrap();
    fs::write(sftp_nested.join("limit-beta"), b"beta").unwrap();
    let auxiliary = ssh_auxiliary_lease(state, &profile.id).unwrap();
    let bounded_sftp = auxiliary.sftp().await.unwrap();
    let SftpBackendSession::Russh(session) = &*bounded_sftp else {
        panic!("expected russh SFTP backend");
    };
    let error = match session
        .read_dir_bounded(sftp_nested.display().to_string(), 1)
        .await
    {
        Ok(_) => panic!("russh SFTP returned a silently truncated directory"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("directory entry count exceeds 1"));
    drop(bounded_sftp);
    drop(auxiliary);
    fs::remove_file(sftp_nested.join("limit-alpha")).unwrap();
    fs::remove_file(sftp_nested.join("limit-beta")).unwrap();

    let sftp_new_file = root.join("sftp-created-file.txt");
    file_operation_inner(
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
        StartExternalDropRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
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
        let task = wait_for_transfer_terminal_state(state, &task.id).await;
        assert_eq!(
            task.status,
            TransferStatus::Completed,
            "recursive external SCP drop failed: {:?}",
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
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: sftp_source.display().to_string(),
            destination: format!("remote:{}/", sftp_nested.display()),
        },
    )
    .await
    .unwrap();
    let sftp_upload = wait_for_transfer_terminal_state(state, &sftp_upload.id).await;
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
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
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: format!("remote:{}", renamed_sftp_file.display()),
            destination: format!("remote:{}", copied_sftp_file.display()),
        },
    )
    .await
    .unwrap();
    let sftp_copy = wait_for_transfer_terminal_state(state, &sftp_copy.id).await;
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
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: format!("remote:{}", renamed_sftp_file.display()),
            destination: sftp_download_target.display().to_string(),
        },
    )
    .await
    .unwrap();
    let sftp_download = wait_for_transfer_terminal_state(state, &sftp_download.id).await;
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
        state,
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
        state,
        StartFileBatchRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
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
        let task = wait_for_transfer_terminal_state(state, &task.id).await;
        assert_eq!(
            task.status,
            TransferStatus::Completed,
            "recursive SCP download failed: {:?}",
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
        state,
        StartFileBatchRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
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
    let renamed_task = wait_for_transfer_terminal_state(state, &renamed_download.tasks[0].id).await;
    assert_eq!(renamed_task.status, TransferStatus::Completed);
    assert_eq!(
        fs::read(recursive_download_root.join("copied (1).bin")).unwrap(),
        sftp_payload
    );

    file_operation_inner(
        state,
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
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: empty_sftp_source.display().to_string(),
            destination: format!("remote:{}", empty_sftp_target.display()),
        },
    )
    .await
    .unwrap();
    let empty_sftp_upload = wait_for_transfer_terminal_state(state, &empty_sftp_upload.id).await;
    assert_eq!(
        empty_sftp_upload.status,
        TransferStatus::Completed,
        "empty SFTP upload failed: {:?}",
        empty_sftp_upload.message
    );
    assert_eq!(fs::metadata(&empty_sftp_target).unwrap().len(), 0);
}

use super::*;

#[test]
fn default_transfer_directory_resolves_only_relative_local_paths() {
    let mut profile = test_ssh_profile();
    let default_dir = std::env::temp_dir().join("portmate-transfer-default");
    profile.transfer.default_local_dir = Some(default_dir.to_string_lossy().into_owned());

    let upload = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: "input.bin".to_string(),
            destination: "remote:/tmp/input.bin".to_string(),
        },
    )
    .unwrap();
    assert_eq!(
        upload.source,
        default_dir.join("input.bin").to_string_lossy()
    );
    assert_eq!(upload.destination, "remote:/tmp/input.bin");

    let absolute_destination = default_dir.join("download.bin");
    let download = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
            source: "ssh:/tmp/download.bin".to_string(),
            destination: absolute_destination.to_string_lossy().into_owned(),
        },
    )
    .unwrap();
    assert_eq!(download.source, "ssh:/tmp/download.bin");
    assert_eq!(download.destination, absolute_destination.to_string_lossy());
}

#[test]
fn sftp_transfer_paths_reject_root_and_dot_components() {
    for path in ["/", "//", "~", "/tmp/../input.bin", "/tmp/./input.bin"] {
        let target_error = validate_remote_transfer_path(path, "SFTP 远端目标路径")
            .expect_err("unsafe SFTP destination was accepted");
        assert!(target_error.contains("SFTP 远端目标路径"), "{target_error}");
        let source_error = validate_remote_transfer_path(path, "SFTP 远端源路径")
            .expect_err("unsafe SFTP source was accepted");
        assert!(source_error.contains("SFTP 远端源路径"), "{source_error}");
    }
    assert!(validate_remote_transfer_path("/tmp/portmate/", "SFTP 远端目标路径").is_ok());
    assert!(
        validate_remote_transfer_path(r"C:\Users\operator\input.bin", "SFTP 远端源路径").is_ok()
    );
}

#[test]
fn scp_transfer_paths_reject_root_and_dot_components() {
    for path in ["/", "//", "~", "/tmp/../input.bin", "/tmp/./input.bin"] {
        let target_error = validate_remote_transfer_path(path, "SCP 远端目标路径")
            .expect_err("unsafe SCP destination was accepted");
        assert!(target_error.contains("SCP 远端目标路径"), "{target_error}");
        let source_error = validate_remote_transfer_path(path, "SCP 远端源路径")
            .expect_err("unsafe SCP source was accepted");
        assert!(source_error.contains("SCP 远端源路径"), "{source_error}");
    }
    assert!(validate_remote_transfer_path("/tmp/portmate/", "SCP 远端目标路径").is_ok());
}

#[test]
fn modem_transfer_paths_reject_root_and_dot_components() {
    let root = std::env::temp_dir().join(format!("portmate-modem-paths-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.bin");
    fs::write(&source, b"payload").unwrap();

    for path in ["/", "//", "~", "/tmp/../input.bin", "/tmp/./input.bin"] {
        let upload_error = match modem_direction(&StartTransferRequest {
            session_id: "session".to_string(),
            protocol: TransferProtocol::Zmodem,
            source: source.display().to_string(),
            destination: format!("remote:{path}"),
        }) {
            Err(error) => error,
            Ok(_) => panic!("unsafe Modem upload destination was accepted"),
        };
        assert!(
            upload_error.contains("Modem 远端目标路径"),
            "{upload_error}"
        );

        let download_error = match modem_direction(&StartTransferRequest {
            session_id: "session".to_string(),
            protocol: TransferProtocol::Zmodem,
            source: format!("remote:{path}"),
            destination: root.join("download.bin").display().to_string(),
        }) {
            Err(error) => error,
            Ok(_) => panic!("unsafe Modem download source was accepted"),
        };
        assert!(
            download_error.contains("Modem 远端源路径"),
            "{download_error}"
        );

        let implicit_error = match modem_direction(&StartTransferRequest {
            session_id: "session".to_string(),
            protocol: TransferProtocol::Xmodem,
            source: source.display().to_string(),
            destination: path.to_string(),
        }) {
            Err(error) => error,
            Ok(_) => panic!("unsafe implicit Modem upload destination was accepted"),
        };
        assert!(
            implicit_error.contains("Modem 远端目标路径"),
            "{implicit_error}"
        );
    }

    let accepted = modem_direction(&StartTransferRequest {
        session_id: "session".to_string(),
        protocol: TransferProtocol::Ymodem,
        source: source.display().to_string(),
        destination: "remote:/tmp/portmate/".to_string(),
    })
    .unwrap();
    match accepted {
        ModemDirection::Upload {
            remote_destination, ..
        } => {
            assert_eq!(remote_destination, "/tmp/portmate/")
        }
        _ => panic!("expected Modem upload direction"),
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_transfer_path_classification_covers_unix_windows_and_unc_forms() {
    for path in ["/tmp/input.bin", "//server/share/input.bin"] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Unix),
            LocalTransferPathKind::Absolute
        );
    }
    for path in ["input.bin", "nested/input.bin", r"nested\input.bin"] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Unix),
            LocalTransferPathKind::Relative
        );
    }
    for path in [
        r"C:\Users\operator\input.bin",
        "D:/data/input.bin",
        r"C:input.bin",
        r"\input.bin",
        r"\\server\share\input.bin",
    ] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Unix),
            LocalTransferPathKind::ForeignAnchored
        );
    }

    for path in [
        r"C:\Users\operator\input.bin",
        "D:/data/input.bin",
        r"\\server\share\input.bin",
        "//server/share/input.bin",
    ] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Windows),
            LocalTransferPathKind::Absolute
        );
    }
    for path in [r"\input.bin", "/input.bin"] {
        assert_eq!(
            classify_local_transfer_path(path, LocalTransferPathPlatform::Windows),
            LocalTransferPathKind::RootedWithoutDrive
        );
    }
    assert_eq!(
        classify_local_transfer_path(r"C:input.bin", LocalTransferPathPlatform::Windows),
        LocalTransferPathKind::DriveRelative
    );
    assert_eq!(
        classify_local_transfer_path("nested/input.bin", LocalTransferPathPlatform::Windows),
        LocalTransferPathKind::Relative
    );
}

#[test]
fn transfer_paths_reject_non_native_or_ambiguous_local_roots() {
    let mut profile = test_ssh_profile();
    profile.transfer.default_local_dir = Some("relative/default".to_string());
    let error = validate_transfer_default_local_dir(&profile).unwrap_err();
    assert!(error.contains("完整绝对路径"), "{error}");

    profile.transfer.default_local_dir = None;
    let foreign_local_path = if cfg!(windows) {
        "C:input.bin"
    } else {
        r"C:\Users\operator\input.bin"
    };
    let error = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: foreign_local_path.to_string(),
            destination: "remote:/tmp/input.bin".to_string(),
        },
    )
    .unwrap_err();
    assert!(
        error.contains("不兼容") || error.contains("drive-relative"),
        "{error}"
    );

    let remote_windows_path = r"remote:C:\Users\operator\input.bin";
    let request = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: remote_windows_path.to_string(),
            destination: std::env::temp_dir()
                .join("input.bin")
                .to_string_lossy()
                .into_owned(),
        },
    )
    .unwrap();
    assert_eq!(request.source, remote_windows_path);
}

#[test]
fn file_manager_local_paths_reject_foreign_roots_and_filesystem_roots() {
    let foreign = if cfg!(windows) {
        "/tmp/portmate-foreign"
    } else {
        r"C:\Users\operator\foreign"
    };
    assert!(validate_native_local_path(foreign).is_err());
    assert!(validate_local_mutating_path(foreign).is_err());
    assert!(validate_local_drop_destination(foreign).is_err());
    assert!(list_local_files(foreign).is_err());
    assert!(local_file_properties(foreign).is_err());

    let filesystem_root = if cfg!(windows) { r"C:\" } else { "/" };
    assert!(validate_local_mutating_path(filesystem_root).is_err());
    assert!(validate_local_mutating_path("~").is_err());
    assert_eq!(
        validate_native_local_path("nested/child").unwrap(),
        expand_identity_path("nested/child")
    );
    assert!(validate_local_mutating_path("nested/../child").is_err());
    assert!(validate_local_mutating_path("nested/./child").is_err());
}

#[test]
fn file_manager_remote_mutating_paths_reject_parent_components() {
    for path in [
        "/tmp/..",
        "/tmp/./file",
        "nested/../outside",
        "../outside",
        "/",
        "//",
        "~",
    ] {
        assert!(
            validate_remote_mutating_path(path).is_err(),
            "unsafe remote path was accepted: {path}"
        );
        assert!(normalize_remote_batch_source(path).is_err());
        assert!(validate_remote_drop_destination(path).is_err());
    }
    assert!(validate_remote_drop_destination("/tmp/portmate/").is_ok());
    assert_eq!(
        validate_remote_mutating_path("/tmp/portmate/file").unwrap(),
        "/tmp/portmate/file"
    );
}

#[test]
fn file_manager_local_file_creation_is_exclusive() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("new-file.txt");
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    tauri::async_runtime::block_on(async {
        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: None,
                path: file.display().to_string(),
                remote: false,
            },
            FileOperation::CreateFile,
        )
        .await
        .unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"");

        fs::write(&file, b"existing contents").unwrap();
        let error = file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: None,
                path: file.display().to_string(),
                remote: false,
            },
            FileOperation::CreateFile,
        )
        .await
        .unwrap_err();
        assert!(error.contains("新建本地文件失败"), "{error}");
    });

    assert_eq!(fs::read(&file).unwrap(), b"existing contents");
}

#[test]
fn file_manager_local_batch_delete_removes_files_and_directories() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("remove.txt");
    let directory = root.path().join("remove-tree");
    fs::create_dir_all(directory.join("nested")).unwrap();
    fs::write(&file, b"remove").unwrap();
    fs::write(directory.join("nested/value.txt"), b"remove nested").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    tauri::async_runtime::block_on(delete_paths_inner(
        &state,
        DeletePathsRequest {
            session_id: None,
            paths: vec![file.display().to_string(), directory.display().to_string()],
            remote: false,
        },
    ))
    .unwrap();

    assert!(!file.exists());
    assert!(!directory.exists());
}

#[test]
fn file_manager_local_batch_delete_preflights_directory_children() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("remove-tree");
    let child = directory.join("value.txt");
    fs::create_dir(&directory).unwrap();
    fs::write(&child, b"keep").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(delete_paths_inner(
        &state,
        DeletePathsRequest {
            session_id: None,
            paths: vec![directory.display().to_string(), child.display().to_string()],
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("目录及其子项"), "{error}");
    assert!(directory.is_dir());
    assert_eq!(fs::read(&child).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn file_manager_local_batch_delete_removes_a_final_symlink_only() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("protected.txt");
    let link = root.path().join("remove-link");
    fs::write(&target, b"protected").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    tauri::async_runtime::block_on(delete_paths_inner(
        &state,
        DeletePathsRequest {
            session_id: None,
            paths: vec![link.display().to_string()],
            remote: false,
        },
    ))
    .unwrap();

    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"protected");
}

#[test]
fn file_manager_local_move_moves_multiple_selected_paths() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir_all(source.join("nested")).unwrap();
    fs::create_dir(&destination).unwrap();
    let file = source.join("report.txt");
    let directory = source.join("nested");
    fs::write(&file, b"report").unwrap();
    fs::write(directory.join("detail.txt"), b"detail").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    tauri::async_runtime::block_on(move_paths_inner(
        &state,
        MovePathsRequest {
            session_id: None,
            paths: vec![file.display().to_string(), directory.display().to_string()],
            destination: destination.display().to_string(),
            remote: false,
        },
    ))
    .unwrap();

    assert!(!file.exists());
    assert!(!directory.exists());
    assert_eq!(fs::read(destination.join("report.txt")).unwrap(), b"report");
    assert_eq!(
        fs::read(destination.join("nested/detail.txt")).unwrap(),
        b"detail"
    );
}

#[test]
fn file_manager_local_move_rejects_collisions_before_any_mutation() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let first = source.join("first.txt");
    let second = source.join("second.txt");
    fs::write(&first, b"first source").unwrap();
    fs::write(&second, b"second source").unwrap();
    fs::write(destination.join("second.txt"), b"existing target").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(move_paths_inner(
        &state,
        MovePathsRequest {
            session_id: None,
            paths: vec![first.display().to_string(), second.display().to_string()],
            destination: destination.display().to_string(),
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("已存在"), "{error}");
    assert_eq!(fs::read(&first).unwrap(), b"first source");
    assert_eq!(fs::read(&second).unwrap(), b"second source");
    assert!(!destination.join("first.txt").exists());
    assert_eq!(
        fs::read(destination.join("second.txt")).unwrap(),
        b"existing target"
    );
}

#[test]
fn file_manager_local_move_rejects_a_directory_destination_inside_the_source() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let directory = source.join("tree");
    let nested_destination = directory.join("nested");
    fs::create_dir_all(&nested_destination).unwrap();
    fs::write(directory.join("detail.txt"), b"detail").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(move_paths_inner(
        &state,
        MovePathsRequest {
            session_id: None,
            paths: vec![directory.display().to_string()],
            destination: nested_destination.display().to_string(),
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("自身内部"), "{error}");
    assert_eq!(fs::read(directory.join("detail.txt")).unwrap(), b"detail");
}

#[test]
fn file_manager_local_move_rejects_a_selected_directory_and_its_child() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let destination = root.path().join("destination");
    let directory = source.join("tree");
    let child = directory.join("detail.txt");
    fs::create_dir_all(&directory).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::write(&child, b"detail").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(move_paths_inner(
        &state,
        MovePathsRequest {
            session_id: None,
            paths: vec![directory.display().to_string(), child.display().to_string()],
            destination: destination.display().to_string(),
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("目录及其子项"), "{error}");
    assert!(directory.is_dir());
    assert_eq!(fs::read(&child).unwrap(), b"detail");
    assert!(!destination.join("tree").exists());
    assert!(!destination.join("detail.txt").exists());
}

#[test]
fn file_manager_local_rename_refuses_to_replace_an_existing_target() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.txt");
    let target = root.path().join("target.txt");
    fs::write(&source, b"source contents").unwrap();
    fs::write(&target, b"target contents").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let error = tauri::async_runtime::block_on(rename_path_inner(
        &state,
        RenamePathRequest {
            session_id: None,
            old_path: source.display().to_string(),
            new_path: target.display().to_string(),
            remote: false,
        },
    ))
    .unwrap_err();

    assert!(error.contains("已存在"), "{error}");
    assert_eq!(fs::read(&source).unwrap(), b"source contents");
    assert_eq!(fs::read(&target).unwrap(), b"target contents");
}

#[cfg(unix)]
#[test]
fn local_directory_creation_rejects_symlink_components() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("target");
    let link = root.path().join("link");
    let renamed_link = root.path().join("renamed-link");
    fs::create_dir(&target).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let nested = link.join("nested");
    let error = reject_local_symlink_components(&nested, false, "test path").unwrap_err();

    assert!(error.contains("符号链接"), "{error}");
    assert!(!target.join("nested").exists());
    assert!(reject_local_symlink_components(&link, true, "final link").is_ok());

    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );
    let file_error = tauri::async_runtime::block_on(file_operation_inner(
        &state,
        FileOperationRequest {
            session_id: None,
            path: link.join("new-file.txt").display().to_string(),
            remote: false,
        },
        FileOperation::CreateFile,
    ))
    .unwrap_err();
    assert!(file_error.contains("符号链接"), "{file_error}");
    assert!(!target.join("new-file.txt").exists());
    tauri::async_runtime::block_on(async {
        rename_path_inner(
            &state,
            RenamePathRequest {
                session_id: None,
                old_path: link.display().to_string(),
                new_path: renamed_link.display().to_string(),
                remote: false,
            },
        )
        .await
        .unwrap();
        assert!(fs::symlink_metadata(&renamed_link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(target.is_dir());

        file_operation_inner(
            &state,
            FileOperationRequest {
                session_id: None,
                path: renamed_link.display().to_string(),
                remote: false,
            },
            FileOperation::Delete,
        )
        .await
        .unwrap();
    });
    assert!(fs::symlink_metadata(&renamed_link).is_err());
    assert!(target.is_dir());
}

#[test]
fn modem_file_names_normalize_windows_and_unix_separators() {
    assert_eq!(
        portable_file_name(r"C:\Users\operator\report.bin"),
        Some("report.bin".to_string())
    );
    assert_eq!(
        portable_file_name(r"\\server\share\report.bin"),
        Some("report.bin".to_string())
    );
    assert_eq!(
        portable_file_name("/var/tmp/report.bin"),
        Some("report.bin".to_string())
    );
    assert_eq!(portable_file_name("../"), None);
    assert_eq!(
        local_file_name(r"C:\Users\operator\report.bin"),
        "report.bin"
    );
    assert_eq!(
        remote_parent_and_file_name(r"C:\Users\operator\report.bin"),
        (r"C:\Users\operator".to_string(), "report.bin".to_string())
    );
    assert_eq!(
        remote_parent_and_file_name("/report.bin"),
        ("/".to_string(), "report.bin".to_string())
    );

    let root = std::env::temp_dir().join(format!("portmate-modem-name-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target =
        zmodem_local_target_path(root.to_str().unwrap(), r"C:\Users\operator\report.bin", 0)
            .unwrap();
    assert_eq!(target, root.join("report.bin"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn remote_file_names_are_safe_for_local_directory_targets() {
    assert_eq!(remote_file_name("/var/tmp/report.bin"), "report.bin");
    assert_eq!(
        remote_file_name(r"C:\Users\operator\report.bin"),
        "report.bin"
    );
    assert_eq!(remote_file_name("../"), "portmate-file.bin");
    assert_eq!(remote_file_name("."), "portmate-file.bin");

    let root = std::env::temp_dir().join(format!("portmate-remote-name-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target =
        local_destination_file_path(&format!("{}/", root.display()), "../outside.bin").unwrap();
    assert_eq!(target, root.join("outside.bin"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn local_file_listing_and_chmod_do_not_follow_symbolic_links() {
    let root = std::env::temp_dir().join(format!("portmate-file-links-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let protected = root.join("protected.txt");
    fs::write(&protected, b"protected").unwrap();
    let link = root.join("linked.txt");
    std::os::unix::fs::symlink(&protected, &link).unwrap();

    let entries = list_local_files(root.to_str().unwrap()).unwrap();
    let entry = entries
        .iter()
        .find(|entry| entry.name == "linked.txt")
        .unwrap();
    assert!(!entry.is_dir);
    assert_eq!(entry.size, 0);

    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let error = tauri::async_runtime::block_on(chmod_path_inner(
        &state,
        ChmodPathRequest {
            session_id: None,
            path: link.display().to_string(),
            mode: 0o600,
            remote: false,
        },
    ))
    .unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let _ = fs::remove_dir_all(root);
}

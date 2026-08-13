#[test]
fn local_file_listing_rejects_oversized_directories_without_partial_results() {
    let root = canonical_test_tempdir();
    for name in ["alpha", "beta", "gamma"] {
        fs::write(root.path().join(name), name).unwrap();
    }

    let error = list_local_files_with_limit(root.path().to_str().unwrap(), 2).unwrap_err();
    assert!(error.contains("目录条目超过 2 条"), "{error}");
    assert_eq!(
        list_local_files_with_limit(root.path().to_str().unwrap(), 3)
            .unwrap()
            .len(),
        3
    );
}

#[cfg(unix)]
#[test]
fn local_file_listing_does_not_publish_lossy_non_utf8_paths() {
    use std::os::unix::ffi::OsStringExt;

    let root = canonical_test_tempdir();
    let invalid_name =
        std::ffi::OsString::from_vec(vec![b'i', b'n', b'v', b'a', b'l', b'i', b'd', 0xff]);
    fs::write(root.path().join(invalid_name), b"payload").unwrap();

    let error = list_local_files(root.path().to_str().unwrap()).unwrap_err();
    assert!(error.contains("非 UTF-8 文件名"), "{error}");
}

#[cfg(unix)]
#[test]
fn local_file_manager_never_trims_an_enumerated_path() {
    let root = canonical_test_tempdir();
    let plain = root.path().join("report.txt");
    let spaced = root.path().join("report.txt ");
    let renamed = root.path().join("renamed.txt ");
    fs::write(&plain, b"plain").unwrap();
    fs::write(&spaced, b"spaced payload").unwrap();
    let state = test_app_state(
        test_ssh_profile(),
        root.path().join("portmate-store.sqlite3"),
    );

    let entries = list_local_files(root.path().to_str().unwrap()).unwrap();
    assert!(entries.iter().any(|entry| entry.name == "report.txt"));
    assert!(entries.iter().any(|entry| entry.name == "report.txt "));
    let properties = local_file_properties(spaced.to_str().unwrap()).unwrap();
    assert_eq!(properties.path, spaced.display().to_string());
    assert_eq!(properties.size, b"spaced payload".len() as u64);

    let drop_plan = plan_external_drop(&[spaced.display().to_string()], None).unwrap();
    assert_eq!(drop_plan.files.len(), 1);
    assert_eq!(drop_plan.files[0].source, spaced);
    assert_eq!(drop_plan.files[0].relative, PathBuf::from("report.txt "));

    tauri::async_runtime::block_on(rename_path_inner(
        &state,
        RenamePathRequest {
            session_id: None,
            old_path: spaced.display().to_string(),
            new_path: renamed.display().to_string(),
            remote: false,
        },
    ))
    .unwrap();
    assert!(!spaced.exists());
    assert_eq!(fs::read(&renamed).unwrap(), b"spaced payload");
    assert_eq!(fs::read(&plain).unwrap(), b"plain");

    tauri::async_runtime::block_on(file_operation_inner(
        &state,
        FileOperationRequest {
            session_id: None,
            path: renamed.display().to_string(),
            remote: false,
        },
        FileOperation::Delete,
    ))
    .unwrap();
    assert!(!renamed.exists());
    assert_eq!(fs::read(&plain).unwrap(), b"plain");
}

#[test]
fn file_manager_local_file_creation_is_exclusive() {
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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
    let root = canonical_test_tempdir();
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

#[cfg(unix)]
#[test]
fn local_file_listing_and_chmod_do_not_follow_symbolic_links() {
    let root = canonical_test_temp_path("portmate-file-links");
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

use super::*;

#[test]
fn local_file_properties_reports_file_metadata() {
    let root = std::env::temp_dir().join(format!("portmate-file-props-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("payload.bin");
    fs::write(&target, b"payload").unwrap();

    let properties = local_file_properties(target.to_str().unwrap()).unwrap();
    assert_eq!(properties.name, "payload.bin");
    assert_eq!(properties.path, target.display().to_string());
    assert!(!properties.remote);
    assert_eq!(properties.kind, "file");
    assert!(properties.is_file);
    assert!(!properties.is_dir);
    assert_eq!(properties.size, 7);
    assert!(properties.modified.is_some());
    #[cfg(unix)]
    assert!(properties.permissions.is_some());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_drop_plan_preserves_directories_and_skips_unsafe_entries() {
    let root = std::env::temp_dir().join(format!("portmate-drop-plan-{}", Uuid::new_v4()));
    let source = root.join("source-tree");
    let nested = source.join("nested");
    let empty = source.join("empty");
    let destination = root.join("destination");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(&empty).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("top.txt"), b"top").unwrap();
    fs::write(nested.join("payload.bin"), b"payload").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&nested, source.join("nested-link")).unwrap();

    let paths = vec![
        source.display().to_string(),
        nested.join("payload.bin").display().to_string(),
    ];
    let destination = destination.canonicalize().unwrap();
    let plan = plan_external_drop(&paths, Some(&destination)).unwrap();

    assert_eq!(
        plan.directories,
        vec![
            PathBuf::from("source-tree"),
            PathBuf::from("source-tree/empty"),
            PathBuf::from("source-tree/nested"),
        ]
    );
    assert_eq!(
        plan.files
            .iter()
            .map(|file| file.relative.clone())
            .collect::<Vec<_>>(),
        vec![
            PathBuf::from("source-tree/nested/payload.bin"),
            PathBuf::from("source-tree/top.txt"),
        ]
    );
    assert_eq!(plan.total_bytes, 10);
    assert!(plan
        .skipped
        .iter()
        .any(|item| item.contains("already included")));
    #[cfg(unix)]
    assert!(plan
        .skipped
        .iter()
        .any(|item| item.contains("symbolic link")));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_drop_plan_rejects_self_descendants_and_target_conflicts() {
    let root = std::env::temp_dir().join(format!("portmate-drop-guards-{}", Uuid::new_v4()));
    let first = root.join("first/shared");
    let second = root.join("second/shared");
    let destination = root.join("destination");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(first.join("one.txt"), b"one").unwrap();

    let root_destination = root.canonicalize().unwrap();
    let self_error = plan_external_drop(
        &[first.display().to_string()],
        Some(&root.join("first").canonicalize().unwrap()),
    )
    .unwrap_err();
    assert!(self_error.contains("复制到自身"), "{self_error}");

    let descendant = first.join("child-target");
    fs::create_dir_all(&descendant).unwrap();
    let descendant_error = plan_external_drop(
        &[first.display().to_string()],
        Some(&descendant.canonicalize().unwrap()),
    )
    .unwrap_err();
    assert!(descendant_error.contains("子目录"), "{descendant_error}");

    let conflict_error = plan_external_drop(
        &[first.display().to_string(), second.display().to_string()],
        Some(&destination.canonicalize().unwrap()),
    )
    .unwrap_err();
    assert!(
        conflict_error.contains("冲突的目标目录"),
        "{conflict_error}"
    );
    assert!(root_destination.is_dir());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn file_batch_conflict_policies_fail_skip_overwrite_and_rename() {
    let root = std::env::temp_dir().join(format!("portmate-conflicts-{}", Uuid::new_v4()));
    let source = root.join("source/report.txt");
    let destination = root.join("destination");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(&source, b"new report").unwrap();
    fs::write(destination.join("report.txt"), b"old report").unwrap();
    let destination = destination.canonicalize().unwrap();
    let paths = vec![source.display().to_string()];

    tauri::async_runtime::block_on(async {
        let mut fail = plan_external_drop(&paths, Some(&destination)).unwrap();
        let error = apply_external_drop_conflicts(
            &mut fail,
            None,
            destination.to_str().unwrap(),
            Some(&destination),
            false,
            TransferConflictPolicy::Fail,
        )
        .await
        .unwrap_err();
        assert!(error.contains("目标文件已存在"), "{error}");

        let mut skip = plan_external_drop(&paths, Some(&destination)).unwrap();
        apply_external_drop_conflicts(
            &mut skip,
            None,
            destination.to_str().unwrap(),
            Some(&destination),
            false,
            TransferConflictPolicy::Skip,
        )
        .await
        .unwrap();
        assert!(skip.files.is_empty());
        assert_eq!(skip.skipped.len(), 1);
        assert_eq!(skip.total_bytes, 0);

        let mut overwrite = plan_external_drop(&paths, Some(&destination)).unwrap();
        apply_external_drop_conflicts(
            &mut overwrite,
            None,
            destination.to_str().unwrap(),
            Some(&destination),
            false,
            TransferConflictPolicy::Overwrite,
        )
        .await
        .unwrap();
        assert_eq!(overwrite.files[0].relative, PathBuf::from("report.txt"));
        assert_eq!(overwrite.total_bytes, 10);

        let mut rename = plan_external_drop(&paths, Some(&destination)).unwrap();
        apply_external_drop_conflicts(
            &mut rename,
            None,
            destination.to_str().unwrap(),
            Some(&destination),
            false,
            TransferConflictPolicy::Rename,
        )
        .await
        .unwrap();
        assert_eq!(rename.files[0].relative, PathBuf::from("report (1).txt"));
        assert_eq!(rename.total_bytes, 10);
    });

    assert_eq!(
        numbered_batch_relative_path("nested/archive.tar.gz", 2).unwrap(),
        "nested/archive.tar (2).gz"
    );
    assert!(validate_batch_relative_path("../escape").is_err());
    assert!(validate_batch_relative_path("folder\\escape").is_err());
    assert!(validate_batch_relative_path("C:/escape").is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_drop_local_batch_copies_nested_files_through_transfer_queue() {
    let root = std::env::temp_dir().join(format!("portmate-drop-local-{}", Uuid::new_v4()));
    let source = root.join("incoming");
    let nested = source.join("nested");
    let empty = source.join("empty");
    let destination = root.join("destination");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(&empty).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("alpha.txt"), b"alpha").unwrap();
    fs::write(nested.join("beta.bin"), b"beta").unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));

    tauri::async_runtime::block_on(async {
        let result = start_external_drop_inner(
            &state,
            StartExternalDropRequest {
                session_id: profile.id.clone(),
                paths: vec![source.display().to_string()],
                destination: destination.display().to_string(),
                remote: false,
                conflict_policy: TransferConflictPolicy::Fail,
            },
        )
        .await
        .unwrap();
        assert_eq!(result.tasks.len(), 2);
        assert_eq!(result.directories_prepared, 3);
        assert_eq!(result.total_bytes, 9);
        assert!(result.skipped.is_empty());
        for task in result.tasks {
            let task = wait_for_transfer_terminal_state(&state, &task.id).await;
            assert_eq!(
                task.status,
                TransferStatus::Completed,
                "local recursive drop failed: {:?}",
                task.message
            );
        }
    });

    let copied = destination.join("incoming");
    assert_eq!(fs::read(copied.join("alpha.txt")).unwrap(), b"alpha");
    assert_eq!(fs::read(copied.join("nested/beta.bin")).unwrap(), b"beta");
    assert!(copied.join("empty").is_dir());

    let _ = fs::remove_dir_all(root);
}

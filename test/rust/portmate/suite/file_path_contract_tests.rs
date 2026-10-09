#[test]
fn file_command_types_keep_stable_serde_contract() {
    let rename: RenamePathRequest = serde_json::from_value(serde_json::json!({
        "sessionId": "ssh-session-1",
        "oldPath": "/tmp/old name.txt",
        "newPath": r"C:\Users\operator\new name.txt",
        "remote": true
    }))
    .unwrap();
    assert_eq!(rename.session_id.as_deref(), Some("ssh-session-1"));
    assert_eq!(rename.old_path, "/tmp/old name.txt");
    assert_eq!(rename.new_path, r"C:\Users\operator\new name.txt");
    assert!(rename.remote);

    let properties = FileProperties {
        name: "link".to_string(),
        path: "/tmp/link".to_string(),
        remote: false,
        kind: "symlink".to_string(),
        is_dir: false,
        is_file: false,
        is_symlink: true,
        size: 0,
        permissions: Some(0o777),
        modified: None,
        accessed: None,
        created: None,
    };
    let value = serde_json::to_value(properties).unwrap();
    assert_eq!(value["isSymlink"], true);
    assert_eq!(value["permissions"], 0o777);
    assert!(value.get("is_symlink").is_none());
}

#[test]
fn local_and_remote_path_helpers_preserve_significant_edge_whitespace() {
    let unix_path = "/tmp/ report.txt ";
    assert_eq!(
        validate_native_local_path_with_home(unix_path, LocalTransferPathPlatform::Unix, None,)
            .unwrap(),
        PathBuf::from(unix_path)
    );
    let windows_path = r"C:\Temp\ report.txt ";
    assert_eq!(
        validate_native_local_path_with_home(
            windows_path,
            LocalTransferPathPlatform::Windows,
            None,
        )
        .unwrap(),
        PathBuf::from(windows_path)
    );

    let remote_path = "/tmp/ report.txt ";
    assert_eq!(
        validate_remote_mutating_path(remote_path).unwrap(),
        remote_path
    );
    assert_eq!(
        portable_file_name(remote_path).as_deref(),
        Some(" report.txt ")
    );
    assert_eq!(
        normalize_remote_batch_source(remote_path).unwrap(),
        remote_path
    );
    assert_eq!(remote_parent_path(remote_path).as_deref(), Some("/tmp"));
    assert!(validate_remote_drop_destination("/tmp/ destination ").is_ok());
    assert!(validate_remote_transfer_path(remote_path, "remote path").is_ok());
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
    assert_eq!(
        remote_parent_and_file_name("/tmp/ report.bin "),
        ("/tmp".to_string(), " report.bin ".to_string())
    );

    let root = std::env::temp_dir().join(format!("portmate-modem-name-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target =
        zmodem_local_target_path(root.to_str().unwrap(), r"C:\Users\operator\report.bin", 0)
            .unwrap();
    assert_eq!(target, root.join("report.bin"));
    let exact_target = zmodem_local_target_path(
        root.join("download.bin ").to_str().unwrap(),
        "ignored.bin",
        0,
    )
    .unwrap();
    assert_eq!(exact_target, root.join("download.bin "));
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

#[test]
fn local_transfer_target_preparation_creates_parents_without_creating_the_file() {
    let root = canonical_test_tempdir();
    let target = root
        .path()
        .join("new directory")
        .join("nested")
        .join("payload.bin");

    prepare_local_transfer_target_path(&target, "SFTP local target path").unwrap();

    assert!(target.parent().unwrap().is_dir());
    assert!(!target.exists());
    fs::write(&target, b"existing payload").unwrap();
    prepare_local_transfer_target_path(&target, "SFTP local target path").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"existing payload");
}

#[test]
fn local_transfer_target_preparation_rejects_a_file_in_the_parent_chain() {
    let root = canonical_test_tempdir();
    let parent_file = root.path().join("not-a-directory");
    fs::write(&parent_file, b"protected contents").unwrap();
    let target = parent_file.join("payload.bin");

    assert!(prepare_local_transfer_target_path(&target, "SFTP local target path").is_err());
    assert_eq!(fs::read(&parent_file).unwrap(), b"protected contents");
    assert!(!target.exists());
}

#[cfg(windows)]
#[test]
fn windows_local_transfer_target_accepts_canonical_verbatim_drive_paths() {
    use std::path::{Component, Prefix};

    let root = canonical_test_tempdir();
    // Rust's Windows canonicalize produces the same \\?\C:\ form returned
    // by the native file picker. Do not strip that prefix in this regression.
    let canonical = root.path().canonicalize().unwrap();
    assert!(matches!(
        canonical.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_))
    ));
    let target = canonical.join("new directory").join("payload.bin");

    prepare_local_transfer_target_path(&target, "SFTP local target path").unwrap();
    assert!(target.parent().unwrap().is_dir());
    assert!(!target.exists());
    let mut output = PendingLocalTransferOutput::create(&target, "SFTP local target path").unwrap();
    output
        .file_mut()
        .unwrap()
        .write_all(b"download contents")
        .unwrap();
    output.finish().unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"download contents");
}

#[cfg(windows)]
#[test]
fn windows_local_transfer_target_accepts_drive_roots_and_regular_drive_paths() {
    use std::path::{Component, Prefix};

    let root = canonical_test_tempdir();
    let canonical = root.path().canonicalize().unwrap();
    let regular = PathBuf::from(canonical.to_str().unwrap().strip_prefix(r"\\?\").unwrap());
    assert!(matches!(
        regular.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    ));

    for base in [&canonical, &regular] {
        let drive_root = base.ancestors().last().unwrap();
        assert!(drive_root.has_root());
        reject_local_symlink_components(drive_root, false, "local drive root").unwrap();
        let target = base.join("regular directory").join("payload.bin");
        prepare_local_transfer_target_path(&target, "SFTP local target path").unwrap();
        assert!(target.parent().unwrap().is_dir());
        assert!(!target.exists());
    }
}

#[cfg(windows)]
#[test]
fn windows_local_path_guard_does_not_accept_an_incomplete_verbatim_drive_prefix() {
    let root = canonical_test_tempdir();
    let canonical = root.path().canonicalize().unwrap();
    let prefix = PathBuf::from(canonical.components().next().unwrap().as_os_str());

    assert_eq!(prefix.components().count(), 1);
    assert!(reject_local_symlink_components(&prefix, false, "local target path").is_err());
}

#[cfg(windows)]
#[test]
fn windows_verbatim_target_still_rejects_junction_ancestors() {
    let root = canonical_test_tempdir();
    let protected = root.path().join("protected directory");
    let junction = root.path().join("junction");
    fs::create_dir(&protected).unwrap();
    // Directory junction creation does not require Developer Mode or the
    // symbolic-link privilege, so this also runs on ordinary Windows CI.
    let result = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&protected)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "create junction failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let verbatim_junction = root.path().canonicalize().unwrap().join("junction");
    let target = verbatim_junction.join("nested").join("payload.bin");

    let error = prepare_local_transfer_target_path(&target, "SFTP local target path").unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert!(!protected.join("nested").exists());
    assert!(reject_local_symlink_components(&verbatim_junction, false, "target link").is_err());
    assert!(reject_local_symlink_components(&verbatim_junction, true, "final link").is_ok());
    // Remove just the junction, never recursively operate on the protected target.
    fs::remove_dir(&junction).unwrap();
    assert!(protected.is_dir());
}

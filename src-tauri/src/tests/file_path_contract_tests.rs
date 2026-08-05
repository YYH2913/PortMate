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

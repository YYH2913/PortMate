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
fn transfer_home_paths_expand_before_the_profile_default_directory() {
    let mut profile = test_ssh_profile();
    profile.transfer.default_local_dir = Some("~/Downloads".to_string());
    let home = Path::new("native-home");

    let request = prepare_transfer_request_with_home(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: "~/upload.bin".to_string(),
            destination: "relative-download.bin".to_string(),
        },
        LocalTransferPathPlatform::Unix,
        Some(home),
    )
    .unwrap();

    assert_eq!(Path::new(&request.source), home.join("upload.bin"));
    assert_eq!(
        Path::new(&request.destination),
        home.join("Downloads").join("relative-download.bin")
    );
    assert!(!request.source.contains("Downloads/~"));
}

#[test]
fn transfer_home_alias_uses_platform_separator_rules_and_requires_a_home() {
    let home = Path::new("native-home");
    assert_eq!(
        resolve_transfer_default_local_dir_with_home(
            Some(r"~\Downloads"),
            LocalTransferPathPlatform::Windows,
            Some(home),
        )
        .unwrap(),
        Some(home.join(r"Downloads").to_str().unwrap().to_string())
    );
    assert_eq!(
        resolve_default_local_transfer_path_with_home(
            r"~\upload.bin",
            Some("ignored-default"),
            LocalTransferPathPlatform::Windows,
            Some(home),
        )
        .unwrap(),
        home.join(r"upload.bin").to_str().unwrap()
    );
    assert_eq!(
        resolve_default_local_transfer_path_with_home(
            r"~\literal.bin",
            Some("unix-default"),
            LocalTransferPathPlatform::Unix,
            Some(home),
        )
        .unwrap(),
        Path::new("unix-default")
            .join(r"~\literal.bin")
            .to_str()
            .unwrap()
    );

    let error = resolve_default_local_transfer_path_with_home(
        "~/upload.bin",
        None,
        LocalTransferPathPlatform::Unix,
        None,
    )
    .unwrap_err();
    assert!(error.contains("用户主目录不可用"), "{error}");
}

#[test]
fn empty_remote_transfer_markers_are_never_treated_as_local_paths() {
    let profile = test_ssh_profile();

    for marker in ["remote:", "remote:   ", "ssh:", "ssh:\t"] {
        assert_eq!(
            remote_path(marker),
            Some(&marker[marker.find(':').unwrap() + 1..])
        );
        for protocol in [
            TransferProtocol::Sftp,
            TransferProtocol::Scp,
            TransferProtocol::Xmodem,
            TransferProtocol::Ymodem,
            TransferProtocol::Zmodem,
        ] {
            let error = prepare_transfer_request(
                &profile,
                StartTransferRequest {
                    session_id: profile.id.clone(),
                    protocol,
                    source: marker.to_string(),
                    destination: "output.bin".to_string(),
                },
            )
            .unwrap_err();
            assert!(error.contains("远端传输源路径"), "{marker}: {error}");
        }

        let error = prepare_transfer_request(
            &profile,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Sftp,
                source: "input.bin".to_string(),
                destination: marker.to_string(),
            },
        )
        .unwrap_err();
        assert!(error.contains("远端传输目标路径"), "{marker}: {error}");
    }
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
fn local_tilde_paths_follow_native_home_and_separator_rules() {
    let home = Path::new("/home/operator");
    assert_eq!(expand_identity_path_with_home("~", Some(home), false), home);
    assert_eq!(
        expand_identity_path_with_home("~/.ssh/id_ed25519", Some(home), false),
        home.join(".ssh/id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home("~//.ssh/id_ed25519", Some(home), false),
        home.join(".ssh/id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home(r"~\.ssh\id_ed25519", Some(home), true),
        home.join(r".ssh\id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home(r"~\\.ssh\id_ed25519", Some(home), true),
        home.join(r".ssh\id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home(r"~/C:\Windows", Some(home), true),
        PathBuf::from(r"~/C:\Windows")
    );
    assert_eq!(
        expand_identity_path_with_home(r"~\.ssh\id_ed25519", Some(home), false),
        PathBuf::from(r"~\.ssh\id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home("~/relative", None, true),
        PathBuf::from("~/relative")
    );
}

#[test]
fn local_mutations_protect_the_platform_native_home_directory() {
    let root = tempfile::tempdir().unwrap();
    let unix_home = root.path().join("unix-home");
    let windows_profile = root.path().join("windows-profile");
    fs::create_dir_all(&unix_home).unwrap();
    fs::create_dir_all(&windows_profile).unwrap();

    let windows_home =
        preferred_native_home_path(Some(unix_home.clone()), Some(windows_profile.clone()), true)
            .unwrap();
    assert_eq!(windows_home, windows_profile);
    assert!(validate_local_mutating_path_with_home(
        windows_profile.to_str().unwrap(),
        Some(&windows_home),
    )
    .unwrap_err()
    .contains("用户主目录"));
    assert!(validate_local_mutating_path_with_home(
        unix_home.to_str().unwrap(),
        Some(&windows_home),
    )
    .is_ok());

    let unix_native_home = preferred_native_home_path(
        Some(unix_home.clone()),
        Some(windows_profile.clone()),
        false,
    )
    .unwrap();
    assert_eq!(unix_native_home, unix_home);
    assert!(validate_local_mutating_path_with_home(
        unix_home.to_str().unwrap(),
        Some(&unix_native_home),
    )
    .unwrap_err()
    .contains("用户主目录"));
    assert_eq!(
        preferred_native_home_path(None, Some(windows_profile.clone()), false),
        Some(windows_profile.clone())
    );
    assert_eq!(
        preferred_native_home_path(Some(unix_home.clone()), None, true),
        Some(unix_home)
    );
}

#[test]
fn file_manager_remote_mutating_paths_reject_parent_components() {
    for path in [
        "/tmp/..",
        "/tmp/./file",
        "nested/../outside",
        "../outside",
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
    assert!(validate_remote_mutating_path("/").is_err());
    assert!(normalize_remote_batch_source("/").is_err());
    assert!(validate_remote_drop_destination("/").is_ok());
    assert!(validate_remote_drop_destination("/tmp/portmate/").is_ok());
    assert_eq!(
        validate_remote_mutating_path("/tmp/portmate/file").unwrap(),
        "/tmp/portmate/file"
    );
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
fn transfer_default_directory_preserves_significant_edge_whitespace() {
    let directory = if cfg!(windows) {
        r"C:\PortMate Downloads "
    } else {
        "/tmp/PortMate Downloads "
    };
    let platform = if cfg!(windows) {
        LocalTransferPathPlatform::Windows
    } else {
        LocalTransferPathPlatform::Unix
    };
    assert_eq!(
        resolve_transfer_default_local_dir_with_home(Some(directory), platform, None).unwrap(),
        Some(directory.to_string())
    );
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

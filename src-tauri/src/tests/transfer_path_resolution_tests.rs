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

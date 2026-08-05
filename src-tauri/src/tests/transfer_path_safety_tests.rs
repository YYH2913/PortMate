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

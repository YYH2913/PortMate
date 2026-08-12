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
fn device_load_endpoints_are_protocol_bound_and_injection_safe() {
    let xmodem = parse_load_receiver_endpoint(
        "load:loadx?address=0x80000000&baud=115200",
        &TransferProtocol::Xmodem,
    )
    .unwrap()
    .unwrap();
    assert_eq!(xmodem.command, "loadx");
    assert_eq!(xmodem.address.as_deref(), Some("0x80000000"));
    assert_eq!(xmodem.baud_rate, Some(115_200));
    assert_eq!(xmodem.command_line(), "loadx 0x80000000 115200\r");

    let ymodem = parse_load_receiver_endpoint("load:loady", &TransferProtocol::Ymodem)
        .unwrap()
        .unwrap();
    assert_eq!(ymodem.command_line(), "loady\r");

    for (endpoint, protocol) in [
        ("load:loady", TransferProtocol::Xmodem),
        ("load:loadx?address=8000%3Brun%20evil", TransferProtocol::Xmodem),
        ("load:loadx?address=8000&baud=0", TransferProtocol::Xmodem),
        ("load:loadx?baud=115200", TransferProtocol::Xmodem),
        ("load:loadx?address=8000&unknown=1", TransferProtocol::Xmodem),
        ("load://loadx?address=8000", TransferProtocol::Xmodem),
        ("load:loadz#suffix", TransferProtocol::Zmodem),
        ("load:loadx", TransferProtocol::Sftp),
    ] {
        assert!(
            parse_load_receiver_endpoint(endpoint, &protocol).is_err(),
            "unsafe or mismatched endpoint was accepted: {endpoint}"
        );
    }
}

#[test]
fn device_load_endpoint_is_a_local_modem_upload_destination() {
    let root = std::env::temp_dir().join(format!("portmate-load-endpoint-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let source = root.join("firmware.bin");
    fs::write(&source, b"firmware").unwrap();
    let profile = test_serial_profile(portmate_core::SerialConnection {
        port: "test-port".to_string(),
        baud_rate: 115_200,
        data_bits: 8,
        stop_bits: 1,
        parity: "none".to_string(),
        flow_control: "none".to_string(),
        dtr: false,
        rts: false,
        reconnect: false,
        reconnect_delay_ms: 1_000,
        receive_idle_timeout_enabled: false,
        receive_idle_timeout_seconds: 60,
    });

    let prepared = prepare_transfer_request(
        &profile,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Xmodem,
            source: source.display().to_string(),
            destination: "load:loadx?address=80000000".to_string(),
        },
    )
    .unwrap();
    assert_eq!(prepared.destination, "load:loadx?address=80000000");
    let (local_source, receiver) = device_modem_upload(&prepared).unwrap().unwrap();
    assert_eq!(local_source, source.display().to_string());
    assert_eq!(receiver.command_line(), "loadx 80000000\r");

    for (source, destination) in [
        ("load:loadx", source.to_str().unwrap()),
        ("remote:/tmp/firmware.bin", "load:loadx"),
    ] {
        assert!(prepare_transfer_request(
            &profile,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: TransferProtocol::Xmodem,
                source: source.to_string(),
                destination: destination.to_string(),
            },
        )
        .is_err());
    }
    let _ = fs::remove_dir_all(root);
}

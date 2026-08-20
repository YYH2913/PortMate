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

#[test]
fn tftp_load_endpoints_accept_long_timeouts_and_reject_unsafe_values() {
    let spec = parse_tftp_receiver_endpoint(
        "load:tftpboot?address=0x81800000&fileName=image.bin&deviceIp=192.168.255.1&serverIp=192.168.255.2&bindHost=0.0.0.0&bindPort=1069&timeoutSeconds=3600",
    )
    .unwrap()
    .unwrap();
    assert_eq!(spec.address.as_deref(), Some("0x81800000"));
    assert_eq!(spec.file_name.as_deref(), Some("image.bin"));
    assert_eq!(spec.device_ip.to_string(), "192.168.255.1");
    assert_eq!(spec.server_ip.unwrap().to_string(), "192.168.255.2");
    assert_eq!(spec.bind_host.unwrap().to_string(), "0.0.0.0");
    assert_eq!(spec.bind_port, 1_069);
    assert_eq!(spec.timeout, Duration::from_secs(3_600));
    let commands = spec
        .command_lines("ignored.bin", "192.168.255.2".parse().unwrap(), 1_069)
        .unwrap();
    assert_eq!(
        commands,
        "setenv ipaddr 192.168.255.1\rsetenv serverip 192.168.255.2\rsetenv tftpdstp 1069\rtftpboot 0x81800000 ignored.bin\r"
    );
    assert!(!commands.contains("saveenv"));

    let defaults = parse_tftp_receiver_endpoint(
        "load:tftpboot?deviceIp=192.168.255.1&bindPort=0",
    )
    .unwrap()
    .unwrap();
    assert_eq!(defaults.bind_port, 0);
    assert_eq!(defaults.timeout, Duration::from_secs(60));
    assert_eq!(
        defaults
            .command_lines("firmware.bin", "192.168.255.2".parse().unwrap(), 69)
            .unwrap(),
        "setenv ipaddr 192.168.255.1\rsetenv serverip 192.168.255.2\rsetenv tftpdstp\rtftpboot ${loadaddr} firmware.bin\r"
    );

    for endpoint in [
        "load:tftpboot",
        "load:tftpboot?deviceIp=0.0.0.0",
        "load:tftpboot?deviceIp=192.168.1.1&address=1%3Bsaveenv",
        "load:tftpboot?deviceIp=192.168.1.1&fileName=fw%3Bsaveenv",
        "load:tftpboot?deviceIp=192.168.1.1&bindPort=70000",
        "load:tftpboot?deviceIp=192.168.1.1&timeoutSeconds=4",
        "load:tftpboot?deviceIp=192.168.1.1&timeoutSeconds=18446744073709551615",
        "load:tftpboot?deviceIp=192.168.1.1&unknown=1",
        "load:tftpboot?deviceIp=192.168.1.1&deviceIp=192.168.1.2",
        "load:loadx?deviceIp=192.168.1.1",
    ] {
        assert!(
            parse_tftp_receiver_endpoint(endpoint).is_err(),
            "unsafe TFTP endpoint was accepted: {endpoint}"
        );
    }
}

#[test]
fn tftp_commands_are_sent_as_independent_cr_terminated_lines() {
    let commands = "setenv ipaddr 192.168.255.1\rsetenv serverip 192.168.255.2\rsetenv tftpdstp 1069\rtftpboot 0x81800000 firmware.bin\r";
    assert_eq!(
        split_tftp_command_lines(commands),
        vec![
            "setenv ipaddr 192.168.255.1\r",
            "setenv serverip 192.168.255.2\r",
            "setenv tftpdstp 1069\r",
            "tftpboot 0x81800000 firmware.bin\r",
        ]
    );
}

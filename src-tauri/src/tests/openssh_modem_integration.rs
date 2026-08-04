use super::*;

#[cfg(unix)]
pub(super) async fn exercise_openssh_modem_transfers(
    state: &AppState,
    profile: &SessionProfile,
    root: &std::path::Path,
    modem_tools_available: bool,
) {
    if !modem_tools_available {
        eprintln!("skipping modem OpenSSH coverage: lrzsz tools are not installed");
        return;
    }

    let zmodem_source = root.join("zmodem-upload-source.bin");
    let zmodem_remote = root.join("zmodem-remote.bin");
    let zmodem_download = root.join("zmodem-download-target.bin");
    let zmodem_payload = b"PortMate ZModem\x00binary\xffpayload\n";
    fs::write(&zmodem_source, zmodem_payload).unwrap();

    let upload = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Zmodem,
            source: zmodem_source.display().to_string(),
            destination: format!("remote:{}", zmodem_remote.display()),
        },
    )
    .await
    .unwrap();
    let upload = wait_for_transfer_terminal_state(state, &upload.id).await;
    assert_eq!(
        upload.status,
        TransferStatus::Completed,
        "ZModem upload failed: {:?}",
        upload.message
    );
    assert_eq!(upload.bytes_done, zmodem_payload.len() as u64);
    assert_eq!(fs::read(&zmodem_remote).unwrap(), zmodem_payload);
    assert!(!PathBuf::from(remote_resume_part_path(zmodem_remote.to_str().unwrap())).exists());

    let download = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Zmodem,
            source: format!("remote:{}", zmodem_remote.display()),
            destination: zmodem_download.display().to_string(),
        },
    )
    .await
    .unwrap();
    let download = wait_for_transfer_terminal_state(state, &download.id).await;
    assert_eq!(
        download.status,
        TransferStatus::Completed,
        "ZModem download failed: {:?}",
        download.message
    );
    assert_eq!(download.bytes_done, zmodem_payload.len() as u64);
    assert_eq!(fs::read(&zmodem_download).unwrap(), zmodem_payload);

    let xmodem_source = root.join("xmodem-upload-source.bin");
    let xmodem_remote = root.join("xmodem-remote.bin");
    let xmodem_download = root.join("xmodem-download-target.bin");
    let xmodem_payload = b"PortMate XModem integration payload\n".repeat(8);
    assert!(xmodem_payload.len() > XMODEM_BLOCK_SIZE);
    fs::write(&xmodem_source, &xmodem_payload).unwrap();
    let upload = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Xmodem,
            source: xmodem_source.display().to_string(),
            destination: format!("remote:{}", xmodem_remote.display()),
        },
    )
    .await
    .unwrap();
    let upload = wait_for_transfer_terminal_state(state, &upload.id).await;
    let xmodem_screen = state
        .store
        .lock()
        .unwrap()
        .screen(&profile.id)
        .unwrap_or_default();
    assert_eq!(
        upload.status,
        TransferStatus::Completed,
        "XModem upload failed: {:?}; screen={xmodem_screen:?}",
        upload.message,
    );
    assert_eq!(upload.bytes_done, xmodem_payload.len() as u64);
    assert_eq!(fs::read(&xmodem_remote).unwrap(), xmodem_payload);
    assert!(!PathBuf::from(remote_resume_part_path(xmodem_remote.to_str().unwrap())).exists());

    let download = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Xmodem,
            source: format!("remote:{}", xmodem_remote.display()),
            destination: xmodem_download.display().to_string(),
        },
    )
    .await
    .unwrap();
    let download = wait_for_transfer_terminal_state(state, &download.id).await;
    assert_eq!(
        download.status,
        TransferStatus::Completed,
        "XModem download failed: {:?}",
        download.message
    );
    assert_eq!(download.bytes_done, xmodem_payload.len() as u64);
    assert_eq!(fs::read(&xmodem_download).unwrap(), xmodem_payload);

    let ymodem_source = root.join("ymodem-upload-source.bin");
    let ymodem_remote = root.join("ymodem-remote.bin");
    let ymodem_download = root.join("ymodem-download-target.bin");
    let ymodem_payload = b"PortMate YModem\x00binary\xffpayload\n".repeat(40);
    assert!(ymodem_payload.len() > YMODEM_BLOCK_SIZE);
    fs::write(&ymodem_source, &ymodem_payload).unwrap();
    let upload = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Ymodem,
            source: ymodem_source.display().to_string(),
            destination: format!("remote:{}", ymodem_remote.display()),
        },
    )
    .await
    .unwrap();
    let upload = wait_for_transfer_terminal_state(state, &upload.id).await;
    let ymodem_screen = state
        .store
        .lock()
        .unwrap()
        .screen(&profile.id)
        .unwrap_or_default();
    assert_eq!(
        upload.status,
        TransferStatus::Completed,
        "YModem upload failed: {:?}; screen={ymodem_screen:?}",
        upload.message,
    );
    assert_eq!(upload.bytes_done, ymodem_payload.len() as u64);
    assert_eq!(fs::read(&ymodem_remote).unwrap(), ymodem_payload);
    assert!(!PathBuf::from(remote_resume_part_path(ymodem_remote.to_str().unwrap())).exists());

    let download = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Ymodem,
            source: format!("remote:{}", ymodem_remote.display()),
            destination: ymodem_download.display().to_string(),
        },
    )
    .await
    .unwrap();
    let download = wait_for_transfer_terminal_state(state, &download.id).await;
    assert_eq!(
        download.status,
        TransferStatus::Completed,
        "YModem download failed: {:?}",
        download.message
    );
    assert_eq!(download.bytes_done, ymodem_payload.len() as u64);
    assert_eq!(fs::read(&ymodem_download).unwrap(), ymodem_payload);
}

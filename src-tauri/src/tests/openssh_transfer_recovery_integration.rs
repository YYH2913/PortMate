use super::*;

#[cfg(unix)]
pub(super) async fn exercise_openssh_scp_and_transfer_recovery(
    state: &AppState,
    profile: &SessionProfile,
    root: &std::path::Path,
) {
    let upload_source = root.join("scp-upload-source.bin");
    let remote_file = root.join("scp-remote.bin");
    let download_target = root.join("scp-download-target.bin");
    let payload = b"PortMate OpenSSH SCP integration payload\n";
    fs::write(&upload_source, payload).unwrap();
    let remote_part = PathBuf::from(remote_resume_part_path(remote_file.to_str().unwrap()));
    fs::write(&remote_part, b"wrong-prefix").unwrap();
    let upload = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
            source: upload_source.display().to_string(),
            destination: format!("remote:{}", remote_file.display()),
        },
    )
    .await
    .unwrap();
    let upload = wait_for_transfer_terminal_state(state, &upload.id).await;
    assert_eq!(
        upload.status,
        TransferStatus::Completed,
        "SCP upload failed: {:?}",
        upload.message
    );
    assert_eq!(upload.bytes_done, payload.len() as u64);
    assert_eq!(fs::read(&remote_file).unwrap(), payload);
    assert!(!remote_part.exists());

    let download_part = local_resume_part_path(&download_target);
    fs::write(&download_part, &payload[..15]).unwrap();
    let download = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
            source: format!("remote:{}", remote_file.display()),
            destination: download_target.display().to_string(),
        },
    )
    .await
    .unwrap();
    let download = wait_for_transfer_terminal_state(state, &download.id).await;
    assert_eq!(
        download.status,
        TransferStatus::Completed,
        "SCP download failed: {:?}",
        download.message
    );
    assert_eq!(download.bytes_done, payload.len() as u64);
    assert_eq!(fs::read(&download_target).unwrap(), payload);
    assert!(!download_part.exists());

    let denied_target = format!("/proc/portmate-transfer-denied-{}.bin", Uuid::new_v4());
    for protocol in [TransferProtocol::Sftp, TransferProtocol::Scp] {
        let failed_upload = start_transfer_inner(
            state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: protocol.clone(),
                source: upload_source.display().to_string(),
                destination: format!("remote:{denied_target}"),
            },
        )
        .await
        .unwrap();
        let failed_upload = wait_for_transfer_terminal_state(state, &failed_upload.id).await;
        assert_eq!(
            failed_upload.status,
            TransferStatus::Failed,
            "{protocol:?} server-side write failure was not reported: {:?}",
            failed_upload.message
        );
        let message = failed_upload.message.unwrap_or_default();
        assert!(
            message.contains("SFTP") || message.contains("SCP"),
            "{protocol:?} failure lacked protocol context: {message}"
        );
        assert!(
            !state
                .transfer_cancellations
                .lock()
                .unwrap()
                .contains_key(&failed_upload.id),
            "{protocol:?} failed transfer retained its cancellation handle"
        );
    }

    {
        let mut store = state.store.lock().unwrap();
        let mut limited = store.profile(&profile.id).unwrap();
        limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
        store.upsert_profile(limited);
    }
    let cancel_source = root.join("sftp-cancel-source.bin");
    let cancel_remote = root.join("sftp-cancel-remote.bin");
    let cancel_remote_part =
        PathBuf::from(remote_resume_part_path(cancel_remote.to_str().unwrap()));
    // Keep enough limited payload remaining that a heavily loaded parallel test
    // runner cannot finish the transfer before the cancellation poll is scheduled.
    let cancel_payload = (0..2 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    fs::write(&cancel_source, &cancel_payload).unwrap();
    let cancelled_upload = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Sftp,
            source: cancel_source.display().to_string(),
            destination: format!("remote:{}", cancel_remote.display()),
        },
    )
    .await
    .unwrap();
    wait_for_transfer_progress(state, &cancelled_upload.id, "limited SFTP upload").await;
    let cancelling = cancel_transfer_inner(state, &cancelled_upload.id).unwrap();
    assert_eq!(cancelling.status, TransferStatus::Cancelled);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !state
                .transfer_cancellations
                .lock()
                .unwrap()
                .contains_key(&cancelled_upload.id)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("cancelled SFTP worker did not stop");
    let cancelled = state
        .store
        .lock()
        .unwrap()
        .transfer_by_id(&cancelled_upload.id)
        .unwrap();
    assert_eq!(cancelled.status, TransferStatus::Cancelled);
    assert!(!cancel_remote.exists());
    let partial_size = fs::metadata(&cancel_remote_part).unwrap().len();
    assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

    {
        let mut store = state.store.lock().unwrap();
        let mut unlimited = store.profile(&profile.id).unwrap();
        unlimited.transfer.rate_limit_bytes_per_second = None;
        store.upsert_profile(unlimited);
    }
    let retried = retry_transfer_inner(state, &cancelled_upload.id)
        .await
        .unwrap();
    let retried = wait_for_transfer_terminal_state(state, &retried.id).await;
    assert_eq!(
        retried.status,
        TransferStatus::Completed,
        "SFTP retry failed: {:?}",
        retried.message
    );
    assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
    assert_eq!(fs::read(&cancel_remote).unwrap(), cancel_payload);
    assert!(!cancel_remote_part.exists());

    {
        let mut store = state.store.lock().unwrap();
        let mut limited = store.profile(&profile.id).unwrap();
        limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
        store.upsert_profile(limited);
    }
    let scp_cancel_source = root.join("scp-cancel-source.bin");
    let scp_cancel_remote = root.join("scp-cancel-remote.bin");
    let scp_cancel_remote_part =
        PathBuf::from(remote_resume_part_path(scp_cancel_remote.to_str().unwrap()));
    fs::write(&scp_cancel_source, &cancel_payload).unwrap();
    let cancelled_scp_upload = start_transfer_inner(
        state,
        StartTransferRequest {
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Scp,
            source: scp_cancel_source.display().to_string(),
            destination: format!("remote:{}", scp_cancel_remote.display()),
        },
    )
    .await
    .unwrap();
    wait_for_transfer_progress(state, &cancelled_scp_upload.id, "limited SCP upload").await;
    let cancelling = cancel_transfer_inner(state, &cancelled_scp_upload.id).unwrap();
    assert_eq!(cancelling.status, TransferStatus::Cancelled);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !state
                .transfer_cancellations
                .lock()
                .unwrap()
                .contains_key(&cancelled_scp_upload.id)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("cancelled SCP worker did not stop");
    let cancelled = state
        .store
        .lock()
        .unwrap()
        .transfer_by_id(&cancelled_scp_upload.id)
        .unwrap();
    assert_eq!(cancelled.status, TransferStatus::Cancelled);
    assert!(!scp_cancel_remote.exists());
    let partial_size = fs::metadata(&scp_cancel_remote_part).unwrap().len();
    assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

    {
        let mut store = state.store.lock().unwrap();
        let mut unlimited = store.profile(&profile.id).unwrap();
        unlimited.transfer.rate_limit_bytes_per_second = None;
        store.upsert_profile(unlimited);
    }
    let retried = retry_transfer_inner(state, &cancelled_scp_upload.id)
        .await
        .unwrap();
    let retried = wait_for_transfer_terminal_state(state, &retried.id).await;
    assert_eq!(
        retried.status,
        TransferStatus::Completed,
        "SCP retry failed: {:?}",
        retried.message
    );
    assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
    assert_eq!(fs::read(&scp_cancel_remote).unwrap(), cancel_payload);
    assert!(!scp_cancel_remote_part.exists());

    for (label, protocol) in [
        ("sftp", TransferProtocol::Sftp),
        ("scp", TransferProtocol::Scp),
    ] {
        {
            let mut store = state.store.lock().unwrap();
            let mut limited = store.profile(&profile.id).unwrap();
            limited.transfer.rate_limit_bytes_per_second = Some(64 * 1024);
            store.upsert_profile(limited);
        }
        let disconnect_remote = root.join(format!("{label}-disconnect-remote.bin"));
        let disconnect_remote_part =
            PathBuf::from(remote_resume_part_path(disconnect_remote.to_str().unwrap()));
        let interrupted_upload = start_transfer_inner(
            state,
            StartTransferRequest {
                session_id: profile.id.clone(),
                protocol: protocol.clone(),
                source: cancel_source.display().to_string(),
                destination: format!("remote:{}", disconnect_remote.display()),
            },
        )
        .await
        .unwrap();
        wait_for_transfer_progress(
            state,
            &interrupted_upload.id,
            &format!("limited {label} upload"),
        )
        .await;

        let disconnected = close_session_inner(state, profile.id.clone())
            .await
            .unwrap();
        assert_eq!(disconnected.runtime.status, SessionStatus::Disconnected);
        let interrupted = wait_for_transfer_terminal_state(state, &interrupted_upload.id).await;
        assert_eq!(
            interrupted.status,
            TransferStatus::Failed,
            "{protocol:?} SSH disconnect was not reported as a failure: {:?}",
            interrupted.message
        );
        assert!(
            !state
                .transfer_cancellations
                .lock()
                .unwrap()
                .contains_key(&interrupted.id),
            "{protocol:?} disconnected transfer retained its cancellation handle"
        );
        assert!(!disconnect_remote.exists());
        let partial_size = fs::metadata(&disconnect_remote_part).unwrap().len();
        assert!(partial_size > 0 && partial_size < cancel_payload.len() as u64);

        let reopened = open_ssh_session(state, profile.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(reopened.runtime.status, SessionStatus::Connected);
        {
            let mut store = state.store.lock().unwrap();
            let mut unlimited = store.profile(&profile.id).unwrap();
            unlimited.transfer.rate_limit_bytes_per_second = None;
            store.upsert_profile(unlimited);
        }
        let retried = retry_transfer_inner(state, &interrupted_upload.id)
            .await
            .unwrap();
        let retried = wait_for_transfer_terminal_state(state, &retried.id).await;
        assert_eq!(
            retried.status,
            TransferStatus::Completed,
            "{protocol:?} retry after reconnect failed: {:?}",
            retried.message
        );
        assert_eq!(retried.bytes_done, cancel_payload.len() as u64);
        assert_eq!(fs::read(&disconnect_remote).unwrap(), cancel_payload);
        assert!(!disconnect_remote_part.exists());
    }
}

use super::*;

pub(super) const MODEM_SOH: u8 = 0x01;
pub(super) const MODEM_STX: u8 = 0x02;
pub(super) const MODEM_EOT: u8 = 0x04;
pub(super) const MODEM_ACK: u8 = 0x06;
pub(super) const MODEM_NAK: u8 = 0x15;
pub(super) const MODEM_CAN: u8 = 0x18;
pub(super) const MODEM_CRC_REQUEST: u8 = b'C';
pub(super) const MODEM_EOF: u8 = 0x1a;
pub(super) const MODEM_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const MODEM_ACK_TIMEOUT: Duration = Duration::from_secs(12);
pub(super) const REMOTE_MODEM_READY_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const MODEM_MAX_RETRIES: usize = 10;
pub(super) const XMODEM_BLOCK_SIZE: usize = 128;
pub(super) const YMODEM_BLOCK_SIZE: usize = 1024;

pub(super) async fn transfer_file_via_xmodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    if let Some(bytes) = transfer_file_to_device_modem(state, request, progress).await? {
        return Ok(bytes);
    }
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let source_size = local_transfer_source_size(&local_source)?;
            let remote_part = remote_resume_part_path(&remote_destination);
            let completion_token = Uuid::new_v4().simple().to_string();
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Xmodem,
                true,
                &remote_part,
            )
            .await?;
            let remote_started = remote_start.is_some();
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let bytes = xmodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                remote_started,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
                let command = xmodem_remote_finalize_command(
                    &remote_part,
                    &remote_destination,
                    source_size,
                    &completion_token,
                );
                let _ = send_text_inner(state.session_io(), request.session_id.clone(), command)
                    .await?;
                wait_for_xmodem_remote_completion(
                    &mut completion_receiver,
                    &completion_token,
                    progress,
                    &request.session_id,
                )
                .await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Xmodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let bytes = xmodem_receive_file(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
            }
            Ok(bytes)
        }
    }
}

pub(super) async fn transfer_file_via_ymodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    if let Some(bytes) = transfer_file_to_device_modem(state, request, progress).await? {
        return Ok(bytes);
    }
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_part = remote_resume_part_path(&remote_destination);
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Ymodem,
                true,
                &remote_part,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let receiver_destination = if remote_start.is_some() {
                remote_part.as_str()
            } else {
                remote_destination.as_str()
            };
            let bytes = ymodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                Some(receiver_destination),
                remote_start.is_some(),
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
                finalize_remote_modem_upload(
                    state,
                    &request.session_id,
                    &mut completion_receiver,
                    &remote_part,
                    &remote_destination,
                    progress,
                )
                .await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Ymodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let bytes = ymodem_receive_file(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
            }
            Ok(bytes)
        }
    }
}

pub(super) async fn transfer_file_via_zmodem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    if let Some(bytes) = transfer_file_to_device_modem(state, request, progress).await? {
        return Ok(bytes);
    }
    match modem_direction(request)? {
        ModemDirection::Upload {
            local_source,
            remote_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_part = remote_resume_part_path(&remote_destination);
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Zmodem,
                true,
                &remote_part,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let receiver_destination = if remote_start.is_some() {
                remote_part.as_str()
            } else {
                remote_destination.as_str()
            };
            let bytes = zmodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                Some(receiver_destination),
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
                finalize_remote_modem_upload(
                    state,
                    &request.session_id,
                    &mut completion_receiver,
                    &remote_part,
                    &remote_destination,
                    progress,
                )
                .await?;
            }
            Ok(bytes)
        }
        ModemDirection::Download {
            remote_source,
            local_destination,
        } => {
            let receiver = runtime_tap_receiver(state, &request.session_id)?;
            let mut completion_receiver = runtime_tap_receiver(state, &request.session_id)?;
            let remote_start = maybe_start_remote_modem(
                state,
                &request.session_id,
                TransferProtocol::Zmodem,
                false,
                &remote_source,
            )
            .await?;
            let reader = modem_reader_after_start(
                receiver,
                remote_start.as_ref(),
                progress,
                &request.session_id,
            )
            .await?;
            let bytes = zmodem_receive_files(
                state,
                &request.session_id,
                reader,
                &local_destination,
                progress,
            )
            .await?;
            if let Some(remote_start) = remote_start.as_ref() {
                wait_for_remote_modem_completion(
                    &mut completion_receiver,
                    remote_start,
                    progress,
                    &request.session_id,
                )
                .await?;
            }
            Ok(bytes)
        }
    }
}

pub(super) enum ModemDirection {
    Upload {
        local_source: String,
        remote_destination: String,
    },
    Download {
        remote_source: String,
        local_destination: String,
    },
}

pub(super) fn modem_direction(request: &StartTransferRequest) -> Result<ModemDirection, String> {
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);

    match (source_remote, destination_remote) {
        (None, Some(remote_destination)) => {
            validate_remote_transfer_path(remote_destination, "Modem 远端目标路径")?;
            if local_transfer_entry(Path::new(&request.source), "本地传输源")?.is_none() {
                return Err("本地传输源不存在".to_string());
            }
            Ok(ModemDirection::Upload {
                local_source: request.source.clone(),
                remote_destination: remote_destination.to_string(),
            })
        }
        (Some(remote_source), None) => {
            validate_remote_transfer_path(remote_source, "Modem 远端源路径")?;
            Ok(ModemDirection::Download {
                remote_source: remote_source.to_string(),
                local_destination: request.destination.clone(),
            })
        }
        (None, None) => {
            if local_transfer_entry(Path::new(&request.source), "本地传输源")?.is_some() {
                validate_remote_transfer_path(&request.destination, "Modem 远端目标路径")?;
                Ok(ModemDirection::Upload {
                    local_source: request.source.clone(),
                    remote_destination: request.destination.clone(),
                })
            } else {
                Err("Modem transfer expects local -> remote:path upload or remote:path -> local download".to_string())
            }
        }
        _ => Err(
            "Modem transfer expects local -> remote:path upload or remote:path -> local download"
                .to_string(),
        ),
    }
}

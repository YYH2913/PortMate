use super::*;

pub(super) async fn ymodem_send_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    remote_destination: Option<&str>,
    auto_remote_receiver: bool,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut file, total) = open_local_transfer_source(Path::new(local_source), "YModem 本地源")?;
    if !modem_wait_for_receiver(&mut reader).await? {
        return Err("YModem receiver did not request CRC mode".to_string());
    }

    let (_, remote_name) = remote_destination
        .map(remote_parent_and_file_name)
        .unwrap_or_else(|| ("".to_string(), local_file_name(local_source)));
    let name = if remote_name.is_empty() {
        local_file_name(local_source)
    } else {
        remote_name
    };
    let metadata = ymodem_metadata_block(&name, total)?;
    modem_send_packet_with_retries(
        state,
        session_id,
        &mut reader,
        MODEM_SOH,
        0,
        &metadata,
        true,
    )
    .await
    .map_err(|error| format!("YModem metadata block failed: {error}"))?;
    let _ = modem_wait_for_crc_request(&mut reader, Duration::from_secs(10)).await;

    let mut block_no = 1_u8;
    let mut bytes_done = 0_u64;
    let mut buffer = [0_u8; YMODEM_BLOCK_SIZE];
    while bytes_done < total {
        check_modem_cancelled(state, session_id, progress).await?;
        let limit = (total - bytes_done).min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..limit])
            .map_err(|error| format!("读取 YModem 本地文件失败: {error}"))?;
        if read == 0 {
            return Err("YModem 本地文件在传输期间提前结束".to_string());
        }
        modem_send_packet_with_retries(
            state,
            session_id,
            &mut reader,
            MODEM_STX,
            block_no,
            &buffer[..read],
            true,
        )
        .await
        .map_err(|error| format!("YModem data block {block_no} failed: {error}"))?;
        bytes_done += read as u64;
        progress.update(bytes_done, total).await?;
        block_no = block_no.wrapping_add(1);
    }
    ensure_local_transfer_source_size(&file, total, "YModem 本地源")?;
    modem_finish_eot(state, session_id, &mut reader)
        .await
        .map_err(|error| format!("YModem EOT handshake failed: {error}"))?;
    let _ = modem_wait_for_crc_request(&mut reader, Duration::from_secs(10)).await;
    if auto_remote_receiver {
        modem_finish_auto_remote_ymodem_batch(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("YModem final empty block failed: {error}"))?;
    } else {
        let empty = vec![0_u8; XMODEM_BLOCK_SIZE];
        modem_send_packet_with_retries(state, session_id, &mut reader, MODEM_SOH, 0, &empty, true)
            .await
            .map_err(|error| format!("YModem final empty block failed: {error}"))?;
    }
    Ok(bytes_done)
}

pub(super) async fn modem_finish_auto_remote_ymodem_batch(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    let empty = vec![0_u8; XMODEM_BLOCK_SIZE];
    let packet = modem_packet_bytes(MODEM_SOH, 0, &empty, XMODEM_BLOCK_SIZE, true);
    for _ in 0..3 {
        write_runtime_bytes(state, session_id, &packet).await?;
        match modem_wait_for_ack(reader, Duration::from_secs(2)).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Err("remote rejected final YModem empty block".to_string())
}

pub(super) async fn ymodem_receive_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let marker = modem_wait_for_packet_marker(state, session_id, &mut reader).await?;
    if marker == MODEM_EOT {
        return Err("YModem sender ended before metadata block".to_string());
    }
    let metadata = modem_read_packet(&mut reader, marker).await?;
    if metadata.block_no != 0 {
        return Err("YModem metadata block missing".to_string());
    }
    let (name, expected_size) = parse_ymodem_metadata(&metadata.data);
    if name.is_empty() {
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
        return Err("YModem sender sent empty batch".to_string());
    }
    write_runtime_bytes(state, session_id, &[MODEM_ACK, MODEM_CRC_REQUEST]).await?;

    let destination = ymodem_local_target_path(local_destination, &name)?;
    let mut expected = 1_u8;
    let mut output = PendingLocalTransferOutput::create(&destination, "YModem 本地目标文件")?;
    let mut trailing_padding = 0_u64;
    let mut bytes_received = 0_u64;
    let mut bytes_written = 0_u64;
    let total = expected_size.unwrap_or(0) as u64;
    loop {
        check_modem_cancelled(state, session_id, progress).await?;
        let marker = modem_wait_for_next_marker(&mut reader, Duration::from_secs(15)).await?;
        if marker == MODEM_EOT {
            write_runtime_bytes(state, session_id, &[MODEM_ACK, MODEM_CRC_REQUEST]).await?;
            if let Ok(final_marker) =
                modem_wait_for_next_marker(&mut reader, Duration::from_secs(5)).await
            {
                if final_marker != MODEM_EOT {
                    let final_packet = modem_read_packet(&mut reader, final_marker).await?;
                    if final_packet.block_no == 0 {
                        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
                    }
                }
            }
            break;
        }
        let packet = modem_read_packet(&mut reader, marker).await?;
        if packet.block_no == expected {
            if let Some(expected_size) = expected_size {
                append_modem_data_with_size_limit(
                    &mut output,
                    &packet.data,
                    expected_size as u64,
                    &mut bytes_written,
                )?;
            } else {
                append_modem_data_without_trailing_padding(
                    &mut output,
                    &packet.data,
                    &mut trailing_padding,
                    &mut bytes_written,
                )?;
            }
            bytes_received = bytes_received
                .checked_add(packet.data.len() as u64)
                .ok_or_else(|| "YModem 接收字节数溢出".to_string())?;
            progress.update(bytes_received, total).await?;
            expected = expected.wrapping_add(1);
        }
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
    }

    output.finish()?;
    Ok(bytes_written)
}

pub(super) fn ymodem_metadata_block(name: &str, total: u64) -> Result<Vec<u8>, String> {
    if name.contains('\0') {
        return Err("YModem 文件名不能包含 NUL".to_string());
    }
    let metadata_text = format!("{name}\0{total} ");
    if metadata_text.len() > XMODEM_BLOCK_SIZE {
        return Err(format!(
            "YModem 文件名和大小元数据超过 {XMODEM_BLOCK_SIZE} 字节，无法无损编码"
        ));
    }
    let mut metadata = vec![0_u8; XMODEM_BLOCK_SIZE];
    metadata[..metadata_text.len()].copy_from_slice(metadata_text.as_bytes());
    Ok(metadata)
}

pub(super) fn ymodem_local_target_path(
    local_destination: &str,
    incoming_name: &str,
) -> Result<PathBuf, String> {
    if local_destination.trim().is_empty() {
        return Err("YModem 本地目标路径不能为空".to_string());
    }
    let destination = PathBuf::from(local_destination);
    let ends_with_separator = local_destination.ends_with('/') || local_destination.ends_with('\\');
    if destination.is_dir() || ends_with_separator {
        let safe_name =
            portable_file_name(incoming_name).unwrap_or_else(|| "ymodem-file.bin".to_string());
        Ok(destination.join(safe_name))
    } else {
        Ok(destination)
    }
}

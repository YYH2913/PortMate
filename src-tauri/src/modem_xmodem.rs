use super::*;

pub(super) async fn xmodem_send_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    auto_remote_receiver: bool,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut file, total) = open_local_transfer_source(Path::new(local_source), "XModem 本地源")?;
    let crc = modem_wait_for_receiver(&mut reader).await?;
    let mut block_no = 1_u8;
    let mut bytes_done = 0_u64;
    let mut buffer = [0_u8; XMODEM_BLOCK_SIZE];

    while bytes_done < total {
        check_modem_cancelled(state, session_id, progress).await?;
        let limit = (total - bytes_done).min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..limit])
            .map_err(|error| format!("读取 XModem 本地文件失败: {error}"))?;
        if read == 0 {
            return Err("XModem 本地文件在传输期间提前结束".to_string());
        }
        modem_send_packet_with_retries(
            state,
            session_id,
            &mut reader,
            MODEM_SOH,
            block_no,
            &buffer[..read],
            crc,
        )
        .await
        .map_err(|error| format!("XModem data block {block_no} failed: {error}"))?;
        bytes_done += read as u64;
        progress.update(bytes_done, total).await?;
        block_no = block_no.wrapping_add(1);
    }
    ensure_local_transfer_source_size(&file, total, "XModem 本地源")?;
    if auto_remote_receiver {
        modem_finish_auto_remote_xmodem(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("XModem EOT handshake failed: {error}"))?;
    } else {
        modem_finish_eot(state, session_id, &mut reader)
            .await
            .map_err(|error| format!("XModem EOT handshake failed: {error}"))?;
    }
    Ok(bytes_done)
}

pub(super) async fn modem_finish_auto_remote_xmodem(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    for _ in 0..3 {
        write_runtime_bytes(state, session_id, &[MODEM_EOT]).await?;
        match modem_wait_for_ack(reader, Duration::from_secs(2)).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Err("remote did not ACK modem EOT".to_string())
}

pub(super) async fn xmodem_receive_file(
    state: &AppState,
    session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut expected = 1_u8;
    let mut output =
        PendingLocalTransferOutput::create(Path::new(local_destination), "XModem 本地目标文件")?;
    let mut trailing_padding = 0_u64;
    let mut bytes_received = 0_u64;
    let mut bytes_written = 0_u64;
    let mut first_packet = true;

    loop {
        check_modem_cancelled(state, session_id, progress).await?;
        let marker = if first_packet {
            first_packet = false;
            modem_wait_for_packet_marker(state, session_id, &mut reader).await?
        } else {
            modem_wait_for_next_marker(&mut reader, Duration::from_secs(15)).await?
        };
        if marker == MODEM_EOT {
            write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
            break;
        }
        let packet = match modem_read_packet(&mut reader, marker).await {
            Ok(packet) => packet,
            Err(error) => {
                write_runtime_bytes(state, session_id, &[MODEM_NAK]).await?;
                return Err(error);
            }
        };
        if packet.block_no == expected {
            append_modem_data_without_trailing_padding(
                &mut output,
                &packet.data,
                &mut trailing_padding,
                &mut bytes_written,
            )?;
            bytes_received = bytes_received
                .checked_add(packet.data.len() as u64)
                .ok_or_else(|| "XModem 接收字节数溢出".to_string())?;
            progress.update(bytes_received, 0).await?;
            expected = expected.wrapping_add(1);
        }
        write_runtime_bytes(state, session_id, &[MODEM_ACK]).await?;
    }

    output.finish()?;
    Ok(bytes_written)
}

use super::*;

pub(super) struct ModemPacket {
    pub(super) block_no: u8,
    pub(super) data: Vec<u8>,
}

pub(super) async fn modem_wait_for_receiver(reader: &mut ModemByteReader) -> Result<bool, String> {
    let started = Instant::now();
    loop {
        let remaining = Duration::from_secs(45).saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("modem receiver did not send NAK/C within 45s".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(3)))
            .await
        {
            Ok(MODEM_CRC_REQUEST) => return Ok(true),
            Ok(MODEM_NAK) => return Ok(false),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn modem_wait_for_crc_request(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<(), String> {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("timed out waiting for YModem CRC request".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(2)))
            .await
        {
            Ok(MODEM_CRC_REQUEST) => return Ok(()),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn modem_send_packet_with_retries(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
    marker: u8,
    block_no: u8,
    payload: &[u8],
    crc: bool,
) -> Result<(), String> {
    let size = if marker == MODEM_STX {
        YMODEM_BLOCK_SIZE
    } else {
        XMODEM_BLOCK_SIZE
    };
    let packet = modem_packet_bytes(marker, block_no, payload, size, crc);
    modem_send_packet_bytes_with_retries(
        state,
        session_id,
        reader,
        block_no,
        &packet,
        MODEM_ACK_TIMEOUT,
    )
    .await
}

pub(super) async fn modem_send_packet_bytes_with_retries(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
    block_no: u8,
    packet: &[u8],
    ack_timeout: Duration,
) -> Result<(), String> {
    for _ in 0..MODEM_MAX_RETRIES {
        write_runtime_bytes(state, session_id, packet).await?;
        match modem_wait_for_ack(reader, ack_timeout).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Err(format!(
        "modem block {block_no} was not acknowledged after {MODEM_MAX_RETRIES} attempts"
    ))
}

pub(super) fn modem_packet_bytes(
    marker: u8,
    block_no: u8,
    payload: &[u8],
    size: usize,
    crc: bool,
) -> Vec<u8> {
    let mut data = vec![MODEM_EOF; size];
    data[..payload.len().min(size)].copy_from_slice(&payload[..payload.len().min(size)]);
    let mut packet = Vec::with_capacity(3 + size + if crc { 2 } else { 1 });
    packet.push(marker);
    packet.push(block_no);
    packet.push(255_u8.wrapping_sub(block_no));
    packet.extend_from_slice(&data);
    if crc {
        let crc = crc16_xmodem(&data);
        packet.extend_from_slice(&crc.to_be_bytes());
    } else {
        packet.push(data.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte)));
    }
    packet
}

pub(super) enum ModemAck {
    Ack,
    Nak,
}

pub(super) async fn modem_wait_for_ack(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<ModemAck, String> {
    let started = Instant::now();
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err("timed out waiting for modem ACK".to_string());
        }
        match reader
            .next_byte(remaining.min(Duration::from_secs(2)))
            .await
        {
            Ok(MODEM_ACK) => return Ok(ModemAck::Ack),
            Ok(MODEM_NAK) => return Ok(ModemAck::Nak),
            Ok(MODEM_CAN) => return Err("modem transfer cancelled by remote".to_string()),
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
}

pub(super) async fn modem_finish_eot(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<(), String> {
    modem_finish_eot_with_timeout(state, session_id, reader, MODEM_ACK_TIMEOUT).await
}

pub(super) async fn modem_finish_eot_with_timeout(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
    ack_timeout: Duration,
) -> Result<(), String> {
    for _ in 0..MODEM_MAX_RETRIES {
        write_runtime_bytes(state, session_id, &[MODEM_EOT]).await?;
        match modem_wait_for_ack(reader, ack_timeout).await {
            Ok(ModemAck::Ack) => return Ok(()),
            Ok(ModemAck::Nak) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Err(format!(
        "remote did not ACK modem EOT after {MODEM_MAX_RETRIES} attempts"
    ))
}

pub(super) async fn modem_wait_for_packet_marker(
    state: &AppState,
    session_id: &str,
    reader: &mut ModemByteReader,
) -> Result<u8, String> {
    for _ in 0..24 {
        write_runtime_bytes(state, session_id, &[MODEM_CRC_REQUEST]).await?;
        match modem_wait_for_next_marker(reader, Duration::from_secs(3)).await {
            Ok(marker) => return Ok(marker),
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }
    }
    Err("timed out waiting for modem packet".to_string())
}

pub(super) async fn modem_wait_for_next_marker(
    reader: &mut ModemByteReader,
    timeout: Duration,
) -> Result<u8, String> {
    loop {
        match reader.next_byte(timeout).await? {
            MODEM_SOH => return Ok(MODEM_SOH),
            MODEM_STX => return Ok(MODEM_STX),
            MODEM_EOT => return Ok(MODEM_EOT),
            MODEM_CAN => return Err("modem transfer cancelled by remote".to_string()),
            _ => {}
        }
    }
}

pub(super) async fn modem_read_packet(
    reader: &mut ModemByteReader,
    marker: u8,
) -> Result<ModemPacket, String> {
    let size = match marker {
        MODEM_SOH => XMODEM_BLOCK_SIZE,
        MODEM_STX => YMODEM_BLOCK_SIZE,
        _ => return Err(format!("unexpected modem packet marker: {marker}")),
    };
    let header = reader.read_exact(2, Duration::from_secs(5)).await?;
    let block_no = header[0];
    let inverse = header[1];
    if block_no != 255_u8.wrapping_sub(inverse) {
        return Err("modem packet block number check failed".to_string());
    }
    let mut data = reader.read_exact(size + 2, Duration::from_secs(8)).await?;
    let received_crc = u16::from_be_bytes([data[size], data[size + 1]]);
    data.truncate(size);
    let actual_crc = crc16_xmodem(&data);
    if received_crc != actual_crc {
        return Err(format!(
            "modem packet CRC mismatch: received={received_crc:04x} actual={actual_crc:04x}"
        ));
    }
    Ok(ModemPacket { block_no, data })
}

pub(super) fn crc16_xmodem(data: &[u8]) -> u16 {
    let mut crc = 0_u16;
    for byte in data {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
pub(super) fn write_local_transfer_file(path: &str, data: &[u8]) -> Result<(), String> {
    let mut output = PendingLocalTransferOutput::create(Path::new(path), "本地传输目标路径")?;
    output
        .file_mut()?
        .write_all(data)
        .map_err(|error| format!("写入本地文件失败: {error}"))?;
    output.finish()
}

pub(super) struct PendingLocalTransferOutput {
    target: PathBuf,
    pub(super) temp: PathBuf,
    file: Option<fs::File>,
    finished: bool,
}

impl PendingLocalTransferOutput {
    pub(super) fn create(target: &Path, label: &str) -> Result<Self, String> {
        prepare_local_transfer_target_path(target, label)?;
        let (file, temp) = open_new_local_transfer_file(target)?;
        Ok(Self {
            target: target.to_path_buf(),
            temp,
            file: Some(file),
            finished: false,
        })
    }

    pub(super) fn file_mut(&mut self) -> Result<&mut fs::File, String> {
        self.file
            .as_mut()
            .ok_or_else(|| "本地传输临时文件已关闭".to_string())
    }

    pub(super) fn finish(mut self) -> Result<(), String> {
        let file = self
            .file
            .take()
            .ok_or_else(|| "本地传输临时文件已关闭".to_string())?;
        file.sync_all()
            .map_err(|error| format!("写入本地文件失败: {error}"))?;
        drop(file);
        finalize_local_resume_file(&self.temp, &self.target)?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for PendingLocalTransferOutput {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.file.take();
        let _ = fs::remove_file(&self.temp);
    }
}

pub(super) fn append_modem_data_without_trailing_padding(
    output: &mut PendingLocalTransferOutput,
    data: &[u8],
    trailing_padding: &mut u64,
    bytes_written: &mut u64,
) -> Result<(), String> {
    let Some(last_content) = data.iter().rposition(|byte| *byte != MODEM_EOF) else {
        *trailing_padding = trailing_padding
            .checked_add(data.len() as u64)
            .ok_or_else(|| "Modem 填充字节数溢出".to_string())?;
        return Ok(());
    };

    if *trailing_padding > 0 {
        write_modem_padding(output.file_mut()?, *trailing_padding)
            .map_err(|error| format!("写入本地 Modem 文件失败: {error}"))?;
        *bytes_written = bytes_written
            .checked_add(*trailing_padding)
            .ok_or_else(|| "Modem 写入字节数溢出".to_string())?;
        *trailing_padding = 0;
    }

    let trailing_count = data.len().saturating_sub(last_content.saturating_add(1)) as u64;
    let data = &data[..=last_content];
    output
        .file_mut()?
        .write_all(data)
        .map_err(|error| format!("写入本地 Modem 文件失败: {error}"))?;
    *bytes_written = bytes_written
        .checked_add(data.len() as u64)
        .ok_or_else(|| "Modem 写入字节数溢出".to_string())?;
    *trailing_padding = trailing_count;
    Ok(())
}

pub(super) fn append_modem_data_with_size_limit(
    output: &mut PendingLocalTransferOutput,
    data: &[u8],
    limit: u64,
    bytes_written: &mut u64,
) -> Result<(), String> {
    let remaining = limit.saturating_sub(*bytes_written);
    let count = remaining.min(data.len() as u64) as usize;
    if count == 0 {
        return Ok(());
    }
    output
        .file_mut()?
        .write_all(&data[..count])
        .map_err(|error| format!("写入本地 Modem 文件失败: {error}"))?;
    *bytes_written = bytes_written
        .checked_add(count as u64)
        .ok_or_else(|| "Modem 写入字节数溢出".to_string())?;
    Ok(())
}

pub(super) fn write_modem_padding(file: &mut fs::File, count: u64) -> std::io::Result<()> {
    let padding = [MODEM_EOF; 1024];
    let mut remaining = count;
    while remaining > 0 {
        let count = remaining.min(padding.len() as u64) as usize;
        file.write_all(&padding[..count])?;
        remaining -= count as u64;
    }
    Ok(())
}
pub(super) fn parse_ymodem_metadata(data: &[u8]) -> (String, Option<usize>) {
    let name_end = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let name = String::from_utf8_lossy(&data[..name_end]).to_string();
    let rest = if name_end < data.len() {
        &data[name_end + 1..]
    } else {
        &[]
    };
    let rest = String::from_utf8_lossy(rest);
    let size = rest
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<usize>().ok());
    (name, size)
}

pub(super) fn local_file_name(path: &str) -> String {
    portable_file_name(path).unwrap_or_else(|| "portmate-transfer.bin".to_string())
}

pub(super) fn remote_parent_and_file_name(path: &str) -> (String, String) {
    let normalized = path.trim_end_matches(['/', '\\']);
    let Some((index, separator)) = normalized
        .char_indices()
        .rev()
        .find(|(_, character)| matches!(character, '/' | '\\'))
    else {
        return (
            String::new(),
            portable_file_name(normalized).unwrap_or_default(),
        );
    };
    let parent = if index == 0 {
        "/".to_string()
    } else {
        normalized[..index].to_string()
    };
    let name_start = index + separator.len_utf8();
    let name = portable_file_name(&normalized[name_start..]).unwrap_or_default();
    (parent, name)
}
pub(super) fn is_modem_timeout(error: &str) -> bool {
    error.contains("timeout") || error.contains("timed out")
}

use super::*;
use zmodem2::{Action, Event, FileInfo, Position};

pub(super) async fn zmodem_send_file(
    state: &AppState, _session_id: &str, mut reader: ModemByteReader,
    local_source: &str, remote_destination: Option<&str>, progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut file, total) = open_local_transfer_source(Path::new(local_source), "ZModem")?;
    let size = u32::try_from(total).map_err(|_| "ZModem 当前状态机只支持 4 GiB 以内的单文件".to_string())?;
    let (_, remote_name) = remote_destination.map(remote_parent_and_file_name)
        .unwrap_or_else(|| ("".to_string(), local_file_name(local_source)));
    let file_name = if remote_name.is_empty() { local_file_name(local_source) } else { remote_name };
    let mut sender = zmodem2::Sender::new().map_err(zmodem_error)?;
    sender.start_file(FileInfo::new(file_name.as_bytes(), Some(Position::new(size)))).map_err(zmodem_error)?;
    let mut input = Vec::new();
    let mut file_buf = [0_u8; 1024];
    let mut session_done = false;
    let mut bytes_done = 0_u64;
    let mut last_progress = Instant::now();
    loop {
        check_modem_cancelled(state, &reader, progress).await?;
        match sender.poll() {
            Action::WriteWire(bytes) => {
                let bytes = bytes.to_vec();
                reader.write_runtime_bytes(state, &bytes).await?;
                sender.wire_written(bytes.len());
            }
            Action::ReadFile { offset, max_len } => {
                file.seek(std::io::SeekFrom::Start(u64::from(offset.get())))
                    .map_err(|e| format!("ZModem 本地文件 seek 失败: {e}"))?;
                let len = max_len.min(file_buf.len());
                let read = file.read(&mut file_buf[..len]).map_err(|e| format!("ZModem 读取文件失败: {e}"))?;
                if read == 0 && max_len > 0 { return Err("ZModem 本地文件提前结束".into()); }
                sender.submit_file(&file_buf[..read]).map_err(zmodem_error)?;
                bytes_done = bytes_done.max(u64::from(offset.get()) + read as u64);
                progress.update(bytes_done.min(total), total).await?;
            }
            Action::Event(Event::FileCompleted) => sender.finish().map_err(zmodem_error)?,
            Action::Event(Event::SessionCompleted) => session_done = true,
            Action::Event(Event::Aborted) => return Err("ZModem 远端取消发送".into()),
            Action::Idle if session_done => return Ok(total),
            Action::Idle => {
                read_zmodem_input(&mut reader, &mut input, last_progress).await?;
                if !input.is_empty() {
                    let consumed = sender.submit_wire(&input).map_err(zmodem_error)?;
                    if consumed > 0 {
                        input.drain(..consumed);
                        last_progress = Instant::now();
                    } else { tokio::time::sleep(Duration::from_millis(5)).await; }
                }
                continue;
            }
            _ => return Err("ZModem sender 返回意外协议动作".into()),
        }
        last_progress = Instant::now();
    }
}

pub(super) async fn zmodem_receive_files(
    state: &AppState, _session_id: &str, mut reader: ModemByteReader,
    local_destination: &str, progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut receiver = zmodem2::Receiver::new().map_err(zmodem_error)?;
    let mut input = Vec::new();
    let mut current_file: Option<(fs::File, PathBuf, PathBuf)> = None;
    let mut received_files = 0;
    let mut bytes_done = 0;
    let mut session_done = false;
    let mut last_progress = Instant::now();
    loop {
        check_modem_cancelled(state, &reader, progress).await?;
        match receiver.poll() {
            Action::WriteWire(bytes) => {
                let bytes = bytes.to_vec();
                reader.write_runtime_bytes(state, &bytes).await?;
                receiver.wire_written(bytes.len());
            }
            Action::Event(Event::FileStarted(info)) => {
                if current_file.is_some() { return Err("ZModem 前一个文件尚未完成".into()); }
                let incoming = String::from_utf8_lossy(info.name);
                let target = zmodem_local_target_path(local_destination, &incoming, received_files)?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| format!("创建 ZModem 本地目录失败: {e}"))?;
                }
                let (file, temp) = open_new_local_transfer_file(&target)?;
                current_file = Some((file, target, temp));
            }
            Action::WriteFile(bytes) => {
                let Some((file, path, _)) = current_file.as_mut() else {
                    return Err("ZModem 收到文件数据但还没有文件头".into());
                };
                file.write_all(bytes).map_err(|e| format!("写入 ZModem 本地文件失败 {}: {e}", path.display()))?;
                let length = bytes.len();
                receiver.file_written(length).map_err(zmodem_error)?;
                bytes_done += length as u64;
                progress.update(bytes_done, 0).await?;
            }
            Action::Event(Event::FileCompleted) => {
                let Some((mut file, target, temp)) = current_file.take() else {
                    return Err("ZModem 缺少待完成文件".into());
                };
                file.flush().map_err(|e| format!("刷新 ZModem 本地文件失败: {e}"))?;
                drop(file);
                finalize_local_resume_file(&temp, &target)?;
                received_files += 1;
            }
            Action::Event(Event::SessionCompleted) => session_done = true,
            Action::Event(Event::Aborted) => return Err("ZModem 远端取消接收".into()),
            Action::Idle if session_done => {
                if current_file.is_some() { return Err("ZModem 会话结束但文件未完成".into()); }
                return Ok(bytes_done);
            }
            Action::Idle => {
                read_zmodem_input(&mut reader, &mut input, last_progress).await?;
                if !input.is_empty() {
                    let consumed = receiver.submit_wire(&input).map_err(zmodem_error)?;
                    if consumed > 0 {
                        input.drain(..consumed);
                        last_progress = Instant::now();
                    } else { tokio::time::sleep(Duration::from_millis(5)).await; }
                }
                continue;
            }
            _ => return Err("ZModem receiver 返回意外协议动作".into()),
        }
        last_progress = Instant::now();
    }
}

fn zmodem_error(error: zmodem2::Error) -> String { format!("ZModem 协议处理失败: {error}") }

async fn read_zmodem_input(reader: &mut ModemByteReader, input: &mut Vec<u8>, last_progress: Instant) -> Result<(), String> {
    if last_progress.elapsed() > Duration::from_secs(90) { return Err("ZModem idle timeout".into()); }
    // Drain buffered input before reading again to bound memory under backpressure.
    if !input.is_empty() { return Ok(()); }
    match reader.next_chunk(Duration::from_millis(30), 4096).await {
        Ok(bytes) => input.extend_from_slice(&bytes),
        Err(error) if is_modem_timeout(&error) => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

pub(super) fn zmodem_local_target_path(
    local_destination: &str,
    incoming_name: &str,
    received_files: usize,
) -> Result<PathBuf, String> {
    if local_destination.trim().is_empty() {
        return Err("ZModem 本地目标路径不能为空".to_string());
    }
    let incoming =
        portable_file_name(incoming_name).unwrap_or_else(|| "zmodem-file.bin".to_string());
    let base = expand_identity_path(local_destination);
    let ends_with_separator = local_destination.ends_with('/') || local_destination.ends_with('\\');

    if base.is_dir() || ends_with_separator {
        return Ok(base.join(incoming));
    }
    if received_files == 0 {
        return Ok(base);
    }
    Ok(base
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .join(incoming))
}

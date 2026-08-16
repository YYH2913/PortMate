use super::*;

pub(super) async fn zmodem_send_file(
    state: &AppState,
    _session_id: &str,
    mut reader: ModemByteReader,
    local_source: &str,
    remote_destination: Option<&str>,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut file, total) = open_local_transfer_source(Path::new(local_source), "ZModem")?;
    let size = u32::try_from(total)
        .map_err(|_| "ZModem 当前状态机只支持 4 GiB 以内的单文件".to_string())?;
    let (_, remote_name) = remote_destination
        .map(remote_parent_and_file_name)
        .unwrap_or_else(|| ("".to_string(), local_file_name(local_source)));
    let file_name = if remote_name.is_empty() {
        local_file_name(local_source)
    } else {
        remote_name
    };

    let mut sender =
        zmodem2::Sender::new().map_err(|error| format!("ZModem sender 初始化失败: {error}"))?;
    sender
        .start_file(file_name.as_bytes(), size)
        .map_err(|error| format!("ZModem 文件发送启动失败: {error}"))?;

    let mut input_buf = Vec::<u8>::new();
    let mut input_offset = 0_usize;
    let mut file_buf = vec![0_u8; 1024];
    let mut session_done = false;
    let mut last_progress = Instant::now();
    let mut bytes_done = 0_u64;

    while !session_done || !sender.drain_outgoing().is_empty() {
        check_modem_cancelled(state, &reader, progress).await?;
        let mut progressed = false;

        let outgoing = sender.drain_outgoing().to_vec();
        if !outgoing.is_empty() {
            reader.write_runtime_bytes(state, &outgoing).await?;
            sender.advance_outgoing(outgoing.len());
            progressed = true;
        }

        if let Some(request) = sender.poll_file() {
            file.seek(std::io::SeekFrom::Start(u64::from(request.offset)))
                .map_err(|error| format!("ZModem 本地文件 seek 失败: {error}"))?;
            let read_len = request.len.min(file_buf.len());
            let read = file
                .read(&mut file_buf[..read_len])
                .map_err(|error| format!("ZModem 读取本地文件失败: {error}"))?;
            if read == 0 && request.len > 0 {
                return Err("ZModem 本地文件提前结束".to_string());
            }
            sender
                .feed_file(&file_buf[..read])
                .map_err(|error| format!("ZModem 发送文件块失败: {error}"))?;
            bytes_done = bytes_done.max(u64::from(request.offset) + read as u64);
            progress
                .update(bytes_done.min(u64::from(size)), u64::from(size))
                .await?;
            progressed = true;
        }

        match reader.next_chunk(Duration::from_millis(30), 4096).await {
            Ok(bytes) if !bytes.is_empty() => {
                input_buf.extend_from_slice(&bytes);
                progressed = true;
            }
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }

        if sender.drain_outgoing().is_empty() && input_offset < input_buf.len() {
            let consumed = sender
                .feed_incoming(&input_buf[input_offset..])
                .map_err(|error| format!("ZModem 接收远端响应失败: {error}"))?;
            if consumed > 0 {
                input_offset += consumed;
                progressed = true;
                if input_offset == input_buf.len() {
                    input_buf.clear();
                    input_offset = 0;
                } else if input_offset > 4096 {
                    input_buf.drain(..input_offset);
                    input_offset = 0;
                }
            }
        }

        if let Some(event) = sender.poll_event() {
            match event {
                zmodem2::SenderEvent::FileComplete => {
                    sender
                        .finish_session()
                        .map_err(|error| format!("ZModem 结束会话失败: {error}"))?;
                }
                zmodem2::SenderEvent::SessionComplete => {
                    session_done = true;
                }
            }
            progressed = true;
        }

        if progressed {
            last_progress = Instant::now();
        } else if last_progress.elapsed() > Duration::from_secs(90) {
            return Err("ZModem upload idle timeout".to_string());
        } else {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    Ok(u64::from(size))
}

pub(super) async fn zmodem_receive_files(
    state: &AppState,
    _session_id: &str,
    mut reader: ModemByteReader,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let mut modem_receiver =
        zmodem2::Receiver::new().map_err(|error| format!("ZModem receiver 初始化失败: {error}"))?;
    let mut input_buf = Vec::<u8>::new();
    let mut input_offset = 0_usize;
    let mut current_file: Option<(fs::File, PathBuf, PathBuf)> = None;
    let mut received_files = 0_usize;
    let mut bytes_done = 0_u64;
    let mut session_done = false;
    let mut last_progress = Instant::now();

    while !session_done || !modem_receiver.drain_outgoing().is_empty() {
        check_modem_cancelled(state, &reader, progress).await?;
        let mut progressed = false;

        let outgoing = modem_receiver.drain_outgoing().to_vec();
        if !outgoing.is_empty() {
            reader.write_runtime_bytes(state, &outgoing).await?;
            modem_receiver.advance_outgoing(outgoing.len());
            progressed = true;
        }

        while let Some(event) = modem_receiver.poll_event() {
            match event {
                zmodem2::ReceiverEvent::FileStart => {
                    let incoming = String::from_utf8_lossy(modem_receiver.file_name()).to_string();
                    let target =
                        zmodem_local_target_path(local_destination, &incoming, received_files)?;
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent).map_err(|error| {
                            format!("创建 ZModem 本地目录失败 {}: {error}", parent.display())
                        })?;
                    }
                    let (file, temp) = open_new_local_transfer_file(&target)?;
                    current_file = Some((file, target, temp));
                }
                zmodem2::ReceiverEvent::FileComplete => {
                    if let Some((mut file, target, temp)) = current_file.take() {
                        file.flush()
                            .map_err(|error| format!("刷新 ZModem 本地文件失败: {error}"))?;
                        drop(file);
                        finalize_local_resume_file(&temp, &target)?;
                    }
                    received_files += 1;
                }
                zmodem2::ReceiverEvent::SessionComplete => {
                    session_done = true;
                }
            }
            progressed = true;
        }

        let file_data = modem_receiver.drain_file().to_vec();
        if !file_data.is_empty() {
            let Some((file, path, _)) = current_file.as_mut() else {
                return Err("ZModem 收到文件数据但还没有文件头".to_string());
            };
            file.write_all(&file_data)
                .map_err(|error| format!("写入 ZModem 本地文件失败 {}: {error}", path.display()))?;
            modem_receiver
                .advance_file(file_data.len())
                .map_err(|error| format!("ZModem 文件写入确认失败: {error}"))?;
            bytes_done += file_data.len() as u64;
            progress.update(bytes_done, 0).await?;
            progressed = true;
        }

        match reader.next_chunk(Duration::from_millis(30), 4096).await {
            Ok(bytes) if !bytes.is_empty() => {
                input_buf.extend_from_slice(&bytes);
                progressed = true;
            }
            Ok(_) => {}
            Err(error) if is_modem_timeout(&error) => {}
            Err(error) => return Err(error),
        }

        if modem_receiver.drain_outgoing().is_empty()
            && modem_receiver.drain_file().is_empty()
            && input_offset < input_buf.len()
        {
            let consumed = modem_receiver
                .feed_incoming(&input_buf[input_offset..])
                .map_err(|error| format!("ZModem 接收远端数据失败: {error}"))?;
            if consumed > 0 {
                input_offset += consumed;
                progressed = true;
                if input_offset == input_buf.len() {
                    input_buf.clear();
                    input_offset = 0;
                } else if input_offset > 4096 {
                    input_buf.drain(..input_offset);
                    input_offset = 0;
                }
            }
        }

        if progressed {
            last_progress = Instant::now();
        } else if last_progress.elapsed() > Duration::from_secs(90) {
            return Err("ZModem download idle timeout".to_string());
        } else {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    if let Some((mut file, target, temp)) = current_file.take() {
        file.flush()
            .map_err(|error| format!("刷新 ZModem 本地文件失败: {error}"))?;
        drop(file);
        finalize_local_resume_file(&temp, &target)?;
    }

    Ok(bytes_done)
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

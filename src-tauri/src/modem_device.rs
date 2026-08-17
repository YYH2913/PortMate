use super::*;

const LOAD_BAUD_SWITCH_MARKER: &str = "press ENTER";
const LOAD_BAUD_RESTORE_MARKER: &str = "press ESC";
const LOAD_BAUD_RESTORE_TIMEOUT: Duration = Duration::from_secs(10);
const LOAD_BAUD_DEVICE_SETTLE_DELAY: Duration = Duration::from_millis(150);
const LOAD_BAUD_HOST_SETTLE_DELAY: Duration = Duration::from_millis(50);

struct DeviceModemSession {
    reader: ModemByteReader,
    baud_restore: Option<DeviceBaudRestore>,
}

struct DeviceBaudRestore {
    original_baud_rate: u32,
    receiver: broadcast::Receiver<Vec<u8>>,
    binding: ModemRuntimeBinding,
}

pub(super) fn device_modem_upload(
    request: &StartTransferRequest,
) -> Result<Option<(String, LoadReceiverSpec)>, String> {
    let Some(receiver) = parse_load_receiver_endpoint(&request.destination, &request.protocol)?
    else {
        return Ok(None);
    };
    if has_remote_transfer_prefix(&request.source) {
        return Err("load: 设备接收端点只支持从 PortMate 本机文件上传".to_string());
    }
    if local_transfer_entry(Path::new(&request.source), "Modem 本地传输源")?.is_none() {
        return Err("Modem 本地传输源不存在".to_string());
    }
    Ok(Some((request.source.clone(), receiver)))
}

pub(super) async fn transfer_file_to_device_modem(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<Option<u64>, String> {
    let Some((local_source, receiver)) = device_modem_upload(request)? else {
        return Ok(None);
    };
    let session = start_device_modem(state, &request.session_id, &receiver, progress).await?;
    let DeviceModemSession {
        reader,
        baud_restore,
    } = session;
    let result = match request.protocol {
        TransferProtocol::Xmodem => {
            xmodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                false,
                progress,
            )
            .await
        }
        TransferProtocol::Ymodem => {
            ymodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                None,
                false,
                progress,
            )
            .await
        }
        TransferProtocol::Zmodem => {
            zmodem_send_file(
                state,
                &request.session_id,
                reader,
                &local_source,
                None,
                progress,
            )
            .await
        }
        TransferProtocol::Sftp | TransferProtocol::Scp | TransferProtocol::Tftp => {
            unreachable!("load endpoint validation excludes non-Modem protocols")
        }
    };
    finish_device_modem(state, &request.session_id, baud_restore, result)
        .await
        .map(Some)
}

async fn start_device_modem(
    state: &AppState,
    session_id: &str,
    receiver: &LoadReceiverSpec,
    progress: &TransferProgressContext,
) -> Result<DeviceModemSession, String> {
    let binding = transfer_modem_binding(state, session_id, progress).await?;
    let tap_receiver = binding.subscribe();
    let baud_restore_receiver = receiver
        .baud_rate
        .map(|_| binding.subscribe());
    let original_baud_rate = receiver
        .baud_rate
        .map(|_| current_serial_runtime_baud(state, session_id, binding.runtime_id()))
        .transpose()?;

    binding
        .write_runtime_bytes(state, receiver.command_line().as_bytes())
        .await?;

    let Some(target_baud_rate) = receiver.baud_rate else {
        return Ok(DeviceModemSession {
            reader: binding.reader_with_receiver(tap_receiver, Arc::clone(&progress.cancel)),
            baud_restore: None,
        });
    };
    let original_baud_rate = original_baud_rate.expect("baud rate was read above");
    if target_baud_rate == original_baud_rate {
        return Ok(DeviceModemSession {
            reader: binding.reader_with_receiver(tap_receiver, Arc::clone(&progress.cancel)),
            baud_restore: None,
        });
    }

    let reader = ModemByteReader::after_marker_for_binding(
        tap_receiver,
        LOAD_BAUD_SWITCH_MARKER,
        Arc::clone(&progress.cancel),
        &binding,
    )
    .await
    .map_err(|error| format!("等待设备切换 load 波特率失败: {error}"))?;
    tokio::time::sleep(LOAD_BAUD_DEVICE_SETTLE_DELAY).await;
    set_serial_runtime_baud(state, session_id, binding.runtime_id(), target_baud_rate)?;
    tokio::time::sleep(LOAD_BAUD_HOST_SETTLE_DELAY).await;
    if let Err(error) = binding.write_runtime_bytes(state, b"\r").await {
        let restore_error = restore_serial_runtime_baud(
            state,
            session_id,
            binding.runtime_id(),
            original_baud_rate,
        )
        .err();
        return Err(match restore_error {
            Some(restore_error) => format!(
                "切换 load 波特率后确认设备失败: {error}; 恢复串口波特率失败: {restore_error}"
            ),
            None => format!("切换 load 波特率后确认设备失败: {error}"),
        });
    }

    Ok(DeviceModemSession {
        reader,
        baud_restore: Some(DeviceBaudRestore {
            original_baud_rate,
            receiver: baud_restore_receiver.expect("baud restore receiver was created above"),
            binding,
        }),
    })
}

async fn finish_device_modem<T>(
    state: &AppState,
    session_id: &str,
    baud_restore: Option<DeviceBaudRestore>,
    result: Result<T, String>,
) -> Result<T, String> {
    let Some(mut restore) = baud_restore else {
        return result;
    };

    if result.is_err() {
        let _ = restore
            .binding
            .write_runtime_bytes(state, &[MODEM_CAN, MODEM_CAN, MODEM_CAN])
            .await;
    }
    let prompt_result = wait_for_device_output_marker(
        &mut restore.receiver,
        LOAD_BAUD_RESTORE_MARKER,
        LOAD_BAUD_RESTORE_TIMEOUT,
        &restore.binding,
    )
    .await;
    if prompt_result.is_ok() {
        tokio::time::sleep(LOAD_BAUD_DEVICE_SETTLE_DELAY).await;
    }
    let restore_result = restore_serial_runtime_baud(
        state,
        session_id,
        restore.binding.runtime_id(),
        restore.original_baud_rate,
    );
    if restore_result.is_ok() {
        tokio::time::sleep(LOAD_BAUD_HOST_SETTLE_DELAY).await;
    }
    let confirm_result = match &restore_result {
        Ok(()) => restore.binding.write_runtime_bytes(state, &[0x1b]).await,
        Err(_) => Ok(()),
    };

    let cleanup_error = prompt_result
        .err()
        .or_else(|| restore_result.err())
        .or_else(|| confirm_result.err());
    match (result, cleanup_error) {
        (Ok(value), None) => Ok(value),
        (Ok(_), Some(error)) => Err(format!("文件已发送，但恢复 load 串口波特率失败: {error}")),
        (Err(error), None) => Err(error),
        (Err(error), Some(cleanup_error)) => {
            Err(format!("{error}; 恢复 load 串口波特率失败: {cleanup_error}"))
        }
    }
}

fn current_serial_runtime_baud(
    state: &AppState,
    session_id: &str,
    expected_runtime_id: &str,
) -> Result<u32, String> {
    let writer = serial_runtime_writer(state, session_id, expected_runtime_id)?;
    let result = writer
        .lock()
        .map_err(|error| error.to_string())?
        .baud_rate()
        .map_err(|error| format!("读取串口当前波特率失败: {error}"));
    result
}

fn set_serial_runtime_baud(
    state: &AppState,
    session_id: &str,
    expected_runtime_id: &str,
    baud_rate: u32,
) -> Result<(), String> {
    let writer = serial_runtime_writer(state, session_id, expected_runtime_id)?;
    let result = writer
        .lock()
        .map_err(|error| error.to_string())?
        .set_baud_rate(baud_rate)
        .map_err(|error| format!("设置串口波特率 {baud_rate} 失败: {error}"));
    result
}

pub(super) fn restore_serial_runtime_baud(
    state: &AppState,
    session_id: &str,
    expected_runtime_id: &str,
    baud_rate: u32,
) -> Result<(), String> {
    set_serial_runtime_baud(state, session_id, expected_runtime_id, baud_rate)
}

pub(super) fn serial_runtime_writer(
    state: &AppState,
    session_id: &str,
    expected_runtime_id: &str,
) -> Result<Arc<Mutex<SerialPortHandle>>, String> {
    state
        .serial
        .lock()
        .map_err(|error| error.to_string())?
        .get(session_id)
        .filter(|runtime| {
            runtime.runtime_id == expected_runtime_id && !runtime.closed.load(Ordering::SeqCst)
        })
        .ok_or_else(|| "load 波特率参数仅支持已连接的串口会话".to_string())?
        .writer
        .as_ref()
        .map(Arc::clone)
        .ok_or_else(|| "串口正在重连，无法切换 load 波特率".to_string())
}

async fn wait_for_device_output_marker(
    receiver: &mut broadcast::Receiver<Vec<u8>>,
    marker: &str,
    timeout: Duration,
    binding: &ModemRuntimeBinding,
) -> Result<(), String> {
    let started = Instant::now();
    let marker = marker.as_bytes();
    let mut buffered = Vec::new();
    loop {
        binding.ensure_current()?;
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(format!(
                "等待设备输出 `{}` 超时",
                String::from_utf8_lossy(marker)
            ));
        }
        match tokio::time::timeout(remaining.min(MODEM_CANCEL_POLL_INTERVAL), receiver.recv())
            .await
        {
            Ok(Ok(bytes)) => {
                buffered.extend_from_slice(&bytes);
                if buffered
                    .windows(marker.len())
                    .any(|window| window == marker)
                {
                    return Ok(());
                }
                if buffered.len() > 64 * 1024 {
                    let keep = marker.len().saturating_sub(1);
                    buffered.drain(..buffered.len().saturating_sub(keep));
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => {
                return Err("设备 load 输出流已关闭".to_string())
            }
            Err(_) => {}
        }
    }
}

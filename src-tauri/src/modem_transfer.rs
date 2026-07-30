use super::*;

pub(super) fn runtime_tap_receiver(
    state: &AppState,
    session_id: &str,
) -> Result<broadcast::Receiver<Vec<u8>>, String> {
    if let Some(tap) = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.tcp.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    if let Some(tap) = {
        let connections = state.serial.lock().map_err(|error| error.to_string())?;
        connections
            .get(session_id)
            .map(|runtime| runtime.tap.clone())
    } {
        return Ok(tap.subscribe());
    }
    Err("需要先连接会话才能执行 X/Y/ZModem 传输".to_string())
}

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

pub(super) async fn check_modem_cancelled(
    state: &AppState,
    session_id: &str,
    progress: &TransferProgressContext,
) -> Result<(), String> {
    if progress.cancel.load(Ordering::SeqCst) {
        let _ = write_runtime_bytes(state, session_id, &[MODEM_CAN, MODEM_CAN, MODEM_CAN]).await;
        Err(TRANSFER_CANCELLED_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

pub(super) struct ModemByteReader {
    receiver: broadcast::Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
    cancel: Arc<AtomicBool>,
    connection: Option<(Arc<Mutex<SessionStore>>, String)>,
}

impl ModemByteReader {
    pub(super) fn new(receiver: broadcast::Receiver<Vec<u8>>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            receiver,
            pending: VecDeque::new(),
            cancel,
            connection: None,
        }
    }

    pub(super) fn watch_connection(
        mut self,
        store: Arc<Mutex<SessionStore>>,
        session_id: String,
    ) -> Self {
        self.connection = Some((store, session_id));
        self
    }

    pub(super) fn check_interrupted(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
        }
        if let Some((store, session_id)) = &self.connection {
            ensure_modem_session_connected(store, session_id)?;
        }
        Ok(())
    }

    pub(super) async fn after_marker(
        mut receiver: broadcast::Receiver<Vec<u8>>,
        marker: &str,
        cancel: Arc<AtomicBool>,
        connection: Option<(Arc<Mutex<SessionStore>>, String)>,
    ) -> Result<Self, String> {
        let started = Instant::now();
        let marker = marker.as_bytes();
        let mut buffered = Vec::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            if let Some((store, session_id)) = &connection {
                ensure_modem_session_connected(store, session_id)?;
            }
            let remaining = REMOTE_MODEM_READY_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("remote modem readiness marker timed out".to_string());
            }
            match tokio::time::timeout(remaining.min(MODEM_CANCEL_POLL_INTERVAL), receiver.recv())
                .await
            {
                Ok(Ok(bytes)) => {
                    buffered.extend_from_slice(&bytes);
                    if let Some(offset) = buffered
                        .windows(marker.len())
                        .position(|window| window == marker)
                    {
                        return Ok(Self {
                            receiver,
                            pending: buffered[offset + marker.len()..].iter().copied().collect(),
                            cancel,
                            connection,
                        });
                    }
                    if buffered.len() > 64 * 1024 {
                        let keep = marker.len().saturating_sub(1);
                        buffered.drain(..buffered.len().saturating_sub(keep));
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("remote modem readiness stream closed".to_string())
                }
                Err(_) => {}
            }
        }
    }

    pub(super) async fn next_byte(&mut self, timeout: Duration) -> Result<u8, String> {
        let started = Instant::now();
        loop {
            self.check_interrupted()?;
            if let Some(byte) = self.pending.pop_front() {
                return Ok(byte);
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("modem byte timeout".to_string());
            }
            match tokio::time::timeout(
                remaining.min(MODEM_CANCEL_POLL_INTERVAL),
                self.receiver.recv(),
            )
            .await
            {
                Ok(Ok(bytes)) => self.pending.extend(bytes),
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("modem byte stream closed".to_string())
                }
                Err(_) => {}
            }
        }
    }

    pub(super) async fn read_exact(
        &mut self,
        len: usize,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::with_capacity(len);
        while bytes.len() < len {
            bytes.push(self.next_byte(timeout).await?);
        }
        Ok(bytes)
    }

    pub(super) async fn next_chunk(
        &mut self,
        timeout: Duration,
        max_len: usize,
    ) -> Result<Vec<u8>, String> {
        self.check_interrupted()?;
        if !self.pending.is_empty() {
            let take = self.pending.len().min(max_len);
            return Ok(self.pending.drain(..take).collect());
        }

        let started = Instant::now();
        loop {
            self.check_interrupted()?;
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("modem byte timeout".to_string());
            }
            match tokio::time::timeout(
                remaining.min(MODEM_CANCEL_POLL_INTERVAL),
                self.receiver.recv(),
            )
            .await
            {
                Ok(Ok(bytes)) => {
                    if bytes.len() <= max_len {
                        return Ok(bytes);
                    }
                    self.pending.extend(bytes[max_len..].iter().copied());
                    return Ok(bytes[..max_len].to_vec());
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    return Err("modem byte stream closed".to_string())
                }
                Err(_) => {}
            }
        }
    }
}

pub(super) fn ensure_modem_session_connected(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<(), String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    let status = store
        .runtimes
        .iter()
        .find(|runtime| runtime.session_id == session_id)
        .map(|runtime| runtime.status)
        .ok_or_else(|| format!("modem session runtime missing: {session_id}"))?;
    if status == SessionStatus::Connected {
        Ok(())
    } else {
        Err(format!("modem session disconnected ({status:?})"))
    }
}

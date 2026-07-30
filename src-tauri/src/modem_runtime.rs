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

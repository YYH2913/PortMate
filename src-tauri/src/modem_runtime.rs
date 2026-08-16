use super::*;

#[derive(Clone)]
pub(super) struct ModemRuntimeBinding {
    session_id: String,
    runtime_id: String,
    tap: broadcast::Sender<Vec<u8>>,
    watch: ModemConnectionWatch,
}

#[derive(Clone)]
struct ModemConnectionWatch {
    store: Arc<Mutex<SessionStore>>,
    runtimes: Option<RuntimeRegistry>,
    session_id: String,
    runtime_id: Option<String>,
    runtime_kind: Option<ModemRuntimeKind>,
}

#[derive(Clone, Copy)]
enum ModemRuntimeKind {
    Ssh,
    Shell,
    Tcp,
    Serial,
}

pub(super) enum ModemRuntimeCompletionGuard<'a> {
    Ssh(MutexGuard<'a, HashMap<String, SshRuntime>>),
    Shell(MutexGuard<'a, HashMap<String, ShellRuntime>>),
    Tcp(MutexGuard<'a, HashMap<String, TcpRuntime>>),
    Serial(MutexGuard<'a, HashMap<String, SerialRuntime>>),
}

impl ModemRuntimeBinding {
    pub(super) fn runtime_id(&self) -> &str {
        &self.runtime_id
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<Vec<u8>> {
        self.tap.subscribe()
    }

    pub(super) fn reader_with_receiver(
        &self,
        receiver: broadcast::Receiver<Vec<u8>>,
        cancel: Arc<AtomicBool>,
    ) -> ModemByteReader {
        ModemByteReader {
            receiver,
            pending: VecDeque::new(),
            cancel,
            connection: Some(self.watch.clone()),
        }
    }

    pub(super) fn ensure_current(&self) -> Result<(), String> {
        self.watch.ensure_current()
    }

    pub(super) fn completion_guard(&self) -> Result<ModemRuntimeCompletionGuard<'_>, String> {
        let runtimes = self
            .watch
            .runtimes
            .as_ref()
            .ok_or_else(|| "Modem runtime binding 缺少 runtime registry".to_string())?;
        let guard = match self
            .watch
            .runtime_kind
            .ok_or_else(|| "Modem runtime binding 缺少 transport 类型".to_string())?
        {
            ModemRuntimeKind::Ssh => ModemRuntimeCompletionGuard::Ssh(
                runtimes.ssh.lock().map_err(|error| error.to_string())?,
            ),
            ModemRuntimeKind::Shell => ModemRuntimeCompletionGuard::Shell(
                runtimes.shell.lock().map_err(|error| error.to_string())?,
            ),
            ModemRuntimeKind::Tcp => ModemRuntimeCompletionGuard::Tcp(
                runtimes.tcp.lock().map_err(|error| error.to_string())?,
            ),
            ModemRuntimeKind::Serial => ModemRuntimeCompletionGuard::Serial(
                runtimes.serial.lock().map_err(|error| error.to_string())?,
            ),
        };
        Ok(guard)
    }

    pub(super) async fn write_runtime_bytes(
        &self,
        state: &AppState,
        bytes: &[u8],
    ) -> Result<(), String> {
        self.ensure_current()?;
        write_runtime_bytes_for_runtime(state, &self.session_id, bytes, Some(&self.runtime_id))
            .await
    }
}

impl ModemRuntimeCompletionGuard<'_> {
    pub(super) fn permits_completion(
        &self,
        binding: &ModemRuntimeBinding,
        session_id: &str,
    ) -> bool {
        if binding.session_id != session_id {
            return false;
        }
        match self {
            Self::Ssh(runtimes) => runtimes
                .get(session_id)
                .is_none_or(|runtime| runtime.runtime_id == binding.runtime_id),
            Self::Shell(runtimes) => runtimes
                .get(session_id)
                .is_none_or(|runtime| runtime.runtime_id == binding.runtime_id),
            Self::Tcp(runtimes) => runtimes
                .get(session_id)
                .is_none_or(|runtime| runtime.runtime_id == binding.runtime_id),
            Self::Serial(runtimes) => runtimes
                .get(session_id)
                .is_none_or(|runtime| runtime.runtime_id == binding.runtime_id),
        }
    }
}

impl ModemConnectionWatch {
    #[cfg(test)]
    fn store_only(store: Arc<Mutex<SessionStore>>, session_id: String) -> Self {
        Self {
            store,
            runtimes: None,
            session_id,
            runtime_id: None,
            runtime_kind: None,
        }
    }

    fn ensure_current(&self) -> Result<(), String> {
        ensure_modem_session_connected(&self.store, &self.session_id)?;
        if let (Some(runtimes), Some(runtime_id), Some(runtime_kind)) =
            (&self.runtimes, &self.runtime_id, self.runtime_kind)
        {
            ensure_modem_runtime_current(runtimes, &self.session_id, runtime_id, runtime_kind)?;
        }
        Ok(())
    }
}

pub(super) fn runtime_modem_binding(
    state: &AppState,
    session_id: &str,
) -> Result<ModemRuntimeBinding, String> {
    let mut target = {
        let connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.get(session_id).map(|runtime| {
            (
                ModemRuntimeKind::Ssh,
                runtime.runtime_id.clone(),
                runtime.tap.clone(),
                runtime.closed.load(Ordering::SeqCst),
            )
        })
    };
    if target.is_none() {
        let connections = state.shell.lock().map_err(|error| error.to_string())?;
        target = connections.get(session_id).map(|runtime| {
            (
                ModemRuntimeKind::Shell,
                runtime.runtime_id.clone(),
                runtime.tap.clone(),
                runtime.closed.load(Ordering::SeqCst),
            )
        });
    }
    if target.is_none() {
        let connections = state.tcp.lock().map_err(|error| error.to_string())?;
        target = connections.get(session_id).map(|runtime| {
            (
                ModemRuntimeKind::Tcp,
                runtime.runtime_id.clone(),
                runtime.tap.clone(),
                runtime.closed.load(Ordering::SeqCst),
            )
        });
    }
    if target.is_none() {
        let connections = state.serial.lock().map_err(|error| error.to_string())?;
        target = connections.get(session_id).map(|runtime| {
            (
                ModemRuntimeKind::Serial,
                runtime.runtime_id.clone(),
                runtime.tap.clone(),
                runtime.closed.load(Ordering::SeqCst),
            )
        });
    }
    let target = target.ok_or_else(|| "需要先连接会话才能执行 X/Y/ZModem 传输".to_string())?;
    if target.3 {
        return Err("Modem 来源连接已关闭或正在重连".to_string());
    }
    let watch = ModemConnectionWatch {
        store: Arc::clone(&state.store),
        runtimes: Some(state.runtimes()),
        session_id: session_id.to_string(),
        runtime_id: Some(target.1.clone()),
        runtime_kind: Some(target.0),
    };
    let binding = ModemRuntimeBinding {
        session_id: session_id.to_string(),
        runtime_id: target.1,
        tap: target.2,
        watch,
    };
    binding.ensure_current()?;
    Ok(binding)
}

pub(super) async fn transfer_modem_binding(
    state: &AppState,
    session_id: &str,
    progress: &TransferProgressContext,
) -> Result<ModemRuntimeBinding, String> {
    let binding = runtime_modem_binding(state, session_id)?;
    let cancellation = state
        .transfer_cancellations
        .lock()
        .map_err(|error| error.to_string())?
        .get(&progress.task_id)
        .cloned();
    if let Some(cancellation) = cancellation {
        cancellation.bind_modem_runtime(binding.clone())?;
    }
    if progress.cancel.load(Ordering::SeqCst) {
        let _ = binding
            .write_runtime_bytes(state, &[MODEM_CAN, MODEM_CAN, MODEM_CAN])
            .await;
        return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
    }
    binding.ensure_current()?;
    Ok(binding)
}

#[cfg(test)]
pub(super) fn runtime_tap_receiver(
    state: &AppState,
    session_id: &str,
) -> Result<broadcast::Receiver<Vec<u8>>, String> {
    runtime_modem_binding(state, session_id).map(|binding| binding.subscribe())
}

pub(super) async fn check_modem_cancelled(
    state: &AppState,
    reader: &ModemByteReader,
    progress: &TransferProgressContext,
) -> Result<(), String> {
    if progress.cancel.load(Ordering::SeqCst) {
        let _ = reader
            .write_runtime_bytes(state, &[MODEM_CAN, MODEM_CAN, MODEM_CAN])
            .await;
        Err(TRANSFER_CANCELLED_MESSAGE.to_string())
    } else {
        reader.check_interrupted()
    }
}

pub(super) struct ModemByteReader {
    receiver: broadcast::Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
    cancel: Arc<AtomicBool>,
    connection: Option<ModemConnectionWatch>,
}

impl ModemByteReader {
    #[cfg(test)]
    pub(super) fn new(receiver: broadcast::Receiver<Vec<u8>>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            receiver,
            pending: VecDeque::new(),
            cancel,
            connection: None,
        }
    }

    #[cfg(test)]
    pub(super) fn watch_connection(
        mut self,
        store: Arc<Mutex<SessionStore>>,
        session_id: String,
    ) -> Self {
        self.connection = Some(ModemConnectionWatch::store_only(store, session_id));
        self
    }

    pub(super) fn runtime_id(&self) -> Option<&str> {
        self.connection
            .as_ref()
            .and_then(|watch| watch.runtime_id.as_deref())
    }

    pub(super) async fn write_runtime_bytes(
        &self,
        state: &AppState,
        bytes: &[u8],
    ) -> Result<(), String> {
        self.check_interrupted()?;
        let session_id = self
            .connection
            .as_ref()
            .map(|watch| watch.session_id.as_str())
            .ok_or_else(|| "Modem reader 未绑定会话".to_string())?;
        write_runtime_bytes_for_runtime(state, session_id, bytes, self.runtime_id()).await
    }

    pub(super) fn check_interrupted(&self) -> Result<(), String> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
        }
        if let Some(connection) = &self.connection {
            connection.ensure_current()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) async fn after_marker(
        receiver: broadcast::Receiver<Vec<u8>>,
        marker: &str,
        cancel: Arc<AtomicBool>,
        connection: Option<(Arc<Mutex<SessionStore>>, String)>,
    ) -> Result<Self, String> {
        Self::after_marker_with_watch(
            receiver,
            marker,
            cancel,
            connection
                .map(|(store, session_id)| ModemConnectionWatch::store_only(store, session_id)),
        )
        .await
    }

    pub(super) async fn after_marker_for_binding(
        receiver: broadcast::Receiver<Vec<u8>>,
        marker: &str,
        cancel: Arc<AtomicBool>,
        binding: &ModemRuntimeBinding,
    ) -> Result<Self, String> {
        Self::after_marker_with_watch(receiver, marker, cancel, Some(binding.watch.clone())).await
    }

    async fn after_marker_with_watch(
        mut receiver: broadcast::Receiver<Vec<u8>>,
        marker: &str,
        cancel: Arc<AtomicBool>,
        connection: Option<ModemConnectionWatch>,
    ) -> Result<Self, String> {
        let started = Instant::now();
        let marker = marker.as_bytes();
        let mut buffered = Vec::new();
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            let remaining = REMOTE_MODEM_READY_TIMEOUT.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err("remote modem readiness marker timed out".to_string());
            }
            let bytes = match receiver.try_recv() {
                Ok(bytes) => Some(bytes),
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Closed) => {
                    return Err("remote modem readiness stream closed".to_string())
                }
                Err(broadcast::error::TryRecvError::Empty) => {
                    if let Some(connection) = &connection {
                        connection.ensure_current()?;
                    }
                    match tokio::time::timeout(
                        remaining.min(MODEM_CANCEL_POLL_INTERVAL),
                        receiver.recv(),
                    )
                    .await
                    {
                        Ok(Ok(bytes)) => Some(bytes),
                        Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                        Ok(Err(broadcast::error::RecvError::Closed)) => {
                            return Err("remote modem readiness stream closed".to_string())
                        }
                        Err(_) => None,
                    }
                }
            };
            if let Some(bytes) = bytes {
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
        }
    }

    pub(super) async fn next_byte(&mut self, timeout: Duration) -> Result<u8, String> {
        let started = Instant::now();
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            if let Some(byte) = self.pending.pop_front() {
                return Ok(byte);
            }
            match self.receiver.try_recv() {
                Ok(bytes) => {
                    self.pending.extend(bytes);
                    continue;
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Closed) => {
                    self.check_interrupted()?;
                    return Err("modem byte stream closed".to_string());
                }
                Err(broadcast::error::TryRecvError::Empty) => {}
            }
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
        let started = Instant::now();
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                return Err(TRANSFER_CANCELLED_MESSAGE.to_string());
            }
            if !self.pending.is_empty() {
                let take = self.pending.len().min(max_len);
                return Ok(self.pending.drain(..take).collect());
            }
            match self.receiver.try_recv() {
                Ok(bytes) if bytes.len() <= max_len => return Ok(bytes),
                Ok(bytes) => {
                    self.pending.extend(bytes[max_len..].iter().copied());
                    return Ok(bytes[..max_len].to_vec());
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(broadcast::error::TryRecvError::Closed) => {
                    self.check_interrupted()?;
                    return Err("modem byte stream closed".to_string());
                }
                Err(broadcast::error::TryRecvError::Empty) => {}
            }
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

fn ensure_modem_runtime_current(
    runtimes: &RuntimeRegistry,
    session_id: &str,
    runtime_id: &str,
    runtime_kind: ModemRuntimeKind,
) -> Result<(), String> {
    let current = match runtime_kind {
        ModemRuntimeKind::Ssh => runtimes
            .ssh
            .lock()
            .map_err(|error| error.to_string())?
            .get(session_id)
            .is_some_and(|runtime| {
                runtime.runtime_id == runtime_id && !runtime.closed.load(Ordering::SeqCst)
            }),
        ModemRuntimeKind::Shell => runtimes
            .shell
            .lock()
            .map_err(|error| error.to_string())?
            .get(session_id)
            .is_some_and(|runtime| {
                runtime.runtime_id == runtime_id && !runtime.closed.load(Ordering::SeqCst)
            }),
        ModemRuntimeKind::Tcp => runtimes
            .tcp
            .lock()
            .map_err(|error| error.to_string())?
            .get(session_id)
            .is_some_and(|runtime| {
                runtime.runtime_id == runtime_id && !runtime.closed.load(Ordering::SeqCst)
            }),
        ModemRuntimeKind::Serial => runtimes
            .serial
            .lock()
            .map_err(|error| error.to_string())?
            .get(session_id)
            .is_some_and(|runtime| {
                runtime.runtime_id == runtime_id && !runtime.closed.load(Ordering::SeqCst)
            }),
    };
    if current {
        Ok(())
    } else {
        Err("Modem 来源连接已关闭或被新连接替换".to_string())
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

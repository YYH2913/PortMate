use super::*;

pub(super) type SerialCaptureMap = Arc<Mutex<HashMap<String, Arc<Mutex<SerialCaptureBuffer>>>>>;
pub(super) type ActiveCommandMap = Arc<Mutex<HashMap<String, String>>>;
pub(super) type SessionLifecycleLanes =
    Mutex<HashMap<(PathBuf, String), Weak<tokio::sync::Mutex<()>>>>;
pub(super) type SessionOpenCancellations =
    Mutex<HashMap<(PathBuf, String), Vec<Weak<SessionOpenCancellation>>>>;

pub(super) static SESSION_LIFECYCLE_LANES: OnceLock<SessionLifecycleLanes> = OnceLock::new();
pub(super) static SESSION_OPEN_CANCELLATIONS: OnceLock<SessionOpenCancellations> = OnceLock::new();

pub(super) struct SessionOpenCancellation {
    cancelled: AtomicBool,
    changed: tokio::sync::Notify,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

impl SessionOpenCancellation {
    pub(super) fn new(slot: tokio::sync::OwnedSemaphorePermit) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            changed: tokio::sync::Notify::new(),
            _slot: slot,
        }
    }

    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.changed.notify_one();
    }

    pub(super) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub(super) async fn wait(&self) {
        let changed = self.changed.notified();
        if self.is_cancelled() {
            return;
        }
        changed.await;
    }
}

#[derive(Clone)]
pub struct AppState {
    pub(super) app_handle: Option<AppHandle>,
    pub(super) store: Arc<Mutex<SessionStore>>,
    pub(super) credential_ops: Arc<Mutex<()>>,
    pub(super) credential_lock_path: PathBuf,
    pub(super) system_event_sink: Arc<Mutex<Option<SystemEventSinkGuard>>>,
    pub(super) session_open_slots: Arc<tokio::sync::Semaphore>,
    pub(super) ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    pub(super) ssh_auxiliary_slots: Arc<tokio::sync::Semaphore>,
    pub(super) tmux_controls: TmuxControlMap,
    pub(super) tmux_control_slots: Arc<tokio::sync::Semaphore>,
    pub(super) shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    pub(super) tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    pub(super) serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
    pub(super) serial_captures: SerialCaptureMap,
    pub(super) active_commands: ActiveCommandMap,
    pub(super) tunnels: Arc<Mutex<HashMap<String, TunnelRuntime>>>,
    pub(super) tunnel_connection_slots: Arc<tokio::sync::Semaphore>,
    pub(super) transfer_cancellations: Arc<Mutex<HashMap<String, Arc<TransferCancellation>>>>,
    pub(super) transfer_task_slots: Arc<tokio::sync::Semaphore>,
    pub(super) transfer_lanes: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    pub(super) sysmon_slots: Arc<tokio::sync::Semaphore>,
    pub(super) trigger_command_slots: Arc<tokio::sync::Semaphore>,
    pub(super) trigger_send_batch_slots: Arc<tokio::sync::Semaphore>,
    pub(super) pending_mcp_approvals: PendingMcpApprovalMap,
    pub(super) one_time_host_keys: Arc<Mutex<HashMap<String, Vec<portmate_core::TrustedHostKey>>>>,
    pub(super) ipc_publication: Arc<Mutex<IpcPublicationState>>,
    #[cfg(test)]
    pub(super) ssh_reconnect_install_error: Arc<Mutex<Option<String>>>,
    pub(super) store_path: PathBuf,
}

pub(super) struct CredentialOperationGuard<'a> {
    _local: MutexGuard<'a, ()>,
    _file: fs::File,
}

pub(super) fn lock_credential_operations(
    state: &AppState,
) -> Result<CredentialOperationGuard<'_>, String> {
    let local = state
        .credential_ops
        .lock()
        .map_err(|error| error.to_string())?;
    if let Some(parent) = state.credential_lock_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("无法创建凭据操作锁目录 {}: {error}", parent.display()))?;
        }
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&state.credential_lock_path)
        .map_err(|error| format!("无法打开凭据操作锁: {error}"))?;
    file.lock()
        .map_err(|error| format!("无法获取凭据操作锁: {error}"))?;
    Ok(CredentialOperationGuard {
        _local: local,
        _file: file,
    })
}

#[derive(Clone)]
pub(super) struct RuntimeRegistry {
    pub(super) ssh: Arc<Mutex<HashMap<String, SshRuntime>>>,
    pub(super) shell: Arc<Mutex<HashMap<String, ShellRuntime>>>,
    pub(super) tcp: Arc<Mutex<HashMap<String, TcpRuntime>>>,
    pub(super) serial: Arc<Mutex<HashMap<String, SerialRuntime>>>,
}

#[derive(Clone)]
pub(super) struct SessionIo {
    pub(super) app_handle: Option<AppHandle>,
    pub(super) store: Arc<Mutex<SessionStore>>,
    pub(super) runtimes: RuntimeRegistry,
    pub(super) serial_captures: SerialCaptureMap,
    pub(super) active_commands: ActiveCommandMap,
    pub(super) trigger_command_slots: Arc<tokio::sync::Semaphore>,
    pub(super) trigger_send_batch_slots: Arc<tokio::sync::Semaphore>,
    pub(super) store_path: PathBuf,
}

impl AppState {
    pub(super) fn runtimes(&self) -> RuntimeRegistry {
        RuntimeRegistry {
            ssh: Arc::clone(&self.ssh),
            shell: Arc::clone(&self.shell),
            tcp: Arc::clone(&self.tcp),
            serial: Arc::clone(&self.serial),
        }
    }

    pub(super) fn session_io(&self) -> SessionIo {
        SessionIo {
            app_handle: self.app_handle.clone(),
            store: Arc::clone(&self.store),
            runtimes: self.runtimes(),
            serial_captures: Arc::clone(&self.serial_captures),
            active_commands: Arc::clone(&self.active_commands),
            trigger_command_slots: Arc::clone(&self.trigger_command_slots),
            trigger_send_batch_slots: Arc::clone(&self.trigger_send_batch_slots),
            store_path: self.store_path.clone(),
        }
    }
}

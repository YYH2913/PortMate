use super::*;

pub(super) type SerialCaptureMap = Arc<Mutex<HashMap<String, Arc<Mutex<SerialCaptureBuffer>>>>>;
pub(super) type ActiveCommandMap = Arc<Mutex<HashMap<String, String>>>;
pub(super) type CommitValidation = Box<dyn FnOnce() -> Result<(), String> + Send>;
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
    pub(super) session_credentials: Arc<Mutex<SessionCredentialRegistry>>,
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
    pub(super) mcp_content_transfer_staging: Arc<Mutex<HashMap<String, PathBuf>>>,
    pub(super) transfer_task_slots: Arc<tokio::sync::Semaphore>,
    pub(super) transfer_lanes: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    pub(super) sysmon_slots: Arc<tokio::sync::Semaphore>,
    pub(super) trigger_command_slots: Arc<tokio::sync::Semaphore>,
    pub(super) trigger_send_batch_slots: Arc<tokio::sync::Semaphore>,
    pub(super) pending_mcp_approvals: PendingMcpApprovalMap,
    pub(super) mcp_http_process: Arc<Mutex<McpHttpProcessRegistry>>,
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

pub(super) fn remove_runtime_if_owned<Runtime>(
    registry: &Mutex<HashMap<String, Runtime>>,
    session_id: &str,
    owns_runtime: impl FnOnce(&Runtime) -> bool,
) -> Result<Option<Runtime>, String> {
    let mut runtimes = registry.lock().map_err(|error| error.to_string())?;
    let owned = runtimes.get(session_id).is_some_and(owns_runtime);
    Ok(if owned {
        runtimes.remove(session_id)
    } else {
        None
    })
}

pub(super) fn expand_identity_path(path: &str) -> PathBuf {
    let home = native_home_path();
    expand_identity_path_with_home(path, home.as_deref(), cfg!(windows))
}

pub(super) fn native_home_path() -> Option<PathBuf> {
    preferred_native_home_path(
        environment_path("HOME"),
        environment_path("USERPROFILE"),
        cfg!(windows),
    )
}

pub(super) fn preferred_native_home_path(
    home: Option<PathBuf>,
    user_profile: Option<PathBuf>,
    windows: bool,
) -> Option<PathBuf> {
    if windows {
        user_profile.or(home)
    } else {
        home.or(user_profile)
    }
}

fn environment_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(super) fn expand_identity_path_with_home(
    path: &str,
    home: Option<&Path>,
    windows: bool,
) -> PathBuf {
    let relative = local_home_relative_path(path, windows);

    match (home, relative) {
        (Some(home), Some(relative)) => home.join(relative),
        _ => PathBuf::from(path),
    }
}

pub(super) fn local_home_relative_path(path: &str, windows: bool) -> Option<&str> {
    let relative = if path == "~" {
        ""
    } else if let Some(relative) = path.strip_prefix("~/") {
        relative
    } else if windows {
        path.strip_prefix(r"~\")?
    } else {
        return None;
    };
    let relative = if windows {
        relative.trim_start_matches(['/', '\\'])
    } else {
        relative.trim_start_matches('/')
    };
    let has_windows_drive_prefix = windows
        && relative.len() >= 2
        && relative.as_bytes()[0].is_ascii_alphabetic()
        && relative.as_bytes()[1] == b':';
    (!has_windows_drive_prefix).then_some(relative)
}

pub(super) fn has_local_home_prefix(path: &str, windows: bool) -> bool {
    path == "~" || path.starts_with("~/") || (windows && path.starts_with(r"~\"))
}

#[derive(Debug, Default)]
enum ReaderStartState {
    #[default]
    Pending,
    Started,
    Cancelled,
}

#[derive(Debug, Default)]
pub(super) struct ReaderStartGate {
    state: Mutex<ReaderStartState>,
    changed: Condvar,
}

impl ReaderStartGate {
    pub(super) fn start(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state = ReaderStartState::Started;
        self.changed.notify_all();
    }

    pub(super) fn cancel(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state = ReaderStartState::Cancelled;
        self.changed.notify_all();
    }

    pub(super) fn wait(&self) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match *state {
                ReaderStartState::Pending => {
                    state = self
                        .changed
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                ReaderStartState::Started => return true,
                ReaderStartState::Cancelled => return false,
            }
        }
    }
}

pub(super) fn truncate_for_log(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    if trimmed.len() <= limit {
        return trimmed.to_string();
    }
    let boundary = trimmed
        .char_indices()
        .take_while(|(index, _)| *index <= limit)
        .map(|(index, _)| index)
        .last()
        .unwrap_or(0);
    format!("{}...", &trimmed[..boundary])
}

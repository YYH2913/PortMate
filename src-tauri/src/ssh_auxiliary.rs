use super::*;

pub(super) const SFTP_REQUEST_TIMEOUT_SECONDS: u64 = 20;
pub(super) const MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS: usize = 256;
pub(super) const SSH_AUXILIARY_SETUP_TIMEOUT: Duration = Duration::from_secs(10);

struct SshAuxiliaryResources {
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    sftp: Arc<tokio::sync::Mutex<Option<SftpBackendSession>>>,
}

fn ssh_resources_for_auxiliary_operation<T>(
    state: &AppState,
    session_id: &str,
    inspect: impl FnOnce(&SshRuntime) -> Result<T, String>,
) -> Result<(T, SshAuxiliaryResources), String> {
    let connections = state.ssh.lock().map_err(|error| error.to_string())?;
    let runtime = connections
        .get(session_id)
        .ok_or_else(|| "需要先连接 SSH/Tmux 会话才能执行远端操作".to_string())?;
    Ok((
        inspect(runtime)?,
        SshAuxiliaryResources {
            handle: Arc::clone(&runtime.handle),
            sftp: Arc::clone(&runtime.sftp),
        },
    ))
}

pub(super) struct SshAuxiliaryLease {
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    sftp: Arc<tokio::sync::Mutex<Option<SftpBackendSession>>>,
    _slot: tokio::sync::OwnedSemaphorePermit,
}

impl SshAuxiliaryLease {
    pub(super) fn handle(&self) -> Arc<tokio::sync::Mutex<SshBackendSession>> {
        Arc::clone(&self.handle)
    }

    pub(super) async fn sftp(&self) -> Result<SftpSessionLease, String> {
        let mut session = tokio::time::timeout(
            SSH_AUXILIARY_SETUP_TIMEOUT,
            Arc::clone(&self.sftp).lock_owned(),
        )
        .await
        .map_err(|_| {
            format!(
                "SFTP operation lock timed out after {} ms",
                SSH_AUXILIARY_SETUP_TIMEOUT.as_millis()
            )
        })?;
        if session.is_none() {
            *session = Some(open_sftp_session(self.handle()).await?);
        }
        Ok(SftpSessionLease { session })
    }
}

pub(super) struct SftpSessionLease {
    session: tokio::sync::OwnedMutexGuard<Option<SftpBackendSession>>,
}

impl Deref for SftpSessionLease {
    type Target = SftpBackendSession;

    fn deref(&self) -> &Self::Target {
        self.session
            .as_ref()
            .expect("SFTP session lease must contain an initialized session")
    }
}

impl SftpSessionLease {
    pub(super) fn invalidate(&mut self) {
        self.session.take();
    }
}

pub(super) fn acquire_ssh_auxiliary_slot(
    state: &AppState,
) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    Arc::clone(&state.ssh_auxiliary_slots)
        .try_acquire_owned()
        .map_err(|_| {
            format!(
                "SSH auxiliary operation limit reached ({MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS})"
            )
        })
}

pub(super) fn ssh_auxiliary_lease(
    state: &AppState,
    session_id: &str,
) -> Result<SshAuxiliaryLease, String> {
    ssh_auxiliary_lease_with_runtime(state, session_id, |_| Ok(())).map(|(_, lease)| lease)
}

pub(super) fn ssh_auxiliary_lease_with_runtime<T>(
    state: &AppState,
    session_id: &str,
    inspect: impl FnOnce(&SshRuntime) -> Result<T, String>,
) -> Result<(T, SshAuxiliaryLease), String> {
    let slot = acquire_ssh_auxiliary_slot(state)?;
    let (inspection, resources) =
        ssh_resources_for_auxiliary_operation(state, session_id, inspect)?;
    Ok((
        inspection,
        SshAuxiliaryLease {
            handle: resources.handle,
            sftp: resources.sftp,
            _slot: slot,
        },
    ))
}

pub(super) async fn open_sftp_session(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
) -> Result<SftpBackendSession, String> {
    let timeout = SSH_AUXILIARY_SETUP_TIMEOUT;
    let started = Instant::now();
    let handle = tokio::time::timeout(timeout, handle.lock())
        .await
        .map_err(|_| format!("SFTP handle lock 超时（{} ms）", timeout.as_millis()))?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("SFTP setup 超时（{} ms）", timeout.as_millis()))?;

    let setup = async {
        let sftp = match &*handle {
            SshBackendSession::Russh(handle) => {
                let channel = handle
                    .channel_open_session()
                    .await
                    .map_err(|error| format!("SFTP 打开 SSH channel 失败: {error}"))?;
                channel
                    .request_subsystem(true, "sftp")
                    .await
                    .map_err(|error| format!("SFTP subsystem 启动失败: {error}"))?;
                let session = SftpSession::new(channel.into_stream())
                    .await
                    .map_err(|error| format!("SFTP 初始化失败: {error}"))?;
                SftpBackendSession::from_russh(session)
            }
            SshBackendSession::Libssh(session) => {
                let session = session.clone();
                tokio::task::spawn_blocking(move || session.sftp())
                    .await
                    .map_err(|error| format!("libssh SFTP setup worker failed: {error}"))?
                    .map(SftpBackendSession::from_libssh)
                    .map_err(|error| format!("SFTP 初始化失败: {error}"))?
            }
        };
        sftp.set_timeout(SFTP_REQUEST_TIMEOUT_SECONDS);
        Ok::<_, String>(sftp)
    };
    match bounded_connection_step(setup, remaining).await {
        Ok(sftp) => Ok(sftp),
        Err(BoundedConnectionStepError::Failed(error)) => Err(error),
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning =
                request_backend_disconnect_with_timeout(&handle, "PortMate SFTP setup timeout")
                    .await
                    .map(|warning| format!("; {warning}"))
                    .unwrap_or_default();
            Err(format!(
                "SFTP setup 超时（{} ms）{cleanup_warning}",
                timeout.as_millis()
            ))
        }
    }
}

#[cfg(all(test, unix))]
pub(super) async fn open_sftp_session_with_timeout<H: client::Handler>(
    handle: Arc<tokio::sync::Mutex<client::Handle<H>>>,
    timeout: Duration,
) -> Result<SftpBackendSession, String> {
    let started = Instant::now();
    let handle = tokio::time::timeout(timeout, handle.lock())
        .await
        .map_err(|_| format!("SFTP handle lock 超时（{} ms）", timeout.as_millis()))?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("SFTP setup 超时（{} ms）", timeout.as_millis()))?;

    let setup = async {
        let channel = handle
            .channel_open_session()
            .await
            .map_err(|error| format!("SFTP 打开 SSH channel 失败: {error}"))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| format!("SFTP subsystem 启动失败: {error}"))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| format!("SFTP 初始化失败: {error}"))?;
        sftp.set_timeout(SFTP_REQUEST_TIMEOUT_SECONDS);
        Ok::<_, String>(SftpBackendSession::from_russh(sftp))
    };
    match bounded_connection_step(setup, remaining).await {
        Ok(sftp) => Ok(sftp),
        Err(BoundedConnectionStepError::Failed(error)) => Err(error),
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning =
                request_ssh_disconnect_with_timeout(&handle, "PortMate SFTP setup timeout")
                    .await
                    .map(|warning| format!("; {warning}"))
                    .unwrap_or_default();
            Err(format!(
                "SFTP setup 超时（{} ms）{cleanup_warning}",
                timeout.as_millis()
            ))
        }
    }
}

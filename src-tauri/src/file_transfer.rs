use super::*;

pub(super) const REMOTE_COPY_IO_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
pub(super) const REMOTE_COPY_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);
pub(super) const SFTP_REQUEST_TIMEOUT_SECONDS: u64 = 20;
pub(super) const MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS: usize = 256;
pub(super) const SSH_AUXILIARY_SETUP_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn transfer_file_via_sftp(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);
    if let Some(source) = source_remote {
        validate_remote_transfer_path(source, "SFTP 远端源路径")?;
    }
    if let Some(destination) = destination_remote {
        validate_remote_transfer_path(destination, "SFTP 远端目标路径")?;
    }

    match (source_remote, destination_remote) {
        (None, None) => {
            copy_local_file_for_transfer(&request.source, &request.destination, progress).await
        }
        remote_paths => {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let sftp = auxiliary.sftp().await?;
            let transfer = async {
                match remote_paths {
                    (None, Some(remote_destination)) => {
                        sftp_upload(&sftp, &request.source, remote_destination, progress).await
                    }
                    (Some(remote_source), None) => {
                        sftp_download(&sftp, remote_source, &request.destination, progress).await
                    }
                    (Some(remote_source), Some(remote_destination)) => {
                        sftp_remote_copy(&sftp, remote_source, remote_destination, progress).await
                    }
                    (None, None) => unreachable!("local transfer handled before opening SFTP"),
                }
            };
            let result = await_sftp_transfer_with_cancellation(transfer, progress).await;
            result
        }
    }
}

pub(super) async fn await_sftp_transfer_with_cancellation<T, F>(
    operation: F,
    progress: &TransferProgressContext,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    progress.check_cancelled()?;
    tokio::pin!(operation);
    loop {
        tokio::select! {
            result = &mut operation => return result,
            _ = tokio::time::sleep(TRANSFER_CANCEL_POLL_INTERVAL) => {
                progress.check_cancelled()?;
            }
        }
    }
}

pub(super) async fn transfer_file_via_local_or_scp(
    state: &AppState,
    request: &StartTransferRequest,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    progress.check_cancelled()?;
    let source_remote = remote_path(&request.source);
    let destination_remote = remote_path(&request.destination);
    if let Some(source) = source_remote {
        validate_remote_transfer_path(source, "SCP 远端源路径")?;
    }
    if let Some(destination) = destination_remote {
        validate_remote_transfer_path(destination, "SCP 远端目标路径")?;
    }

    match (source_remote, destination_remote) {
        (None, None) => {
            copy_local_file_for_transfer(&request.source, &request.destination, progress).await
        }
        (None, Some(remote_destination)) => {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let handle = auxiliary.handle();
            scp_upload(handle, &request.source, remote_destination, progress).await
        }
        (Some(remote_source), None) => {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let handle = auxiliary.handle();
            scp_download(handle, remote_source, &request.destination, progress).await
        }
        (Some(remote_source), Some(remote_destination)) => {
            let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
            let handle = auxiliary.handle();
            remote_copy(handle, remote_source, remote_destination, progress).await
        }
    }
}

pub(super) fn remote_path(value: &str) -> Option<&str> {
    value
        .strip_prefix("remote:")
        .or_else(|| value.strip_prefix("ssh:"))
        .filter(|path| !path.trim().is_empty())
}

pub(super) fn validate_remote_transfer_path(path: &str, label: &str) -> Result<(), String> {
    let trimmed = path.trim();
    let normalized = trimmed.trim_end_matches('/');
    if trimmed.is_empty()
        || normalized.is_empty()
        || matches!(normalized, "." | ".." | "~" | "/" | "//")
        || trimmed.contains('\0')
        || remote_path_has_dot_components(normalized)
    {
        return Err(format!(
            "{label}不能为空、包含 NUL、使用 . / .. 分量或指向根目录"
        ));
    }
    Ok(())
}

struct SshAuxiliaryResources {
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    sftp: Arc<tokio::sync::Mutex<Option<SftpSession>>>,
}

fn ssh_resources_for_auxiliary_operation(
    state: &AppState,
    session_id: &str,
) -> Result<SshAuxiliaryResources, String> {
    let connections = state.ssh.lock().map_err(|error| error.to_string())?;
    connections
        .get(session_id)
        .map(|runtime| SshAuxiliaryResources {
            handle: Arc::clone(&runtime.handle),
            sftp: Arc::clone(&runtime.sftp),
        })
        .ok_or_else(|| "需要先连接 SSH/Tmux 会话才能执行远端操作".to_string())
}

pub(super) struct SshAuxiliaryLease {
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    sftp: Arc<tokio::sync::Mutex<Option<SftpSession>>>,
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
    session: tokio::sync::OwnedMutexGuard<Option<SftpSession>>,
}

impl Deref for SftpSessionLease {
    type Target = SftpSession;

    fn deref(&self) -> &Self::Target {
        self.session
            .as_ref()
            .expect("SFTP session lease must contain an initialized session")
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
    let slot = acquire_ssh_auxiliary_slot(state)?;
    let resources = ssh_resources_for_auxiliary_operation(state, session_id)?;
    Ok(SshAuxiliaryLease {
        handle: resources.handle,
        sftp: resources.sftp,
        _slot: slot,
    })
}

pub(super) async fn copy_local_file_for_transfer(
    source: &str,
    destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let source = PathBuf::from(source);
    let destination = PathBuf::from(destination);
    let (mut input, total) = open_local_transfer_source(&source, "local transfer")?;
    prepare_local_transfer_target_path(&destination, "本地传输目标路径")?;
    let temp_destination = local_resume_part_path(&destination);
    let mut copied =
        local_resume_offset_matching_local_source(&mut input, &temp_destination, total, progress)?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, total).await?;
    }
    if total > 0 && copied == total {
        finalize_local_resume_file(&temp_destination, &destination)?;
        return Ok(copied);
    }
    let mut output = open_local_resume_writer(&temp_destination, copied)
        .map_err(|error| format!("local transfer create failed: {error}"))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        progress.check_cancelled()?;
        let read = input
            .read(&mut buffer)
            .map_err(|error| format!("local transfer read failed: {error}"))?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .map_err(|error| format!("local transfer write failed: {error}"))?;
        copied += read as u64;
        ensure_transfer_not_oversized(copied, total, "local transfer")?;
        progress.update(copied, total).await?;
    }
    ensure_exact_transfer_size(copied, total, "local transfer")?;
    output
        .flush()
        .map_err(|error| format!("local transfer flush failed: {error}"))?;
    drop(output);
    finalize_local_resume_file(&temp_destination, &destination)?;
    Ok(copied)
}

pub(super) async fn open_sftp_session(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
) -> Result<SftpSession, String> {
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
        let channel = handle
            .russh_compat()?
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

#[cfg(test)]
pub(super) async fn open_sftp_session_with_timeout<H: client::Handler>(
    handle: Arc<tokio::sync::Mutex<client::Handle<H>>>,
    timeout: Duration,
) -> Result<SftpSession, String> {
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
        Ok::<_, String>(sftp)
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

/// A transfer writes to a temp sibling of the real destination and is only
/// renamed onto it after a full success; on any error the temp is best-effort
/// removed. Otherwise a mid-transfer failure leaves a partial file at the real
/// destination path with nothing distinguishing it from a complete one.
pub(super) fn local_resume_part_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("portmate-transfer");
    target.with_file_name(format!("{name}.portmate-part"))
}

pub(super) fn local_resume_offset(path: &Path, total: u64) -> Result<u64, String> {
    let Some(metadata) = local_transfer_entry(path, "本地断点文件")? else {
        return Ok(0);
    };
    let size = metadata.len();
    if size <= total {
        Ok(size)
    } else {
        fs::remove_file(path)
            .map_err(|error| format!("删除过大的断点文件失败 {}: {error}", path.display()))?;
        Ok(0)
    }
}

pub(super) fn open_local_transfer_reader(path: &Path, label: &str) -> Result<fs::File, String> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options
        .open(path)
        .map_err(|error| format!("打开{label}失败 {}: {error}", path.display()))
}

pub(super) fn local_resume_offset_matching_local_source(
    source: &mut fs::File,
    part_path: &Path,
    total: u64,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let offset = local_resume_offset(part_path, total)?;
    if offset == 0 {
        return Ok(0);
    }
    source
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|error| format!("local transfer source seek failed: {error}"))?;
    let mut part = open_local_transfer_reader(part_path, "本地断点文件")?;
    let matches = compare_local_prefix(source, &mut part, offset, progress)?;
    source
        .seek(std::io::SeekFrom::Start(if matches { offset } else { 0 }))
        .map_err(|error| format!("local transfer source seek failed: {error}"))?;
    Ok(if matches { offset } else { 0 })
}

pub(super) fn compare_local_prefix(
    left: &mut fs::File,
    right: &mut fs::File,
    length: u64,
    progress: &TransferProgressContext,
) -> Result<bool, String> {
    let mut left_buffer = vec![0_u8; 64 * 1024];
    let mut right_buffer = vec![0_u8; 64 * 1024];
    let mut compared = 0_u64;
    while compared < length {
        progress.check_cancelled()?;
        let remaining = usize::try_from(length - compared).unwrap_or(usize::MAX);
        let take = remaining.min(left_buffer.len());
        match left.read_exact(&mut left_buffer[..take]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(format!("读取本地源文件前缀失败: {error}")),
        }
        match right.read_exact(&mut right_buffer[..take]) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(format!("读取本地断点文件前缀失败: {error}")),
        }
        if left_buffer[..take] != right_buffer[..take] {
            return Ok(false);
        }
        compared += take as u64;
    }
    Ok(true)
}

pub(super) fn ensure_exact_transfer_size(
    copied: u64,
    total: u64,
    label: &str,
) -> Result<(), String> {
    if copied == total {
        Ok(())
    } else {
        Err(format!(
            "{label} size mismatch: copied {copied}, expected {total}"
        ))
    }
}

pub(super) fn ensure_transfer_not_oversized(
    copied: u64,
    total: u64,
    label: &str,
) -> Result<(), String> {
    if copied <= total {
        Ok(())
    } else {
        Err(format!(
            "{label} size mismatch: copied {copied}, expected {total}"
        ))
    }
}

pub(super) fn open_local_resume_writer(path: &Path, offset: u64) -> std::io::Result<fs::File> {
    if let Err(error) = local_transfer_entry(path, "本地断点文件") {
        return Err(std::io::Error::other(error));
    }
    if offset == 0 {
        let mut options = OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        options.open(path)
    } else {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        options.open(path)
    }
}

pub(super) fn finalize_local_resume_file(temp: &Path, target: &Path) -> Result<(), String> {
    if local_transfer_entry(temp, "本地断点文件")?.is_none() {
        return Err(format!("本地断点文件不存在: {}", temp.display()));
    }
    if local_transfer_entry(target, "本地目标文件")?.is_some() {
        fs::remove_file(target)
            .map_err(|error| format!("删除旧目标文件失败 {}: {error}", target.display()))?;
    }
    fs::rename(temp, target).map_err(|error| {
        format!(
            "重命名本地目标文件失败 {} -> {}: {error}",
            temp.display(),
            target.display()
        )
    })
}

pub(super) fn local_transfer_entry(
    path: &Path,
    label: &str,
) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("{label}不能是符号链接: {}", path.display()))
        }
        Ok(metadata) if !metadata.is_file() => {
            Err(format!("{label}不是普通文件: {}", path.display()))
        }
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("检查{label}失败 {}: {error}", path.display())),
    }
}

pub(super) fn open_local_transfer_source(
    path: &Path,
    label: &str,
) -> Result<(fs::File, u64), String> {
    let metadata = local_transfer_entry(path, label)?
        .ok_or_else(|| format!("{label}不存在: {}", path.display()))?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options
        .open(path)
        .map_err(|error| format!("打开{label}失败 {}: {error}", path.display()))?;
    let opened_metadata = file
        .metadata()
        .map_err(|error| format!("检查{label}失败 {}: {error}", path.display()))?;
    if !opened_metadata.is_file() || opened_metadata.len() != metadata.len() {
        return Err(format!("{label}在打开前后发生变化: {}", path.display()));
    }
    Ok((file, metadata.len()))
}

pub(super) fn ensure_local_transfer_source_size(
    file: &fs::File,
    expected_size: u64,
    label: &str,
) -> Result<(), String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("检查{label}失败: {error}"))?;
    if metadata.is_file() && metadata.len() == expected_size {
        Ok(())
    } else {
        Err(format!("{label}在传输期间发生变化"))
    }
}

pub(super) fn local_transfer_source_size(path: &str) -> Result<u64, String> {
    local_transfer_entry(Path::new(path), "本地传输源")?
        .map(|metadata| metadata.len())
        .ok_or_else(|| format!("本地传输源不存在: {path}"))
}

pub(super) fn open_new_local_transfer_file(target: &Path) -> Result<(fs::File, PathBuf), String> {
    let _ = local_transfer_entry(target, "本地目标文件")?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-transfer");
    let temp = target.with_file_name(format!(".{name}.portmate-{}.part", Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(&temp)
        .map_err(|error| format!("创建本地传输临时文件失败 {}: {error}", temp.display()))?;
    Ok((file, temp))
}

pub(super) fn remote_resume_part_path(target: &str) -> String {
    match target.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/{name}.portmate-part"),
        None => format!("{target}.portmate-part"),
    }
}

pub(super) async fn sftp_resume_offset(
    sftp: &SftpSession,
    path: &str,
    total: u64,
) -> Result<u64, String> {
    let Some(size) = sftp_regular_file_size(sftp, path, "SFTP 断点文件").await? else {
        return Ok(0);
    };
    if size <= total {
        Ok(size)
    } else {
        sftp.remove_file(path.to_string())
            .await
            .map_err(|error| format!("SFTP 删除过大的断点文件失败 {path}: {error}"))?;
        Ok(0)
    }
}

pub(super) async fn sftp_resume_offset_matching_local_source(
    sftp: &SftpSession,
    part_path: &str,
    total: u64,
    source: &mut fs::File,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let offset = sftp_resume_offset(sftp, part_path, total).await?;
    if offset == 0 {
        return Ok(0);
    }
    source
        .seek(std::io::SeekFrom::Start(0))
        .map_err(|error| format!("SFTP 本地源文件 seek 失败: {error}"))?;
    let mut part = sftp
        .open(part_path.to_string())
        .await
        .map_err(|error| format!("SFTP 打开断点文件失败 {part_path}: {error}"))?;
    let matches = compare_sftp_and_local_prefix(&mut part, source, offset, progress).await;
    let _ = part.shutdown().await;
    let matches = matches?;
    source
        .seek(std::io::SeekFrom::Start(if matches { offset } else { 0 }))
        .map_err(|error| format!("SFTP 本地源文件 seek 失败: {error}"))?;
    Ok(if matches { offset } else { 0 })
}

pub(super) async fn local_resume_offset_matching_sftp_source(
    source: &mut russh_sftp::client::fs::File,
    part_path: &Path,
    total: u64,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let offset = local_resume_offset(part_path, total)?;
    if offset == 0 {
        return Ok(0);
    }
    source
        .seek(std::io::SeekFrom::Start(0))
        .await
        .map_err(|error| format!("SFTP 远端源文件 seek 失败: {error}"))?;
    let mut part = open_local_transfer_reader(part_path, "本地断点文件")?;
    let matches = compare_sftp_and_local_prefix(source, &mut part, offset, progress).await?;
    source
        .seek(std::io::SeekFrom::Start(if matches { offset } else { 0 }))
        .await
        .map_err(|error| format!("SFTP 远端源文件 seek 失败: {error}"))?;
    Ok(if matches { offset } else { 0 })
}

pub(super) async fn sftp_resume_offset_matching_sftp_source(
    sftp: &SftpSession,
    source: &mut russh_sftp::client::fs::File,
    part_path: &str,
    total: u64,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let offset = sftp_resume_offset(sftp, part_path, total).await?;
    if offset == 0 {
        return Ok(0);
    }
    source
        .seek(std::io::SeekFrom::Start(0))
        .await
        .map_err(|error| format!("SFTP 远端源文件 seek 失败: {error}"))?;
    let mut part = sftp
        .open(part_path.to_string())
        .await
        .map_err(|error| format!("SFTP 打开断点文件失败 {part_path}: {error}"))?;
    let matches = compare_sftp_prefixes(source, &mut part, offset, progress).await;
    let _ = part.shutdown().await;
    let matches = matches?;
    source
        .seek(std::io::SeekFrom::Start(if matches { offset } else { 0 }))
        .await
        .map_err(|error| format!("SFTP 远端源文件 seek 失败: {error}"))?;
    Ok(if matches { offset } else { 0 })
}

pub(super) async fn compare_sftp_and_local_prefix(
    remote: &mut russh_sftp::client::fs::File,
    local: &mut fs::File,
    length: u64,
    progress: &TransferProgressContext,
) -> Result<bool, String> {
    let mut remote_buffer = vec![0_u8; 64 * 1024];
    let mut local_buffer = vec![0_u8; 64 * 1024];
    let mut compared = 0_u64;
    while compared < length {
        progress.check_cancelled()?;
        let remaining = usize::try_from(length - compared).unwrap_or(usize::MAX);
        let take = remaining.min(remote_buffer.len());
        match remote.read_exact(&mut remote_buffer[..take]).await {
            Ok(_) => {}
            Err(error) => return prefix_read_mismatch_or_error(error, "SFTP 远端断点文件"),
        }
        match std::io::Read::read_exact(local, &mut local_buffer[..take]) {
            Ok(()) => {}
            Err(error) => return prefix_read_mismatch_or_error(error, "本地源文件"),
        }
        if remote_buffer[..take] != local_buffer[..take] {
            return Ok(false);
        }
        compared += take as u64;
    }
    Ok(true)
}

pub(super) async fn compare_sftp_prefixes(
    left: &mut russh_sftp::client::fs::File,
    right: &mut russh_sftp::client::fs::File,
    length: u64,
    progress: &TransferProgressContext,
) -> Result<bool, String> {
    let mut left_buffer = vec![0_u8; 64 * 1024];
    let mut right_buffer = vec![0_u8; 64 * 1024];
    let mut compared = 0_u64;
    while compared < length {
        progress.check_cancelled()?;
        let remaining = usize::try_from(length - compared).unwrap_or(usize::MAX);
        let take = remaining.min(left_buffer.len());
        match left.read_exact(&mut left_buffer[..take]).await {
            Ok(_) => {}
            Err(error) => return prefix_read_mismatch_or_error(error, "SFTP 远端源文件"),
        }
        match right.read_exact(&mut right_buffer[..take]).await {
            Ok(_) => {}
            Err(error) => return prefix_read_mismatch_or_error(error, "SFTP 远端断点文件"),
        }
        if left_buffer[..take] != right_buffer[..take] {
            return Ok(false);
        }
        compared += take as u64;
    }
    Ok(true)
}

pub(super) fn prefix_read_mismatch_or_error(
    error: std::io::Error,
    label: &str,
) -> Result<bool, String> {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        Ok(false)
    } else {
        Err(format!("读取{label}前缀失败: {error}"))
    }
}

pub(super) async fn sftp_regular_file_size(
    sftp: &SftpSession,
    path: &str,
    label: &str,
) -> Result<Option<u64>, String> {
    let metadata = match sftp.symlink_metadata(path.to_string()).await {
        Ok(metadata) => metadata,
        Err(error) => {
            let exists = match sftp.try_exists(path.to_string()).await {
                Ok(exists) => exists,
                Err(exists_error) => {
                    return Err(format!(
                        "{label}无法读取远端属性 {path}: {error}; existence check failed: {exists_error}"
                    ));
                }
            };
            if exists {
                return Err(format!("{label}无法读取远端属性 {path}: {error}"));
            }
            return Ok(None);
        }
    };
    if metadata.is_symlink() {
        return Err(format!("{label}不能是符号链接: {path}"));
    }
    if !metadata.is_regular() {
        return Err(format!("{label}不是普通文件: {path}"));
    }
    Ok(Some(metadata.len()))
}

pub(super) async fn sftp_open_resume_writer(
    sftp: &SftpSession,
    path: &str,
    offset: u64,
) -> Result<russh_sftp::client::fs::File, String> {
    let _ = sftp_regular_file_size(sftp, path, "SFTP 断点文件").await?;
    let flags = if offset == 0 {
        OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE
    } else {
        OpenFlags::CREATE | OpenFlags::WRITE
    };
    let mut file = sftp
        .open_with_flags(path.to_string(), flags)
        .await
        .map_err(|error| format!("SFTP 打开断点文件失败 {path}: {error}"))?;
    if offset > 0 {
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|error| format!("SFTP 断点文件 seek 失败 {path}: {error}"))?;
    }
    Ok(file)
}

pub(super) async fn sftp_finalize_resume_file(
    sftp: &SftpSession,
    temp: &str,
    target: &str,
) -> Result<(), String> {
    if sftp_regular_file_size(sftp, temp, "SFTP 断点文件")
        .await?
        .is_none()
    {
        return Err(format!("SFTP 断点文件不存在: {temp}"));
    }
    if sftp_regular_file_size(sftp, target, "SFTP 目标文件")
        .await?
        .is_some()
    {
        sftp.remove_file(target.to_string())
            .await
            .map_err(|error| format!("SFTP 删除旧目标文件失败 {target}: {error}"))?;
    }
    sftp.rename(temp.to_string(), target.to_string())
        .await
        .map_err(|error| format!("SFTP 重命名断点文件失败 {temp} -> {target}: {error}"))
}

pub(super) async fn sftp_upload(
    sftp: &SftpSession,
    local_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let (mut local_file, total) =
        open_local_transfer_source(Path::new(local_source), "SFTP upload")?;
    let file_name = Path::new(local_source)
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("portmate-upload.bin");
    let target = sftp_destination_file_path(sftp, remote_destination, file_name).await?;
    let temp_target = remote_resume_part_path(&target);
    let mut copied = sftp_resume_offset_matching_local_source(
        sftp,
        &temp_target,
        total,
        &mut local_file,
        progress,
    )
    .await?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, total).await?;
    }
    if total > 0 && copied == total {
        sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
        return Ok(copied);
    }
    let mut remote_file = sftp_open_resume_writer(sftp, &temp_target, copied).await?;

    let copy_result: Result<u64, String> = async {
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            progress.check_cancelled()?;
            let read = local_file
                .read(&mut buffer)
                .map_err(|error| format!("读取本地文件失败 {local_source}: {error}"))?;
            if read == 0 {
                break;
            }
            remote_file
                .write_all(&buffer[..read])
                .await
                .map_err(|error| format!("SFTP 写入远端文件失败 {temp_target}: {error}"))?;
            copied += read as u64;
            ensure_transfer_not_oversized(copied, total, "SFTP upload")?;
            progress.update(copied, total).await?;
        }
        ensure_exact_transfer_size(copied, total, "SFTP upload")?;
        remote_file
            .flush()
            .await
            .map_err(|error| format!("SFTP 刷新远端文件失败 {temp_target}: {error}"))?;
        remote_file
            .shutdown()
            .await
            .map_err(|error| format!("SFTP 关闭远端文件失败 {temp_target}: {error}"))?;
        Ok(copied)
    }
    .await;

    match copy_result {
        Ok(copied) => {
            sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
            Ok(copied)
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn sftp_download(
    sftp: &SftpSession,
    remote_source: &str,
    local_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let total = sftp_regular_file_size(sftp, remote_source, "SFTP 远端源文件")
        .await?
        .ok_or_else(|| format!("SFTP 远端源文件不存在: {remote_source}"))?;
    let mut remote_file = sftp
        .open(remote_source.to_string())
        .await
        .map_err(|error| format!("SFTP 打开远端文件失败 {remote_source}: {error}"))?;
    let target = local_destination_file_path(local_destination, remote_source)?;
    prepare_local_transfer_target_path(&target, "SFTP 本地目标路径")?;
    let temp_target = local_resume_part_path(&target);
    let mut copied =
        local_resume_offset_matching_sftp_source(&mut remote_file, &temp_target, total, progress)
            .await?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, total).await?;
    }
    if total > 0 && copied == total {
        finalize_local_resume_file(&temp_target, &target)?;
        let _ = remote_file.shutdown().await;
        return Ok(copied);
    }
    let mut local_file = open_local_resume_writer(&temp_target, copied)
        .map_err(|error| format!("创建本地目标文件失败 {}: {error}", temp_target.display()))?;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        progress.check_cancelled()?;
        let read = remote_file
            .read(&mut buffer)
            .await
            .map_err(|error| format!("SFTP 读取远端文件失败 {remote_source}: {error}"))?;
        if read == 0 {
            break;
        }
        local_file
            .write_all(&buffer[..read])
            .map_err(|error| format!("写入本地目标文件失败 {}: {error}", temp_target.display()))?;
        copied += read as u64;
        ensure_transfer_not_oversized(copied, total, "SFTP download")?;
        progress.update(copied, total).await?;
    }
    ensure_exact_transfer_size(copied, total, "SFTP download")?;
    local_file
        .flush()
        .map_err(|error| format!("刷新本地目标文件失败 {}: {error}", temp_target.display()))?;
    drop(local_file);
    finalize_local_resume_file(&temp_target, &target)?;
    let _ = remote_file.shutdown().await;
    Ok(copied)
}

pub(super) async fn sftp_remote_copy(
    sftp: &SftpSession,
    remote_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    let total = sftp_regular_file_size(sftp, remote_source, "SFTP 远端源文件")
        .await?
        .ok_or_else(|| format!("SFTP 远端源文件不存在: {remote_source}"))?;
    let mut source_file = sftp
        .open(remote_source.to_string())
        .await
        .map_err(|error| format!("SFTP 打开远端源文件失败 {remote_source}: {error}"))?;
    let file_name = remote_file_name(remote_source);
    let target = sftp_destination_file_path(sftp, remote_destination, &file_name).await?;
    let temp_target = remote_resume_part_path(&target);
    let mut copied = sftp_resume_offset_matching_sftp_source(
        sftp,
        &mut source_file,
        &temp_target,
        total,
        progress,
    )
    .await?;
    progress.set_rate_baseline(copied);
    if copied > 0 {
        progress.update(copied, total).await?;
    }
    if total > 0 && copied == total {
        sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
        let _ = source_file.shutdown().await;
        return Ok(copied);
    }
    let mut target_file = sftp_open_resume_writer(sftp, &temp_target, copied).await?;

    let copy_result: Result<u64, String> = async {
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            progress.check_cancelled()?;
            let read = source_file
                .read(&mut buffer)
                .await
                .map_err(|error| format!("SFTP 读取远端源文件失败 {remote_source}: {error}"))?;
            if read == 0 {
                break;
            }
            target_file
                .write_all(&buffer[..read])
                .await
                .map_err(|error| format!("SFTP 写入远端目标文件失败 {temp_target}: {error}"))?;
            copied += read as u64;
            ensure_transfer_not_oversized(copied, total, "SFTP remote copy")?;
            progress.update(copied, total).await?;
        }
        ensure_exact_transfer_size(copied, total, "SFTP remote copy")?;
        target_file
            .flush()
            .await
            .map_err(|error| format!("SFTP 刷新远端目标文件失败 {temp_target}: {error}"))?;
        target_file
            .shutdown()
            .await
            .map_err(|error| format!("SFTP 关闭远端目标文件失败 {temp_target}: {error}"))?;
        Ok(copied)
    }
    .await;
    let _ = source_file.shutdown().await;

    match copy_result {
        Ok(copied) => {
            sftp_finalize_resume_file(sftp, &temp_target, &target).await?;
            Ok(copied)
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn sftp_destination_file_path(
    sftp: &SftpSession,
    remote_destination: &str,
    source_name: &str,
) -> Result<String, String> {
    let destination = remote_destination.trim();
    if destination.is_empty() {
        return Err("SFTP 远端目标路径不能为空".to_string());
    }

    if destination.ends_with('/') {
        sftp_create_dir_all(sftp, destination).await?;
        return Ok(remote_join_path(destination, source_name));
    }

    if let Ok(metadata) = sftp.metadata(destination.to_string()).await {
        if metadata.is_dir() {
            return Ok(remote_join_path(destination, source_name));
        }
    }

    if let Some(parent) = remote_parent_path(destination) {
        if parent != "." && parent != "/" {
            sftp_create_dir_all(sftp, &parent).await?;
        }
    }
    Ok(destination.to_string())
}

pub(super) fn local_destination_file_path(
    local_destination: &str,
    remote_source: &str,
) -> Result<PathBuf, String> {
    let destination = expand_identity_path(local_destination.trim());
    if local_destination.trim().is_empty() {
        return Err("本地目标路径不能为空".to_string());
    }
    let source_name = remote_file_name(remote_source);
    let ends_with_separator = local_destination.ends_with('/') || local_destination.ends_with('\\');
    if destination.is_dir() || ends_with_separator {
        Ok(destination.join(source_name))
    } else {
        Ok(destination)
    }
}

pub(super) async fn sftp_create_dir_all(sftp: &SftpSession, path: &str) -> Result<(), String> {
    let path = path.trim().trim_end_matches('/');
    if path.is_empty() || path == "." || path == "/" {
        return Ok(());
    }

    let mut current = if path.starts_with('/') {
        "/".to_string()
    } else {
        String::new()
    };
    for part in path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        current = remote_join_path(&current, part);
        match sftp.create_dir(current.clone()).await {
            Ok(()) => {}
            Err(error) => match sftp.symlink_metadata(current.clone()).await {
                Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {}
                _ => return Err(format!("SFTP 创建远端目录失败 {current}: {error}")),
            },
        }
    }
    Ok(())
}

pub(super) async fn reject_remote_symlink_components(
    sftp: &SftpSession,
    path: &str,
    allow_final_symlink: bool,
    label: &str,
) -> Result<(), String> {
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let mut current = if path.starts_with('/') {
        "/".to_string()
    } else {
        String::new()
    };
    for (index, part) in parts.iter().enumerate() {
        current = remote_join_path(&current, part);
        let metadata = match sftp.symlink_metadata(current.clone()).await {
            Ok(metadata) => metadata,
            Err(error) => match sftp.try_exists(current.clone()).await {
                Ok(false) => break,
                Ok(true) => {
                    return Err(format!("无法检查{label} {current}: {error}"));
                }
                Err(exists_error) => {
                    return Err(format!(
                        "无法检查{label} {current}: {error}; existence check failed: {exists_error}"
                    ));
                }
            },
        };
        if metadata.is_symlink() && !(allow_final_symlink && index + 1 == parts.len()) {
            return Err(format!("{label}不能经过符号链接: {current}"));
        }
    }
    Ok(())
}

pub(super) fn remote_parent_path(path: &str) -> Option<String> {
    let path = path.trim().trim_end_matches('/');
    let index = path.rfind('/')?;
    if index == 0 {
        Some("/".to_string())
    } else {
        Some(path[..index].to_string())
    }
}

pub(super) fn remote_file_name(path: &str) -> String {
    portable_file_name(path).unwrap_or_else(|| "portmate-file.bin".to_string())
}

pub(super) fn remote_join_path(parent: &str, name: &str) -> String {
    let name = name.trim_matches('/');
    if parent.is_empty() || parent == "." {
        name.to_string()
    } else if parent == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}

pub(super) async fn remote_copy(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    remote_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
) -> Result<u64, String> {
    remote_copy_with_timeouts(
        handle,
        remote_source,
        remote_destination,
        progress,
        REMOTE_COPY_IO_IDLE_TIMEOUT,
        REMOTE_COPY_TOTAL_TIMEOUT,
    )
    .await
}

pub(super) async fn remote_copy_with_timeouts<H: SshExecChannelOpener>(
    handle: H,
    remote_source: &str,
    remote_destination: &str,
    progress: &TransferProgressContext,
    idle_timeout: Duration,
    total_timeout: Duration,
) -> Result<u64, String> {
    let command = remote_copy_command(remote_source, remote_destination);
    let mut channel = handle
        .open_exec_channel(&command, SSH_AUXILIARY_SETUP_TIMEOUT, "SSH remote copy")
        .await?;

    let mut output = Vec::new();
    let mut stderr = Vec::new();
    let mut exit_status = None;
    let mut eof_received_at: Option<Instant> = None;
    let mut reported = RemoteCopyMarkers::default();
    let started = Instant::now();
    let mut last_progress = Instant::now();

    let outcome = async {
        loop {
            progress.check_cancelled()?;
            if ssh_exec_status_grace_expired(eof_received_at) {
                break;
            }
            let message = {
                let wait = channel.wait();
                tokio::pin!(wait);
                loop {
                    let idle_remaining = idle_timeout.saturating_sub(last_progress.elapsed());
                    if idle_remaining.is_zero() {
                        break Err(format!(
                            "SSH remote copy 空闲超时（{} ms）",
                            idle_timeout.as_millis()
                        ));
                    }
                    let total_remaining = total_timeout.saturating_sub(started.elapsed());
                    if total_remaining.is_zero() {
                        break Err(format!(
                            "SSH remote copy 总超时（{} ms）",
                            total_timeout.as_millis()
                        ));
                    }
                    tokio::select! {
                        message = &mut wait => break Ok(message),
                        _ = tokio::time::sleep(
                            idle_remaining
                                .min(total_remaining)
                                .min(TRANSFER_CANCEL_POLL_INTERVAL)
                        ) => {
                            progress.check_cancelled()?;
                        }
                    }
                }
            }?;

            match message {
                Some(SshBackendMessage::Data(data)) => {
                    append_bounded_ssh_exec_data(
                        &mut output,
                        &data,
                        MAX_SSH_EXEC_STDOUT_BYTES,
                        "remote copy stdout",
                    )?;
                    let markers = remote_copy_markers(&output);
                    validate_remote_copy_markers(&markers, &reported)?;
                    let made_progress = markers != reported;
                    if markers.total.is_some() && markers.total != reported.total {
                        let total = markers.total.unwrap_or_default();
                        progress.update(0, total).await?;
                        reported.total = Some(total);
                    }
                    if markers.resume.is_some() && markers.resume != reported.resume {
                        let resume_bytes = markers.resume.unwrap_or_default();
                        progress.set_rate_baseline(resume_bytes);
                        progress
                            .update(resume_bytes, markers.total.or(reported.total).unwrap_or(0))
                            .await?;
                        reported.resume = Some(resume_bytes);
                    }
                    if markers.progress.is_some() && markers.progress != reported.progress {
                        let progress_bytes = markers.progress.unwrap_or_default();
                        progress
                            .update(
                                progress_bytes,
                                markers.total.or(reported.total).unwrap_or(0),
                            )
                            .await?;
                        reported.progress = Some(progress_bytes);
                    }
                    if markers.done.is_some() && markers.done != reported.done {
                        let done = markers.done.unwrap_or_default();
                        progress
                            .update(done, reported.total.unwrap_or(done))
                            .await?;
                        reported.done = Some(done);
                    }
                    if made_progress {
                        last_progress = Instant::now();
                    }
                }
                Some(SshBackendMessage::ExtendedData { data, .. }) => append_bounded_ssh_exec_data(
                    &mut stderr,
                    &data,
                    MAX_SSH_EXEC_STDERR_BYTES,
                    "remote copy stderr",
                )?,
                Some(message) => {
                    if ssh_exec_message_completes(&message, &mut exit_status, &mut eof_received_at)
                    {
                        break;
                    }
                }
                None => break,
            }
        }

        if let Some(code) = exit_status.filter(|code| *code != 0) {
            return Err(format!(
                "SSH remote copy 返回非零状态 {code}: {}",
                String::from_utf8_lossy(&stderr)
            ));
        }

        let markers = remote_copy_markers(&output);
        let bytes = markers.done.ok_or_else(|| {
            format!(
                "remote copy completed but done marker was missing: {}",
                String::from_utf8_lossy(&output)
            )
        })?;
        progress
            .update(bytes, reported.total.unwrap_or(bytes))
            .await?;
        Ok(bytes)
    }
    .await;
    close_ssh_channel_bounded(&channel).await;
    outcome
}

pub(super) fn remote_copy_command(remote_source: &str, remote_destination: &str) -> String {
    format!(
        concat!(
            "src={}; dst={}; target=; part=; pid=; ",
            "remote_name=${{src##*/}}; if [ -z \"$remote_name\" ]; then remote_name=portmate-file.bin; fi; ",
            "case \"$dst\" in */) target=\"${{dst%/}}/$remote_name\" ;; ",
            "*) if [ -d \"$dst\" ]; then target=\"${{dst%/}}/$remote_name\"; else target=\"$dst\"; fi ;; esac; ",
            "case \"$target\" in */*) part=\"${{target%/*}}/${{target##*/}}.portmate-part\" ;; ",
            "*) part=\"$target.portmate-part\" ;; esac; ",
            "portable_path() {{ case \"$1\" in -*) printf './%s\\n' \"$1\" ;; *) printf '%s\\n' \"$1\" ;; esac; }}; ",
            "src=$(portable_path \"$src\") || exit 1; target=$(portable_path \"$target\") || exit 1; part=$(portable_path \"$part\") || exit 1; ",
            "reject_link() {{ if [ -L \"$1\" ]; then printf 'PortMate refuses symbolic link: %s\\n' \"$1\" >&2; return 1; fi; }}; ",
            "file_size() {{ value=$(wc -c < \"$1\") || return 1; value=$(printf '%s' \"$value\" | tr -d '[:space:]') || return 1; case \"$value\" in ''|*[!0-9]*) return 1 ;; esac; printf '%s\\n' \"$value\"; }}; ",
            "cleanup() {{ if [ -n \"$pid\" ]; then kill \"$pid\" 2>/dev/null || :; fi; }}; ",
            "trap cleanup INT TERM HUP EXIT; ",
            "if ! reject_link \"$src\" || [ ! -f \"$src\" ]; then exit 1; fi; ",
            "if ! reject_link \"$part\" || ! reject_link \"$target\"; then exit 1; fi; ",
            "if ! total=$(file_size \"$src\"); then exit 1; fi; ",
            "printf '__PORTMATE_SIZE__%s\\n' \"$total\"; ",
            "offset=0; ",
            "if [ -e \"$part\" ]; then ",
            "if current=$(file_size \"$part\" 2>/dev/null); then ",
            "if [ \"$current\" -le \"$total\" ]; then ",
            "if [ \"$current\" -eq 0 ] || head -c \"$current\" \"$src\" | cmp -s - \"$part\"; then offset=$current; else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "else : > \"$part\" || exit 1; fi; ",
            "printf '__PORTMATE_RESUME__%s\\n' \"$offset\"; ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$offset\"; ",
            "if [ \"$offset\" -lt \"$total\" ]; then ",
            "tail -c +$((offset + 1)) \"$src\" >> \"$part\" & pid=$!; ",
            "while kill -0 \"$pid\" 2>/dev/null; do ",
            "if current=$(file_size \"$part\" 2>/dev/null); then ",
            "printf '__PORTMATE_PROGRESS__%s\\n' \"$current\"; ",
            "fi; sleep 0.25; done; ",
            "wait \"$pid\"; status=$?; pid=; ",
            "if [ \"$status\" -ne 0 ]; then exit \"$status\"; fi; ",
            "fi; ",
            "final=$(file_size \"$part\") || exit 1; ",
            "if [ \"$final\" -ne \"$total\" ]; then ",
            "printf 'PortMate remote copy size mismatch: %s of %s\\n' \"$final\" \"$total\" >&2; exit 1; ",
            "fi; ",
            "if ! reject_link \"$part\" || ! reject_link \"$target\"; then exit 1; fi; ",
            "mv -f \"$part\" \"$target\" || exit 1; ",
            "final_target=$(file_size \"$target\") || exit 1; printf '__PORTMATE_DONE__%s\\n' \"$final_target\""
        ),
        shell_quote(remote_source),
        shell_quote(remote_destination)
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct RemoteCopyMarkers {
    pub(super) total: Option<u64>,
    pub(super) resume_candidate: Option<u64>,
    pub(super) resume: Option<u64>,
    pub(super) progress: Option<u64>,
    pub(super) done: Option<u64>,
}

pub(super) fn validate_remote_copy_markers(
    markers: &RemoteCopyMarkers,
    reported: &RemoteCopyMarkers,
) -> Result<(), String> {
    if let (Some(previous), Some(current)) = (reported.total, markers.total) {
        if current != previous {
            return Err(format!(
                "SSH remote copy size marker changed from {previous} to {current}"
            ));
        }
    }
    if let (Some(previous), Some(current)) = (reported.resume, markers.resume) {
        if current != previous {
            return Err(format!(
                "SSH remote copy resume marker changed from {previous} to {current}"
            ));
        }
    }
    if let (Some(previous), Some(current)) = (reported.progress, markers.progress) {
        if current < previous {
            return Err(format!(
                "SSH remote copy progress marker moved backwards from {previous} to {current}"
            ));
        }
    }
    if let (Some(previous), Some(current)) = (reported.done, markers.done) {
        if current != previous {
            return Err(format!(
                "SSH remote copy done marker changed from {previous} to {current}"
            ));
        }
    }

    let total = markers.total.or(reported.total);
    for (label, value) in [
        ("resume", markers.resume),
        ("progress", markers.progress),
        ("done", markers.done),
    ] {
        let Some(value) = value else {
            continue;
        };
        let total = total.ok_or_else(|| {
            format!("SSH remote copy {label} marker arrived before the size marker")
        })?;
        if value > total {
            return Err(format!(
                "SSH remote copy {label} marker {value} exceeds size {total}"
            ));
        }
    }
    if let (Some(total), Some(done)) = (total, markers.done) {
        if done != total {
            return Err(format!(
                "SSH remote copy done marker {done} does not match size {total}"
            ));
        }
    }
    Ok(())
}

pub(super) fn remote_copy_markers(output: &[u8]) -> RemoteCopyMarkers {
    let text = String::from_utf8_lossy(output);
    let mut markers = RemoteCopyMarkers::default();
    for line in text.split_inclusive('\n') {
        if !line.ends_with('\n') {
            continue;
        }
        if let Some(value) = line.trim().strip_prefix("__PORTMATE_SIZE__") {
            markers.total = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_RESUME_CANDIDATE__") {
            markers.resume_candidate = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_RESUME__") {
            markers.resume = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_PROGRESS__") {
            markers.progress = value.trim().parse::<u64>().ok();
        } else if let Some(value) = line.trim().strip_prefix("__PORTMATE_DONE__") {
            markers.done = value.trim().parse::<u64>().ok();
        }
    }
    markers
}

use super::*;

const MCP_HTTP_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const MCP_HTTP_READY_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MCP_HTTP_READY_CONNECT_TIMEOUT: Duration = Duration::from_millis(100);
const MCP_HTTP_READY_IO_TIMEOUT: Duration = Duration::from_millis(250);
const MAX_MCP_HTTP_READY_RESPONSE_BYTES: usize = 4 * 1024;
const MAX_MCP_HTTP_PROCESS_DIAGNOSTIC_BYTES: usize = 16 * 1024;
const MAX_MCP_HTTP_PROCESS_MESSAGE_CHARACTERS: usize = 1_024;

type McpHttpProcessDiagnostics = Arc<Mutex<VecDeque<u8>>>;

#[derive(Default)]
pub(super) struct McpHttpProcessRegistry {
    process: Option<ManagedMcpHttpProcess>,
    failure: Option<McpHttpProcessFailure>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct McpHttpProcessOwner {
    pid: u32,
    started_at: DateTime<Utc>,
}

struct ManagedMcpHttpProcess {
    child: std::process::Child,
    endpoint: String,
    connect_address: std::net::SocketAddr,
    started_at: DateTime<Utc>,
    ready: bool,
    diagnostics: McpHttpProcessDiagnostics,
}

struct McpHttpProcessFailure {
    pid: u32,
    endpoint: String,
    started_at: DateTime<Utc>,
    exit_status: String,
    diagnostics: McpHttpProcessDiagnostics,
}

impl Drop for ManagedMcpHttpProcess {
    fn drop(&mut self) {
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => {
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
}

impl ManagedMcpHttpProcess {
    fn owner(&self) -> McpHttpProcessOwner {
        McpHttpProcessOwner {
            pid: self.child.id(),
            started_at: self.started_at,
        }
    }
}

impl McpHttpProcessFailure {
    fn owner(&self) -> McpHttpProcessOwner {
        McpHttpProcessOwner {
            pid: self.pid,
            started_at: self.started_at,
        }
    }
}

pub(super) fn mcp_http_runtime_status_inner(
    state: &AppState,
) -> Result<McpHttpRuntimeStatus, String> {
    let pending_probe = {
        let mut registry = state
            .mcp_http_process
            .lock()
            .map_err(|error| error.to_string())?;
        reap_mcp_http_process(&mut registry)?;
        if let Some(process) = registry.process.as_ref() {
            if process.ready {
                return Ok(active_mcp_http_runtime_status(process));
            }
            Some((process.connect_address, process.child.id(), process.started_at))
        } else if let Some(failure) = registry.failure.as_ref() {
            return Ok(failed_mcp_http_runtime_status(failure));
        } else {
            None
        }
    };

    let Some((connect_address, pid, started_at)) = pending_probe else {
        return Ok(stopped_mcp_http_runtime_status());
    };
    let ready = probe_mcp_http_ready(connect_address);

    let mut registry = state
        .mcp_http_process
        .lock()
        .map_err(|error| error.to_string())?;
    reap_mcp_http_process(&mut registry)?;
    if let Some(process) = registry.process.as_mut() {
        if ready && process.child.id() == pid && process.started_at == started_at {
            process.ready = true;
        }
        return Ok(active_mcp_http_runtime_status(process));
    }
    if let Some(failure) = registry.failure.as_ref() {
        return Ok(failed_mcp_http_runtime_status(failure));
    }
    Ok(stopped_mcp_http_runtime_status())
}

pub(super) fn mcp_http_runtime_status_for_owner(
    state: &AppState,
    owner: McpHttpProcessOwner,
) -> Result<Option<McpHttpRuntimeStatus>, String> {
    let pending_probe = {
        let mut registry = state
            .mcp_http_process
            .lock()
            .map_err(|error| error.to_string())?;
        reap_mcp_http_process(&mut registry)?;
        if let Some(process) = registry
            .process
            .as_ref()
            .filter(|process| process.owner() == owner)
        {
            if process.ready {
                return Ok(Some(active_mcp_http_runtime_status(process)));
            }
            process.connect_address
        } else if let Some(failure) = registry
            .failure
            .as_ref()
            .filter(|failure| failure.owner() == owner)
        {
            return Ok(Some(failed_mcp_http_runtime_status(failure)));
        } else {
            return Ok(None);
        }
    };

    let ready = probe_mcp_http_ready(pending_probe);
    let mut registry = state
        .mcp_http_process
        .lock()
        .map_err(|error| error.to_string())?;
    reap_mcp_http_process(&mut registry)?;
    if let Some(process) = registry
        .process
        .as_mut()
        .filter(|process| process.owner() == owner)
    {
        if ready {
            process.ready = true;
        }
        return Ok(Some(active_mcp_http_runtime_status(process)));
    }
    if let Some(failure) = registry
        .failure
        .as_ref()
        .filter(|failure| failure.owner() == owner)
    {
        return Ok(Some(failed_mcp_http_runtime_status(failure)));
    }
    Ok(None)
}

pub(super) async fn start_mcp_http_runtime_inner(
    state: &AppState,
) -> Result<McpHttpRuntimeStatus, String> {
    let owner = start_mcp_http_process(state)?;
    let deadline = Instant::now() + MCP_HTTP_STARTUP_TIMEOUT;
    loop {
        let Some(status) = mcp_http_runtime_status_for_owner(state, owner)? else {
            return Err("MCP HTTP sidecar 启动已被停止或新的托管实例替换".to_string());
        };
        match status.phase {
            McpHttpRuntimePhase::Running => return Ok(status),
            McpHttpRuntimePhase::Failed => {
                return Err(status
                    .message
                    .unwrap_or_else(|| "MCP HTTP sidecar 启动失败".to_string()));
            }
            McpHttpRuntimePhase::Stopped => {
                return Err("MCP HTTP sidecar 在启动期间停止".to_string());
            }
            McpHttpRuntimePhase::Starting => {}
        }
        if Instant::now() >= deadline {
            let _ = stop_mcp_http_runtime_if_owned(state, owner);
            return Err(format!(
                "MCP HTTP sidecar 未能在 {} 秒内开始监听",
                MCP_HTTP_STARTUP_TIMEOUT.as_secs()
            ));
        }
        tokio::time::sleep(MCP_HTTP_READY_POLL_INTERVAL).await;
    }
}

pub(super) fn stop_mcp_http_runtime_inner(
    state: &AppState,
) -> Result<McpHttpRuntimeStatus, String> {
    let mut registry = state
        .mcp_http_process
        .lock()
        .map_err(|error| error.to_string())?;
    registry.failure = None;
    if let Some(process) = registry.process.take() {
        stop_mcp_http_process(process)?;
    }
    Ok(stopped_mcp_http_runtime_status())
}

pub(super) fn stop_mcp_http_runtime_if_owned(
    state: &AppState,
    owner: McpHttpProcessOwner,
) -> Result<bool, String> {
    let mut registry = state
        .mcp_http_process
        .lock()
        .map_err(|error| error.to_string())?;
    reap_mcp_http_process(&mut registry)?;
    if registry
        .failure
        .as_ref()
        .is_some_and(|failure| failure.owner() == owner)
    {
        return Ok(true);
    }
    if registry
        .process
        .as_ref()
        .is_none_or(|process| process.owner() != owner)
    {
        return Ok(false);
    }
    let process = registry.process.take().expect("owned MCP process present");
    stop_mcp_http_process(process)?;
    Ok(true)
}

fn stop_mcp_http_process(mut process: ManagedMcpHttpProcess) -> Result<(), String> {
    let running = process
        .child
        .try_wait()
        .map_err(|error| format!("无法检查 MCP HTTP sidecar: {error}"))?
        .is_none();
    if running {
        if let Err(kill_error) = process.child.kill() {
            let exited = process
                .child
                .try_wait()
                .map_err(|error| format!("无法重新检查 MCP HTTP sidecar: {error}"))?
                .is_some();
            if !exited {
                return Err(format!("无法停止 MCP HTTP sidecar: {kill_error}"));
            }
        }
    }
    process
        .child
        .wait()
        .map_err(|error| format!("无法回收 MCP HTTP sidecar: {error}"))?;
    Ok(())
}

pub(super) fn lock_stopped_mcp_http_runtime<'a>(
    state: &'a AppState,
    action: &str,
) -> Result<MutexGuard<'a, McpHttpProcessRegistry>, String> {
    let mut registry = state
        .mcp_http_process
        .lock()
        .map_err(|error| error.to_string())?;
    reap_mcp_http_process(&mut registry)?;
    if registry.process.is_some() {
        return Err(format!("请先停止 MCP HTTP 服务，再{action}"));
    }
    Ok(registry)
}

pub(super) fn shutdown_mcp_http_runtime(state: &AppState) {
    if let Err(error) = stop_mcp_http_runtime_inner(state) {
        eprintln!("PortMate: failed to stop managed MCP HTTP sidecar: {error}");
    }
}

fn start_mcp_http_process(state: &AppState) -> Result<McpHttpProcessOwner, String> {
    let mut registry = state
        .mcp_http_process
        .lock()
        .map_err(|error| error.to_string())?;
    reap_mcp_http_process(&mut registry)?;
    if registry.process.is_some() {
        return Err("MCP HTTP 服务已经由 PortMate 托管运行".to_string());
    }
    let settings = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .mcp_http_settings
        .clone();
    if !has_secret_ref(MCP_HTTP_TOKEN_REF) {
        return Err("请先生成 MCP HTTP Token".to_string());
    }
    let executable = mcp_sidecar_executable_path();
    if !executable.is_file() {
        return Err(format!(
            "找不到 MCP sidecar 可执行文件: {}",
            executable.display()
        ));
    }
    let config = build_mcp_http_config_for_request(
        true,
        &executable,
        &state.store_path,
        settings,
    )?;

    let bind_ip = config
        .settings
        .listen_host
        .parse::<std::net::IpAddr>()
        .map_err(|error| format!("无效的 MCP HTTP 监听地址: {error}"))?;
    let connect_ip = match bind_ip {
        std::net::IpAddr::V4(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        }
        std::net::IpAddr::V6(ip) if ip.is_unspecified() => {
            std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
        }
        _ => bind_ip,
    };
    let connect_address = std::net::SocketAddr::new(connect_ip, config.settings.port);
    let mut command = mcp_http_process_command(&executable, &state.store_path, &config);
    let child = command
        .spawn()
        .map_err(|error| format!("无法启动 MCP HTTP sidecar: {error}"))?;
    Ok(install_mcp_http_process(
        &mut registry,
        child,
        config.endpoint,
        connect_address,
    ))
}

fn install_mcp_http_process(
    registry: &mut McpHttpProcessRegistry,
    mut child: std::process::Child,
    endpoint: String,
    connect_address: std::net::SocketAddr,
) -> McpHttpProcessOwner {
    let diagnostics = child
        .stderr
        .take()
        .map(capture_mcp_http_process_diagnostics)
        .unwrap_or_else(|| Arc::new(Mutex::new(VecDeque::new())));
    let started_at = Utc::now();
    let owner = McpHttpProcessOwner {
        pid: child.id(),
        started_at,
    };
    registry.failure = None;
    registry.process = Some(ManagedMcpHttpProcess {
        child,
        endpoint,
        connect_address,
        started_at,
        ready: false,
        diagnostics,
    });
    owner
}

pub(super) fn mcp_http_process_command(
    executable: &Path,
    store_path: &Path,
    config: &McpHttpConfig,
) -> Command {
    let address = std::net::SocketAddr::new(
        config
            .settings
            .listen_host
            .parse()
            .expect("normalized MCP HTTP listen address"),
        config.settings.port,
    );
    let mut command = Command::new(executable);
    command
        .arg("--http")
        .env("PORTMATE_STORE_PATH", store_path)
        .env("PORTMATE_MCP_HTTP", "1")
        .env("PORTMATE_MCP_HTTP_ADDR", address.to_string())
        .env(
            "PORTMATE_MCP_HTTP_ORIGINS",
            config.settings.allowed_origins.join(","),
        )
        .env("PORTMATE_MCP_CLIENT_ID", &config.settings.client_id)
        .env(
            "PORTMATE_MCP_HTTP_ALLOW_REMOTE",
            u8::from(config.remote_access).to_string(),
        )
        .env(
            "PORTMATE_MCP_TRUSTED",
            u8::from(config.settings.trusted).to_string(),
        )
        .env("PORTMATE_MCP_PARENT_PID", std::process::id().to_string())
        .env_remove("PORTMATE_MCP_HTTP_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
pub(super) fn install_test_mcp_http_process(
    state: &AppState,
    command: &mut Command,
    endpoint: String,
    connect_address: std::net::SocketAddr,
) -> Result<McpHttpProcessOwner, String> {
    let mut registry = state
        .mcp_http_process
        .lock()
        .map_err(|error| error.to_string())?;
    reap_mcp_http_process(&mut registry)?;
    if registry.process.is_some() {
        return Err("MCP HTTP test process is already running".to_string());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|error| format!("failed to start MCP HTTP test process: {error}"))?;
    Ok(install_mcp_http_process(
        &mut registry,
        child,
        endpoint,
        connect_address,
    ))
}

fn capture_mcp_http_process_diagnostics(
    mut stderr: std::process::ChildStderr,
) -> McpHttpProcessDiagnostics {
    let diagnostics = Arc::new(Mutex::new(VecDeque::new()));
    let output = Arc::clone(&diagnostics);
    std::thread::spawn(move || {
        let mut chunk = [0_u8; 1_024];
        loop {
            let read = match stderr.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let Ok(mut output) = output.lock() else {
                return;
            };
            output.extend(&chunk[..read]);
            while output.len() > MAX_MCP_HTTP_PROCESS_DIAGNOSTIC_BYTES {
                output.pop_front();
            }
        }
    });
    diagnostics
}

fn reap_mcp_http_process(registry: &mut McpHttpProcessRegistry) -> Result<(), String> {
    let Some(mut process) = registry.process.take() else {
        return Ok(());
    };
    let status = process
        .child
        .try_wait()
        .map_err(|error| format!("无法检查 MCP HTTP sidecar: {error}"))?;
    if let Some(status) = status {
        registry.failure = Some(McpHttpProcessFailure {
            pid: process.child.id(),
            endpoint: process.endpoint.clone(),
            started_at: process.started_at,
            exit_status: status.to_string(),
            diagnostics: Arc::clone(&process.diagnostics),
        });
    } else {
        registry.process = Some(process);
    }
    Ok(())
}

fn mcp_http_failure_message(failure: &McpHttpProcessFailure) -> String {
    let diagnostics = failure
        .diagnostics
        .lock()
        .map(|bytes| bytes.iter().copied().collect::<Vec<_>>())
        .unwrap_or_default();
    let detail = String::from_utf8_lossy(&diagnostics)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let message = if detail.is_empty() {
        format!("MCP HTTP sidecar 已退出（{}）", failure.exit_status)
    } else {
        format!(
            "MCP HTTP sidecar 已退出（{}）：{detail}",
            failure.exit_status
        )
    };
    message
        .chars()
        .take(MAX_MCP_HTTP_PROCESS_MESSAGE_CHARACTERS)
        .collect()
}

pub(super) fn probe_mcp_http_ready(connect_address: std::net::SocketAddr) -> bool {
    let Ok(mut stream) =
        std::net::TcpStream::connect_timeout(&connect_address, MCP_HTTP_READY_CONNECT_TIMEOUT)
    else {
        return false;
    };
    if stream
        .set_read_timeout(Some(MCP_HTTP_READY_IO_TIMEOUT))
        .is_err()
        || stream
            .set_write_timeout(Some(MCP_HTTP_READY_IO_TIMEOUT))
            .is_err()
    {
        return false;
    }
    let request = format!(
        "OPTIONS /mcp HTTP/1.1\r\nHost: {connect_address}\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() || stream.flush().is_err() {
        return false;
    }

    let mut response = Vec::with_capacity(512);
    let mut chunk = [0_u8; 512];
    while !response.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return false,
            Ok(read) => read,
        };
        if read > MAX_MCP_HTTP_READY_RESPONSE_BYTES.saturating_sub(response.len()) {
            return false;
        }
        response.extend_from_slice(&chunk[..read]);
    }

    let Ok(headers) = std::str::from_utf8(&response) else {
        return false;
    };
    let mut lines = headers.split("\r\n");
    lines.next() == Some("HTTP/1.1 204 No Content")
        && lines.any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("MCP-Protocol-Version") && !value.trim().is_empty()
            })
        })
}

fn active_mcp_http_runtime_status(process: &ManagedMcpHttpProcess) -> McpHttpRuntimeStatus {
    McpHttpRuntimeStatus {
        phase: if process.ready {
            McpHttpRuntimePhase::Running
        } else {
            McpHttpRuntimePhase::Starting
        },
        endpoint: Some(process.endpoint.clone()),
        pid: Some(process.child.id()),
        started_at: Some(process.started_at),
        message: (!process.ready).then(|| "正在等待 MCP HTTP 监听就绪".to_string()),
    }
}

fn failed_mcp_http_runtime_status(failure: &McpHttpProcessFailure) -> McpHttpRuntimeStatus {
    McpHttpRuntimeStatus {
        phase: McpHttpRuntimePhase::Failed,
        endpoint: Some(failure.endpoint.clone()),
        pid: None,
        started_at: Some(failure.started_at),
        message: Some(mcp_http_failure_message(failure)),
    }
}

fn stopped_mcp_http_runtime_status() -> McpHttpRuntimeStatus {
    McpHttpRuntimeStatus {
        phase: McpHttpRuntimePhase::Stopped,
        endpoint: None,
        pid: None,
        started_at: None,
        message: None,
    }
}

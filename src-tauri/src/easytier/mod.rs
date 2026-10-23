//! Optional, explicitly started EasyTier component. No system service is installed.
use super::*;
use config::EasyTierConfig;
use tauri::Manager;

mod config;
mod install;

const VERSION: &str = "2.6.4";
const MAX_LOG_LINES: usize = 160;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EasyTierError {
    code: &'static str,
    detail: String,
}

fn failure(code: &'static str, detail: impl ToString) -> EasyTierError {
    EasyTierError {
        code,
        detail: detail.to_string(),
    }
}

#[derive(Clone, Serialize)]
pub(crate) struct InstallProgress {
    downloaded: u64,
    total: Option<u64>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Peer {
    id: String,
    hostname: String,
    ipv4: String,
    route: String,
    latency: String,
    received: String,
    sent: String,
    protocol: String,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct NetworkStatus {
    phase: &'static str,
    pid: Option<u32>,
    virtual_ipv4: String,
    hostname: String,
    peers: Vec<Peer>,
    logs: VecDeque<String>,
    error: Option<EasyTierError>,
}

struct Running {
    id: String,
    child: tokio::process::Child,
    tree: host_script_process::ProcessTree,
    _files: tempfile::TempDir,
    cli: PathBuf,
    rpc: String,
    started: Instant,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.tree.terminate();
        let _ = self.child.start_kill();
    }
}

#[derive(Default)]
struct Inner {
    running: Option<Running>,
    status: NetworkStatus,
}

#[derive(Default)]
pub(crate) struct EasyTierRuntime {
    gate: tokio::sync::Mutex<()>,
    inner: Arc<Mutex<Inner>>,
    exiting: AtomicBool,
}

impl Drop for EasyTierRuntime {
    fn drop(&mut self) {
        inner(self).running.take();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EasyTierView {
    config: EasyTierConfig,
    component_ready: bool,
    component_version: &'static str,
    download_supported: bool,
    status: NetworkStatus,
}

fn authorize(window: &WebviewWindow) -> Result<(), EasyTierError> {
    if window.label() == "main" || window.label().starts_with("workspace-") {
        Ok(())
    } else {
        Err(failure(
            "unavailable",
            "EasyTier requires a workspace window",
        ))
    }
}

fn root(state: &AppState) -> Result<PathBuf, EasyTierError> {
    state
        .store_path
        .parent()
        .map(|path| path.join("easytier"))
        .ok_or_else(|| failure("config-failed", "Application data directory is unavailable"))
}

fn private_directory(path: &Path) -> Result<(), EasyTierError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|e| failure("config-failed", e))?;
    let metadata = fs::symlink_metadata(path).map_err(|e| failure("config-failed", e))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(failure(
            "config-failed",
            "Expected a regular private directory",
        ));
    }
    Ok(())
}

fn inner(runtime: &EasyTierRuntime) -> std::sync::MutexGuard<'_, Inner> {
    runtime
        .inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn view(state: &AppState, runtime: &EasyTierRuntime) -> Result<EasyTierView, EasyTierError> {
    let root = root(state)?;
    let config = config::load_config(&root)?;
    let directory = install::component_directory(&root, &config);
    let mut status = inner(runtime).status.clone();
    if status.phase.is_empty() {
        status.phase = "idle";
    }
    Ok(EasyTierView {
        config,
        component_ready: install::executable(&directory, "easytier-core").is_ok()
            && install::executable(&directory, "easytier-cli").is_ok(),
        component_version: VERSION,
        download_supported: install::release_asset().is_ok(),
        status,
    })
}

fn ensure_idle(runtime: &EasyTierRuntime) -> Result<(), EasyTierError> {
    if runtime.exiting.load(Ordering::SeqCst) {
        return Err(failure("unavailable", "PortMate is exiting"));
    }
    if inner(runtime).running.is_some() {
        return Err(failure(
            "already-running",
            "Disconnect before changing the configuration or component",
        ));
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn easy_tier_status(
    window: WebviewWindow,
    state: State<'_, AppState>,
    runtime: State<'_, EasyTierRuntime>,
) -> Result<EasyTierView, EasyTierError> {
    authorize(&window)?;
    view(&state, &runtime)
}

#[tauri::command]
pub(crate) async fn save_easy_tier_config(
    window: WebviewWindow,
    state: State<'_, AppState>,
    runtime: State<'_, EasyTierRuntime>,
    config: EasyTierConfig,
) -> Result<EasyTierView, EasyTierError> {
    authorize(&window)?;
    let _gate = runtime
        .gate
        .try_lock()
        .map_err(|_| failure("busy", "EasyTier operation is already running"))?;
    ensure_idle(&runtime)?;
    config::save_config(&root(&state)?, &config)?;
    view(&state, &runtime)
}

#[tauri::command]
pub(crate) async fn install_easy_tier(
    window: WebviewWindow,
    state: State<'_, AppState>,
    runtime: State<'_, EasyTierRuntime>,
    on_progress: tauri::ipc::Channel<InstallProgress>,
) -> Result<EasyTierView, EasyTierError> {
    authorize(&window)?;
    let _gate = runtime
        .gate
        .try_lock()
        .map_err(|_| failure("busy", "EasyTier operation is already running"))?;
    ensure_idle(&runtime)?;
    update_commands::ensure_not_updating(&state).map_err(|e| failure("busy", e))?;
    install::install_component(root(&state)?, on_progress).await?;
    view(&state, &runtime)
}

fn clean_environment(command: &mut tokio::process::Command) {
    // Upstream accepts configuration via ET_*; inherited values must not
    // override the UI, start a config-server client, or expose its control port.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("ET_") {
            command.env_remove(name);
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
}

async fn bounded_output<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> Result<Vec<u8>, EasyTierError> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = reader
            .read(&mut buffer)
            .await
            .map_err(|e| failure("status-failed", e))?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len() + count > limit {
            return Err(failure(
                "status-failed",
                "EasyTier command output exceeds its limit",
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

async fn command_output(path: &Path, args: &[&str]) -> Result<Vec<u8>, EasyTierError> {
    let mut command = tokio::process::Command::new(path);
    clean_environment(&mut command);
    command.args(args).current_dir(
        path.parent()
            .ok_or_else(|| failure("component-missing", "Invalid component path"))?,
    );
    let (mut child, _tree) =
        host_script_process::spawn(&mut command).map_err(|e| failure("component-failed", e))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| failure("component-failed", "Missing command output"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| failure("component-failed", "Missing command error output"))?;
    tokio::time::timeout(Duration::from_secs(4), async {
        let (status, out, _) = tokio::try_join!(
            async {
                child
                    .wait()
                    .await
                    .map_err(|e| failure("component-failed", e))
            },
            bounded_output(stdout, 256 * 1024),
            bounded_output(stderr, 16 * 1024),
        )?;
        if !status.success() {
            return Err(failure(
                "component-failed",
                "EasyTier command did not succeed",
            ));
        }
        Ok(out)
    })
    .await
    .map_err(|_| failure("status-failed", "EasyTier command timed out"))?
}

#[tauri::command]
pub(crate) async fn start_easy_tier(
    window: WebviewWindow,
    state: State<'_, AppState>,
    runtime: State<'_, EasyTierRuntime>,
    config: EasyTierConfig,
    network_secret: String,
) -> Result<EasyTierView, EasyTierError> {
    authorize(&window)?;
    let secret = Zeroizing::new(network_secret);
    let _gate = runtime
        .gate
        .try_lock()
        .map_err(|_| failure("busy", "EasyTier operation is already running"))?;
    ensure_idle(&runtime)?;
    config.validate()?;
    config::validate_secret(&secret)?;
    let root = root(&state)?;
    let directory = install::component_directory(&root, &config);
    let core = install::executable(&directory, "easytier-core")?;
    let cli = install::executable(&directory, "easytier-cli")?;
    for binary in [&core, &cli] {
        let output = command_output(binary, &["--version"]).await?;
        if !String::from_utf8_lossy(&output)
            .split_whitespace()
            .any(|word| word == VERSION || word.starts_with(&format!("{VERSION}-")))
        {
            return Err(failure(
                "version-mismatch",
                "EasyTier core and CLI must both be version 2.6.4",
            ));
        }
    }
    ensure_idle(&runtime)?;
    update_commands::ensure_not_updating(&state).map_err(|e| failure("busy", e))?;
    config::save_config(&root, &config)?;
    let files = tempfile::Builder::new()
        .prefix("run-")
        .tempdir_in(&root)
        .map_err(|e| failure("config-failed", e))?;
    let id = Uuid::new_v4().to_string();
    let configuration = Zeroizing::new(config::render_config(&config, &secret, &id));
    let file_path = files.path().join("network.toml");
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(&file_path)
        .and_then(|mut file| file.write_all(configuration.as_bytes()))
        .map_err(|e| failure("config-failed", e))?;
    let reservation =
        std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| failure("start-failed", e))?;
    let rpc = reservation
        .local_addr()
        .map_err(|e| failure("start-failed", e))?
        .to_string();
    let mut command = tokio::process::Command::new(&core);
    clean_environment(&mut command);
    command
        .current_dir(&directory)
        .arg("--config-file")
        .arg(&file_path)
        .args([
            "--rpc-portal",
            &rpc,
            "--rpc-portal-whitelist",
            "127.0.0.1/32",
            "--console-log-level",
            "warn",
            "--multi-thread",
            "true",
        ]);
    drop(reservation);
    let (mut child, tree) =
        host_script_process::spawn(&mut command).map_err(|e| failure("start-failed", e))?;
    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    {
        let mut guard = inner(&runtime);
        // Shutdown can occur while the version check is awaiting output.
        if runtime.exiting.load(Ordering::SeqCst) {
            tree.terminate();
            return Err(failure("unavailable", "PortMate is exiting"));
        }
        guard.status = NetworkStatus {
            phase: "starting",
            pid,
            ..NetworkStatus::default()
        };
        guard.running = Some(Running {
            id: id.clone(),
            child,
            tree,
            _files: files,
            cli,
            rpc,
            started: Instant::now(),
        });
    }
    let secret = Arc::new(secret);
    if let Some(stdout) = stdout {
        tokio::spawn(read_logs(
            stdout,
            Arc::downgrade(&runtime.inner),
            id.clone(),
            Arc::clone(&secret),
        ));
    }
    if let Some(stderr) = stderr {
        tokio::spawn(read_logs(
            stderr,
            Arc::downgrade(&runtime.inner),
            id.clone(),
            secret,
        ));
    }
    tokio::spawn(supervise(Arc::downgrade(&runtime.inner), id));
    view(&state, &runtime)
}

fn sanitize_log(line: &[u8], secret: &str) -> String {
    let line = String::from_utf8_lossy(line);
    if ["network_secret", "networkSecret", "NetworkIdentity"]
        .iter()
        .any(|needle| line.contains(needle))
    {
        return "[network credentials omitted]".into();
    }
    let escaped = serde_json::to_string(secret).unwrap_or_default();
    let mut text = line.replace(secret, "[redacted]");
    if escaped.len() > 2 {
        text = text.replace(&escaped[1..escaped.len() - 1], "[redacted]");
    }
    text.chars()
        .filter(|ch| !ch.is_control() || *ch == '\t')
        .take(1024)
        .collect()
}

async fn read_logs<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    inner: std::sync::Weak<Mutex<Inner>>,
    id: String,
    secret: Arc<Zeroizing<String>>,
) {
    let mut buffer = [0; 4096];
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let count = match reader.read(&mut buffer).await {
            Ok(count) => count,
            Err(_) => return,
        };
        for byte in &buffer[..count] {
            if *byte == b'\n' {
                push_log(
                    &inner,
                    &id,
                    if oversized {
                        "[oversized log line omitted]".into()
                    } else {
                        sanitize_log(&line, &secret)
                    },
                );
                line.clear();
                oversized = false;
            } else if line.len() < 16 * 1024 {
                line.push(*byte);
            } else {
                oversized = true;
            }
        }
        if count == 0 {
            if !line.is_empty() && !oversized {
                push_log(&inner, &id, sanitize_log(&line, &secret));
            }
            return;
        }
    }
}

fn push_log(inner: &std::sync::Weak<Mutex<Inner>>, id: &str, text: String) {
    let Some(inner) = inner.upgrade() else { return };
    let mut guard = inner
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if guard
        .running
        .as_ref()
        .is_none_or(|running| running.id != id)
    {
        return;
    }
    guard.status.logs.push_back(text);
    while guard.status.logs.len() > MAX_LOG_LINES {
        guard.status.logs.pop_front();
    }
}

async fn supervise(inner: std::sync::Weak<Mutex<Inner>>, id: String) {
    loop {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let Some(shared) = inner.upgrade() else {
            return;
        };
        let request = {
            let mut guard = shared
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(running) = guard.running.as_mut().filter(|running| running.id == id) else {
                return;
            };
            let error = match running.child.try_wait() {
                Ok(None) => None,
                Ok(Some(status)) => Some(format!("EasyTier exited: {status}")),
                Err(error) => Some(format!("Cannot inspect EasyTier: {error}")),
            };
            if let Some(error) = error {
                guard.running.take();
                guard.status.phase = "failed";
                guard.status.pid = None;
                guard.status.peers.clear();
                guard.status.error = Some(failure("network-failed", error));
                return;
            }
            (
                running.cli.clone(),
                running.rpc.clone(),
                running.started.elapsed(),
            )
        };
        let result = query_network(&request.0, &request.1, &id).await;
        let mut guard = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard
            .running
            .as_ref()
            .is_none_or(|running| running.id != id)
        {
            return;
        }
        match result {
            Ok((ip, hostname, peers)) => {
                guard.status.phase = "running";
                guard.status.virtual_ipv4 = ip;
                guard.status.hostname = hostname;
                guard.status.peers = peers;
                guard.status.error = None;
            }
            Err(error)
                if request.2 > Duration::from_secs(20) && guard.status.phase == "starting" =>
            {
                guard.running.take();
                guard.status.phase = "failed";
                guard.status.pid = None;
                guard.status.error = Some(error);
                return;
            }
            Err(error) => {
                guard.status.peers.clear();
                if guard.status.phase == "running" {
                    guard.status.error = Some(error);
                }
            }
        }
    }
}

async fn query_network(
    cli: &Path,
    rpc: &str,
    id: &str,
) -> Result<(String, String, Vec<Peer>), EasyTierError> {
    let node = command_output(cli, &["-p", rpc, "-o", "json", "-i", id, "node"]).await?;
    let node: serde_json::Value = serde_json::from_slice(&node)
        .map_err(|_| failure("status-failed", "Invalid EasyTier node status"))?;
    if node.get("inst_id").and_then(|v| v.as_str()) != Some(id) {
        return Err(failure(
            "status-failed",
            "EasyTier control endpoint identity does not match",
        ));
    }
    let own_peer = node
        .get("peer_id")
        .and_then(|v| v.as_u64())
        .map(|v| v.to_string())
        .unwrap_or_default();
    let output = command_output(cli, &["-p", rpc, "-o", "json", "-i", id, "peer"]).await?;
    let list: Vec<serde_json::Value> = serde_json::from_slice(&output)
        .map_err(|_| failure("status-failed", "Invalid EasyTier peer status"))?;
    let text = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(256)
            .collect::<String>()
    };
    let peers = list
        .iter()
        .filter(|value| text(value, "id") != own_peer)
        .take(256)
        .map(|value| Peer {
            id: text(value, "id"),
            hostname: text(value, "hostname"),
            ipv4: text(value, "ipv4"),
            route: text(value, "cost"),
            latency: text(value, "lat_ms"),
            received: text(value, "rx_bytes"),
            sent: text(value, "tx_bytes"),
            protocol: text(value, "tunnel_proto"),
        })
        .collect();
    Ok((text(&node, "ipv4_addr"), text(&node, "hostname"), peers))
}

async fn stop(runtime: &EasyTierRuntime) -> Result<(), EasyTierError> {
    let running = {
        let mut guard = inner(runtime);
        guard.status.phase = "stopping";
        guard.running.take()
    };
    let result = if let Some(mut running) = running {
        running.tree.terminate();
        let _ = running.child.start_kill();
        tokio::time::timeout(Duration::from_secs(5), running.child.wait())
            .await
            .map_err(|_| failure("stop-failed", "Timed out waiting for EasyTier to exit"))
            .and_then(|result| result.map(|_| ()).map_err(|e| failure("stop-failed", e)))
    } else {
        Ok(())
    };
    let mut guard = inner(runtime);
    guard.status.phase = if result.is_ok() { "idle" } else { "failed" };
    guard.status.pid = None;
    guard.status.peers.clear();
    guard.status.virtual_ipv4.clear();
    guard.status.error = result.as_ref().err().cloned();
    result
}

#[tauri::command]
pub(crate) async fn stop_easy_tier(
    window: WebviewWindow,
    state: State<'_, AppState>,
    runtime: State<'_, EasyTierRuntime>,
) -> Result<EasyTierView, EasyTierError> {
    authorize(&window)?;
    let _gate = runtime
        .gate
        .try_lock()
        .map_err(|_| failure("busy", "EasyTier operation is already running"))?;
    stop(&runtime).await?;
    view(&state, &runtime)
}

pub(super) async fn stop_before_update(state: &AppState) -> Result<(), String> {
    if let Some(runtime) = state
        .app_handle
        .as_ref()
        .and_then(|app| app.try_state::<EasyTierRuntime>())
    {
        let _gate = runtime.gate.lock().await;
        stop(&runtime).await.map_err(|error| error.detail)?;
    }
    Ok(())
}

pub(super) fn shutdown_for_app(state: &AppState) {
    if let Some(runtime) = state
        .app_handle
        .as_ref()
        .and_then(|app| app.try_state::<EasyTierRuntime>())
    {
        runtime.exiting.store(true, Ordering::SeqCst);
        // Dropping the process tree kills all owned descendants immediately;
        // kill-on-drop also covers cancellation of the async supervisor.
        inner(&runtime).running.take();
    }
}

#[cfg(test)]
#[path = "../../../test/rust/portmate/unit/easytier.rs"]
mod tests;

#[cfg(all(test, target_os = "linux", not(feature = "desktop")))]
#[path = "../../../test/rust/portmate/unit/easytier_lifecycle.rs"]
mod lifecycle_tests;

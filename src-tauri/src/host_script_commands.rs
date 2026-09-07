//! Host skills are independent of terminal sessions. Only desktop-authored source
//! and execution settings reach the interpreter; MCP input is data, never code.
use super::*;
use portmate_core::{
    custom_scripts::{host_script_tool_definition, validate_host_script_parameters},
    HostScriptLanguage,
};

const OUTPUT_LIMIT: usize = 128 * 1024;
type RunKey = (PathBuf, String, String);
static RUNS: OnceLock<Mutex<HashMap<RunKey, Arc<RunControl>>>> = OnceLock::new();
static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

#[derive(Default)]
struct RunControl {
    cancelled: AtomicBool,
    tree: Mutex<Option<Weak<super::host_script_process::ProcessTree>>>,
}
impl RunControl {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(tree) = self
            .tree
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
        {
            tree.terminate();
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunHostScriptRequest {
    pub script_id: String,
    pub expected_updated_at: DateTime<Utc>,
    pub run_id: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HostScriptResult {
    pub run_id: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub failure: Option<String>,
}

pub(super) fn host_script_for_client(
    store: &SessionStore,
    id: &str,
    client: Option<&str>,
) -> Result<CustomScript, String> {
    let script = store
        .custom_scripts
        .iter()
        .find(|s| s.id == id)
        .ok_or("unknown or unavailable host script")?;
    validate_custom_script(script)?;
    if client.is_some_and(|id| !script.allows_host_client(id)) {
        return Err("host script is not exposed to this MCP client".into());
    }
    Ok(script.clone())
}

pub(super) fn host_script_tools(
    store: &SessionStore,
    client: &str,
) -> Vec<portmate_core::McpToolDefinition> {
    if !store.mcp_can_read(client, McpScope::ReadScripts, None) {
        return Vec::new();
    }
    store
        .custom_scripts
        .iter()
        .filter(|s| s.allows_host_client(client) && validate_custom_script(s).is_ok())
        .filter_map(host_script_tool_definition)
        .collect()
}

struct RunRegistration(RunKey);
impl Drop for RunRegistration {
    fn drop(&mut self) {
        if let Some(runs) = RUNS.get() {
            runs.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&self.0);
        }
    }
}

pub(super) fn cancel_owner(path: &Path, owner: Option<&str>) {
    if let Some(runs) = RUNS.get() {
        for ((p, o, _), cancelled) in runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
        {
            if p == path && owner.is_none_or(|owner| owner == o) {
                cancelled.cancel();
            }
        }
    }
}

#[tauri::command]
pub(crate) fn cancel_host_script(
    state: State<'_, AppState>,
    window: WebviewWindow,
    run_id: String,
) -> Result<(), String> {
    let runs = RUNS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|e| e.to_string())?;
    let key = (
        state.store_path.clone(),
        format!("window:{}", window.label()),
        run_id,
    );
    let run = runs
        .get(&key)
        .ok_or("host script is not running in this window")?;
    run.cancel();
    Ok(())
}

fn check_run(
    state: &AppState,
    request: &RunHostScriptRequest,
    client: Option<&str>,
) -> Result<CustomScript, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    check_run_in_store(&store, request, client)
}

fn check_run_in_store(
    store: &SessionStore,
    request: &RunHostScriptRequest,
    client: Option<&str>,
) -> Result<CustomScript, String> {
    let script = host_script_for_client(store, &request.script_id, client)?;
    if script.updated_at != request.expected_updated_at {
        return Err("host script changed; refresh and approve again".into());
    }
    if client.is_some_and(|id| !store.mcp_can(id, McpScope::RunScripts, None)) {
        return Err("MCP host script grant was revoked or expired".into());
    }
    validate_host_script_parameters(&script.host, &request.parameters)?;
    Ok(script)
}

pub(super) async fn run_host_script_inner(
    state: &AppState,
    request: RunHostScriptRequest,
    owner: &str,
    client: Option<&str>,
    validation: Option<CommitValidation>,
) -> Result<HostScriptResult, String> {
    Uuid::parse_str(&request.run_id).map_err(|_| "host script run ID must be a UUID")?;
    let _slot = SLOTS
        .try_acquire()
        .map_err(|_| "host script concurrency limit reached (4)")?;
    let script = check_run(state, &request, client)?;
    let cancelled = Arc::new(RunControl::default());
    let key = (
        state.store_path.clone(),
        owner.to_string(),
        request.run_id.clone(),
    );
    {
        let mut runs = RUNS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .map_err(|e| e.to_string())?;
        if runs.contains_key(&key) {
            return Err("duplicate host script run ID".into());
        }
        runs.insert(key.clone(), cancelled.clone());
    }
    let _registration = RunRegistration(key);
    let host = &script.host;
    let (mut command, _files) = prepare_command(&script, &request.parameters)?;
    if let Some(validate) = validation {
        validate()?;
    }
    // Hold the store through spawn: another thread must not revoke or edit
    // between checking the grant/version and starting the interpreter.
    let (mut child, process_tree) = {
        let store = state.store.lock().map_err(|e| e.to_string())?;
        check_run_in_store(&store, &request, client)?;
        if cancelled.cancelled.load(Ordering::SeqCst) {
            return Err("host script cancelled".into());
        }
        super::host_script_process::spawn(&mut command)?
    };
    let process_tree = Arc::new(process_tree);
    *cancelled.tree.lock().map_err(|e| e.to_string())? = Some(Arc::downgrade(&process_tree));
    if cancelled.cancelled.load(Ordering::SeqCst) {
        cancelled.cancel();
    }
    let mut stdout = child.stdout.take().ok_or("missing host script stdout")?;
    let mut stderr = child.stderr.take().ok_or("missing host script stderr")?;
    let mut stdin = child.stdin.take().ok_or("missing host script stdin")?;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let input = serde_json::to_vec(&request.parameters).map_err(|e| e.to_string())?;
    let completed = {
        let work = async {
            let (status, (), (), ()) = tokio::try_join!(
                async { child.wait().await.map_err(|e| e.to_string()) },
                read_output(&mut stdout, &mut out),
                read_output(&mut stderr, &mut err),
                async {
                    // Scripts may intentionally ignore stdin and use PORTMATE_INPUT_JSON.
                    match stdin.write_all(&input).await {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
                        Err(e) => return Err(e.to_string()),
                    }
                    drop(stdin);
                    Ok(())
                }
            )?;
            Ok::<_, String>(status)
        };
        let interrupted = async {
            loop {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if cancelled.cancelled.load(Ordering::SeqCst) {
                    return "host script cancelled".to_string();
                }
                if let Err(error) = check_run(state, &request, client) {
                    return error;
                }
            }
        };
        tokio::select! {
            result = work => result,
            reason = interrupted => Err(reason),
            _ = tokio::time::sleep(Duration::from_secs(host.timeout_seconds)) => Err("host script timed out".into()),
        }
    };
    // Also retire descendants on normal exit; background daemons are not skills.
    drop(process_tree);
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    let completed = if cancelled.cancelled.load(Ordering::SeqCst) {
        Err("host script cancelled".into())
    } else {
        completed
    };
    let (exit_code, failure) = match completed {
        Ok(status) => (
            status.code(),
            (!status.success()).then(|| format!("host script exited with {status}")),
        ),
        Err(error) => (None, Some(error)),
    };
    Ok(HostScriptResult {
        run_id: request.run_id,
        exit_code,
        stdout: String::from_utf8_lossy(&out).into_owned(),
        stderr: String::from_utf8_lossy(&err).into_owned(),
        failure,
    })
}

async fn read_output(
    reader: &mut (impl AsyncRead + Unpin),
    output: &mut Vec<u8>,
) -> Result<(), String> {
    let mut buffer = [0; 8192];
    loop {
        let read = reader.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if read == 0 {
            return Ok(());
        }
        let remaining = OUTPUT_LIMIT - output.len();
        output.extend_from_slice(&buffer[..read.min(remaining)]);
        if read > remaining {
            return Err("host script output exceeded 128 KiB per stream".into());
        }
    }
}

fn prepare_command(
    script: &CustomScript,
    parameters: &serde_json::Value,
) -> Result<(tokio::process::Command, tempfile::TempDir), String> {
    let host = &script.host;
    let cwd = if host.working_directory.is_empty() {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or("host home directory is unavailable")?
    } else {
        PathBuf::from(&host.working_directory)
    };
    if !cwd.is_absolute() || !cwd.is_dir() {
        return Err("host working directory must be an existing absolute directory".into());
    }
    let program = if host.interpreter.is_empty() {
        match host.language {
            HostScriptLanguage::Python => {
                if cfg!(windows) {
                    "python.exe"
                } else {
                    "python3"
                }
            }
            HostScriptLanguage::Shell => {
                if cfg!(windows) {
                    "pwsh.exe"
                } else {
                    "/bin/sh"
                }
            }
        }
        .to_string()
    } else {
        if !Path::new(&host.interpreter).is_absolute() {
            return Err("interpreter path is not absolute on this host".into());
        }
        host.interpreter.clone()
    };
    // Resolve PATH before changing cwd; never search the script working directory.
    let executable = resolve_interpreter(&program)?;
    let files = tempfile::Builder::new()
        .prefix("portmate-host-script-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let extension = match host.language {
        HostScriptLanguage::Python => "py",
        HostScriptLanguage::Shell => {
            if cfg!(windows) {
                "ps1"
            } else {
                "sh"
            }
        }
    };
    let path = files.path().join(format!("script.{extension}"));
    fs::write(&path, &script.content).map_err(|e| e.to_string())?;
    let mut command = tokio::process::Command::new(executable);
    match host.language {
        HostScriptLanguage::Python => {
            command.args(["-I", "-u", "-X", "utf8"]).arg(&path);
        }
        HostScriptLanguage::Shell => {
            if cfg!(windows) {
                command
                    .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
                    .arg(&path);
            } else {
                command.arg(&path);
            }
        }
    }
    // Do not inherit bridge tokens or Python/shell startup injection variables.
    let environment = [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "SYSTEMDRIVE",
        "PATHEXT",
    ];
    command.env_clear();
    for name in environment {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.env(
        "PORTMATE_INPUT_JSON",
        serde_json::to_string(parameters).map_err(|e| e.to_string())?,
    );
    for (key, value) in parameters
        .as_object()
        .ok_or("parameters must be an object")?
    {
        command.env(
            format!("PORTMATE_PARAM_{key}"),
            value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string()),
        );
    }
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok((command, files))
}

fn resolve_interpreter(program: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(program);
    if path.is_absolute() {
        return Ok(path);
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths).filter(|p| p.is_absolute()) {
            let candidate = directory.join(program);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(format!(
        "interpreter {program} was not found; install it or set its absolute path"
    ))
}

#[tauri::command]
pub(crate) async fn run_host_script(
    state: State<'_, AppState>,
    window: WebviewWindow,
    request: RunHostScriptRequest,
) -> Result<HostScriptResult, String> {
    check_run(state.inner(), &request, None)?;
    let audit_id = Uuid::new_v4().to_string();
    {
        let mut store = state.store.lock().map_err(|e| e.to_string())?;
        commit_store_mutation(&mut store, &state.store_path, |store| {
            store.record_audit(AuditRecord {
                id: audit_id.clone(),
                ts: Utc::now(),
                actor: "desktop-user".into(),
                action: "run_host_script".into(),
                session_id: None,
                decision: "authorized".into(),
                details: BTreeMap::from([
                    ("scriptId".into(), request.script_id.clone()),
                    ("runId".into(), request.run_id.clone()),
                ]),
            });
            Ok(())
        })?;
    }
    let result = run_host_script_inner(
        state.inner(),
        request,
        &format!("window:{}", window.label()),
        None,
        None,
    )
    .await;
    let decision = if result.as_ref().is_ok_and(|r| r.failure.is_none()) {
        "succeeded"
    } else {
        "failed"
    };
    if let Err(error) = finish_applied_mcp_write_audit(state.inner(), &audit_id, decision, None) {
        eprintln!("PortMate: host script audit finalization failed: {error}");
    }
    result
}

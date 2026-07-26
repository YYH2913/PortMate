use super::*;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty};

pub(super) struct ShellRuntime {
    pub(super) runtime_id: String,
    pub(super) master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub(super) writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    pub(super) closed: Arc<AtomicBool>,
}

pub(super) fn open_shell_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    let shell = match &profile.connection {
        ConnectionConfig::Shell(shell) => shell.clone(),
        _ => return Err("profile is not shell-backed".to_string()),
    };
    let program = if shell.program.trim().is_empty() {
        default_shell_program()
    } else {
        shell.program.trim().to_string()
    };

    if let Some(existing) = {
        let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        existing.closed.store(true, Ordering::SeqCst);
        if let Ok(mut child) = existing.child.lock() {
            let _ = child.kill();
        }
    }

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: profile.terminal.rows,
            cols: profile.terminal.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("Shell PTY 打开失败: {error}"))?;

    let mut command = CommandBuilder::new(&program);
    command.args(shell.args.iter());
    apply_shell_terminal_color_env(&mut command, profile.terminal.term.as_str());
    if let Some(cwd) = shell
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        command.cwd(cwd);
    }

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Shell 启动失败 {program}: {error}"))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("Shell PTY reader 创建失败: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("Shell PTY writer 创建失败: {error}"))?;

    let runtime_id = Uuid::new_v4().to_string();
    let closed = Arc::new(AtomicBool::new(false));
    let reader_start_gate = Arc::new(ReaderStartGate::default());
    let (tap, _) = broadcast::channel(1024);
    let child = Arc::new(Mutex::new(child));
    let master = Arc::new(Mutex::new(pair.master));
    let writer = Arc::new(Mutex::new(writer));
    {
        let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
        connections.insert(
            profile.id.clone(),
            ShellRuntime {
                runtime_id: runtime_id.clone(),
                master,
                writer,
                tap: tap.clone(),
                child: Arc::clone(&child),
                closed: Arc::clone(&closed),
            },
        );
    }

    if let Err(error) = std::thread::Builder::new()
        .name(format!("portmate-shell-{}", profile.id))
        .spawn(read_shell_pty(ShellReadTask {
            io: state.session_io(),
            session_id: profile.id.clone(),
            runtime_id: runtime_id.clone(),
            program: program.clone(),
            tap,
            closed: Arc::clone(&closed),
            start_gate: Arc::clone(&reader_start_gate),
            child: Arc::clone(&child),
            reader,
        }))
    {
        closed.store(true, Ordering::SeqCst);
        reader_start_gate.cancel();
        if let Ok(mut child) = child.lock() {
            let _ = child.kill();
        }
        remove_runtime_if_owned(&state.shell, &profile.id, |runtime| {
            runtime.runtime_id == runtime_id
        })?;
        return Err(format!("Shell PTY 读取线程启动失败: {error}"));
    }

    let finalize_result = match state.store.lock() {
        Ok(mut store) => {
            commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
                mark_session_connected_with_events(
                    next_store,
                    &profile,
                    [format!("PortMate: shell started ({program})")],
                )
            })
        }
        Err(error) => Err(error.to_string()),
    };
    match finalize_result {
        Ok(summary) => {
            reader_start_gate.start();
            Ok(summary)
        }
        Err(error) => {
            closed.store(true, Ordering::SeqCst);
            reader_start_gate.cancel();
            if let Ok(mut child) = child.lock() {
                let _ = child.kill();
            }
            let cleanup_error = remove_runtime_if_owned(&state.shell, &profile.id, |runtime| {
                runtime.runtime_id == runtime_id
            })
            .err();
            if let Some(cleanup_error) = cleanup_error {
                Err(format!(
                    "{error}; Shell runtime cleanup failed: {cleanup_error}"
                ))
            } else {
                Err(error)
            }
        }
    }
}

struct ShellReadTask {
    io: SessionIo,
    session_id: String,
    runtime_id: String,
    program: String,
    tap: broadcast::Sender<Vec<u8>>,
    closed: Arc<AtomicBool>,
    start_gate: Arc<ReaderStartGate>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    reader: Box<dyn Read + Send>,
}

fn read_shell_pty(task: ShellReadTask) -> impl FnOnce() + Send + 'static {
    move || {
        let ShellReadTask {
            io,
            session_id,
            runtime_id,
            program,
            tap,
            closed,
            start_gate,
            child,
            mut reader,
        } = task;
        if !start_gate.wait() {
            return;
        }
        let mut buffer = vec![0_u8; 8192];
        let mut last_persist = Instant::now();
        let mut has_unpersisted_stream = false;
        let mut disconnect_reason = None;

        while !closed.load(Ordering::SeqCst) {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    if let Some(reason) = shell_child_disconnect_reason(&child, &program) {
                        disconnect_reason = Some(reason);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(size) => {
                    let bytes = buffer[..size].to_vec();
                    let _ = tap.send(bytes.clone());
                    record_channel_bytes(
                        &io,
                        &session_id,
                        Some(&runtime_id),
                        EventStream::Stdout,
                        &bytes,
                        String::from_utf8_lossy(&bytes).to_string(),
                    );
                    has_unpersisted_stream = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    disconnect_reason = Some(
                        wait_for_shell_child_disconnect_reason(&child, &program)
                            .unwrap_or_else(|| format!("shell read failed on {program}: {error}")),
                    );
                    break;
                }
            }

            if has_unpersisted_stream && last_persist.elapsed() >= STREAM_PERSIST_INTERVAL {
                if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                    eprintln!("PortMate: failed to persist shell stream data: {error}");
                }
                has_unpersisted_stream = false;
                last_persist = Instant::now();
            }
        }

        if has_unpersisted_stream {
            if let Err(error) = persist_store_arc(&io.store_path, &io.store) {
                eprintln!("PortMate: failed to persist final shell stream data: {error}");
            }
        }

        let disconnect_reason = portmate_core::normalize_session_disconnect_reason(
            &disconnect_reason.unwrap_or_else(|| format!("shell closed ({program})")),
        )
        .unwrap_or_else(|| format!("shell closed ({program})"));

        let removed_current = {
            let mut connections = match io.runtimes.shell.lock() {
                Ok(connections) => connections,
                Err(_) => return,
            };
            if connections
                .get(&session_id)
                .is_some_and(|runtime| runtime.runtime_id == runtime_id)
            {
                connections.remove(&session_id);
                true
            } else {
                false
            }
        };

        if removed_current {
            clear_active_command(&io, &session_id);
            if let Ok(mut store) = io.store.lock() {
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some(disconnect_reason.clone()),
                );
                store.record_system_event(&session_id, format!("PortMate: {disconnect_reason}"));
                if let Err(error) =
                    persist_applied_store(&store, &io.store_path, "shell disconnect state")
                {
                    eprintln!("PortMate: failed to persist shell close event: {error}");
                }
            }
        }
    }
}

fn shell_child_disconnect_reason(
    child: &Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    program: &str,
) -> Option<String> {
    let mut child = match child.lock() {
        Ok(child) => child,
        Err(error) => {
            return Some(format!(
                "shell process status lock failed on {program}: {error}"
            ));
        }
    };
    match child.try_wait() {
        Ok(Some(status)) => Some(shell_exit_status_disconnect_reason(program, &status)),
        Ok(None) => None,
        Err(error) => Some(format!(
            "shell process status query failed on {program}: {error}"
        )),
    }
}

fn wait_for_shell_child_disconnect_reason(
    child: &Mutex<Box<dyn portable_pty::Child + Send + Sync>>,
    program: &str,
) -> Option<String> {
    let started = Instant::now();
    let timeout = Duration::from_millis(200);
    loop {
        if let Some(reason) = shell_child_disconnect_reason(child, program) {
            return Some(reason);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return None;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(10)));
    }
}

pub(super) fn shell_exit_status_disconnect_reason(
    program: &str,
    status: &portable_pty::ExitStatus,
) -> String {
    match status.signal() {
        Some(signal) => format!("shell process exited by signal {signal} ({program})"),
        None => format!(
            "shell process exited with status {} ({program})",
            status.exit_code()
        ),
    }
}

fn default_shell_program() -> String {
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    } else {
        if let Ok(shell) = std::env::var("SHELL") {
            let shell = shell.trim();
            if !shell.is_empty() {
                return shell.to_string();
            }
        }
        [
            "/bin/zsh",
            "/usr/bin/zsh",
            "/usr/local/bin/zsh",
            "/opt/homebrew/bin/zsh",
            "/bin/bash",
            "/usr/bin/bash",
        ]
        .into_iter()
        .find(|candidate| Path::new(candidate).exists())
        .unwrap_or("/bin/sh")
        .to_string()
    }
}

fn apply_shell_terminal_color_env(command: &mut CommandBuilder, term: &str) {
    command.env("TERM", normalized_terminal_name(term));
    command.env("COLORTERM", "truecolor");
    command.env("CLICOLOR", "1");
    command.env("CLICOLOR_FORCE", "1");
    command.env("FORCE_COLOR", "1");
    command.env("TERM_PROGRAM", "PortMate");
    command.env_remove("NO_COLOR");
}

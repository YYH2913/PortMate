use super::*;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty};

pub(super) async fn write_shell_bytes(
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    closed: Arc<AtomicBool>,
    bytes: &[u8],
    cancellation: Option<Arc<AtomicBool>>,
) -> Result<(), String> {
    let bytes = bytes.to_vec();
    let worker_closed = Arc::clone(&closed);
    let worker_cancel = cancellation.clone();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut worker = tokio::task::spawn_blocking(move || {
        let cancelled = || {
            worker_closed.load(Ordering::SeqCst)
                || Instant::now() >= deadline
                || worker_cancel
                    .as_ref()
                    .is_some_and(|flag| flag.load(Ordering::SeqCst))
        };
        let mut writer = loop {
            if cancelled() {
                return Err("Shell write cancelled".into());
            }
            match writer.try_lock() {
                Ok(writer) => break writer,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let mut remaining = bytes.as_slice();
        while !remaining.is_empty() {
            if cancelled() {
                return Err("Shell write cancelled".into());
            }
            match writer.write(&remaining[..remaining.len().min(4096)]) {
                Ok(0) => return Err("Shell PTY write returned zero".into()),
                Ok(size) => remaining = &remaining[size..],
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("Shell PTY write failed: {error}")),
            }
        }
        Ok(())
    });
    loop {
        tokio::select! {
            result = &mut worker => {
                let result = result.map_err(|error| error.to_string())?;
                if result.is_err() {
                    closed.store(true, Ordering::SeqCst);
                    tokio::task::spawn_blocking(move || {
                        if let Ok(mut child) = child.lock() { let _ = child.kill(); }
                    });
                }
                return result;
            },
            _ = tokio::time::sleep(Duration::from_millis(25)) => {
                if closed.load(Ordering::SeqCst) || Instant::now() >= deadline
                    || cancellation.as_ref().is_some_and(|flag| flag.load(Ordering::SeqCst)) {
                    // A partially written frame cannot be retried safely. End this
                    // PTY generation and unblock the worker by terminating its child.
                    closed.store(true, Ordering::SeqCst);
                    tokio::task::spawn_blocking(move || {
                        if let Ok(mut child) = child.lock() { let _ = child.kill(); }
                    });
                    return Err("Shell write cancelled or timed out; session terminated".into());
                }
            }
        }
    }
}

pub(super) struct ShellRuntime {
    pub(super) runtime_id: String,
    pub(super) master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pub(super) writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    pub(super) closed: Arc<AtomicBool>,
}

pub(super) struct PreparedShellSession {
    profile: SessionProfile,
    program: String,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    reader: Option<Box<dyn Read + Send>>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
}

impl Drop for PreparedShellSession {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct ShellLaunchPaths {
    pub(super) program: PathBuf,
    pub(super) cwd: Option<PathBuf>,
}

pub(super) fn validate_shell_profile_paths(profile: &SessionProfile) -> Result<(), String> {
    let ConnectionConfig::Shell(shell) = &profile.connection else {
        return Ok(());
    };
    resolve_shell_launch_paths(shell).map(|_| ())
}

pub(super) fn resolve_shell_launch_paths(
    shell: &portmate_core::ShellConnection,
) -> Result<ShellLaunchPaths, String> {
    let home = native_home_path();
    let default_program = default_shell_program();
    resolve_shell_launch_paths_with_home(
        shell,
        &default_program,
        current_local_transfer_path_platform(),
        home.as_deref(),
    )
}

pub(super) fn resolve_shell_launch_paths_with_home(
    shell: &portmate_core::ShellConnection,
    default_program: &str,
    platform: LocalTransferPathPlatform,
    home: Option<&Path>,
) -> Result<ShellLaunchPaths, String> {
    validate_shell_arguments(shell)?;
    let configured_program = shell.program.as_str();
    let program = validate_native_local_path_with_home(
        if configured_program.trim().is_empty() {
            default_program
        } else {
            configured_program
        },
        platform,
        home,
    )
    .map_err(|error| format!("Shell 程序路径无效: {error}"))?;
    let cwd = shell
        .cwd
        .as_deref()
        .filter(|cwd| !cwd.trim().is_empty())
        .map(|cwd| {
            validate_native_local_path_with_home(cwd, platform, home)
                .map_err(|error| format!("Shell 工作目录无效: {error}"))
        })
        .transpose()?;
    Ok(ShellLaunchPaths { program, cwd })
}

fn validate_shell_arguments(shell: &portmate_core::ShellConnection) -> Result<(), String> {
    if shell.args.len() > portmate_core::MAX_SHELL_ARGUMENTS {
        return Err(format!(
            "Shell 参数数量超过 {}",
            portmate_core::MAX_SHELL_ARGUMENTS
        ));
    }
    for (index, argument) in shell.args.iter().enumerate() {
        if argument.contains('\0') {
            return Err(format!("Shell 参数 {} 不能包含 NUL", index + 1));
        }
        if argument.chars().count() > portmate_core::MAX_SHELL_ARGUMENT_CHARACTERS {
            return Err(format!(
                "Shell 参数 {} 超过 {} 个字符",
                index + 1,
                portmate_core::MAX_SHELL_ARGUMENT_CHARACTERS
            ));
        }
    }
    Ok(())
}

pub(super) fn prepare_shell_session(
    profile: SessionProfile,
) -> Result<PreparedShellSession, String> {
    let shell = match &profile.connection {
        ConnectionConfig::Shell(shell) => shell.clone(),
        _ => return Err("profile is not shell-backed".to_string()),
    };
    let launch = resolve_shell_launch_paths(&shell)?;
    if let Some(cwd) = &launch.cwd {
        let metadata = fs::metadata(cwd)
            .map_err(|error| format!("Shell 工作目录不可用 {}: {error}", cwd.display()))?;
        if !metadata.is_dir() {
            return Err(format!("Shell 工作目录不是目录: {}", cwd.display()));
        }
    }
    let program = launch.program.to_string_lossy().into_owned();

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: profile.terminal.rows,
            cols: profile.terminal.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("Shell PTY 打开失败: {error}"))?;

    let mut command = CommandBuilder::new(&launch.program);
    command.args(shell.args.iter());
    apply_shell_terminal_color_env(&mut command, profile.terminal.term.as_str());
    if let Some(cwd) = &launch.cwd {
        command.cwd(cwd);
    }

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Shell 启动失败 {program}: {error}"))?;
    drop(pair.slave);
    #[cfg(unix)]
    if let Some(fd) = pair.master.as_raw_fd() {
        // The cloned PTY descriptors share status flags. Both read and write
        // loops handle WouldBlock so close/cancellation never needs a stuck I/O.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            let mut child = child;
            let _ = child.kill();
            return Err(format!(
                "Shell nonblocking PTY setup failed: {}",
                std::io::Error::last_os_error()
            ));
        }
    }

    let mut prepared = PreparedShellSession {
        profile,
        program,
        master: Some(pair.master),
        writer: None,
        reader: None,
        child: Some(child),
    };
    prepared.reader = Some(
        prepared
            .master
            .as_ref()
            .expect("prepared Shell session owns its PTY master")
            .try_clone_reader()
            .map_err(|error| format!("Shell PTY reader 创建失败: {error}"))?,
    );
    prepared.writer = Some(
        prepared
            .master
            .as_ref()
            .expect("prepared Shell session owns its PTY master")
            .take_writer()
            .map_err(|error| format!("Shell PTY writer 创建失败: {error}"))?,
    );
    Ok(prepared)
}

pub(super) fn install_shell_session(
    state: &AppState,
    mut prepared: PreparedShellSession,
) -> Result<SessionSummary, String> {
    let profile = prepared.profile.clone();
    let program = prepared.program.clone();
    let mut connections = state.shell.lock().map_err(|error| error.to_string())?;
    let master = prepared
        .master
        .take()
        .expect("prepared Shell session owns its PTY master");
    let writer = prepared
        .writer
        .take()
        .expect("prepared Shell session owns its PTY writer");
    let reader = prepared
        .reader
        .take()
        .expect("prepared Shell session owns its PTY reader");
    let child = prepared
        .child
        .take()
        .expect("prepared Shell session owns its child process");

    let runtime_id = Uuid::new_v4().to_string();
    let closed = Arc::new(AtomicBool::new(false));
    let reader_start_gate = Arc::new(ReaderStartGate::default());
    let (tap, _) = broadcast::channel(1024);
    let child = Arc::new(Mutex::new(child));
    let master = Arc::new(Mutex::new(master));
    let writer = Arc::new(Mutex::new(writer));
    let existing = connections.insert(
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
    drop(connections);
    if let Some(existing) = existing {
        existing.closed.store(true, Ordering::SeqCst);
        if let Ok(mut child) = existing.child.lock() {
            let _ = child.kill();
        }
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
        let mut disconnect_reason = None;
        let mut decoder = StreamDecoder::default();

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
                    record_channel_bytes_with_accepted_side_effect(
                        &io,
                        &session_id,
                        Some(&runtime_id),
                        EventStream::Stdout,
                        ChannelByteViews::same(&bytes),
                        decoder.feed(&bytes),
                        || {
                            let _ = tap.send(bytes.clone());
                        },
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    disconnect_reason = Some(
                        wait_for_shell_child_disconnect_reason(&child, &program)
                            .unwrap_or_else(|| format!("shell read failed on {program}: {error}")),
                    );
                    break;
                }
            }
        }

        finish_channel_decoder(
            &io,
            &session_id,
            &runtime_id,
            EventStream::Stdout,
            &mut decoder,
        );
        let disconnect_reason = portmate_core::normalize_session_disconnect_reason(
            &disconnect_reason.unwrap_or_else(|| format!("shell closed ({program})")),
        )
        .unwrap_or_else(|| format!("shell closed ({program})"));

        let accepted =
            match with_current_session_runtime_store(&io, &session_id, &runtime_id, |store| {
                clear_active_command(&io, &session_id);
                let _ = store.set_runtime_status_with_reason(
                    &session_id,
                    SessionStatus::Disconnected,
                    Some(disconnect_reason.clone()),
                );
                store.record_system_event(&session_id, format!("PortMate: {disconnect_reason}"));
                if let Err(error) =
                    persist_applied_store(store, &io.store_path, "shell disconnect state")
                {
                    eprintln!("PortMate: failed to persist shell close event: {error}");
                }
            }) {
                Ok(Some(())) => true,
                Ok(None) => false,
                Err(error) => {
                    eprintln!("PortMate: failed to commit shell reader transition: {error}");
                    true
                }
            };
        if accepted {
            if let Ok(Some(runtime)) =
                remove_runtime_if_owned(&io.runtimes.shell, &session_id, |runtime| {
                    runtime.runtime_id == runtime_id
                })
            {
                runtime.closed.store(true, Ordering::SeqCst);
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

#[cfg(test)]
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../test/rust/portmate/support/shell_transport.rs"
));

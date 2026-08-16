use super::*;

#[test]
fn shell_exit_status_disconnect_reason_preserves_code_and_signal() {
    assert_eq!(
        shell_exit_status_disconnect_reason("sh", &portable_pty::ExitStatus::with_exit_code(7)),
        "shell process exited with status 7 (sh)"
    );
    assert_eq!(
        shell_exit_status_disconnect_reason("sh", &portable_pty::ExitStatus::with_signal("TERM")),
        "shell process exited by signal TERM (sh)"
    );
}

#[test]
fn shell_launch_paths_expand_native_home_and_reject_foreign_roots() {
    let mut profile = test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        panic!("expected Shell profile");
    };
    shell.program = "~/.local/bin/zsh".to_string();
    shell.cwd = Some("~/worktree".to_string());
    let home = Path::new("native-home");
    assert_eq!(
        resolve_shell_launch_paths_with_home(
            shell,
            "sh",
            LocalTransferPathPlatform::Unix,
            Some(home),
        )
        .unwrap(),
        ShellLaunchPaths {
            program: home.join(".local/bin/zsh"),
            cwd: Some(home.join("worktree")),
        }
    );

    shell.program = r"~\bin\pwsh.exe".to_string();
    shell.cwd = Some(r"~\worktree".to_string());
    assert_eq!(
        resolve_shell_launch_paths_with_home(
            shell,
            "cmd.exe",
            LocalTransferPathPlatform::Windows,
            Some(home),
        )
        .unwrap(),
        ShellLaunchPaths {
            program: home.join(r"bin\pwsh.exe"),
            cwd: Some(home.join(r"worktree")),
        }
    );

    shell.program = r"C:\Windows\System32\cmd.exe".to_string();
    let error = resolve_shell_launch_paths_with_home(
        shell,
        "sh",
        LocalTransferPathPlatform::Unix,
        Some(home),
    )
    .unwrap_err();
    assert!(error.contains("不兼容"), "{error}");

    shell.program = r"~/C:\Windows\System32\cmd.exe".to_string();
    let error = resolve_shell_launch_paths_with_home(
        shell,
        "cmd.exe",
        LocalTransferPathPlatform::Windows,
        Some(home),
    )
    .unwrap_err();
    assert!(error.contains("盘符后缀"), "{error}");
}

#[test]
fn shell_launch_paths_preserve_significant_edge_whitespace() {
    let mut profile = test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        panic!("expected Shell profile");
    };
    shell.program = "/opt/ custom-shell ".to_string();
    shell.cwd = Some("/srv/ worktree ".to_string());

    let normalized = normalize_session_profile(profile);
    let ConnectionConfig::Shell(mut shell) = normalized.connection else {
        panic!("expected normalized Shell profile");
    };
    assert_eq!(shell.program, "/opt/ custom-shell ");
    assert_eq!(shell.cwd.as_deref(), Some("/srv/ worktree "));
    assert_eq!(
        resolve_shell_launch_paths_with_home(
            &shell,
            "/bin/sh",
            LocalTransferPathPlatform::Unix,
            None,
        )
        .unwrap(),
        ShellLaunchPaths {
            program: PathBuf::from("/opt/ custom-shell "),
            cwd: Some(PathBuf::from("/srv/ worktree ")),
        }
    );

    shell.program = " \t ".to_string();
    shell.cwd = Some(" \n ".to_string());
    assert_eq!(
        resolve_shell_launch_paths_with_home(
            &shell,
            "/bin/default-shell",
            LocalTransferPathPlatform::Unix,
            None,
        )
        .unwrap(),
        ShellLaunchPaths {
            program: PathBuf::from("/bin/default-shell"),
            cwd: None,
        }
    );
}

#[test]
fn shell_open_rejects_a_non_directory_cwd_before_installing_a_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("not-a-directory");
    fs::write(&cwd, b"file").unwrap();
    let mut profile = test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        panic!("expected Shell profile");
    };
    shell.cwd = Some(cwd.display().to_string());
    let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));

    let error = open_shell_session(&state, profile.clone()).unwrap_err();
    assert!(error.contains("Shell 工作目录不是目录"), "{error}");
    assert!(!state.shell.lock().unwrap().contains_key(&profile.id));
}

#[cfg(unix)]
#[test]
fn dropping_a_prepared_shell_session_terminates_its_child_without_installing_a_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let pid_path = temp.path().join("prepared-shell.pid");
    let mut profile = test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        panic!("expected Shell profile");
    };
    shell.args = vec![
        "-c".to_string(),
        "printf '%s' \"$$\" > \"$1\"; while :; do sleep 1; done".to_string(),
        "portmate-shell-test".to_string(),
        pid_path.display().to_string(),
    ];
    let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));

    let prepared = prepare_shell_session(profile.clone()).unwrap();
    let started = Instant::now();
    while !pid_path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "prepared Shell child did not publish its PID"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid = fs::read_to_string(&pid_path)
        .unwrap()
        .parse::<libc::pid_t>()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0);
    assert!(state.shell.lock().unwrap().is_empty());
    assert_eq!(
        state.store.lock().unwrap().summaries()[0].runtime.status,
        SessionStatus::Disconnected
    );

    drop(prepared);

    let stopped = Instant::now();
    loop {
        if unsafe { libc::kill(pid, 0) } == -1 {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ESRCH)
            );
            break;
        }
        assert!(
            stopped.elapsed() < Duration::from_secs(3),
            "dropping the prepared Shell session left its child running"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(state.shell.lock().unwrap().is_empty());
    assert_eq!(
        state.store.lock().unwrap().summaries()[0].runtime.status,
        SessionStatus::Disconnected
    );
}

#[test]
fn shell_profile_path_validation_rejects_the_foreign_platform_before_save() {
    let mut profile = test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        panic!("expected Shell profile");
    };
    shell.program = if cfg!(windows) {
        "/bin/sh".to_string()
    } else {
        r"C:\Windows\System32\cmd.exe".to_string()
    };
    let error = validate_shell_profile_paths(&profile).unwrap_err();
    assert!(
        error.contains("不兼容") || error.contains("盘符") || error.contains("完整 UNC"),
        "{error}"
    );

    assert!(validate_shell_profile_paths(&test_ssh_profile()).is_ok());
}

#[test]
fn shell_profile_validation_bounds_arguments_without_reparsing_them() {
    let mut profile = test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        panic!("expected Shell profile");
    };
    shell.args = vec![
        "-c".to_string(),
        " printf '%s\\n' 'hello world' ".to_string(),
        String::new(),
    ];
    let expected = shell.args.clone();
    assert!(validate_shell_profile_paths(&profile).is_ok());
    let ConnectionConfig::Shell(shell) = &profile.connection else {
        panic!("expected Shell profile");
    };
    assert_eq!(shell.args, expected);

    let mut oversized_count = profile.clone();
    let ConnectionConfig::Shell(shell) = &mut oversized_count.connection else {
        panic!("expected Shell profile");
    };
    shell.args = vec![String::new(); portmate_core::MAX_SHELL_ARGUMENTS + 1];
    let error = validate_shell_profile_paths(&oversized_count).unwrap_err();
    assert!(error.contains("参数数量"), "{error}");

    let mut oversized_argument = profile.clone();
    let ConnectionConfig::Shell(shell) = &mut oversized_argument.connection else {
        panic!("expected Shell profile");
    };
    shell.args = vec!["界".repeat(portmate_core::MAX_SHELL_ARGUMENT_CHARACTERS + 1)];
    let error = validate_shell_profile_paths(&oversized_argument).unwrap_err();
    assert!(error.contains("超过"), "{error}");

    let mut nul_argument = profile;
    let ConnectionConfig::Shell(shell) = &mut nul_argument.connection else {
        panic!("expected Shell profile");
    };
    shell.args = vec!["before\0after".to_string()];
    let error = validate_shell_profile_paths(&nul_argument).unwrap_err();
    assert!(error.contains("NUL"), "{error}");
}

#[cfg(unix)]
#[test]
fn shell_fast_exit_cannot_leave_a_stale_connected_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let mut profile = test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        panic!("expected Shell profile");
    };
    shell.args = vec![
        "-c".to_string(),
        "test \"$1\" = 'two words' || exit 19; exit 7".to_string(),
        "portmate-shell-test".to_string(),
        "two words".to_string(),
    ];
    let state = test_app_state(profile.clone(), temp.path().join("portmate-store.sqlite3"));

    let opened = open_shell_session(&state, profile.clone()).unwrap();
    assert_eq!(opened.runtime.status, SessionStatus::Connected);

    let started = Instant::now();
    let disconnected = loop {
        let summary = state
            .store
            .lock()
            .unwrap()
            .summaries()
            .into_iter()
            .find(|summary| summary.profile.id == profile.id)
            .unwrap();
        if summary.runtime.status == SessionStatus::Disconnected {
            break summary;
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "fast Shell exit left the session connected"
        );
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(
        disconnected.runtime.last_disconnect_reason.as_deref(),
        Some("shell process exited with status 7 (sh)")
    );
    assert!(!state.shell.lock().unwrap().contains_key(&profile.id));
    let store = state.store.lock().unwrap();
    assert_eq!(
        store
            .events
            .iter()
            .filter(|event| {
                event.text.as_deref() == Some("PortMate: shell process exited with status 7 (sh)")
            })
            .count(),
        1
    );
    assert!(store.events.iter().all(|event| {
        event
            .text
            .as_deref()
            .is_none_or(|text| !text.contains("shell closed"))
    }));
}

#[test]
fn reader_start_gate_waits_for_the_connected_commit() {
    let gate = Arc::new(ReaderStartGate::default());
    let (sender, receiver) = std::sync::mpsc::channel();
    let task_gate = Arc::clone(&gate);
    let worker = std::thread::spawn(move || {
        sender.send(task_gate.wait()).unwrap();
    });

    assert!(matches!(
        receiver.recv_timeout(Duration::from_millis(30)),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    ));
    gate.start();
    assert!(receiver.recv_timeout(Duration::from_secs(1)).unwrap());
    worker.join().unwrap();

    let gate = ReaderStartGate::default();
    gate.cancel();
    assert!(!gate.wait());
}

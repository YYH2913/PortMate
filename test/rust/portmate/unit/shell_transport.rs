use super::*;

#[derive(Debug, Default)]
struct ChildState {
    exited: AtomicBool,
    kills: AtomicUsize,
    fail_next_kill: AtomicBool,
    dropped: AtomicBool,
}

#[derive(Debug)]
struct FakeChild {
    state: Arc<ChildState>,
    kill_release: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
}

impl Drop for FakeChild {
    fn drop(&mut self) {
        self.state.dropped.store(true, Ordering::SeqCst);
    }
}

impl portable_pty::ChildKiller for FakeChild {
    fn kill(&mut self) -> std::io::Result<()> {
        self.state.kills.fetch_add(1, Ordering::SeqCst);
        if self.state.fail_next_kill.swap(false, Ordering::SeqCst) {
            return Err(std::io::Error::other("injected signal failure"));
        }
        if let Some(release) = self.kill_release.take() {
            release
                .into_inner()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            self.state.exited.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn clone_killer(&self) -> Box<dyn portable_pty::ChildKiller + Send + Sync> {
        Box::new(Self {
            state: Arc::clone(&self.state),
            kill_release: None,
        })
    }
}

impl portable_pty::Child for FakeChild {
    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        Ok(self
            .state
            .exited
            .load(Ordering::SeqCst)
            .then(|| portable_pty::ExitStatus::with_exit_code(7)))
    }

    fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        panic!("cleanup must poll rather than wait indefinitely")
    }

    fn process_id(&self) -> Option<u32> {
        None
    }

    #[cfg(windows)]
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}

#[test]
fn shell_cleanup_retries_failed_signals_without_reusing_a_reaped_pid() {
    let state = Arc::new(ChildState::default());
    state.fail_next_kill.store(true, Ordering::SeqCst);
    let child: ShellProcess = Box::new(FakeChild {
        state: Arc::clone(&state),
        kill_release: None,
    });
    let ShellChildCleanupStep::Pending {
        process,
        kill_requested,
        warning,
    } = poll_shell_child_cleanup(child, false)
    else {
        panic!("the first signal is rejected");
    };
    assert!(!kill_requested);
    assert!(warning.is_some());
    let ShellChildCleanupStep::Pending {
        process,
        kill_requested,
        warning,
    } = poll_shell_child_cleanup(process, kill_requested)
    else {
        panic!("the child is still awaiting reap");
    };
    assert!(kill_requested);
    assert!(warning.is_none());
    assert_eq!(state.kills.load(Ordering::SeqCst), 2);
    state.exited.store(true, Ordering::SeqCst);
    assert!(matches!(
        poll_shell_child_cleanup(process, kill_requested),
        ShellChildCleanupStep::Exited
    ));
    assert_eq!(state.kills.load(Ordering::SeqCst), 2);
}

#[test]
fn shell_cleanup_retains_one_child_after_the_close_deadline() {
    tauri::async_runtime::block_on(async {
        let state = Arc::new(ChildState::default());
        let child = Arc::new(Mutex::new(ManagedShellChild::new(Box::new(FakeChild {
            state: Arc::clone(&state),
            kill_release: None,
        }))));
        let cleanup = begin_shared_shell_child_cleanup(&child);
        let repeated = begin_shared_shell_child_cleanup(&child);
        drop(child);

        let error = wait_for_shell_child_cleanup(cleanup, Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(error.contains("background reaper"), "{error}");
        assert_eq!(state.kills.load(Ordering::SeqCst), 1);
        assert!(!state.dropped.load(Ordering::SeqCst));

        state.exited.store(true, Ordering::SeqCst);
        wait_for_shell_child_cleanup(repeated, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(state.kills.load(Ordering::SeqCst), 1);
        assert!(state.dropped.load(Ordering::SeqCst));
    });
}

#[test]
fn shell_cleanup_does_not_signal_an_already_reaped_child() {
    tauri::async_runtime::block_on(async {
        let state = Arc::new(ChildState::default());
        state.exited.store(true, Ordering::SeqCst);
        let cleanup = start_shell_child_cleanup(Box::new(FakeChild {
            state: Arc::clone(&state),
            kill_release: None,
        }));
        wait_for_shell_child_cleanup(cleanup, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(state.kills.load(Ordering::SeqCst), 0);
        assert!(state.dropped.load(Ordering::SeqCst));
    });
}

#[test]
fn shell_child_termination_does_not_block_the_async_executor() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let state = Arc::new(ChildState::default());
        let (release, wait_release) = std::sync::mpsc::channel();
        let cleanup = start_shell_child_cleanup(Box::new(FakeChild {
            state: Arc::clone(&state),
            kill_release: Some(Mutex::new(wait_release)),
        }));
        tokio::time::timeout(Duration::from_secs(2), async {
            while state.kills.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        release.send(()).unwrap();
        wait_for_shell_child_cleanup(cleanup, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(state.dropped.load(Ordering::SeqCst));
    });
}

#[cfg(unix)]
fn hup_resistant_profile(pid_path: &Path) -> SessionProfile {
    let mut profile = crate::tests::test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        unreachable!()
    };
    shell.program = "/bin/sh".into();
    // Ignored signals survive exec, so this keeps one owned process and no
    // descendants. It forces portable-pty's HUP grace period -> SIGKILL path.
    shell.args = vec![
        "-c".into(),
        "trap '' HUP; printf '%s' \"$$\" > \"$1\"; exec sleep 30".into(),
        "portmate-shell-reap-test".into(),
        pid_path.display().to_string(),
    ];
    profile
}

#[cfg(unix)]
fn wait_for_test_shell_pid(path: &Path) -> libc::pid_t {
    let started = Instant::now();
    loop {
        if let Ok(raw) = fs::read_to_string(path) {
            if let Ok(pid) = raw.parse() {
                return pid;
            }
        }
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "child PID missing"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
async fn assert_test_shell_reaped(pid: libc::pid_t) {
    let started = Instant::now();
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "Shell child {pid} remains alive or a zombie"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD),
        "PortMate must reap the child, not leave that work to the test"
    );
}

#[cfg(unix)]
#[test]
fn shell_close_reaps_a_child_that_ignores_hup() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let pid_path = root.path().join("shell.pid");
        let profile = hup_resistant_profile(&pid_path);
        let state =
            crate::tests::test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        let prepared = prepare_shell_session(profile.clone()).unwrap();
        let pid = wait_for_test_shell_pid(&pid_path);
        install_shell_session(&state, prepared).unwrap();

        crate::session_close::close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        assert!(state.shell.lock().unwrap().is_empty());
        assert_test_shell_reaped(pid).await;
    });
}

#[cfg(unix)]
#[test]
fn shell_preparation_drop_and_failed_install_reap_hup_resistant_children() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let dropped_pid_path = root.path().join("dropped.pid");
        let prepared = prepare_shell_session(hup_resistant_profile(&dropped_pid_path)).unwrap();
        let dropped_pid = wait_for_test_shell_pid(&dropped_pid_path);
        drop(prepared);
        assert_test_shell_reaped(dropped_pid).await;

        let failed_pid_path = root.path().join("failed.pid");
        let profile = hup_resistant_profile(&failed_pid_path);
        let blocked_parent = root.path().join("not-a-directory");
        fs::write(&blocked_parent, b"blocked").unwrap();
        let state =
            crate::tests::test_app_state(profile.clone(), blocked_parent.join("store.sqlite3"));
        let prepared = prepare_shell_session(profile).unwrap();
        let failed_pid = wait_for_test_shell_pid(&failed_pid_path);
        assert!(install_shell_session(&state, prepared).is_err());
        assert!(state.shell.lock().unwrap().is_empty());
        assert_test_shell_reaped(failed_pid).await;

        let reader_pid_path = root.path().join("reader-failed.pid");
        let profile = hup_resistant_profile(&reader_pid_path);
        let state =
            crate::tests::test_app_state(profile.clone(), root.path().join("reader.sqlite3"));
        let prepared = prepare_shell_session(profile).unwrap();
        let reader_pid = wait_for_test_shell_pid(&reader_pid_path);
        let error = install_shell_session_with_reader(&state, prepared, |_task| {
            Err(std::io::Error::other("injected reader start failure"))
        })
        .unwrap_err();
        assert!(error.contains("injected reader start failure"), "{error}");
        assert!(state.shell.lock().unwrap().is_empty());
        assert_test_shell_reaped(reader_pid).await;
    });
}

#[cfg(unix)]
#[test]
fn shell_cleanup_after_natural_exit_does_not_kill_again() {
    tauri::async_runtime::block_on(async {
        let mut profile = crate::tests::test_shell_profile();
        let ConnectionConfig::Shell(shell) = &mut profile.connection else {
            unreachable!()
        };
        shell.program = "/bin/sh".into();
        shell.args = vec!["-c".into(), "exit 7".into()];
        let mut prepared = prepare_shell_session(profile).unwrap();
        let child = prepared.child.as_mut().unwrap();
        let pid = child.process_id().unwrap() as libc::pid_t;
        let status = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(status.exit_code(), 7);
        let child = Arc::new(Mutex::new(ManagedShellChild::new(
            prepared.child.take().unwrap(),
        )));
        let cleanup = begin_shared_shell_child_cleanup(&child);
        let repeated = begin_shared_shell_child_cleanup(&child);
        wait_for_shell_child_cleanup(cleanup, Duration::from_secs(2))
            .await
            .unwrap();
        wait_for_shell_child_cleanup(repeated, Duration::from_secs(2))
            .await
            .unwrap();
        drop(child);
        assert_test_shell_reaped(pid).await;
    });
}

#[cfg(unix)]
#[test]
fn shell_write_failure_cancel_and_dropped_future_reap_their_child() {
    struct TestWriter {
        started: Arc<AtomicBool>,
        failure: bool,
    }

    impl Write for TestWriter {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            self.started.store(true, Ordering::SeqCst);
            Err(if self.failure {
                std::io::ErrorKind::BrokenPipe
            } else {
                std::io::ErrorKind::WouldBlock
            }
            .into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    tauri::async_runtime::block_on(async {
        for mode in ["failure", "cancel", "drop", "timeout"] {
            let root = tempfile::tempdir().unwrap();
            let pid_path = root.path().join("shell.pid");
            let profile = hup_resistant_profile(&pid_path);
            let state =
                crate::tests::test_app_state(profile.clone(), root.path().join("store.sqlite3"));
            let prepared = prepare_shell_session(profile.clone()).unwrap();
            let pid = wait_for_test_shell_pid(&pid_path);
            install_shell_session(&state, prepared).unwrap();
            let (child, closed) = {
                let runtimes = state.shell.lock().unwrap();
                let runtime = runtimes.get(&profile.id).unwrap();
                (Arc::clone(&runtime.child), Arc::clone(&runtime.closed))
            };
            let started = Arc::new(AtomicBool::new(false));
            let writer: Arc<Mutex<Box<dyn Write + Send>>> =
                Arc::new(Mutex::new(Box::new(TestWriter {
                    started: Arc::clone(&started),
                    failure: mode == "failure",
                })));
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = Arc::clone(&cancel);
            let timeout = if mode == "timeout" {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(10)
            };
            let writing = tokio::spawn(async move {
                write_shell_bytes_with_timeout(
                    writer,
                    child,
                    closed,
                    b"x",
                    Some(worker_cancel),
                    timeout,
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(2), async {
                while !started.load(Ordering::SeqCst) {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            if mode == "drop" {
                writing.abort();
                assert!(writing.await.unwrap_err().is_cancelled());
            } else {
                if mode == "cancel" {
                    cancel.store(true, Ordering::SeqCst);
                }
                assert!(writing.await.unwrap().is_err());
            }
            assert_test_shell_reaped(pid).await;
            crate::session_close::close_session_inner(&state, profile.id)
                .await
                .unwrap();
        }
    });
}

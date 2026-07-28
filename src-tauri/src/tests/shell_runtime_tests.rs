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

#[cfg(unix)]
#[test]
fn shell_fast_exit_cannot_leave_a_stale_connected_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let mut profile = test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        panic!("expected Shell profile");
    };
    shell.args = vec!["-c".to_string(), "exit 7".to_string()];
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

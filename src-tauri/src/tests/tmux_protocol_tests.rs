use super::*;

#[test]
fn tmux_command_types_keep_stable_serde_contract() {
    let request: TmuxMutationRequest = serde_json::from_value(serde_json::json!({
        "sessionId": "ssh-session-1",
        "action": "select-layout",
        "target": "lab:2",
        "layout": "main-horizontal",
        "amount": 12
    }))
    .unwrap();
    assert_eq!(request.session_id, "ssh-session-1");
    assert_eq!(request.action, TmuxMutationAction::SelectLayout);
    assert_eq!(request.target, "lab:2");
    assert_eq!(request.layout, Some(TmuxWindowLayout::MainHorizontal));
    assert_eq!(request.amount, Some(12));
    assert!(request.name.is_none());
    assert!(request.destination.is_none());

    let legacy_status: TmuxControlStatus = serde_json::from_value(serde_json::json!({
        "sessionId": "ssh-session-1",
        "target": "lab",
        "active": true
    }))
    .unwrap();
    assert!(legacy_status.runtime_id.is_none());
    assert_eq!(
        serde_json::to_value(legacy_status).unwrap(),
        serde_json::json!({
            "sessionId": "ssh-session-1",
            "target": "lab",
            "active": true,
            "runtimeId": null
        })
    );
}

#[test]
fn tmux_targets_are_bounded_and_shell_quoted() {
    assert!(tmux_attach_command("  ").is_err());
    assert!(tmux_attach_command("bad\nname").is_err());
    assert!(tmux_attach_command(&"x".repeat(257)).is_err());
    assert_eq!(
        tmux_attach_command("  lab  ").unwrap(),
        "tmux switch-client -t 'lab' || tmux attach -t 'lab' || tmux new-session -A -s 'lab'\r"
    );
    assert_eq!(
        tmux_pane_sync_command("lab'; touch /tmp/portmate-tmux #", true).unwrap(),
        "tmux set-option -w -t 'lab'\\''; touch /tmp/portmate-tmux #' synchronize-panes on"
    );
    assert_eq!(
        tmux_pane_sync_command("lab:2", false).unwrap(),
        "tmux set-option -w -t 'lab:2' synchronize-panes off"
    );
    let request = |action, target: &str, name: Option<&str>| TmuxMutationRequest {
        session_id: "ssh-session-1".to_string(),
        action,
        target: target.to_string(),
        name: name.map(str::to_string),
        destination: None,
        layout: None,
        amount: None,
    };
    assert_eq!(
        tmux_mutation_command(&request(
            TmuxMutationAction::RenameSession,
            "lab",
            Some("release'; touch /tmp/nope #"),
        ))
        .unwrap(),
        "tmux rename-session -t 'lab' 'release'\\''; touch /tmp/nope #'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::KillSession, "lab", None)).unwrap(),
        "tmux kill-session -t 'lab'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::NewWindow, "lab", Some("logs"),))
            .unwrap(),
        "tmux new-window -t 'lab' -n 'logs'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::NewWindow, "lab", None)).unwrap(),
        "tmux new-window -t 'lab'"
    );
    assert_eq!(
        tmux_mutation_command(&request(
            TmuxMutationAction::RenameWindow,
            "lab:2",
            Some("metrics"),
        ))
        .unwrap(),
        "tmux rename-window -t 'lab:2' 'metrics'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::KillWindow, "lab:2", None)).unwrap(),
        "tmux kill-window -t 'lab:2'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::KillPane, "%7", None)).unwrap(),
        "tmux kill-pane -t '%7'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::SelectPane, "%7", None)).unwrap(),
        "tmux select-pane -t '%7'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::BreakPane, "%7", None)).unwrap(),
        "tmux break-pane -d -s '%7'"
    );
    let mut move_pane = request(TmuxMutationAction::MovePaneHorizontal, "%7", None);
    assert!(tmux_mutation_command(&move_pane).is_err());
    move_pane.destination = Some("lab:2'; touch /tmp/nope #".to_string());
    assert_eq!(
        tmux_mutation_command(&move_pane).unwrap(),
        "tmux move-pane -d -h -s '%7' -t 'lab:2'\\''; touch /tmp/nope #'"
    );
    assert_eq!(
        tmux_mutation_event_scope(&move_pane).unwrap(),
        "%7 -> lab:2'; touch /tmp/nope #"
    );
    move_pane.action = TmuxMutationAction::MovePaneVertical;
    move_pane.destination = Some("  lab:2  ".to_string());
    assert_eq!(
        tmux_mutation_command(&move_pane).unwrap(),
        "tmux move-pane -d -v -s '%7' -t 'lab:2'"
    );
    assert_eq!(
        tmux_mutation_event_scope(&move_pane).unwrap(),
        "%7 -> lab:2"
    );
    move_pane.destination = Some("bad\nwindow".to_string());
    assert!(tmux_mutation_command(&move_pane).is_err());
    assert_eq!(
        tmux_mutation_command(&request(
            TmuxMutationAction::SplitPaneHorizontal,
            "%7'; touch /tmp/nope #",
            None,
        ))
        .unwrap(),
        "tmux split-window -h -t '%7'\\''; touch /tmp/nope #'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::SplitPaneVertical, "%7", None,))
            .unwrap(),
        "tmux split-window -v -t '%7'"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::SwapPanePrevious, "%7", None,)).unwrap(),
        "tmux swap-pane -t '%7' -U"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::SwapPaneNext, "%7", None)).unwrap(),
        "tmux swap-pane -t '%7' -D"
    );
    assert_eq!(
        tmux_mutation_command(&request(TmuxMutationAction::ResizePaneLeft, "%7", None)).unwrap(),
        "tmux resize-pane -t '%7' -L 5"
    );
    let mut resize = request(TmuxMutationAction::ResizePaneRight, "%7", None);
    resize.amount = Some(12);
    assert_eq!(
        tmux_mutation_command(&resize).unwrap(),
        "tmux resize-pane -t '%7' -R 12"
    );
    resize.action = TmuxMutationAction::ResizePaneUp;
    assert_eq!(
        tmux_mutation_command(&resize).unwrap(),
        "tmux resize-pane -t '%7' -U 12"
    );
    resize.action = TmuxMutationAction::ResizePaneDown;
    assert_eq!(
        tmux_mutation_command(&resize).unwrap(),
        "tmux resize-pane -t '%7' -D 12"
    );
    resize.amount = Some(0);
    assert!(tmux_mutation_command(&resize).is_err());
    resize.amount = Some(101);
    assert!(tmux_mutation_command(&resize).is_err());
    let mut layout = request(TmuxMutationAction::SelectLayout, "lab:2", None);
    assert!(tmux_mutation_command(&layout).is_err());
    layout.layout = Some(TmuxWindowLayout::Tiled);
    assert_eq!(
        tmux_mutation_command(&layout).unwrap(),
        "tmux select-layout -t 'lab:2' tiled"
    );
    assert_eq!(
        [
            TmuxWindowLayout::EvenHorizontal,
            TmuxWindowLayout::EvenVertical,
            TmuxWindowLayout::MainHorizontal,
            TmuxWindowLayout::MainVertical,
            TmuxWindowLayout::Tiled,
        ]
        .map(tmux_window_layout_argument),
        [
            "even-horizontal",
            "even-vertical",
            "main-horizontal",
            "main-vertical",
            "tiled",
        ]
    );
    assert!(
        tmux_mutation_command(&request(TmuxMutationAction::RenameSession, "lab", None,)).is_err()
    );
    assert!(tmux_mutation_command(&request(
        TmuxMutationAction::RenameWindow,
        "lab:2",
        Some("bad\nname"),
    ))
    .is_err());
}

#[test]
fn tmux_pane_parser_reads_synchronization_state_conservatively() {
    let pane = parse_tmux_pane("lab\t2\t1\t%7\t1\tvim\teditor\t1").unwrap();
    assert_eq!(pane.session, "lab");
    assert_eq!(pane.window_index, 2);
    assert_eq!(pane.pane_index, 1);
    assert_eq!(pane.pane_id, "%7");
    assert!(pane.active);
    assert!(pane.synchronized);
    assert_eq!(pane.command, "vim");
    assert_eq!(pane.title, "editor");

    let separated = format!(
        "lab{0}2{0}1{0}%7{0}1{0}vim{0}editor{0}1",
        TMUX_FIELD_SEPARATOR
    );
    assert_eq!(parse_tmux_pane(&separated).unwrap().pane_id, "%7");

    let legacy = parse_tmux_pane("lab\t0\t0\t%1\t0\tbash\tshell").unwrap();
    assert!(!legacy.synchronized);
    assert!(parse_tmux_pane("\t0\t0\t%1\t0\tbash\tshell\t1").is_none());

    let window = parse_tmux_window("lab\t2\t@4\tmetrics\t3\t1").unwrap();
    assert_eq!(window.session, "lab");
    assert_eq!(window.window_index, 2);
    assert_eq!(window.window_id, "@4");
    assert_eq!(window.name, "metrics");
    assert_eq!(window.panes, 3);
    assert!(window.active);
    assert!(!window.synchronized);
    assert!(parse_tmux_window("\t2\t@4\tmetrics\t3\t1").is_none());
}

#[test]
fn tmux_control_parser_ignores_output_and_tracks_fragmented_state_events() {
    let mut parser = TmuxControlLineParser::default();
    assert_eq!(
        parser.push(b"%out").unwrap(),
        TmuxControlParseResult::default()
    );
    let parsed = parser
        .push(b"put %1 ignored terminal bytes\n%window-add @2\r\n%layout")
        .unwrap();
    assert_eq!(
        parsed,
        TmuxControlParseResult {
            changed: true,
            last_event: Some("window-add"),
        }
    );
    let parsed = parser
        .push(b"-change @2 layout visible flags\n%begin 1 2 3\n")
        .unwrap();
    assert_eq!(
        parsed,
        TmuxControlParseResult {
            changed: true,
            last_event: Some("layout-change"),
        }
    );
    assert_eq!(
        tmux_control_event_kind(b"%window-pane-changed @2 %7"),
        Some("window-pane-changed")
    );
    assert_eq!(
        tmux_control_event_kind(b"%extended-output %7 0 : data"),
        None
    );

    let mut oversized = TmuxControlLineParser::default();
    assert!(oversized
        .push(&vec![b'x'; MAX_TMUX_CONTROL_LINE_BYTES + 1])
        .is_err());
    assert_eq!(
        bounded_tmux_control_error(&"x".repeat(600)).chars().count(),
        515
    );
}

#[test]
fn tmux_control_capacity_bounds_registry_and_rechecks_installation() {
    let runtime = |runtime_id: String, target: String| TmuxControlRuntime {
        runtime_id,
        ssh_runtime_id: "ssh-runtime".to_string(),
        target,
        cancel: Arc::new(AtomicBool::new(false)),
    };
    let mut session_controls = HashMap::new();
    for index in 0..MAX_TMUX_CONTROLS_PER_SESSION {
        let target = format!("target-{index}");
        session_controls.insert(
            ("session-1".to_string(), target.clone()),
            runtime(format!("runtime-{index}"), target),
        );
    }
    let existing_key = ("session-1".to_string(), "target-0".to_string());
    assert!(ensure_tmux_control_capacity(&session_controls, &existing_key).is_ok());
    let session_error = ensure_tmux_control_capacity(
        &session_controls,
        &("session-1".to_string(), "overflow".to_string()),
    )
    .unwrap_err();
    assert!(session_error.contains(&MAX_TMUX_CONTROLS_PER_SESSION.to_string()));
    assert!(ensure_tmux_control_capacity(
        &session_controls,
        &("session-2".to_string(), "allowed".to_string()),
    )
    .is_ok());

    match install_tmux_control_runtime(
        &mut session_controls,
        &existing_key,
        runtime("duplicate".to_string(), "target-0".to_string()),
    )
    .unwrap()
    {
        TmuxControlInstall::Existing(existing) => {
            assert_eq!(existing.runtime_id, "runtime-0")
        }
        TmuxControlInstall::Installed(_) => panic!("active watcher must remain idempotent"),
    }
    session_controls
        .get(&existing_key)
        .unwrap()
        .cancel
        .store(true, Ordering::SeqCst);
    match install_tmux_control_runtime(
        &mut session_controls,
        &existing_key,
        runtime("replacement".to_string(), "target-0".to_string()),
    )
    .unwrap()
    {
        TmuxControlInstall::Installed(Some(previous)) => {
            assert_eq!(previous.runtime_id, "runtime-0")
        }
        _ => panic!("cancelled watcher should be replaceable without growing the registry"),
    }
    assert_eq!(session_controls.len(), MAX_TMUX_CONTROLS_PER_SESSION);
    assert_eq!(
        session_controls.get(&existing_key).unwrap().runtime_id,
        "replacement"
    );
    {
        let stale = session_controls.get_mut(&existing_key).unwrap();
        stale.ssh_runtime_id = "stale-ssh-runtime".to_string();
        stale.cancel.store(false, Ordering::SeqCst);
    }
    match install_tmux_control_runtime(
        &mut session_controls,
        &existing_key,
        runtime("current-parent".to_string(), "target-0".to_string()),
    )
    .unwrap()
    {
        TmuxControlInstall::Installed(Some(previous)) => {
            assert_eq!(previous.runtime_id, "replacement");
            assert_eq!(previous.ssh_runtime_id, "stale-ssh-runtime");
        }
        _ => panic!("a watcher from an earlier SSH runtime must be replaced"),
    }
    assert_eq!(
        session_controls.get(&existing_key).unwrap().ssh_runtime_id,
        "ssh-runtime"
    );

    let mut app_controls = HashMap::new();
    let pending_key = ("pending-session".to_string(), "pending".to_string());
    assert!(ensure_tmux_control_capacity(&app_controls, &pending_key).is_ok());
    for index in 0..MAX_ACTIVE_TMUX_CONTROLS {
        let target = format!("target-{index}");
        app_controls.insert(
            (
                format!("session-{}", index / MAX_TMUX_CONTROLS_PER_SESSION),
                target.clone(),
            ),
            runtime(format!("runtime-{index}"), target),
        );
    }
    let app_error = install_tmux_control_runtime(
        &mut app_controls,
        &pending_key,
        runtime("pending-runtime".to_string(), "pending".to_string()),
    )
    .err()
    .unwrap();
    assert!(app_error.contains("app limit"));
    assert!(app_error.contains(&MAX_ACTIVE_TMUX_CONTROLS.to_string()));
    assert_eq!(app_controls.len(), MAX_ACTIVE_TMUX_CONTROLS);
    assert!(!app_controls.contains_key(&pending_key));
}

#[test]
fn tmux_control_slot_saturation_rejects_before_ssh_lookup() {
    tauri::async_runtime::block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let state = test_app_state(test_ssh_profile(), temp.path().join("store.sqlite3"));
        let _permits = Arc::clone(&state.tmux_control_slots)
            .try_acquire_many_owned(MAX_ACTIVE_TMUX_CONTROLS as u32)
            .unwrap();

        let error = start_tmux_control_inner(&state, "ssh-session-1", "lab")
            .await
            .unwrap_err();

        assert!(error.contains("watcher limit"), "{error}");
        assert!(state.tmux_controls.lock().unwrap().is_empty());
    });
}

#[test]
fn tmux_control_runtime_cancel_is_exact_and_session_cleanup_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let state = test_app_state(test_ssh_profile(), temp.path().join("store.sqlite3"));
    let lab_cancel = Arc::new(AtomicBool::new(false));
    let build_cancel = Arc::new(AtomicBool::new(false));
    let other_cancel = Arc::new(AtomicBool::new(false));
    let mut controls = state.tmux_controls.lock().unwrap();
    for (session_id, target, runtime_id, cancel) in [
        ("session:1", "lab", "control-1", Arc::clone(&lab_cancel)),
        ("session:1", "build", "control-2", Arc::clone(&build_cancel)),
        ("session:2", "lab", "control-3", Arc::clone(&other_cancel)),
    ] {
        controls.insert(
            (session_id.to_string(), target.to_string()),
            TmuxControlRuntime {
                runtime_id: runtime_id.to_string(),
                ssh_runtime_id: "ssh-runtime".to_string(),
                target: target.to_string(),
                cancel,
            },
        );
    }
    drop(controls);

    let cancelled = cancel_tmux_control_runtime(&state, "session:1", "lab")
        .unwrap()
        .unwrap();
    assert_eq!(cancelled.runtime_id, "control-1");
    assert_eq!(cancelled.target, "lab");
    assert!(lab_cancel.load(Ordering::SeqCst));
    assert!(!build_cancel.load(Ordering::SeqCst));
    assert!(!other_cancel.load(Ordering::SeqCst));
    assert_eq!(state.tmux_controls.lock().unwrap().len(), 3);
    assert_eq!(
        cancel_tmux_control_runtime(&state, "session:1", "lab")
            .unwrap()
            .unwrap()
            .runtime_id,
        "control-1"
    );

    let ops_cancel = Arc::new(AtomicBool::new(false));
    state.tmux_controls.lock().unwrap().insert(
        ("session:1".to_string(), "ops".to_string()),
        TmuxControlRuntime {
            runtime_id: "control-4".to_string(),
            ssh_runtime_id: "ssh-runtime".to_string(),
            target: "ops".to_string(),
            cancel: Arc::clone(&ops_cancel),
        },
    );
    let status = stop_tmux_control_inner(&state, "session:1", None, None).unwrap();
    assert!(status.target.is_empty());
    assert!(status.runtime_id.is_none());
    assert!(!status.active);
    assert!(build_cancel.load(Ordering::SeqCst));
    assert!(ops_cancel.load(Ordering::SeqCst));
    assert!(!other_cancel.load(Ordering::SeqCst));
    assert!(!session_has_registered_runtime(&state, "session:1").unwrap());
    assert_eq!(state.tmux_controls.lock().unwrap().len(), 4);

    let stale_status =
        stop_tmux_control_inner(&state, "session:2", Some("lab"), Some("stale-control")).unwrap();
    assert!(stale_status.active);
    assert_eq!(stale_status.runtime_id.as_deref(), Some("control-3"));
    assert!(!other_cancel.load(Ordering::SeqCst));

    let status =
        stop_tmux_control_inner(&state, "session:2", Some("lab"), Some("control-3")).unwrap();
    assert_eq!(status.target, "lab");
    assert_eq!(status.runtime_id.as_deref(), Some("control-3"));
    assert!(!status.active);
    assert!(other_cancel.load(Ordering::SeqCst));
    assert!(!session_has_registered_runtime(&state, "session:2").unwrap());
    assert_eq!(state.tmux_controls.lock().unwrap().len(), 4);
}

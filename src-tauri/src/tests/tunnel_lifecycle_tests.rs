#[test]
fn tunnel_start_commit_failure_closes_runtime_and_rolls_back_store() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-tunnel-start-commit-failure-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let blocked_parent = root.join("not-a-directory");
        fs::write(&blocked_parent, b"blocked").unwrap();
        let profile = test_ssh_profile();
        let state = test_app_state(
            profile.clone(),
            blocked_parent.join("portmate-store.sqlite3"),
        );
        let tunnel = TunnelSpec {
            id: "uncommitted-tunnel".to_string(),
            label: "uncommitted tunnel".to_string(),
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_022,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            route_rules: Vec::new(),
            enabled: true,
        };
        let closed = Arc::new(AtomicBool::new(false));
        state.tunnels.lock().unwrap().insert(
            tunnel.id.clone(),
            TunnelRuntime {
                session_id: profile.id.clone(),
                ssh_runtime_id: "ssh-runtime-1".to_string(),
                spec: tunnel.clone(),
                metrics: Arc::new(TunnelMetrics::default()),
                closed: Arc::clone(&closed),
            },
        );

        let error = commit_started_tunnel(&state, &profile.id, tunnel, None, "ssh-runtime-1")
            .await
            .unwrap_err();

        assert!(error.contains("uncommitted runtime was closed"), "{error}");
        assert!(closed.load(Ordering::SeqCst));
        assert!(state.tunnels.lock().unwrap().is_empty());
        let store = state.store.lock().unwrap();
        assert!(store.events.is_empty());
        let saved = store.profile(&profile.id).unwrap();
        let ConnectionConfig::Ssh(ssh) = saved.connection else {
            panic!("expected SSH profile");
        };
        assert!(ssh.tunnels.is_empty());

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn tunnel_stop_persistence_failure_keeps_local_stop_truth() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-tunnel-stop-persistence-failure-{}",
            Uuid::new_v4()
        ));
        fs::create_dir_all(&root).unwrap();
        let blocked_parent = root.join("not-a-directory");
        fs::write(&blocked_parent, b"blocked").unwrap();
        let tunnel = TunnelSpec {
            id: "stopped-tunnel".to_string(),
            label: "stopped tunnel".to_string(),
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_023,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            route_rules: Vec::new(),
            enabled: true,
        };
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.tunnels.push(tunnel.clone());
        let state = test_app_state(profile.clone(), blocked_parent.join("store.sqlite3"));
        let closed = Arc::new(AtomicBool::new(false));
        state.tunnels.lock().unwrap().insert(
            tunnel.id.clone(),
            TunnelRuntime {
                session_id: profile.id.clone(),
                ssh_runtime_id: "ssh-runtime-1".to_string(),
                spec: tunnel.clone(),
                metrics: Arc::new(TunnelMetrics::default()),
                closed: Arc::clone(&closed),
            },
        );

        let error = stop_tunnel_inner(&state, &tunnel.id).await.unwrap_err();

        assert!(error.contains("tunnel stopped locally"), "{error}");
        assert!(closed.load(Ordering::SeqCst));
        assert!(state.tunnels.lock().unwrap().is_empty());
        let store = state.store.lock().unwrap();
        let saved = store.profile(&profile.id).unwrap();
        let ConnectionConfig::Ssh(ssh) = saved.connection else {
            panic!("expected SSH profile");
        };
        assert_eq!(ssh.tunnels.len(), 1);
        assert!(!ssh.tunnels[0].enabled);
        assert!(store.events.iter().any(|event| {
            event
                .text
                .as_deref()
                .is_some_and(|text| text.contains("tunnel stopped"))
        }));

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn runtime_cleanup_never_removes_a_superseding_entry() {
    let registry = Mutex::new(HashMap::from([(
        "session-1".to_string(),
        "runtime-new".to_string(),
    )]));
    assert!(remove_runtime_if_owned(&registry, "session-1", |runtime| {
        runtime == "runtime-old"
    })
    .unwrap()
    .is_none());
    assert_eq!(
        registry
            .lock()
            .unwrap()
            .get("session-1")
            .map(String::as_str),
        Some("runtime-new")
    );
    assert_eq!(
        remove_runtime_if_owned(&registry, "session-1", |runtime| {
            runtime == "runtime-new"
        })
        .unwrap()
        .as_deref(),
        Some("runtime-new")
    );
    assert!(registry.lock().unwrap().is_empty());
}

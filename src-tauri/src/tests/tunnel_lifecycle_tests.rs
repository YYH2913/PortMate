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
                listener_worker: TunnelListenerWorker::completed(),
            },
        );

        let owner = TunnelRuntimeOwner {
            ssh_runtime_id: "ssh-runtime-1".to_string(),
            closed: Arc::clone(&closed),
        };
        let error = commit_started_tunnel(&state, &profile.id, tunnel, None, &owner)
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
fn tunnel_lifecycle_lane_is_shared_only_by_the_same_store_and_tunnel() {
    let root = tempfile::tempdir().unwrap();
    let profile = test_ssh_profile();
    let state = test_app_state(profile, root.path().join("store.sqlite3"));
    let same = tunnel_lifecycle_lane(&state, "tunnel-a").unwrap();
    let same_again = tunnel_lifecycle_lane(&state, "tunnel-a").unwrap();
    let other = tunnel_lifecycle_lane(&state, "tunnel-b").unwrap();

    assert!(Arc::ptr_eq(&same, &same_again));
    assert!(!Arc::ptr_eq(&same, &other));
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
                listener_worker: TunnelListenerWorker::completed(),
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

#[test]
fn tunnel_client_failure_events_require_the_exact_runtime_generation() {
    let root = tempfile::tempdir().unwrap();
    let profile = test_ssh_profile();
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    let spec = TunnelSpec {
        id: "client-failure-generation".to_string(),
        label: "client failure generation".to_string(),
        mode: TunnelMode::Local,
        bind_host: "127.0.0.1".to_string(),
        bind_port: 10_022,
        target_host: "127.0.0.1".to_string(),
        target_port: 22,
        route_rules: Vec::new(),
        enabled: true,
    };
    let first = TunnelRuntime {
        session_id: profile.id.clone(),
        ssh_runtime_id: "shared-ssh-runtime".to_string(),
        spec: spec.clone(),
        metrics: Arc::new(TunnelMetrics::default()),
        closed: Arc::new(AtomicBool::new(false)),
        listener_worker: TunnelListenerWorker::completed(),
    };
    let first_owner = first.owner();
    state
        .tunnels
        .lock()
        .unwrap()
        .insert(spec.id.clone(), first);

    assert!(record_tunnel_client_failure_if_owned(
        &state.tunnels,
        &state.store,
        &state.store_path,
        &spec.id,
        &first_owner,
        &profile.id,
        "first generation failure",
    )
    .unwrap());

    let replacement = TunnelRuntime {
        session_id: profile.id.clone(),
        ssh_runtime_id: "shared-ssh-runtime".to_string(),
        spec: spec.clone(),
        metrics: Arc::new(TunnelMetrics::default()),
        closed: Arc::new(AtomicBool::new(false)),
        listener_worker: TunnelListenerWorker::completed(),
    };
    let replacement_owner = replacement.owner();
    state
        .tunnels
        .lock()
        .unwrap()
        .insert(spec.id.clone(), replacement);
    assert!(!record_tunnel_client_failure_if_owned(
        &state.tunnels,
        &state.store,
        &state.store_path,
        &spec.id,
        &first_owner,
        &profile.id,
        "stale generation failure",
    )
    .unwrap());
    assert!(record_tunnel_client_failure_if_owned(
        &state.tunnels,
        &state.store,
        &state.store_path,
        &spec.id,
        &replacement_owner,
        &profile.id,
        "replacement generation failure",
    )
    .unwrap());

    let events = state.store.lock().unwrap().events.clone();
    assert!(events.iter().any(|event| {
        event
            .text
            .as_deref()
            .is_some_and(|text| text.contains("first generation failure"))
    }));
    assert!(events.iter().any(|event| {
        event
            .text
            .as_deref()
            .is_some_and(|text| text.contains("replacement generation failure"))
    }));
    assert!(!events.iter().any(|event| {
        event
            .text
            .as_deref()
            .is_some_and(|text| text.contains("stale generation failure"))
    }));
}

#[test]
fn stale_listener_failure_cannot_disable_a_replacement_tunnel() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let mut profile = test_ssh_profile();
        let spec = TunnelSpec {
            id: "listener-failure-generation".to_string(),
            label: "listener failure generation".to_string(),
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_022,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            route_rules: Vec::new(),
            enabled: true,
        };
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.tunnels.push(spec.clone());
        let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        let first = TunnelRuntime {
            session_id: profile.id.clone(),
            ssh_runtime_id: "shared-ssh-runtime".to_string(),
            spec: spec.clone(),
            metrics: Arc::new(TunnelMetrics::default()),
            closed: Arc::new(AtomicBool::new(false)),
            listener_worker: TunnelListenerWorker::completed(),
        };
        let first_owner = first.owner();
        state
            .tunnels
            .lock()
            .unwrap()
            .insert(spec.id.clone(), first);

        let lane = tunnel_lifecycle_lane(&state, &spec.id).unwrap();
        let guard = lane.lock().await;
        let failure_state = state.clone();
        let tunnel_id = spec.id.clone();
        let session_id = profile.id.clone();
        let failed_spec = spec.clone();
        let failure = tokio::spawn(async move {
            fail_tunnel_listener_if_owned(
                &failure_state,
                &tunnel_id,
                &first_owner,
                &session_id,
                &failed_spec,
                "stale listener failure",
            )
            .await
            .unwrap()
        });
        tokio::task::yield_now().await;
        assert!(!failure.is_finished());

        let replacement = TunnelRuntime {
            session_id: profile.id.clone(),
            ssh_runtime_id: "shared-ssh-runtime".to_string(),
            spec: spec.clone(),
            metrics: Arc::new(TunnelMetrics::default()),
            closed: Arc::new(AtomicBool::new(false)),
            listener_worker: TunnelListenerWorker::completed(),
        };
        let replacement_owner = replacement.owner();
        state
            .tunnels
            .lock()
            .unwrap()
            .insert(spec.id.clone(), replacement);
        drop(guard);

        assert!(!failure.await.unwrap());
        let tunnels = state.tunnels.lock().unwrap();
        assert!(tunnels
            .get(&spec.id)
            .is_some_and(|runtime| replacement_owner.owns(runtime)));
        drop(tunnels);
        let store = state.store.lock().unwrap();
        let saved = store.profile(&profile.id).unwrap();
        let ConnectionConfig::Ssh(ssh) = saved.connection else {
            panic!("expected SSH profile");
        };
        assert!(ssh.tunnels.iter().any(|tunnel| tunnel.id == spec.id && tunnel.enabled));
        assert!(!store.events.iter().any(|event| {
            event
                .text
                .as_deref()
                .is_some_and(|text| text.contains("stale listener failure"))
        }));
    });
}

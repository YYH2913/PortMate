#[test]
fn ssh_channel_failure_removes_only_its_tunnel_runtimes() {
    let first_closed = Arc::new(AtomicBool::new(false));
    let second_closed = Arc::new(AtomicBool::new(false));
    let other_closed = Arc::new(AtomicBool::new(false));
    let first_metrics = Arc::new(TunnelMetrics::default());
    let second_metrics = Arc::new(TunnelMetrics::default());
    let other_metrics = Arc::new(TunnelMetrics::default());
    let runtime = |id: &str,
                   session_id: &str,
                   metrics: Arc<TunnelMetrics>,
                   closed: Arc<AtomicBool>| TunnelRuntime {
        session_id: session_id.to_string(),
        ssh_runtime_id: format!("runtime-{session_id}"),
        spec: TunnelSpec {
            id: id.to_string(),
            label: id.to_string(),
            mode: TunnelMode::Local,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 0,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            route_rules: Vec::new(),
            enabled: true,
        },
        metrics,
        closed,
        listener_worker: TunnelListenerWorker::completed(),
    };
    let tunnels = Arc::new(Mutex::new(HashMap::from([
        (
            "first".to_string(),
            runtime(
                "first",
                "session-a",
                Arc::clone(&first_metrics),
                Arc::clone(&first_closed),
            ),
        ),
        (
            "second".to_string(),
            runtime(
                "second",
                "session-a",
                Arc::clone(&second_metrics),
                Arc::clone(&second_closed),
            ),
        ),
        (
            "other".to_string(),
            runtime(
                "other",
                "session-b",
                Arc::clone(&other_metrics),
                Arc::clone(&other_closed),
            ),
        ),
    ])));

    let removed =
        fail_session_tunnel_runtimes(&tunnels, "session-a", "SSH channel closed").unwrap();
    assert_eq!(removed.len(), 2);
    assert!(first_closed.load(Ordering::SeqCst));
    assert!(second_closed.load(Ordering::SeqCst));
    assert!(!other_closed.load(Ordering::SeqCst));
    assert_eq!(
        first_metrics
            .snapshot(TunnelSpec {
                id: "first".to_string(),
                label: String::new(),
                mode: TunnelMode::Local,
                bind_host: String::new(),
                bind_port: 0,
                target_host: String::new(),
                target_port: 0,
                route_rules: Vec::new(),
                enabled: false,
            })
            .last_error
            .as_deref(),
        Some("SSH channel closed")
    );
    let remaining = tunnels.lock().unwrap();
    assert_eq!(remaining.len(), 1);
    assert!(remaining.contains_key("other"));
    assert!(other_metrics
        .snapshot(remaining["other"].spec.clone())
        .last_error
        .is_none());
}

#[test]
fn tunnel_listener_worker_reports_shutdown_completion() {
    tauri::async_runtime::block_on(async {
        let (worker, completion) = TunnelListenerWorker::running();
        let waiter = worker.clone();
        let task = tauri::async_runtime::spawn(async move {
            waiter.wait_shutdown().await;
            drop(completion);
        });

        worker.request_shutdown();
        tokio::time::timeout(Duration::from_secs(1), worker.wait_finished())
            .await
            .expect("listener worker should report completion");
        task.await.unwrap();
        assert!(worker.is_finished());
    });
}

#[test]
fn stopping_tunnel_marks_profile_tunnel_disabled() {
    let mut store = SessionStore::default();
    let mut profile = test_ssh_profile();
    let tunnel = TunnelSpec {
        id: "tunnel-1".to_string(),
        label: "127.0.0.1:10022 -> 127.0.0.1:22".to_string(),
        mode: TunnelMode::Local,
        bind_host: "127.0.0.1".to_string(),
        bind_port: 10022,
        target_host: "127.0.0.1".to_string(),
        target_port: 22,
        route_rules: Vec::new(),
        enabled: true,
    };
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.tunnels.push(tunnel.clone());
    }
    store.upsert_profile(profile);

    let mut stopped = tunnel;
    stopped.enabled = false;
    mark_tunnel_stopped_in_store(&mut store, "ssh-session-1", &stopped);

    let saved = match store.profile("ssh-session-1").unwrap().connection {
        ConnectionConfig::Ssh(ssh) => ssh.tunnels,
        _ => panic!("expected SSH profile"),
    };
    assert_eq!(saved.len(), 1);
    assert!(!saved[0].enabled);
}

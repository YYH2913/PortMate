#[test]
fn libssh_remote_forward_acceptor_registration_recovers_after_worker_exit() {
    tauri::async_runtime::block_on(async {
        let session = libssh_rs::Session::new().unwrap();
        let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
        let runtime_closed = Arc::new(AtomicBool::new(false));
        let started = Arc::new(AtomicBool::new(false));

        assert!(ensure_libssh_remote_forward_acceptor(
            Some(session.clone()),
            Arc::clone(&remote_forwards),
            Arc::clone(&runtime_closed),
            Arc::clone(&started),
        ));
        assert!(started.load(Ordering::SeqCst));
        assert!(!ensure_libssh_remote_forward_acceptor(
            Some(session.clone()),
            Arc::clone(&remote_forwards),
            Arc::clone(&runtime_closed),
            Arc::clone(&started),
        ));

        runtime_closed.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(2), async {
            while started.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("libssh remote forward acceptor did not release its registration");
        assert!(!ensure_libssh_remote_forward_acceptor(
            Some(session.clone()),
            Arc::clone(&remote_forwards),
            Arc::clone(&runtime_closed),
            Arc::clone(&started),
        ));

        runtime_closed.store(false, Ordering::SeqCst);
        assert!(ensure_libssh_remote_forward_acceptor(
            Some(session),
            remote_forwards,
            Arc::clone(&runtime_closed),
            Arc::clone(&started),
        ));
        runtime_closed.store(true, Ordering::SeqCst);
        tokio::time::timeout(Duration::from_secs(2), async {
            while started.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("restarted libssh remote forward acceptor did not stop");
    });
}

#[test]
fn remote_forward_rollback_covers_returned_and_requested_ports_without_duplicates() {
    assert_eq!(remote_forward_rollback_ports(0, Some(0)), vec![0]);
    assert_eq!(remote_forward_rollback_ports(0, Some(41_923)), vec![41_923]);
    assert_eq!(remote_forward_rollback_ports(41_923, None), vec![41_923]);
    assert_eq!(
        remote_forward_rollback_ports(41_923, Some(0)),
        vec![41_923]
    );
    assert_eq!(
        remote_forward_rollback_ports(41_923, Some(41_923)),
        vec![41_923]
    );
    assert_eq!(
        remote_forward_rollback_ports(41_923, Some(41_924)),
        vec![41_924, 41_923]
    );
}

#[test]
fn remote_forward_route_cleanup_requires_the_exact_runtime_generation() {
    let spec = TunnelSpec {
        id: "shared-route-id".to_string(),
        label: "Shared route".to_string(),
        mode: TunnelMode::Remote,
        bind_host: "127.0.0.1".to_string(),
        bind_port: 41_923,
        target_host: "127.0.0.1".to_string(),
        target_port: 22,
        route_rules: Vec::new(),
        enabled: true,
    };
    let original_owner = Arc::new(TunnelMetrics::default());
    let replacement_owner = Arc::new(TunnelMetrics::default());
    let target = TunnelForwardTarget {
        spec: spec.clone(),
        metrics: Arc::clone(&original_owner),
        connection_slots: Arc::new(tokio::sync::Semaphore::new(1)),
    };
    let mut forwards = HashMap::from([
        (
            remote_forward_key(&spec.bind_host, spec.bind_port),
            target.clone(),
        ),
        (remote_forward_port_key(spec.bind_port), target),
    ]);

    ensure_remote_forward_route_slot(&forwards, &spec, &original_owner).unwrap();
    let conflict =
        ensure_remote_forward_route_slot(&forwards, &spec, &replacement_owner).unwrap_err();
    assert!(conflict.contains("already registered"), "{conflict}");

    remove_remote_forward_routes_if_owned(&mut forwards, &spec, &replacement_owner);
    assert_eq!(forwards.len(), 2);
    remove_remote_forward_routes_if_owned(&mut forwards, &spec, &original_owner);
    assert!(forwards.is_empty());
}

#[test]
fn remote_tunnel_listener_probe_parses_linux_bsd_macos_and_unsupported_outputs() {
    let proc_output = "__PORTMATE_PROC__\n  0: 0100007F:0016 00000000:0000 0A 00000000:00000000\n  1: 0100007F:2710 00000000:0000 01 00000000:00000000\n";
    assert_eq!(
        parse_remote_listener_probe(proc_output, 22),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(proc_output, 10_000),
        RemoteListenerProbe::Missing
    );

    let ss_output =
        "__PORTMATE_SS__\nLISTEN 0 128 127.0.0.1:10022 0.0.0.0:*\nLISTEN 0 128 [::]:2200 [::]:*\n";
    assert_eq!(
        parse_remote_listener_probe(ss_output, 10_022),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(ss_output, 2_200),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(ss_output, 22),
        RemoteListenerProbe::Missing
    );

    let sockstat_output = "__PORTMATE_SOCKSTAT__\nUSER COMMAND PID FD PROTO LOCAL ADDRESS FOREIGN ADDRESS\nroot sshd 431 7 tcp4 127.0.0.1:10022 *:*\nroot sshd 431 8 tcp6 [::]:2200 [::]:*\n";
    assert_eq!(
        parse_remote_listener_probe(sockstat_output, 10_022),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(sockstat_output, 22),
        RemoteListenerProbe::Missing
    );

    let sockstat_service_output = "__PORTMATE_SOCKSTAT__\nUSER COMMAND PID FD PROTO LOCAL ADDRESS FOREIGN ADDRESS\nroot sshd 431 7 tcp4 127.0.0.1:ssh *:*\n";
    assert_eq!(
        parse_remote_listener_probe(sockstat_service_output, 22),
        RemoteListenerProbe::Missing
    );

    let lsof_output = "__PORTMATE_LSOF__\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nsshd 912 root 5u IPv4 0x1 0t0 TCP *:2200 (LISTEN)\n";
    assert_eq!(
        parse_remote_listener_probe(lsof_output, 2_200),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(lsof_output, 22),
        RemoteListenerProbe::Missing
    );

    let bsd_netstat_output = "__PORTMATE_NETSTAT__\ntcp4 0 0 127.0.0.1.10022 *.* LISTEN\n";
    assert_eq!(
        parse_remote_listener_probe(bsd_netstat_output, 10_022),
        RemoteListenerProbe::Listening
    );

    let windows_output = "Windows OpenSSH preface\r\n__PORTMATE_WINDOWS_TCP__\r\n22\r\n2200\r\ninvalid\r\n65536\r\n";
    assert_eq!(
        parse_remote_listener_probe(windows_output, 22),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(windows_output, 2_200),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(windows_output, 10_022),
        RemoteListenerProbe::Missing
    );
    assert_eq!(
        parse_remote_listener_probe("__PORTMATE_UNSUPPORTED__\n", 22),
        RemoteListenerProbe::Unsupported
    );
    assert_eq!(
        parse_remote_listener_probe("unexpected output", 22),
        RemoteListenerProbe::Unsupported
    );
}

#[test]
fn remote_tunnel_probe_only_marks_successful_cross_platform_tools() {
    assert!(REMOTE_TUNNEL_PROBE_COMMAND
        .contains("cat /proc/net/tcp /proc/net/tcp6 2>/dev/null || true"));
    assert!(REMOTE_TUNNEL_PROBE_COMMAND
        .contains("command -v sockstat >/dev/null 2>&1 && probe=$(sockstat -46ln 2>/dev/null)"));
    assert!(REMOTE_TUNNEL_PROBE_COMMAND.contains(
        "command -v lsof >/dev/null 2>&1 && probe=$(lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null)"
    ));
    assert!(REMOTE_TUNNEL_PROBE_COMMAND
        .contains("command -v netstat >/dev/null 2>&1 && probe=$(netstat -ltn 2>/dev/null)"));
    assert!(REMOTE_WINDOWS_TUNNEL_PROBE_SCRIPT
        .contains("IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners()"));
    assert!(!REMOTE_WINDOWS_TUNNEL_PROBE_SCRIPT.contains("Get-NetTCPConnection"));
    assert!(windows_powershell_command(REMOTE_WINDOWS_TUNNEL_PROBE_SCRIPT)
        .starts_with("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand "));
    assert!(
        REMOTE_TUNNEL_PROBE_COMMAND.find("sockstat").unwrap()
            < REMOTE_TUNNEL_PROBE_COMMAND.find("netstat").unwrap()
    );
    assert!(
        REMOTE_TUNNEL_PROBE_COMMAND.find("lsof").unwrap()
            < REMOTE_TUNNEL_PROBE_COMMAND.find("netstat").unwrap()
    );
}

#[test]
fn remote_tunnel_health_recovery_preserves_non_health_errors() {
    let metrics = TunnelMetrics::default();
    metrics.record_error("remote forward health check failed: listener missing");
    assert!(metrics.clear_error_with_prefix(REMOTE_TUNNEL_HEALTH_ERROR_PREFIX));
    assert!(metrics
        .snapshot(TunnelSpec {
            id: "remote".to_string(),
            label: "remote".to_string(),
            mode: TunnelMode::Remote,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_022,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            route_rules: Vec::new(),
            enabled: true,
        })
        .last_error
        .is_none());

    metrics.record_error("remote tunnel target connect failed");
    assert!(!metrics.clear_error_with_prefix(REMOTE_TUNNEL_HEALTH_ERROR_PREFIX));
    assert_eq!(
        metrics
            .snapshot(TunnelSpec {
                id: "remote".to_string(),
                label: "remote".to_string(),
                mode: TunnelMode::Remote,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 10_022,
                target_host: "127.0.0.1".to_string(),
                target_port: 22,
                route_rules: Vec::new(),
                enabled: true,
            })
            .last_error
            .as_deref(),
        Some("remote tunnel target connect failed")
    );
}

#[test]
fn tunnel_health_rejects_a_superseding_generation_on_the_same_ssh_runtime() {
    let root = tempfile::tempdir().unwrap();
    let profile = test_ssh_profile();
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    let spec = TunnelSpec {
        id: "health-generation".to_string(),
        label: "health generation".to_string(),
        mode: TunnelMode::Remote,
        bind_host: "127.0.0.1".to_string(),
        bind_port: 10_022,
        target_host: "127.0.0.1".to_string(),
        target_port: 22,
        route_rules: Vec::new(),
        enabled: true,
    };
    let expected = TunnelRuntime {
        session_id: profile.id.clone(),
        ssh_runtime_id: "shared-ssh-runtime".to_string(),
        spec: spec.clone(),
        metrics: Arc::new(TunnelMetrics::default()),
        closed: Arc::new(AtomicBool::new(false)),
        listener_worker: TunnelListenerWorker::completed(),
    };
    state
        .tunnels
        .lock()
        .unwrap()
        .insert(spec.id.clone(), expected.clone());
    ensure_tunnel_runtime_current(&state, &spec.id, &expected).unwrap();

    state.tunnels.lock().unwrap().insert(
        spec.id.clone(),
        TunnelRuntime {
            session_id: profile.id,
            ssh_runtime_id: "shared-ssh-runtime".to_string(),
            spec: spec.clone(),
            metrics: Arc::new(TunnelMetrics::default()),
            closed: Arc::new(AtomicBool::new(false)),
            listener_worker: TunnelListenerWorker::completed(),
        },
    );

    let error = ensure_tunnel_runtime_current(&state, &spec.id, &expected).unwrap_err();
    assert!(error.contains("changed during health check"), "{error}");
}

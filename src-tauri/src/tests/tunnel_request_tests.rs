#[test]
fn tunnel_requests_are_normalized_and_validate_targets_early() {
    let local = normalize_tunnel_request(CreateTunnelRequest {
        session_id: " ssh-session-1 ".to_string(),
        mode: TunnelMode::Local,
        bind_host: " 127.0.0.1 ".to_string(),
        bind_port: 0,
        target_host: " device.internal ".to_string(),
        target_port: 22,
        route_rules: Vec::new(),
        label: Some("  ".to_string()),
    })
    .unwrap();
    assert_eq!(local.session_id, "ssh-session-1");
    assert_eq!(local.bind_host, "127.0.0.1");
    assert_eq!(local.target_host, "device.internal");
    assert!(local.label.is_none());

    let error = normalize_tunnel_request(CreateTunnelRequest {
        target_host: " ".to_string(),
        target_port: 0,
        ..local.clone()
    })
    .unwrap_err();
    assert!(error.contains("require a target host and port"));

    let dynamic = normalize_tunnel_request(CreateTunnelRequest {
        mode: TunnelMode::Dynamic,
        target_host: "ignored".to_string(),
        target_port: 443,
        ..local
    })
    .unwrap();
    assert!(dynamic.target_host.is_empty());
    assert_eq!(dynamic.target_port, 0);

    let dynamic_with_routes = normalize_tunnel_request(CreateTunnelRequest {
        route_rules: vec![
            portmate_core::TunnelRouteRule {
                host: " *.Example.COM. ".to_string(),
                port: Some(443),
            },
            portmate_core::TunnelRouteRule {
                host: "10.9.8.7/8".to_string(),
                port: None,
            },
        ],
        ..dynamic.clone()
    })
    .unwrap();
    assert_eq!(dynamic_with_routes.route_rules[0].host, "*.example.com");
    assert_eq!(dynamic_with_routes.route_rules[1].host, "10.0.0.0/8");

    let duplicate_routes = normalize_tunnel_request(CreateTunnelRequest {
        route_rules: vec![
            portmate_core::TunnelRouteRule {
                host: "Example.COM.".to_string(),
                port: Some(443),
            },
            portmate_core::TunnelRouteRule {
                host: "example.com".to_string(),
                port: Some(443),
            },
        ],
        ..dynamic.clone()
    })
    .unwrap_err();
    assert!(duplicate_routes.contains("duplicate rule"));

    let controlled_route = normalize_tunnel_request(CreateTunnelRequest {
        route_rules: vec![portmate_core::TunnelRouteRule {
            host: "\nexample.com".to_string(),
            port: None,
        }],
        ..dynamic.clone()
    })
    .unwrap_err();
    assert!(controlled_route.contains("route rule 1 host must not contain control characters"));

    let local_with_routes = normalize_tunnel_request(CreateTunnelRequest {
        mode: TunnelMode::Local,
        target_host: "device.internal".to_string(),
        target_port: 22,
        route_rules: dynamic_with_routes.route_rules.clone(),
        ..dynamic.clone()
    })
    .unwrap_err();
    assert!(local_with_routes.contains("only supported by dynamic mode"));

    let too_many_routes = normalize_tunnel_request(CreateTunnelRequest {
        route_rules: (0..=MAX_TUNNEL_ROUTE_RULES)
            .map(|index| portmate_core::TunnelRouteRule {
                host: format!("host-{index}.example"),
                port: None,
            })
            .collect(),
        ..dynamic.clone()
    })
    .unwrap_err();
    assert!(too_many_routes.contains("route rule count exceeds"));

    let dynamic_without_target: CreateTunnelRequest = serde_json::from_value(serde_json::json!({
        "sessionId": "ssh-session-1",
        "mode": "dynamic",
        "bindHost": "127.0.0.1",
        "bindPort": 0
    }))
    .unwrap();
    let dynamic_without_target = normalize_tunnel_request(dynamic_without_target).unwrap();
    assert!(dynamic_without_target.target_host.is_empty());
    assert_eq!(dynamic_without_target.target_port, 0);

    let oversized_host = normalize_tunnel_request(CreateTunnelRequest {
        bind_host: "x".repeat(MAX_TUNNEL_HOST_CHARACTERS + 1),
        ..dynamic.clone()
    })
    .unwrap_err();
    assert!(oversized_host.contains("bind host exceeds"));

    let invalid_label = normalize_tunnel_request(CreateTunnelRequest {
        label: Some(format!(
            "{}\nsecret",
            "x".repeat(MAX_TUNNEL_LABEL_CHARACTERS)
        )),
        ..dynamic.clone()
    })
    .unwrap_err();
    assert!(invalid_label.contains("label must not contain control characters"));

    let whitespace_host = normalize_tunnel_request(CreateTunnelRequest {
        bind_host: "bad host".to_string(),
        ..dynamic
    })
    .unwrap_err();
    assert!(whitespace_host.contains("bind host must not contain whitespace"));

    assert_eq!(
        tunnel_label(
            TunnelMode::Local,
            &"b".repeat(MAX_TUNNEL_HOST_CHARACTERS),
            65_535,
            &"t".repeat(MAX_TUNNEL_HOST_CHARACTERS),
            65_535,
        )
        .chars()
        .count(),
        MAX_TUNNEL_LABEL_CHARACTERS
    );

    assert_eq!(forwarded_tcpip_ports(65_535, 0), Some((65_535, 0)));
    assert_eq!(forwarded_tcpip_ports(65_536, 22), None);
    assert_eq!(forwarded_tcpip_ports(22, 65_536), None);
}

#[test]
fn tunnel_connection_slots_bound_and_release_concurrency() {
    let metrics = TunnelMetrics::default();
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let permit = try_acquire_tunnel_connection(&slots, &metrics).unwrap();
    assert!(try_acquire_tunnel_connection(&slots, &metrics).is_none());
    assert!(metrics
        .last_error
        .lock()
        .unwrap()
        .as_deref()
        .is_some_and(|error| {
            error.starts_with(TUNNEL_CONNECTION_LIMIT_ERROR_PREFIX)
                && error.contains(&MAX_TUNNEL_CONNECTIONS.to_string())
        }));

    drop(permit);
    assert!(try_acquire_tunnel_connection(&slots, &metrics).is_some());
    assert!(metrics.last_error.lock().unwrap().is_none());

    let runtimes = (0..MAX_ACTIVE_TUNNELS)
        .map(|index| {
            let id = format!("runtime-{index}");
            (
                id.clone(),
                TunnelRuntime {
                    session_id: "ssh-session".to_string(),
                    ssh_runtime_id: "ssh-runtime".to_string(),
                    spec: TunnelSpec {
                        id,
                        label: "Tunnel".to_string(),
                        mode: TunnelMode::Local,
                        bind_host: "127.0.0.1".to_string(),
                        bind_port: 10_022,
                        target_host: "device.internal".to_string(),
                        target_port: 22,
                        route_rules: Vec::new(),
                        enabled: true,
                    },
                    metrics: Arc::new(TunnelMetrics::default()),
                    closed: Arc::new(AtomicBool::new(false)),
                    listener_worker: TunnelListenerWorker::completed(),
                },
            )
        })
        .collect::<HashMap<_, _>>();
    assert!(ensure_tunnel_runtime_slot(&runtimes, "next")
        .unwrap_err()
        .contains(&MAX_ACTIVE_TUNNELS.to_string()));
    assert!(ensure_tunnel_runtime_slot(&runtimes, "runtime-0")
        .unwrap_err()
        .contains("already running"));
}

#[test]
fn profile_tunnel_bounds_reclaim_disabled_entries_without_overwriting_enabled_ones() {
    let root = std::env::temp_dir().join(format!("portmate-tunnel-bounds-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let mut profile = test_ssh_profile();
    let session_id = profile.id.clone();
    let tunnel = |id: String, enabled: bool| TunnelSpec {
        id: id.clone(),
        label: id,
        mode: TunnelMode::Local,
        bind_host: "127.0.0.1".to_string(),
        bind_port: 10_000,
        target_host: "device.internal".to_string(),
        target_port: 22,
        route_rules: Vec::new(),
        enabled,
    };
    let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
        panic!("expected SSH profile");
    };
    ssh.tunnels = (0..MAX_TUNNELS_PER_PROFILE - 1)
        .map(|index| tunnel(format!("enabled-{index}"), true))
        .chain(std::iter::once(tunnel("disabled-old".to_string(), false)))
        .collect();
    validate_profile_tunnels(&profile).unwrap();

    let mut legacy = profile.clone();
    let ConnectionConfig::Ssh(ssh) = &mut legacy.connection else {
        panic!("expected SSH profile");
    };
    ssh.tunnels.push(tunnel("invalid\nid".to_string(), true));
    let normalized = normalize_session_profile(legacy);
    let ConnectionConfig::Ssh(ssh) = normalized.connection else {
        panic!("expected SSH profile");
    };
    assert_eq!(ssh.tunnels.len(), MAX_TUNNELS_PER_PROFILE);
    assert!(!ssh.tunnels.iter().any(|tunnel| tunnel.id.contains('\n')));

    let mut duplicate = profile.clone();
    let ConnectionConfig::Ssh(ssh) = &mut duplicate.connection else {
        panic!("expected SSH profile");
    };
    ssh.tunnels[1].id = ssh.tunnels[0].id.clone();
    assert!(validate_profile_tunnels(&duplicate)
        .unwrap_err()
        .contains("duplicate id"));

    let state = test_app_state(profile, root.join("portmate-store.sqlite3"));
    ensure_tunnel_creation_capacity(&state, &session_id).unwrap();
    let replacement = tunnel("replacement".to_string(), true);
    persist_tunnel_to_profile_and_log(&state, &session_id, &replacement, None).unwrap();
    let saved = state.store.lock().unwrap().profile(&session_id).unwrap();
    let ConnectionConfig::Ssh(ssh) = saved.connection else {
        panic!("expected SSH profile");
    };
    assert_eq!(ssh.tunnels.len(), MAX_TUNNELS_PER_PROFILE);
    assert!(ssh.tunnels.iter().all(|tunnel| tunnel.enabled));
    assert!(ssh.tunnels.iter().any(|tunnel| tunnel.id == "replacement"));
    assert!(!ssh.tunnels.iter().any(|tunnel| tunnel.id == "disabled-old"));
    assert!(ensure_tunnel_creation_capacity(&state, &session_id)
        .unwrap_err()
        .contains("has reached"));
    assert!(persist_tunnel_to_profile_and_log(
        &state,
        &session_id,
        &tunnel("must-not-replace".to_string(), true),
        None,
    )
    .unwrap_err()
    .contains("has reached"));
    assert!(persist_tunnel_to_profile_and_log(
        &state,
        "deleted-session",
        &tunnel("orphan".to_string(), true),
        None,
    )
    .unwrap_err()
    .contains("unknown session"));

    let missing = tunnel("missing-stopped".to_string(), false);
    let mut store = state.store.lock().unwrap();
    mark_tunnel_stopped_in_store(&mut store, &session_id, &missing);
    let saved = store.profile(&session_id).unwrap();
    let ConnectionConfig::Ssh(ssh) = saved.connection else {
        panic!("expected SSH profile");
    };
    assert_eq!(ssh.tunnels.len(), MAX_TUNNELS_PER_PROFILE);
    assert!(!ssh.tunnels.iter().any(|tunnel| tunnel.id == missing.id));
    drop(store);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn tunnel_metrics_snapshot_tracks_connections_bytes_and_errors() {
    let metrics = TunnelMetrics::default();
    let spec = TunnelSpec {
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

    metrics.connection_opened();
    metrics.add_tcp_to_ssh_bytes(128);
    metrics.add_ssh_to_tcp_bytes(256);
    metrics.record_error("direct-tcpip open failed");
    let active = metrics.snapshot(spec.clone());
    assert_eq!(active.spec.id, spec.id);
    assert_eq!(active.active_connections, 1);
    assert_eq!(active.total_connections, 1);
    assert_eq!(active.tcp_to_ssh_bytes, 128);
    assert_eq!(active.ssh_to_tcp_bytes, 256);
    assert!(active.last_activity.is_some());
    assert_eq!(
        active.last_error.as_deref(),
        Some("direct-tcpip open failed")
    );

    metrics.clear_error();
    let recovered = metrics.snapshot(spec.clone());
    assert!(recovered.last_error.is_none());

    metrics.connection_closed();
    metrics.connection_closed();
    let closed = metrics.snapshot(spec.clone());
    assert_eq!(closed.active_connections, 0);
    assert_eq!(closed.total_connections, 1);

    let failed_metrics = Arc::new(TunnelMetrics::default());
    let failed_closed = Arc::new(AtomicBool::new(false));
    let tunnels = Arc::new(Mutex::new(HashMap::from([(
        spec.id.clone(),
        TunnelRuntime {
            session_id: "ssh-session-1".to_string(),
            ssh_runtime_id: "runtime-new".to_string(),
            spec: spec.clone(),
            metrics: Arc::clone(&failed_metrics),
            closed: Arc::clone(&failed_closed),
            listener_worker: TunnelListenerWorker::completed(),
        },
    )])));
    assert!(fail_tunnel_runtime_if_owned(
        &tunnels,
        &spec.id,
        "runtime-old",
        "stale listener failed"
    )
    .unwrap()
    .is_none());
    assert_eq!(tunnels.lock().unwrap().len(), 1);
    assert!(!failed_closed.load(Ordering::SeqCst));
    let failed = fail_tunnel_runtime_if_owned(&tunnels, &spec.id, "runtime-new", "listener failed")
        .unwrap()
        .unwrap();
    assert!(tunnels.lock().unwrap().is_empty());
    assert!(failed_closed.load(Ordering::SeqCst));
    assert_eq!(
        failed.metrics.snapshot(spec).last_error.as_deref(),
        Some("listener failed")
    );
}

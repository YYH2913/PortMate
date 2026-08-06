use super::*;

#[test]
fn invalid_mcp_identifiers_are_bounded_before_audit() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-mcp-invalid-identifiers-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let invalid_client = format!("{}\nsecret-client", "x".repeat(1024));
        let error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: invalid_client.clone(),
                trusted_write: true,
                command: "open_session".to_string(),
                args: serde_json::json!({ "sessionId": "bad\nsession" }),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("session ID"));
        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].actor, "<invalid-client-id>");
        assert_eq!(audit[0].session_id, None);
        let encoded = serde_json::to_string(&audit).unwrap();
        assert!(!encoded.contains("secret-client"));
        assert!(!encoded.contains("bad\\nsession"));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn denied_and_invalid_mcp_writes_are_audited_without_arguments() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-denied-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();

        let denied = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "readonly-client".to_string(),
                trusted_write: false,
                command: "open_session".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "password": "denied-password-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(denied.contains("does not permit"));

        let invalid = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "trusted-client".to_string(),
                trusted_write: true,
                command: "run_command".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "command": 42,
                    "password": "invalid-password-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(invalid.contains("missing string argument `command`"));

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].actor, "readonly-client");
        assert_eq!(audit[0].action, "open_session");
        assert_eq!(audit[0].decision, "denied");
        assert_eq!(audit[0].session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(
            audit[0].details.get("scope").map(String::as_str),
            Some("manage-sessions")
        );
        assert_eq!(audit[1].actor, "trusted-client");
        assert_eq!(audit[1].action, "run_command");
        assert_eq!(audit[1].decision, "invalid");
        assert_eq!(
            audit[1].details.get("scope").map(String::as_str),
            Some("write-input")
        );
        let encoded = serde_json::to_string(&audit).unwrap();
        assert!(!encoded.contains("denied-password-secret"));
        assert!(!encoded.contains("invalid-password-secret"));
        assert!(!encoded.contains("\"command\":42"));

        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn trusted_mcp_input_uses_client_actor_and_exact_tool_audit() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 8];
            socket.read_exact(&mut received).await.unwrap();
            received
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-input-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let stream = TcpStream::connect(address).await.unwrap();
        let (_reader, writer) = stream.into_split();
        let (tap, _) = broadcast::channel(8);
        state.tcp.lock().unwrap().insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: Uuid::new_v4().to_string(),
                writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
                tap,
                closed: Arc::new(AtomicBool::new(false)),
                telnet: None,
            },
        );

        let key_event: SessionEvent = serde_json::from_value(
            handle_ipc_request(
                state.clone(),
                IpcRequest {
                    token: "authenticated-token".to_string(),
                    client_id: "mcp-e2e-client".to_string(),
                    trusted_write: true,
                    command: "send_key".to_string(),
                    args: serde_json::json!({
                        "sessionId": profile.id,
                        "key": "Enter"
                    }),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();
        let command_event: SessionEvent = serde_json::from_value(
            handle_ipc_request(
                state.clone(),
                IpcRequest {
                    token: "authenticated-token".to_string(),
                    client_id: "mcp-e2e-client".to_string(),
                    trusted_write: true,
                    command: "run_command".to_string(),
                    args: serde_json::json!({
                        "sessionId": profile.id,
                        "command": "status"
                    }),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();

        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("TCP server timed out")
            .expect("TCP server failed");
        assert_eq!(&received, b"\rstatus\n");
        for event in [&key_event, &command_event] {
            assert_eq!(
                event.annotations.get("actor").map(String::as_str),
                Some("mcp-e2e-client")
            );
        }
        assert!(!key_event.annotations.contains_key("commandId"));
        assert!(command_event.annotations.contains_key("commandId"));
        assert_eq!(
            command_event
                .annotations
                .get("commandState")
                .map(String::as_str),
            Some("started")
        );

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(
            audit.len(),
            2,
            "MCP input must not add implicit send_text audits"
        );
        assert_eq!(audit[0].action, "send_key");
        assert_eq!(audit[1].action, "run_command");
        assert!(audit.iter().all(|record| record.actor == "mcp-e2e-client"));
        assert!(audit.iter().all(|record| record.decision == "succeeded"));
        assert!(audit.iter().all(|record| {
            record.details.get("trustedBootstrap").map(String::as_str) == Some("true")
        }));
        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn failed_mcp_write_finalizes_audit_without_secret_arguments() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-failed-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        let error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "failed-client".to_string(),
                trusted_write: true,
                command: "run_command".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "command": "password=failed-command-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(!error.is_empty());
        assert!(!error.contains("failed-command-secret"));

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].actor, "failed-client");
        assert_eq!(audit[0].action, "run_command");
        assert_eq!(audit[0].decision, "failed");
        assert!(!serde_json::to_string(&audit)
            .unwrap()
            .contains("failed-command-secret"));
        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn applied_mcp_audit_keeps_final_truth_when_persistence_fails() {
    let mut store = SessionStore::default();
    store.record_audit(AuditRecord {
        id: "audit-applied".to_string(),
        ts: Utc::now(),
        actor: "mcp:test-client".to_string(),
        action: "send_text".to_string(),
        session_id: Some("session-1".to_string()),
        decision: "authorized".to_string(),
        details: BTreeMap::new(),
    });

    let error = finish_applied_mcp_write_audit_with(
        &mut store,
        "audit-applied",
        "succeeded",
        Some("approved"),
        |_| Err("disk full".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();

    assert_eq!(error, "disk full");
    let audit = store
        .audit
        .iter()
        .find(|record| record.id == "audit-applied")
        .unwrap();
    assert_eq!(audit.decision, "succeeded");
    assert_eq!(
        audit.details.get("approval").map(String::as_str),
        Some("approved")
    );
    assert_eq!(
        audit
            .details
            .get("finalizationPersistence")
            .map(String::as_str),
        Some("degraded")
    );
}

#[test]
fn applied_mcp_audit_accepts_a_verified_post_commit_error() {
    let mut store = SessionStore::default();
    store.record_audit(AuditRecord {
        id: "audit-verified".to_string(),
        ts: Utc::now(),
        actor: "mcp:test-client".to_string(),
        action: "run_command".to_string(),
        session_id: Some("session-1".to_string()),
        decision: "authorized".to_string(),
        details: BTreeMap::new(),
    });

    finish_applied_mcp_write_audit_with(
        &mut store,
        "audit-verified",
        "failed",
        None,
        |_| Err("post-commit version read failed".to_string()),
        |_| Ok(true),
    )
    .unwrap();

    let audit = store
        .audit
        .iter()
        .find(|record| record.id == "audit-verified")
        .unwrap();
    assert_eq!(audit.decision, "failed");
    assert!(!audit.details.contains_key("finalizationPersistence"));
}

#[test]
fn log_query_limit_matches_mcp_schema_bounds() {
    assert_eq!(bounded_log_query_limit(None), 100);
    assert_eq!(bounded_log_query_limit(Some(0)), 1);
    assert_eq!(bounded_log_query_limit(Some(600)), 600);
    assert_eq!(bounded_log_query_limit(Some(u64::MAX)), 1000);
}

#[test]
fn mcp_http_config_uses_bridge_token_ref_and_loopback_endpoint() {
    let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
    let store_path = Path::new("/home/operator/PortMate Data/portmate-store.sqlite3");
    let config =
        build_mcp_http_config_for_request(true, executable, store_path, McpHttpSettings::default())
            .unwrap();
    assert_eq!(config.token_ref, MCP_HTTP_TOKEN_REF);
    assert_eq!(config.endpoint, "http://127.0.0.1:8787/mcp");
    assert_eq!(config.settings, McpHttpSettings::default());
    assert!(!config.remote_access);
    assert!(config.token_available);
    assert!(config.start_command.contains("PORTMATE_MCP_HTTP=1"));
    assert_eq!(config.executable, executable.to_string_lossy());
    assert_eq!(config.store_path, store_path.to_string_lossy());
    assert!(config
        .start_command
        .contains("/opt/PortMate/bin/portmate-mcp"));
    assert!(config
        .start_command
        .contains("'/home/operator/PortMate Data/portmate-store.sqlite3'"));
    assert!(!config.start_command.contains("cargo run"));
    assert!(!config.start_command.contains(MCP_HTTP_TOKEN_REF));
    assert!(!config.start_command.contains("keychain:"));
    assert!(!config.start_command.contains("PORTMATE_MCP_HTTP_TOKEN"));
    assert!(!config.start_command.contains("example-token-body"));
    #[cfg(not(windows))]
    assert_eq!(
        config.start_command,
        "PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3' PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787' PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787,http://localhost:8787' PORTMATE_MCP_CLIENT_ID='portmate-local' PORTMATE_MCP_HTTP_ALLOW_REMOTE=0 PORTMATE_MCP_TRUSTED=0 '/opt/PortMate/bin/portmate-mcp' --http"
    );
    #[cfg(windows)]
    assert_eq!(
        config.start_command,
        "$env:PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3'; $env:PORTMATE_MCP_HTTP='1'; $env:PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787'; $env:PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787,http://localhost:8787'; $env:PORTMATE_MCP_CLIENT_ID='portmate-local'; $env:PORTMATE_MCP_HTTP_ALLOW_REMOTE='0'; $env:PORTMATE_MCP_TRUSTED='0'; & '/opt/PortMate/bin/portmate-mcp' --http"
    );
}

#[test]
fn mcp_http_config_supports_explicit_remote_listeners_and_validates_origins() {
    let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
    let store_path = Path::new("/tmp/portmate-store.sqlite3");
    let remote = McpHttpSettings {
        listen_host: "0.0.0.0".to_string(),
        port: 9888,
        allowed_origins: vec!["https://console.example.test".to_string()],
        client_id: "automation-client".to_string(),
        trusted: true,
        allow_remote: true,
    };
    let config = build_mcp_http_config_for_request(false, executable, store_path, remote).unwrap();
    assert_eq!(config.endpoint, "http://0.0.0.0:9888/mcp");
    assert!(config.remote_access);
    assert!(config
        .start_command
        .contains("PORTMATE_MCP_HTTP_ALLOW_REMOTE"));
    assert!(config.start_command.contains("PORTMATE_MCP_TRUSTED"));
    assert!(config.start_command.contains("automation-client"));
    assert!(config.start_command.contains("console.example.test"));

    let mut denied = config.settings.clone();
    denied.allow_remote = false;
    assert!(normalize_mcp_http_settings(denied)
        .unwrap_err()
        .contains("explicit remote access"));

    let invalid_origin = McpHttpSettings {
        allowed_origins: vec!["https://console.example.test/path".to_string()],
        ..McpHttpSettings::default()
    };
    assert!(normalize_mcp_http_settings(invalid_origin)
        .unwrap_err()
        .contains("scheme and authority"));

    let invalid_scheme = McpHttpSettings {
        allowed_origins: vec!["ftp://console.example.test".to_string()],
        ..McpHttpSettings::default()
    };
    assert!(normalize_mcp_http_settings(invalid_scheme)
        .unwrap_err()
        .contains("HTTP(S)"));

    let loopback = McpHttpSettings {
        allow_remote: true,
        ..McpHttpSettings::default()
    };
    assert!(
        !normalize_mcp_http_settings(loopback)
            .unwrap()
            .0
            .allow_remote
    );

    let ipv6_loopback = McpHttpSettings {
        listen_host: "[::1]".to_string(),
        port: 9889,
        allowed_origins: Vec::new(),
        ..McpHttpSettings::default()
    };
    let ipv6_config =
        build_mcp_http_config_for_request(false, executable, store_path, ipv6_loopback).unwrap();
    assert_eq!(ipv6_config.settings.listen_host, "::1");
    assert_eq!(
        ipv6_config.settings.allowed_origins,
        vec!["http://[::1]:9889"]
    );
    assert_eq!(ipv6_config.endpoint, "http://[::1]:9889/mcp");
    assert!(!ipv6_config.remote_access);
    assert!(ipv6_config
        .start_command
        .contains("PORTMATE_MCP_HTTP_ADDR='[::1]:9889'"));

    let ipv6_wildcard = McpHttpSettings {
        listen_host: "::".to_string(),
        port: 9890,
        allowed_origins: Vec::new(),
        allow_remote: true,
        ..McpHttpSettings::default()
    };
    let ipv6_wildcard_config =
        build_mcp_http_config_for_request(false, executable, store_path, ipv6_wildcard).unwrap();
    assert_eq!(ipv6_wildcard_config.settings.listen_host, "::");
    assert_eq!(
        ipv6_wildcard_config.settings.allowed_origins,
        vec!["http://127.0.0.1:9890", "http://localhost:9890"]
    );
    assert_eq!(ipv6_wildcard_config.endpoint, "http://[::]:9890/mcp");
    assert!(ipv6_wildcard_config.remote_access);
    assert!(ipv6_wildcard_config
        .start_command
        .contains("PORTMATE_MCP_HTTP_ADDR='[::]:9890'"));
}

#[test]
fn managed_mcp_http_command_uses_saved_settings_without_exposing_the_token() {
    let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
    let store_path = Path::new("/tmp/PortMate Data/portmate-store.sqlite3");
    let config = build_mcp_http_config_for_request(
        true,
        executable,
        store_path,
        McpHttpSettings {
            listen_host: "0.0.0.0".to_string(),
            port: 9911,
            allowed_origins: vec!["https://console.example.test".to_string()],
            client_id: "managed-client".to_string(),
            trusted: true,
            allow_remote: true,
        },
    )
    .unwrap();
    let command = mcp_http_process_command(executable, store_path, &config);
    let args = command
        .get_args()
        .map(|value| value.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let env = command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_string_lossy().to_string(),
                value.map(|value| value.to_string_lossy().to_string()),
            )
        })
        .collect::<HashMap<_, _>>();

    assert_eq!(command.get_program(), executable);
    assert_eq!(args, vec!["--http"]);
    assert_eq!(
        env["PORTMATE_STORE_PATH"].as_deref(),
        Some("/tmp/PortMate Data/portmate-store.sqlite3")
    );
    assert_eq!(
        env["PORTMATE_MCP_HTTP_ADDR"].as_deref(),
        Some("0.0.0.0:9911")
    );
    assert_eq!(
        env["PORTMATE_MCP_HTTP_ORIGINS"].as_deref(),
        Some("https://console.example.test")
    );
    assert_eq!(
        env["PORTMATE_MCP_CLIENT_ID"].as_deref(),
        Some("managed-client")
    );
    assert_eq!(env["PORTMATE_MCP_HTTP_ALLOW_REMOTE"].as_deref(), Some("1"));
    assert_eq!(env["PORTMATE_MCP_TRUSTED"].as_deref(), Some("1"));
    assert_eq!(
        env["PORTMATE_MCP_PARENT_PID"].as_deref(),
        Some(std::process::id().to_string().as_str())
    );
    assert_eq!(env.get("PORTMATE_MCP_HTTP_TOKEN"), Some(&None));
}

#[test]
fn managed_mcp_http_ready_probe_rejects_an_unrelated_listener() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 512];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });

    assert!(!probe_mcp_http_ready(address));
    server.join().unwrap();
}

#[test]
fn managed_mcp_http_runtime_reports_ready_stops_and_retains_bounded_failures() {
    let _guard = shared_runtime_test_guard();
    let root = std::env::temp_dir().join(format!("portmate-mcp-runtime-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);

    let mut fixture = managed_mcp_http_fixture_command(address, false);
    install_test_mcp_http_process(
        &state,
        &mut fixture,
        format!("http://{address}/mcp"),
        address,
    )
    .unwrap();
    let running = wait_for_managed_mcp_http_phase(&state, McpHttpRuntimePhase::Running);
    assert_eq!(
        running.endpoint.as_deref(),
        Some(format!("http://{address}/mcp").as_str())
    );
    assert!(running.pid.is_some());
    assert!(running.started_at.is_some());
    let gate_error = lock_stopped_mcp_http_runtime(&state, "保存配置")
        .err()
        .unwrap();
    assert!(gate_error.contains("请先停止"));
    assert_eq!(
        stop_mcp_http_runtime_inner(&state).unwrap().phase,
        McpHttpRuntimePhase::Stopped
    );
    assert_eq!(
        mcp_http_runtime_status_inner(&state).unwrap().phase,
        McpHttpRuntimePhase::Stopped
    );

    let failed_address = std::net::SocketAddr::from(([127, 0, 0, 1], address.port()));
    let mut failed_fixture = managed_mcp_http_fixture_command(failed_address, true);
    install_test_mcp_http_process(
        &state,
        &mut failed_fixture,
        format!("http://{failed_address}/mcp"),
        failed_address,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let failure = loop {
        let status = mcp_http_runtime_status_inner(&state).unwrap();
        if status.phase == McpHttpRuntimePhase::Failed
            && status
                .message
                .as_deref()
                .is_some_and(|message| message.contains("fixture startup rejected"))
        {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "managed MCP HTTP failure was not reported: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(failure.pid, None);
    assert!(failure.message.unwrap().chars().count() <= 1_024);
    stop_mcp_http_runtime_inner(&state).unwrap();
    let _ = fs::remove_dir_all(root);
}

fn managed_mcp_http_fixture_command(address: std::net::SocketAddr, fail: bool) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "tests::mcp_tests::managed_mcp_http_fixture",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("PORTMATE_TEST_MCP_HTTP_ADDRESS", address.to_string())
        .env("PORTMATE_TEST_MCP_HTTP_FAIL", if fail { "1" } else { "0" });
    command
}

fn wait_for_managed_mcp_http_phase(
    state: &AppState,
    phase: McpHttpRuntimePhase,
) -> McpHttpRuntimeStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = mcp_http_runtime_status_inner(state).unwrap();
        if status.phase == phase {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "managed MCP HTTP phase did not become {phase:?}: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore]
fn managed_mcp_http_fixture() {
    let Some(address) = std::env::var("PORTMATE_TEST_MCP_HTTP_ADDRESS")
        .ok()
        .and_then(|value| value.parse::<std::net::SocketAddr>().ok())
    else {
        return;
    };
    if std::env::var("PORTMATE_TEST_MCP_HTTP_FAIL").as_deref() == Ok("1") {
        eprintln!("fixture startup rejected");
        return;
    }
    let listener = std::net::TcpListener::bind(address).unwrap();
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let mut request = [0_u8; 512];
                if stream.read(&mut request).is_ok() {
                    let _ = stream.write_all(
                        b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nMCP-Protocol-Version: 2025-06-18\r\nConnection: close\r\n\r\n",
                    );
                }
            }
            Err(_) => break,
        }
    }
}

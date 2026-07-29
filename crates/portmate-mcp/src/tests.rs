use super::http_request::read_http_request_with_timeout;
use super::keyring_store::{ensure_keyring_store_with, initialize_persistent_native_keyring_with};
use super::store_loader::{
    ensure_store_schema, load_store_from_path, prepare_loaded_store, STORE_KEY,
};
use super::*;
use rusqlite::{params, Connection as SqliteConnection};
use std::collections::HashMap;
use std::sync::Mutex;

#[test]
fn keyring_initialization_is_persistent_only_and_retries_transient_failures() {
    let initialized = Mutex::new(false);
    let attempts = std::cell::Cell::new(0_u32);
    let first = ensure_keyring_store_with(&initialized, || {
        attempts.set(attempts.get() + 1);
        Err(anyhow!("secret service offline"))
    });
    assert_eq!(first.unwrap_err().to_string(), "secret service offline");
    assert!(!*initialized.lock().unwrap());

    ensure_keyring_store_with(&initialized, || {
        attempts.set(attempts.get() + 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(attempts.get(), 2);
    ensure_keyring_store_with(&initialized, || {
        panic!("successful initialization must be cached")
    })
    .unwrap();

    let selectors = std::cell::RefCell::new(Vec::new());
    let error = initialize_persistent_native_keyring_with(|not_keyutils| {
        selectors.borrow_mut().push(not_keyutils);
        Err(anyhow!("persistent store unavailable"))
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "persistent store unavailable");
    assert_eq!(selectors.into_inner(), vec![true]);
}

fn test_http_config() -> HttpConfig {
    HttpConfig {
        addr: "127.0.0.1:8787".parse().unwrap(),
        token: "secret-token".to_string(),
        allowed_origins: vec!["http://127.0.0.1:8787".to_string()],
    }
}

fn test_snapshot_store(name: &str) -> SessionStore {
    let mut store = SessionStore::default();
    store.upsert_profile(portmate_core::SessionProfile {
        id: "refresh-session".to_string(),
        name: name.to_string(),
        kind: portmate_core::SessionKind::Shell,
        group: "tests".to_string(),
        tags: Vec::new(),
        connection: portmate_core::ConnectionConfig::Shell(portmate_core::ShellConnection {
            program: "/bin/sh".to_string(),
            args: Vec::new(),
            cwd: None,
        }),
        terminal: portmate_core::TerminalSettings::default(),
        logging: portmate_core::LoggingSettings::default(),
        triggers: Vec::new(),
        transfer: portmate_core::TransferSettings::default(),
    });
    store
}

#[test]
fn standalone_store_loading_rejects_oversized_profile_collections() {
    let mut store = test_snapshot_store("profile bound");
    let profile = store.profiles[0].clone();
    store.profiles = vec![profile; portmate_core::MAX_SESSION_PROFILES + 1];

    assert!(prepare_loaded_store(store).is_none());
}

#[test]
fn standalone_sqlite_store_loading_never_creates_or_migrates_a_store() {
    let root = std::env::temp_dir().join(format!("portmate-mcp-read-only-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let missing_path = root.join("missing.sqlite3");
    assert!(load_store_from_path(&missing_path).is_none());
    assert!(!missing_path.exists());

    let empty_path = root.join("empty.sqlite3");
    drop(SqliteConnection::open(&empty_path).unwrap());
    assert!(load_store_from_path(&empty_path).is_none());
    let empty_connection = SqliteConnection::open(&empty_path).unwrap();
    let has_kv_table: bool = empty_connection
        .query_row(
            "select exists(select 1 from sqlite_master where type = 'table' and name = 'kv')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!has_kv_table);
    drop(empty_connection);

    let snapshot_path = root.join("snapshot.sqlite3");
    let connection = SqliteConnection::open(&snapshot_path).unwrap();
    ensure_store_schema(&connection).unwrap();
    connection
        .execute(
            "insert into kv (key, value, updated_at) values (?1, ?2, '2026-07-23T00:00:00Z')",
            params![
                STORE_KEY,
                serde_json::to_string(&test_snapshot_store("read-only snapshot")).unwrap(),
            ],
        )
        .unwrap();
    drop(connection);

    let loaded = load_store_from_path(&snapshot_path).unwrap();
    assert_eq!(loaded.profiles[0].name, "read-only snapshot");

    let _ = fs::remove_dir_all(root);
}

fn sensitive_snapshot_store() -> SessionStore {
    let session_id = "refresh-session";
    let mut store = SessionStore::default();
    store.upsert_profile(portmate_core::SessionProfile {
        id: session_id.to_string(),
        name: "sensitive snapshot".to_string(),
        kind: portmate_core::SessionKind::Ssh,
        group: "tests".to_string(),
        tags: Vec::new(),
        connection: portmate_core::ConnectionConfig::Ssh(portmate_core::SshConnection {
            endpoint: portmate_core::HostEndpoint {
                host: "diagnostic.example".to_string(),
                port: 22,
            },
            username: "operator".to_string(),
            reconnect: true,
            reconnect_delay_ms: 1_000,
            keepalive_enabled: true,
            keepalive_interval_seconds: 30,
            keepalive_max_missed: 3,
            tcp_keepalive_enabled: None,
            proxy: portmate_core::ProxyConfig {
                password_secret_ref: Some("keyring:proxy-credential-ref".to_string()),
                ..Default::default()
            },
            password_secret_ref: Some("keyring:target-credential-ref".to_string()),
            passphrase_secret_ref: Some("stronghold:target-passphrase-ref".to_string()),
            host_key_policy: portmate_core::HostKeyPolicy::profile_alias(session_id),
            trusted_host_keys: Vec::new(),
            identity_policy: portmate_core::IdentityPolicy::default(),
            identity_refs: vec![portmate_core::IdentityRef {
                id: "identity-diagnostic-id".to_string(),
                label: "diagnostic identity".to_string(),
                source: portmate_core::IdentitySource::ProfileVault,
                fingerprint_sha256: Some("SHA256:diagnostic-fingerprint".to_string()),
                path: Some("/home/operator/.ssh/private-key".to_string()),
                secret_ref: Some("stronghold:identity-secret-ref".to_string()),
            }],
            agent_policy: portmate_core::AgentPolicy::default(),
            jumps: vec![portmate_core::JumpHop {
                host: "jump.example".to_string(),
                port: 22,
                username: "jump-operator".to_string(),
                password_secret_ref: Some("keyring:jump-credential-ref".to_string()),
                passphrase_secret_ref: Some("stronghold:jump-passphrase-ref".to_string()),
                identity_ref: Some("identity-diagnostic-id".to_string()),
                host_key_policy: None,
            }],
            tunnels: Vec::new(),
        }),
        terminal: portmate_core::TerminalSettings::default(),
        logging: portmate_core::LoggingSettings {
            path_template: "/home/operator/private-logs/{session}.raw".to_string(),
            ..Default::default()
        },
        triggers: vec![portmate_core::TriggerSpec {
            id: "sensitive-trigger".to_string(),
            label: "password=trigger-label-secret".to_string(),
            matcher: portmate_core::TriggerMatcher::Contains {
                text: "token=trigger-match-secret".to_string(),
                case_sensitive: false,
            },
            actions: vec![portmate_core::TriggerAction::LocalCommand {
                command: "/home/operator/private-scripts/deploy".to_string(),
            }],
            enabled: true,
        }],
        transfer: portmate_core::TransferSettings {
            default_local_dir: Some("/home/operator/private-downloads".to_string()),
            ..Default::default()
        },
    });
    store.runtimes[0].cwd = Some("/home/operator/runtime-cwd".to_string());
    store.runtimes[0].last_disconnect_reason = Some("password=disconnect-secret".to_string());
    let diagnostic_ts = store.runtimes[0].last_activity;
    store
        .record_event(
            session_id,
            portmate_core::EventDirection::Inbound,
            portmate_core::EventStream::Stdout,
            Some("password=event-secret".to_string()),
            Some("v2:/home/operator/private-logs/raw:0:12:digest".to_string()),
            std::collections::BTreeMap::from([(
                "diagnostic".to_string(),
                "token=annotation-secret".to_string(),
            )]),
        )
        .unwrap();
    store.record_timeline_mark(portmate_core::TimelineMark {
        id: "timeline-diagnostic-id".to_string(),
        session_id: session_id.to_string(),
        ts: diagnostic_ts,
        label: "password=timeline-secret".to_string(),
        details: Some("token=timeline-details-secret".to_string()),
    });
    store.record_sysmon_snapshot(portmate_core::SysmonSnapshot {
        session_id: session_id.to_string(),
        ts: diagnostic_ts,
        uptime_seconds: 123,
        cpu_percent: 12.5,
        memory_percent: 34.5,
        rx_kbps: 56.5,
        tx_kbps: 78.5,
        load_average: [0.5, 1.0, 1.5],
        memory_total_bytes: 1024,
        memory_available_bytes: 512,
        processes: vec![portmate_core::SysmonProcess {
            pid: 4242,
            name: "password=sysmon-process-secret".to_string(),
            cpu_percent: 9.5,
            memory_percent: 8.5,
            rss_bytes: 256,
        }],
        disks: vec![portmate_core::SysmonDisk {
            filesystem: "/dev/mapper/private-filesystem".to_string(),
            mount_point: "/srv/private-mount".to_string(),
            total_bytes: 4096,
            available_bytes: 2048,
            used_percent: 50.0,
        }],
        network_interfaces: vec![portmate_core::SysmonNetworkInterface {
            name: "customer-private-interface".to_string(),
            addresses: vec!["10.0.0.25/24".to_string()],
            rx_bytes: 100,
            tx_bytes: 200,
            rx_kbps: 3.5,
            tx_kbps: 4.5,
        }],
    });
    store.record_transfer(portmate_core::TransferTask {
        id: "transfer-diagnostic-id".to_string(),
        session_id: session_id.to_string(),
        protocol: portmate_core::TransferProtocol::Sftp,
        source: "/home/operator/source-secret.txt".to_string(),
        destination: "/srv/private/destination-secret.txt".to_string(),
        bytes_total: 12,
        bytes_done: 12,
        status: portmate_core::TransferStatus::Completed,
        message: Some("token=transfer-message-secret".to_string()),
        started_at: None,
        finished_at: None,
        average_bytes_per_second: Some(6.0),
    });
    store
}

fn list_sessions_text(server: &mut PortMateMcp) -> String {
    let response = handle_json_rpc_value(
        server,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "list_sessions", "arguments": {} }
        }),
    )
    .unwrap()
    .unwrap();
    response["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn explicit_read_grants_filter_sessions_resources_and_global_logs() {
    let mut store = test_snapshot_store("visible snapshot");
    let mut hidden = store.profiles[0].clone();
    hidden.id = "hidden-session".to_string();
    hidden.name = "hidden snapshot".to_string();
    store.upsert_profile(hidden);
    store
        .record_stream_event(
            "refresh-session",
            portmate_core::EventDirection::Inbound,
            portmate_core::EventStream::Stdout,
            "shared-query visible-marker",
        )
        .unwrap();
    store
        .record_stream_event(
            "hidden-session",
            portmate_core::EventDirection::Inbound,
            portmate_core::EventStream::Stdout,
            "shared-query hidden-marker",
        )
        .unwrap();
    store.grants.push(portmate_core::McpGrant {
        client_id: "scoped-reader".to_string(),
        name: "Scoped reader".to_string(),
        scopes: vec![McpScope::ReadSessions, McpScope::ReadLogs],
        allowed_sessions: vec!["refresh-session".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    let mut server = PortMateMcp {
        store,
        store_path: None,
        ipc: None,
        client_id: "scoped-reader".to_string(),
        allow_write: false,
    };

    let sessions = list_sessions_text(&mut server);
    assert!(sessions.contains("visible snapshot"));
    assert!(!sessions.contains("hidden snapshot"));
    let resources = server.resources_list_result().to_string();
    assert!(resources.contains("refresh-session"));
    assert!(!resources.contains("hidden-session"));
    let sse_state = server.sse_state_payload(MCP_PROTOCOL_VERSION).to_string();
    assert!(sse_state.contains("visible snapshot"));
    assert!(!sse_state.contains("hidden snapshot"));

    let search = server
        .tool_call(&json!({
            "name": "search_logs",
            "arguments": { "query": "shared-query" }
        }))
        .unwrap();
    let search = search["content"][0]["text"].as_str().unwrap();
    assert!(search.contains("visible-marker"));
    assert!(!search.contains("hidden-marker"));
    assert!(server
        .tool_call(&json!({
            "name": "read_screen",
            "arguments": { "sessionId": "hidden-session" }
        }))
        .unwrap_err()
        .to_string()
        .contains("does not permit"));

    server.store.grants[0].scopes = vec![McpScope::ReadSessions];
    assert!(server
        .tool_call(&json!({
            "name": "search_logs",
            "arguments": { "query": "shared-query" }
        }))
        .unwrap_err()
        .to_string()
        .contains("ReadLogs"));
    server.store.grants[0].scopes.clear();
    assert!(server
        .tool_call(&json!({ "name": "list_sessions", "arguments": {} }))
        .unwrap_err()
        .to_string()
        .contains("ReadSessions"));
}

#[test]
fn orphaned_snapshot_state_is_not_readable_without_desktop_ipc() {
    let mut store = test_snapshot_store("visible snapshot");
    let event = store
        .record_stream_event(
            "refresh-session",
            portmate_core::EventDirection::Inbound,
            portmate_core::EventStream::Stdout,
            "visible snapshot marker",
        )
        .unwrap();
    let mut orphaned_event = event;
    orphaned_event.id = "orphaned-event".to_string();
    orphaned_event.session_id = "removed-session".to_string();
    orphaned_event.pane_id = "removed-session:main".to_string();
    orphaned_event.text = Some("orphaned snapshot marker".to_string());
    store.events.push(orphaned_event);

    let mut orphaned_transfer = sensitive_snapshot_store().transfers.remove(0);
    orphaned_transfer.id = "orphaned-transfer".to_string();
    orphaned_transfer.session_id = "removed-session".to_string();
    store.record_transfer(orphaned_transfer);

    let mut server = PortMateMcp {
        store,
        store_path: None,
        ipc: None,
        client_id: "fallback-reader".to_string(),
        allow_write: false,
    };

    let search = server
        .tool_call(&json!({
            "name": "search_logs",
            "arguments": { "query": "snapshot marker" }
        }))
        .unwrap();
    let search = search["content"][0]["text"].as_str().unwrap();
    assert!(search.contains("visible snapshot marker"));
    assert!(!search.contains("orphaned snapshot marker"));
    assert!(!server
        .resources_list_result()
        .to_string()
        .contains("orphaned-transfer"));

    for uri in [
        "portmate://sessions/removed-session/log",
        "portmate://transfers/orphaned-transfer",
    ] {
        assert!(server
            .resource_read(&json!({ "uri": uri }))
            .unwrap_err()
            .to_string()
            .contains("unknown or unavailable session"));
    }
    assert!(server
        .tool_call(&json!({
            "name": "tail_log",
            "arguments": { "sessionId": "removed-session" }
        }))
        .unwrap_err()
        .to_string()
        .contains("unknown or unavailable session"));
}

#[test]
fn mcp_read_surfaces_redact_sensitive_metadata_without_mutating_the_store() {
    let store = sensitive_snapshot_store();
    let raw_store = serde_json::to_string(&store).unwrap();
    let mut server = PortMateMcp {
        store,
        store_path: None,
        ipc: None,
        client_id: "redaction-reader".to_string(),
        allow_write: false,
    };

    let resource_text = |server: &PortMateMcp, uri: &str| {
        server.resource_read(&json!({ "uri": uri })).unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let tool_text = |server: &mut PortMateMcp, name: &str, arguments: Value| {
        server
            .tool_call(&json!({ "name": name, "arguments": arguments }))
            .unwrap()["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let surfaces = vec![
        list_sessions_text(&mut server),
        resource_text(&server, "portmate://sessions"),
        resource_text(&server, "portmate://sessions/refresh-session/state"),
        server.sse_state_payload(MCP_PROTOCOL_VERSION).to_string(),
        server
            .prompt_get(&json!({
                "name": "diagnose_session",
                "arguments": { "sessionId": "refresh-session" }
            }))
            .unwrap()
            .to_string(),
        resource_text(&server, "portmate://sessions/refresh-session/log"),
        tool_text(
            &mut server,
            "tail_log",
            json!({ "sessionId": "refresh-session" }),
        ),
        tool_text(
            &mut server,
            "search_logs",
            json!({ "query": "password", "sessionId": "refresh-session" }),
        ),
        resource_text(&server, "portmate://sessions/refresh-session/timeline"),
        resource_text(&server, "portmate://sessions/refresh-session/sysmon"),
        resource_text(&server, "portmate://transfers/transfer-diagnostic-id"),
    ];
    let sensitive_values = [
        "keyring:target-credential-ref",
        "stronghold:target-passphrase-ref",
        "keyring:proxy-credential-ref",
        "/home/operator/.ssh/private-key",
        "stronghold:identity-secret-ref",
        "keyring:jump-credential-ref",
        "stronghold:jump-passphrase-ref",
        "/home/operator/private-logs/{session}.raw",
        "/home/operator/private-downloads",
        "/home/operator/runtime-cwd",
        "disconnect-secret",
        "event-secret",
        "annotation-secret",
        "timeline-secret",
        "timeline-details-secret",
        "/home/operator/source-secret.txt",
        "/srv/private/destination-secret.txt",
        "transfer-message-secret",
        "trigger-label-secret",
        "trigger-match-secret",
        "/home/operator/private-scripts/deploy",
        "v2:/home/operator/private-logs/raw:0:12:digest",
        "sysmon-process-secret",
        "/dev/mapper/private-filesystem",
        "/srv/private-mount",
        "customer-private-interface",
    ];

    for (index, surface) in surfaces.iter().enumerate() {
        for sensitive in sensitive_values {
            assert!(
                !surface.contains(sensitive),
                "MCP read surface {index} leaked {sensitive}: {surface}"
            );
        }
    }
    assert!(surfaces
        .iter()
        .any(|surface| surface.contains("diagnostic.example")));
    assert!(surfaces
        .iter()
        .any(|surface| surface.contains("SHA256:diagnostic-fingerprint")));
    assert!(surfaces.iter().any(|surface| surface.contains("4242")));
    assert!(surfaces.iter().any(|surface| surface.contains("12.5")));
    assert!(surfaces
        .iter()
        .any(|surface| surface.contains("transfer-diagnostic-id")));
    assert!(surfaces.iter().any(|surface| surface.contains("completed")));
    assert_eq!(serde_json::to_string(&server.store).unwrap(), raw_store);
}

fn test_http_request(mut headers: HashMap<String, String>) -> HttpRequest {
    headers
        .entry("content-type".to_string())
        .or_insert_with(|| "application/json".to_string());
    HttpRequest {
        method: "POST".to_string(),
        path: "/mcp".to_string(),
        headers,
        body: serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }))
        .unwrap(),
    }
}

fn test_http_get_request(headers: HashMap<String, String>) -> HttpRequest {
    HttpRequest {
        method: "GET".to_string(),
        path: "/mcp".to_string(),
        headers,
        body: Vec::new(),
    }
}

fn test_tcp_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let client = TcpStream::connect(address).unwrap();
    let (server, _) = listener.accept().unwrap();
    (client, server)
}

fn parse_http_request_bytes(bytes: &[u8]) -> Result<HttpRequest> {
    let (mut client, mut server) = test_tcp_pair();
    let bytes = bytes.to_vec();
    let writer = thread::spawn(move || {
        client.write_all(&bytes).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
    });
    let result = read_http_request_with_timeout(&mut server, Duration::from_secs(1));
    writer.join().unwrap();
    result
}

#[test]
fn stdio_reader_bounds_messages_and_recovers_at_the_next_line() {
    let input = b"abcdefghijkl\n12345678\r\n{\"x\":1}\n";
    let mut reader = io::Cursor::new(input);

    assert_eq!(
        read_stdio_message(&mut reader, 8).unwrap(),
        StdioMessage::TooLarge
    );
    assert_eq!(
        read_stdio_message(&mut reader, 8).unwrap(),
        StdioMessage::Message(b"12345678".to_vec())
    );
    assert_eq!(
        read_stdio_message(&mut reader, 8).unwrap(),
        StdioMessage::Message(b"{\"x\":1}".to_vec())
    );
    assert_eq!(
        read_stdio_message(&mut reader, 8).unwrap(),
        StdioMessage::Eof
    );
}

#[test]
fn json_rpc_response_serialization_is_bounded_and_preserves_id_on_overflow() {
    let compact = json!({ "ok": true });
    let compact_bytes = serde_json::to_vec(&compact).unwrap();
    assert_eq!(
        try_encode_json_with_limit(&compact, compact_bytes.len()).unwrap(),
        Some(compact_bytes.clone())
    );
    assert!(
        try_encode_json_with_limit(&compact, compact_bytes.len() - 1)
            .unwrap()
            .is_none()
    );

    let response = json!({
        "jsonrpc": "2.0",
        "id": "request-7",
        "result": { "content": "x".repeat(1024) }
    });
    let encoded = encode_json_rpc_response(&response, 256).unwrap();
    assert!(encoded.len() <= 256);
    let overflow: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(overflow["id"], "request-7");
    assert_eq!(overflow["error"]["code"], -32603);
    assert!(overflow["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("256-byte limit")));
    assert!(overflow.get("result").is_none());
}

#[test]
fn sse_event_replaces_oversized_state_data() {
    let event = sse_event_with_limit(
        "portmate.state",
        &json!({ "content": "sensitive-marker".repeat(128) }),
        128,
    );

    assert!(event.starts_with("event: portmate.state\n"));
    assert!(event.contains("SSE data exceeds the 128-byte limit"));
    assert!(!event.contains("sensitive-marker"));
    assert!(event.len() < 256);
}

#[test]
fn desktop_ipc_endpoint_rejects_non_loopback_wrong_store_and_unsafe_token_refs() {
    let root = std::env::temp_dir().join(format!("portmate-mcp-endpoint-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let other_store_path = root.join("other-store.sqlite3");
    fs::write(&store_path, b"store").unwrap();
    fs::write(&other_store_path, b"other").unwrap();
    let mut endpoint = IpcEndpointFile {
        addr: "127.0.0.1:43123".to_string(),
        token: None,
        token_ref: Some(format!("keychain:ipc-{}", Uuid::new_v4())),
        store_path: store_path.display().to_string(),
    };

    assert_eq!(
        validate_ipc_endpoint(&endpoint, &store_path).unwrap(),
        "127.0.0.1:43123".parse::<SocketAddr>().unwrap()
    );
    assert!(validate_ipc_endpoint(&endpoint, &other_store_path).is_err());

    endpoint.addr = "192.0.2.1:43123".to_string();
    assert!(validate_ipc_endpoint(&endpoint, &store_path)
        .unwrap_err()
        .to_string()
        .contains("must be loopback"));
    endpoint.addr = "127.0.0.1:43123".to_string();
    endpoint.token_ref = Some("keychain:ipc-not-a-uuid".to_string());
    assert!(validate_ipc_endpoint(&endpoint, &store_path)
        .unwrap_err()
        .to_string()
        .contains("tokenRef is invalid"));
    endpoint.token_ref = Some(format!(
        "keychain:ipc-{}",
        Uuid::new_v4().hyphenated().to_string().to_uppercase()
    ));
    assert!(validate_ipc_endpoint(&endpoint, &store_path)
        .unwrap_err()
        .to_string()
        .contains("tokenRef is invalid"));
    endpoint.token_ref = Some("keychain:mcp-http-token".to_string());
    assert!(validate_ipc_endpoint(&endpoint, &store_path)
        .unwrap_err()
        .to_string()
        .contains("tokenRef is invalid"));
    assert!(endpoint_ipc_token(&endpoint)
        .unwrap_err()
        .to_string()
        .contains("tokenRef is invalid"));

    endpoint.token = Some("inline-token".to_string());
    assert!(validate_ipc_endpoint(&endpoint, &store_path)
        .unwrap_err()
        .to_string()
        .contains("must not contain both"));
    endpoint.token_ref = None;
    assert!(validate_ipc_endpoint(&endpoint, &store_path).is_ok());
    assert_eq!(endpoint_ipc_token(&endpoint).unwrap(), "inline-token");

    let endpoint_path = root.join("portmate-ipc.json");
    fs::write(&endpoint_path, serde_json::to_vec(&endpoint).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(load_ipc_endpoint(&store_path).is_some());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_ipc_endpoint(&store_path).is_none());
        fs::remove_file(&endpoint_path).unwrap();
        std::os::unix::fs::symlink(&store_path, &endpoint_path).unwrap();
        assert!(read_ipc_endpoint_file(&endpoint_path)
            .unwrap_err()
            .to_string()
            .contains("regular file"));
        fs::remove_file(&endpoint_path).unwrap();
    }
    fs::write(&endpoint_path, vec![b'x'; MAX_IPC_ENDPOINT_BYTES + 1]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert!(read_ipc_endpoint_file(&endpoint_path)
        .unwrap_err()
        .to_string()
        .contains("byte limit"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn mcp_refreshes_store_and_endpoint_between_json_rpc_envelopes() {
    let root = std::env::temp_dir().join(format!("portmate-mcp-refresh-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.json");
    let endpoint_path = root.join("portmate-ipc.json");
    let write_store = |name: &str| {
        fs::write(
            &store_path,
            serde_json::to_vec(&test_snapshot_store(name)).unwrap(),
        )
        .unwrap();
    };
    let write_endpoint = |addr: &str, token: &str| {
        fs::write(
            &endpoint_path,
            serde_json::to_vec(&IpcEndpointFile {
                addr: addr.to_string(),
                token: Some(token.to_string()),
                token_ref: None,
                store_path: store_path.display().to_string(),
            })
            .unwrap(),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    };

    write_store("first snapshot");
    write_endpoint("127.0.0.1:0", "first-token");
    let mut server = PortMateMcp {
        store: SessionStore::default(),
        store_path: Some(store_path.clone()),
        ipc: None,
        client_id: "refresh-client".to_string(),
        allow_write: false,
    };

    let first = list_sessions_text(&mut server);
    assert!(first.contains("first snapshot"));
    assert_eq!(
        server.ipc.as_ref().map(|endpoint| endpoint.addr.as_str()),
        Some("127.0.0.1:0")
    );

    write_store("second snapshot");
    write_endpoint("[::1]:0", "second-token");
    let second = list_sessions_text(&mut server);
    assert!(second.contains("second snapshot"));
    assert!(!second.contains("first snapshot"));
    assert_eq!(
        server.ipc.as_ref().map(|endpoint| endpoint.addr.as_str()),
        Some("[::1]:0")
    );

    fs::remove_file(&endpoint_path).unwrap();
    let _ = list_sessions_text(&mut server);
    assert!(server.ipc.is_none());

    fs::remove_file(&store_path).unwrap();
    let deleted = list_sessions_text(&mut server);
    assert!(!deleted.contains("second snapshot"));
    assert!(server.store.profiles.is_empty());

    write_store("third snapshot");
    let third = list_sessions_text(&mut server);
    assert!(third.contains("third snapshot"));
    fs::write(&store_path, b"{not-json").unwrap();
    let corrupt = list_sessions_text(&mut server);
    assert!(!corrupt.contains("third snapshot"));
    assert!(server.store.profiles.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn desktop_ipc_request_and_response_are_bounded() {
    let oversized = IpcRequest {
        token: "token".to_string(),
        client_id: "client".to_string(),
        trusted_write: false,
        command: "send_text".to_string(),
        args: json!({ "sessionId": "session", "text": "x".repeat(128) }),
    };
    let error = encode_ipc_request(&oversized, 64).unwrap_err();
    assert!(error.to_string().contains("64-byte limit"));

    let (mut client, mut server) = test_tcp_pair();
    let writer = thread::spawn(move || {
        server.write_all(&[b'x'; 33]).unwrap();
        server.shutdown(Shutdown::Write).unwrap();
    });
    let error = read_ipc_response_with_limits(&mut client, 32, Duration::from_secs(1)).unwrap_err();
    assert!(error.to_string().contains("32-byte limit"));
    writer.join().unwrap();
}

#[test]
fn http_request_deadline_cannot_be_extended_by_trickle_bytes() {
    let (mut client, mut server) = test_tcp_pair();
    let writer = thread::spawn(move || {
        for byte in b"GET /mcp HTTP/1.1\r\nHost: localhost\r\n" {
            if client.write_all(&[*byte]).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(15));
        }
    });
    let started = Instant::now();
    let error = read_http_request_with_timeout(&mut server, Duration::from_millis(60)).unwrap_err();
    assert_eq!(
        error.downcast_ref::<io::Error>().map(io::Error::kind),
        Some(io::ErrorKind::TimedOut)
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    drop(server);
    writer.join().unwrap();
}

#[test]
fn http_parser_rejects_ambiguous_or_unsupported_framing() {
    for request in [
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer one\r\nAuthorization: Bearer two\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
    ] {
        assert!(parse_http_request_bytes(request)
            .unwrap_err()
            .to_string()
            .contains("duplicate HTTP header"));
    }

    for request in [
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nContent-Length: 2\r\n\r\n2\r\n{}\r\n0\r\n\r\n".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: gzip, chunked\r\n\r\n".as_slice(),
    ] {
        assert!(parse_http_request_bytes(request).is_err());
    }

    let extra = b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}extra";
    assert!(parse_http_request_bytes(extra)
        .unwrap_err()
        .to_string()
        .contains("bytes after its declared body"));

    let malformed = b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nnot-a-header\r\n\r\n";
    assert!(parse_http_request_bytes(malformed)
        .unwrap_err()
        .to_string()
        .contains("invalid HTTP headers"));
}

#[test]
fn http_parser_decodes_chunked_body_and_bounded_trailers() {
    let request = parse_http_request_bytes(
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: ChUnKeD\r\n\r\n1;source=dotnet\r\n{\r\n1\r\n}\r\n0\r\nDigest: sha-256=test\r\n\r\n",
    )
    .unwrap();
    assert_eq!(request.body, b"{}");

    for request in [
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\nX\r\n{}\r\n0\r\n\r\n".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}X\n0\r\n\r\n".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n100001\r\n".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\nextra".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\nContent-Length: 2\r\n\r\n".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\nDigest: bad\0value\r\n\r\n".as_slice(),
    ] {
        assert!(parse_http_request_bytes(request).is_err());
    }
}

#[test]
fn http_parser_combines_repeatable_headers_and_reads_exact_body() {
    let request = parse_http_request_bytes(
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nAccept: text/event-stream\r\nContent-Length: 2\r\n\r\n{}",
    )
    .unwrap();

    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/mcp");
    assert_eq!(
        request.headers.get("accept").map(String::as_str),
        Some("application/json, text/event-stream")
    );
    assert_eq!(request.body, b"{}");
}

#[test]
fn http_connection_limit_rejects_excess_and_releases_completed_slots() {
    let config = test_http_config();
    let active = Arc::new(AtomicUsize::new(0));
    let permit = try_acquire_http_connection(&active, 1).unwrap();
    assert_eq!(active.load(Ordering::Acquire), 1);

    let (mut rejected_client, rejected_server) = test_tcp_pair();
    assert!(!spawn_http_connection(
        rejected_server,
        config.clone(),
        Arc::clone(&active),
        1,
    ));
    let mut rejected_response = String::new();
    rejected_client
        .read_to_string(&mut rejected_response)
        .unwrap();
    assert!(rejected_response.starts_with("HTTP/1.1 503 Service Unavailable"));
    assert!(rejected_response.contains("Connection: close"));
    assert_eq!(active.load(Ordering::Acquire), 1);

    drop(permit);
    assert_eq!(active.load(Ordering::Acquire), 0);
    let (mut accepted_client, accepted_server) = test_tcp_pair();
    accepted_client
        .write_all(b"OPTIONS /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    accepted_client.shutdown(Shutdown::Write).unwrap();
    assert!(spawn_http_connection(
        accepted_server,
        config,
        Arc::clone(&active),
        1,
    ));
    let mut accepted_response = String::new();
    accepted_client
        .read_to_string(&mut accepted_response)
        .unwrap();
    assert!(accepted_response.starts_with("HTTP/1.1 204 No Content"));
    assert!(accepted_response.contains(
        "Access-Control-Allow-Headers: Accept, Authorization, Content-Type, MCP-Protocol-Version, X-PortMate-MCP-Token"
    ));
    for _ in 0..100 {
        if active.load(Ordering::Acquire) == 0 {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(active.load(Ordering::Acquire), 0);
    assert!(try_acquire_http_connection(&active, 1).is_some());
}

#[test]
fn http_origin_requires_allow_list_match_when_present() {
    let config = test_http_config();
    assert!(validate_origin(None, &config).is_ok());
    assert!(validate_origin(Some("http://127.0.0.1:8787"), &config).is_ok());
    assert!(validate_origin(Some("http://evil.example"), &config).is_err());
}

#[test]
fn http_token_accepts_bearer_or_portmate_header() {
    let mut headers = HashMap::new();
    headers.insert(
        "authorization".to_string(),
        "bearer secret-token".to_string(),
    );
    let request = HttpRequest {
        method: "POST".to_string(),
        path: "/mcp".to_string(),
        headers,
        body: Vec::new(),
    };
    assert!(authorized_http_request(&request, "secret-token"));

    let mut invalid_headers = HashMap::new();
    invalid_headers.insert(
        "authorization".to_string(),
        "Bearer secret-token trailing".to_string(),
    );
    assert!(!authorized_http_request(
        &test_http_request(invalid_headers),
        "secret-token"
    ));

    let mut headers = HashMap::new();
    headers.insert(
        "x-portmate-mcp-token".to_string(),
        "secret-token".to_string(),
    );
    let request = HttpRequest {
        method: "POST".to_string(),
        path: "/mcp".to_string(),
        headers,
        body: Vec::new(),
    };
    assert!(authorized_http_request(&request, "secret-token"));
    assert!(!authorized_http_request(&request, "different-token"));
}

#[test]
fn http_post_validates_content_type_and_protocol_version() {
    let config = test_http_config();
    let mut headers = HashMap::new();
    headers.insert(
        "authorization".to_string(),
        "Bearer secret-token".to_string(),
    );
    headers.insert("accept".to_string(), "application/json".to_string());

    let mut missing_content_type = test_http_request(headers.clone());
    missing_content_type.headers.remove("content-type");
    let response = handle_http_request(missing_content_type, &config);
    assert!(response.starts_with("HTTP/1.1 415 Unsupported Media Type"));

    let mut wrong_content_type = test_http_request(headers.clone());
    wrong_content_type
        .headers
        .insert("content-type".to_string(), "text/plain".to_string());
    let response = handle_http_request(wrong_content_type, &config);
    assert!(response.starts_with("HTTP/1.1 415 Unsupported Media Type"));

    let mut unsupported_version = test_http_request(headers.clone());
    unsupported_version
        .headers
        .insert("mcp-protocol-version".to_string(), "2099-01-01".to_string());
    let response = handle_http_request(unsupported_version, &config);
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
    assert!(response.contains("2024-11-05, 2025-03-26, 2025-06-18"));

    let mut historical = test_http_request(headers.clone());
    historical
        .headers
        .insert("mcp-protocol-version".to_string(), "2025-03-26".to_string());
    let response = handle_http_request(historical, &config);
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains("MCP-Protocol-Version: 2025-03-26"));

    let mut compatible = test_http_request(headers);
    compatible.headers.insert(
        "content-type".to_string(),
        "Application/JSON; charset=utf-8".to_string(),
    );
    compatible.headers.insert(
        "mcp-protocol-version".to_string(),
        MCP_PROTOCOL_VERSION.to_string(),
    );
    let response = handle_http_request(compatible, &config);
    assert!(response.starts_with("HTTP/1.1 200 OK"));
}

#[test]
fn http_sse_rejects_unsupported_protocol_versions() {
    let config = test_http_config();
    let mut headers = HashMap::new();
    headers.insert(
        "authorization".to_string(),
        "Bearer secret-token".to_string(),
    );
    headers.insert("accept".to_string(), "text/event-stream".to_string());
    headers.insert("mcp-protocol-version".to_string(), "2099-01-01".to_string());

    let response = handle_http_request(test_http_get_request(headers), &config);
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
}

#[test]
fn http_options_rejects_unknown_paths() {
    let response = handle_http_request(
        HttpRequest {
            method: "OPTIONS".to_string(),
            path: "/unknown".to_string(),
            headers: HashMap::new(),
            body: Vec::new(),
        },
        &test_http_config(),
    );

    assert!(response.starts_with("HTTP/1.1 404 Not Found"));
}

#[test]
fn http_accept_respects_zero_quality_values() {
    let mut headers = HashMap::new();
    headers.insert(
        "accept".to_string(),
        "application/json; q=0.0, text/event-stream; q=1".to_string(),
    );
    let request = test_http_request(headers);

    assert!(!accepts_json_http_response(&request));
    assert!(accepts_sse_http_response(&request));
}

#[test]
fn http_json_rpc_initialize_returns_server_info() {
    let response = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }))
    .unwrap()
    .unwrap();
    assert_eq!(response["id"], json!(1));
    assert_eq!(response["result"]["serverInfo"]["name"], "portmate-mcp");
}

#[test]
fn initialize_negotiates_supported_historical_versions_and_falls_back_to_latest() {
    for version in MCP_PROTOCOL_VERSIONS {
        let response = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "id": version,
            "method": "initialize",
            "params": { "protocolVersion": version }
        }))
        .unwrap()
        .unwrap();
        assert_eq!(response["result"]["protocolVersion"], version);
    }

    let response = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": "future",
        "method": "initialize",
        "params": { "protocolVersion": "2099-01-01" }
    }))
    .unwrap()
    .unwrap();
    assert_eq!(response["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
}

#[test]
fn mcp_lists_concrete_resources_separately_from_templates() {
    let resources = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "resources/list",
        "params": {}
    }))
    .unwrap()
    .unwrap();
    let listed = resources["result"]["resources"].as_array().unwrap();
    assert_eq!(listed[0]["uri"], "portmate://sessions");
    assert!(listed
        .iter()
        .all(|resource| !resource["uri"].as_str().unwrap().contains('{')));

    let templates = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "resources/templates/list",
        "params": {}
    }))
    .unwrap()
    .unwrap();
    let listed = templates["result"]["resourceTemplates"].as_array().unwrap();
    assert!(!listed.is_empty());
    assert!(listed
        .iter()
        .all(|resource| resource["uriTemplate"].as_str().unwrap().contains('{')));
}

#[test]
fn mcp_resource_uris_round_trip_opaque_session_and_transfer_ids() {
    let session_id = "serial/rig 1%温度";
    let transfer_id = "transfer/1 %温度";
    let mut profile = test_snapshot_store("opaque session").profiles.remove(0);
    profile.id = session_id.to_string();
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    store
        .record_stream_event(
            session_id,
            portmate_core::EventDirection::Inbound,
            portmate_core::EventStream::Stdout,
            "opaque resource content",
        )
        .unwrap();
    store.record_transfer(portmate_core::TransferTask {
        id: transfer_id.to_string(),
        session_id: session_id.to_string(),
        protocol: portmate_core::TransferProtocol::Xmodem,
        source: "source".to_string(),
        destination: "destination".to_string(),
        bytes_total: 1,
        bytes_done: 1,
        status: portmate_core::TransferStatus::Completed,
        message: None,
        started_at: None,
        finished_at: None,
        average_bytes_per_second: None,
    });
    let server = PortMateMcp {
        store,
        store_path: None,
        ipc: None,
        client_id: "opaque-reader".to_string(),
        allow_write: false,
    };

    let resources = server.resources_list_result();
    let resources = resources["resources"].as_array().unwrap();
    let screen_uri = resources
        .iter()
        .find(|resource| resource["title"] == "opaque session Screen")
        .and_then(|resource| resource["uri"].as_str())
        .unwrap();
    let transfer_uri = resources
        .iter()
        .find(|resource| resource["title"] == format!("Transfer {transfer_id}"))
        .and_then(|resource| resource["uri"].as_str())
        .unwrap();
    assert_eq!(
        screen_uri,
        "portmate://sessions/serial%2Frig%201%25%E6%B8%A9%E5%BA%A6/screen"
    );
    assert_eq!(
        transfer_uri,
        "portmate://transfers/transfer%2F1%20%25%E6%B8%A9%E5%BA%A6"
    );
    assert!(
        server.resource_read(&json!({ "uri": screen_uri })).unwrap()["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("opaque resource content")
    );
    assert!(server
        .resource_read(&json!({ "uri": transfer_uri }))
        .unwrap()["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains(transfer_id));

    for invalid in [
        "portmate://sessions/a/b/screen",
        "portmate://sessions/a%2/screen",
        "portmate://sessions/a/screen?raw=1",
        "portmate://sessions//screen",
    ] {
        assert!(parse_session_uri(invalid).is_none(), "accepted {invalid}");
    }
    for invalid in [
        "portmate://transfers/a/b",
        "portmate://transfers/a%2",
        "portmate://transfers/a?raw=1",
        "portmate://transfers/",
    ] {
        assert!(parse_transfer_uri(invalid).is_none(), "accepted {invalid}");
    }
}

#[test]
fn mcp_ping_returns_empty_result() {
    let response = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": "ping-1",
        "method": "ping"
    }))
    .unwrap()
    .unwrap();

    assert_eq!(response["id"], "ping-1");
    assert_eq!(response["result"], json!({}));
}

#[test]
fn mcp_log_query_limit_matches_declared_schema_bounds() {
    assert_eq!(bounded_log_query_limit(None), 100);
    assert_eq!(bounded_log_query_limit(Some(0)), 1);
    assert_eq!(bounded_log_query_limit(Some(600)), 600);
    assert_eq!(bounded_log_query_limit(Some(u64::MAX)), 1000);
}

#[test]
fn json_rpc_empty_batch_is_invalid_and_notifications_have_no_payload() {
    let empty = handle_http_json_rpc(json!([])).unwrap().unwrap();
    assert_eq!(empty["error"]["code"], -32600);

    let notification = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }))
    .unwrap();
    assert!(notification.is_none());

    let notification_batch = handle_http_json_rpc(json!([
        {"jsonrpc": "2.0", "method": "notifications/initialized"},
        {"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {}}
    ]))
    .unwrap();
    assert!(notification_batch.is_none());
}

#[test]
fn json_rpc_envelopes_preserve_null_ids_and_reject_invalid_shapes() {
    let null_id = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": null,
        "method": "ping"
    }))
    .unwrap()
    .unwrap();
    assert_eq!(null_id["id"], Value::Null);
    assert_eq!(null_id["result"], json!({}));

    let invalid_id = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": { "nested": true },
        "method": "ping"
    }))
    .unwrap()
    .unwrap();
    assert_eq!(invalid_id["id"], Value::Null);
    assert_eq!(invalid_id["error"]["code"], -32600);

    let invalid_params = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "ping",
        "params": null
    }))
    .unwrap()
    .unwrap();
    assert_eq!(invalid_params["id"], 1);
    assert_eq!(invalid_params["error"]["code"], -32602);

    let invalid_notification = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "method": "ping",
        "params": "invalid"
    }))
    .unwrap();
    assert!(invalid_notification.is_none());
}

#[test]
fn json_rpc_batch_is_bounded_before_dispatch() {
    let accepted = (0..MAX_JSON_RPC_BATCH_ITEMS)
        .map(|id| json!({ "jsonrpc": "2.0", "id": id, "method": "ping" }))
        .collect::<Vec<_>>();
    let accepted = handle_http_json_rpc(Value::Array(accepted))
        .unwrap()
        .unwrap();
    assert_eq!(
        accepted.as_array().map(Vec::len),
        Some(MAX_JSON_RPC_BATCH_ITEMS)
    );

    let oversized = (0..=MAX_JSON_RPC_BATCH_ITEMS)
        .map(|id| json!({ "jsonrpc": "2.0", "id": id, "method": "ping" }))
        .collect::<Vec<_>>();
    let rejected = handle_http_json_rpc(Value::Array(oversized))
        .unwrap()
        .unwrap();
    assert!(!rejected.is_array());
    assert_eq!(rejected["id"], Value::Null);
    assert_eq!(rejected["error"]["code"], -32600);
    assert!(rejected["error"]["message"]
        .as_str()
        .is_some_and(|message| message.contains("128-item limit")));
}

#[test]
fn http_notification_returns_accepted_without_json_null() {
    let config = test_http_config();
    let mut headers = HashMap::new();
    headers.insert(
        "authorization".to_string(),
        "Bearer secret-token".to_string(),
    );
    headers.insert("accept".to_string(), "application/json".to_string());
    headers.insert("content-type".to_string(), "application/json".to_string());
    let request = HttpRequest {
        method: "POST".to_string(),
        path: "/mcp".to_string(),
        headers,
        body: serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .unwrap(),
    };

    let response = handle_http_request(request, &config);

    assert!(response.starts_with("HTTP/1.1 202 Accepted"));
    assert!(response.ends_with("\r\n\r\n"));
    assert!(!response.ends_with("null"));
}

#[test]
fn http_streamable_accept_header_allows_json_response() {
    let config = test_http_config();
    let mut headers = HashMap::new();
    headers.insert(
        "authorization".to_string(),
        "Bearer secret-token".to_string(),
    );
    headers.insert(
        "accept".to_string(),
        "application/json, text/event-stream".to_string(),
    );

    let response = handle_http_request(test_http_request(headers), &config);

    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains("MCP-Protocol-Version: 2025-06-18"));
    assert!(response.contains("\"serverInfo\""));
}

#[test]
fn http_get_sse_accept_header_returns_event_stream() {
    let config = test_http_config();
    let mut headers = HashMap::new();
    headers.insert(
        "authorization".to_string(),
        "Bearer secret-token".to_string(),
    );
    headers.insert("accept".to_string(), "text/event-stream".to_string());

    let response = handle_http_request(test_http_get_request(headers), &config);

    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains("Content-Type: text/event-stream"));
    assert!(response.contains("Connection: keep-alive"));
    assert!(response.contains("event: endpoint"));
    assert!(response.contains("event: portmate.state"));
    assert!(response.contains("\"protocolVersion\":\"2025-06-18\""));
}

#[test]
fn http_post_sse_only_accept_header_returns_message_event() {
    let config = test_http_config();
    let mut headers = HashMap::new();
    headers.insert(
        "authorization".to_string(),
        "Bearer secret-token".to_string(),
    );
    headers.insert("accept".to_string(), "text/event-stream".to_string());

    let response = handle_http_request(test_http_request(headers), &config);

    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.contains("Content-Type: text/event-stream"));
    assert!(response.contains("Content-Length:"));
    assert!(response.contains("event: message"));
    assert!(response.contains("\"serverInfo\""));
}

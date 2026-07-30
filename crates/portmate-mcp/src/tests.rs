use super::desktop_ipc::{
    encode_ipc_request, endpoint_ipc_token, load_ipc_endpoint, read_ipc_endpoint_file,
    read_ipc_response_with_limits, validate_ipc_endpoint, IpcEndpointFile, IpcRequest,
    MAX_IPC_ENDPOINT_BYTES,
};
use super::http_request::read_http_request_with_timeout;
use super::keyring_store::{ensure_keyring_store_with, initialize_persistent_native_keyring_with};
use super::store_loader::{
    ensure_store_schema, load_store_from_path, prepare_loaded_store, STORE_KEY,
};
use super::*;
use rusqlite::{params, Connection as SqliteConnection};
use std::collections::HashMap;
use std::fs;
use std::sync::Mutex;
use std::time::Instant;
use uuid::Uuid;

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
        security: HttpSecurityConfig::new(
            "secret-token".to_string(),
            vec!["http://127.0.0.1:8787".to_string()],
        ),
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

include!("tests_io_transport.rs");
include!("tests_protocol_surface.rs");

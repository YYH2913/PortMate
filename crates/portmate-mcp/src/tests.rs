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
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use portmate_core::{
    MAX_MCP_BRIDGE_REQUEST_BYTES, MAX_MCP_CONTENT_TRANSFER_BYTES, MAX_MCP_CONTENT_UPLOAD_BYTES,
    MCP_CONTENT_UPLOADS_DIRECTORY, MCP_CONTENT_UPLOAD_PAYLOAD_FILE,
    MCP_CONTENT_UPLOAD_STAGING_DIRECTORY,
};
use rusqlite::{params, Connection as SqliteConnection};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Instant;
use uuid::Uuid;

#[test]
fn transport_mode_accepts_explicit_modes_and_preserves_the_environment_default() {
    assert_eq!(
        select_transport_mode(Vec::<&str>::new(), None).unwrap(),
        McpTransportMode::Stdio
    );
    assert_eq!(
        select_transport_mode(Vec::<&str>::new(), Some(std::ffi::OsStr::new("1"))).unwrap(),
        McpTransportMode::Http
    );
    assert_eq!(
        select_transport_mode(["--http"], Some(std::ffi::OsStr::new("0"))).unwrap(),
        McpTransportMode::Http
    );
    assert_eq!(
        select_transport_mode(["--stdio"], Some(std::ffi::OsStr::new("1"))).unwrap(),
        McpTransportMode::Stdio
    );
}

#[test]
fn transport_mode_rejects_unknown_repeated_and_conflicting_arguments() {
    assert!(select_transport_mode(["--htpp"], None)
        .unwrap_err()
        .to_string()
        .contains("unknown MCP argument `--htpp`"));
    for arguments in [["--http", "--http"], ["--stdio", "--http"]] {
        assert!(select_transport_mode(arguments, None)
            .unwrap_err()
            .to_string()
            .contains("selected only once"));
    }
}

fn content_upload_server(root: &std::path::Path, client_id: &str) -> PortMateMcp {
    PortMateMcp {
        store: test_snapshot_store("content upload"),
        store_path: Some(root.join("portmate-store.sqlite3")),
        ipc: None,
        client_id: client_id.to_string(),
        allow_write: true,
    }
}

#[test]
fn content_upload_lifecycle_enforces_offsets_ownership_digest_and_cleanup() {
    let root = std::env::temp_dir().join(format!("portmate-content-upload-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let payload = vec![0x5a; MAX_MCP_CONTENT_TRANSFER_BYTES + 17];
    let sha256 = format!("{:x}", Sha256::digest(&payload));
    let server = content_upload_server(&root, "upload-owner");
    let begin = server
        .begin_content_upload(&json!({
            "sessionId": "refresh-session",
            "protocol": "xmodem",
            "fileName": "firmware.bin",
            "sizeBytes": payload.len(),
            "sha256": sha256,
            "destination": "load:loadx"
        }))
        .unwrap();
    let upload_id = begin["uploadId"].as_str().unwrap();
    assert_eq!(
        begin["maxChunkBytes"],
        MAX_MCP_CONTENT_TRANSFER_BYTES as u64
    );

    let first = &payload[..MAX_MCP_CONTENT_TRANSFER_BYTES];
    let appended = server
        .append_content_upload(&json!({
            "uploadId": upload_id,
            "offset": 0,
            "contentBase64": BASE64_STANDARD.encode(first)
        }))
        .unwrap();
    assert_eq!(
        appended["nextOffset"],
        MAX_MCP_CONTENT_TRANSFER_BYTES as u64
    );
    assert!(!appended["complete"].as_bool().unwrap());
    assert!(server
        .append_content_upload(&json!({
            "uploadId": upload_id,
            "offset": 0,
            "contentBase64": BASE64_STANDARD.encode(&payload[MAX_MCP_CONTENT_TRANSFER_BYTES..])
        }))
        .unwrap_err()
        .to_string()
        .contains("offset mismatch"));

    let other = content_upload_server(&root, "another-client");
    assert!(other
        .cancel_content_upload(&json!({ "uploadId": upload_id }))
        .unwrap_err()
        .to_string()
        .contains("unknown content upload"));

    server
        .append_content_upload(&json!({
            "uploadId": upload_id,
            "offset": MAX_MCP_CONTENT_TRANSFER_BYTES,
            "contentBase64": BASE64_STANDARD.encode(&payload[MAX_MCP_CONTENT_TRANSFER_BYTES..])
        }))
        .unwrap();
    let unavailable = server
        .start_completed_upload_transfer(&json!({ "uploadId": upload_id }))
        .unwrap_err()
        .to_string();
    assert!(unavailable.contains("desktop IPC is not available"));

    let upload_dir = root
        .join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)
        .join(MCP_CONTENT_UPLOADS_DIRECTORY)
        .join(upload_id);
    fs::write(
        upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE),
        vec![0x31; payload.len()],
    )
    .unwrap();
    assert!(server
        .start_completed_upload_transfer(&json!({ "uploadId": upload_id }))
        .unwrap_err()
        .to_string()
        .contains("SHA-256 mismatch"));
    server
        .cancel_content_upload(&json!({ "uploadId": upload_id }))
        .unwrap();
    assert!(!upload_dir.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unified_start_transfer_uses_one_desktop_ipc_command_for_every_source_mode() {
    let root = std::env::temp_dir().join(format!("portmate-unified-transfer-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let received = Arc::new(Mutex::new(Vec::<IpcRequest>::new()));
    let received_by_server = Arc::clone(&received);
    let server_thread = thread::spawn(move || {
        for index in 0..4 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new();
            stream.read_to_end(&mut raw).unwrap();
            let request = serde_json::from_slice::<IpcRequest>(&raw).unwrap();
            received_by_server.lock().unwrap().push(request);
            let response = json!({
                "ok": true,
                "value": {
                    "id": format!("transfer-{index}"),
                    "sessionId": "refresh-session",
                    "protocol": "xmodem",
                    "source": "staged.bin",
                    "destination": "load:loadx",
                    "bytesTotal": 3,
                    "bytesDone": 0,
                    "status": "queued",
                    "message": null
                },
                "error": null
            });
            stream
                .write_all(&serde_json::to_vec(&response).unwrap())
                .unwrap();
        }
    });
    let mut server = content_upload_server(&root, "unified-transfer-client");
    server.ipc = Some(IpcEndpointFile {
        addr: address.to_string(),
        token: Some("unified-transfer-token".to_string()),
        token_ref: None,
        store_path: store_path.display().to_string(),
    });

    server
        .start_transfer_tool(&json!({
            "sessionId": "refresh-session",
            "protocol": "xmodem",
            "source": root.join("firmware.bin").display().to_string(),
            "destination": "load:loadx"
        }))
        .unwrap();
    server
        .start_transfer_tool(&json!({
            "sessionId": "refresh-session",
            "protocol": "xmodem",
            "fileName": "legacy-firmware.bin",
            "contentBase64": BASE64_STANDARD.encode(b"abc"),
            "destination": "load:loadx"
        }))
        .unwrap();
    server
        .start_transfer_tool(&json!({
            "sessionId": "refresh-session",
            "protocol": "xmodem",
            "source": {
                "kind": "mcp",
                "fileName": "firmware.bin",
                "contentBase64": BASE64_STANDARD.encode(b"abc")
            },
            "destination": "load:loadx"
        }))
        .unwrap();
    let upload = server
        .begin_content_upload(&json!({
            "sessionId": "refresh-session",
            "protocol": "xmodem",
            "fileName": "firmware.bin",
            "sizeBytes": 3,
            "sha256": format!("{:x}", Sha256::digest(b"abc")),
            "destination": "load:loadx"
        }))
        .unwrap();
    let upload_id = upload["uploadId"].as_str().unwrap();
    server
        .append_content_upload(&json!({
            "uploadId": upload_id,
            "offset": 0,
            "contentBase64": BASE64_STANDARD.encode(b"abc")
        }))
        .unwrap();
    server
        .start_transfer_tool(&json!({ "uploadId": upload_id }))
        .unwrap();

    server_thread.join().unwrap();
    let requests = received.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests
        .iter()
        .all(|request| request.command == "start_transfer"));
    assert!(requests[0].args.get("source").is_some());
    assert_eq!(
        requests[1].args.get("fileName").and_then(Value::as_str),
        Some("legacy-firmware.bin")
    );
    assert_eq!(
        requests[2].args.get("source"),
        Some(&json!({
            "kind": "mcp",
            "fileName": "firmware.bin",
            "contentBase64": BASE64_STANDARD.encode(b"abc")
        }))
    );
    assert_eq!(requests[3].args, json!({ "uploadId": upload_id }));
    drop(requests);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn start_transfer_rejects_malformed_virtual_sources_before_desktop_ipc() {
    let root = std::env::temp_dir().join(format!("portmate-virtual-source-{}", Uuid::new_v4()));
    let server = content_upload_server(&root, "virtual-source-client");
    let request = |source| {
        json!({
            "sessionId": "refresh-session",
            "protocol": "xmodem",
            "source": source,
            "destination": "load:loadx"
        })
    };

    for source in [
        json!({"kind": "path", "fileName": "firmware.bin", "contentBase64": "AAE="}),
        json!({"kind": "mcp", "contentBase64": "AAE="}),
        json!({"kind": "mcp", "fileName": "firmware.bin", "contentBase64": "AAE=", "path": "C:\\firmware.bin"}),
        json!(["firmware.bin"]),
    ] {
        let error = server
            .start_transfer_tool(&request(source))
            .unwrap_err()
            .to_string();
        assert!(error.contains("source"));
        assert!(!error.contains("desktop IPC is not available"));
    }
}

#[cfg(unix)]
#[test]
fn content_upload_rejects_a_symlinked_private_root() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("portmate-content-link-{}", Uuid::new_v4()));
    let outside = root.join("outside");
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, root.join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)).unwrap();
    let server = content_upload_server(&root, "link-owner");
    assert!(server
        .begin_content_upload(&json!({
            "sessionId": "refresh-session",
            "protocol": "sftp",
            "fileName": "firmware.bin",
            "sizeBytes": 1,
            "sha256": "0".repeat(64),
            "destination": "remote:/tmp/firmware.bin"
        }))
        .unwrap_err()
        .to_string()
        .contains("invalid private content upload directory"));
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn desktop_ipc_request_budget_accepts_a_maximum_inline_content_envelope() {
    let request = IpcRequest {
        token: "bridge-token".to_string(),
        client_id: "content-client".to_string(),
        trusted_write: false,
        command: "start_transfer".to_string(),
        args: json!({
            "sessionId": "refresh-session",
            "protocol": "xmodem",
            "source": {
                "kind": "mcp",
                "fileName": "firmware.bin",
                "contentBase64": BASE64_STANDARD.encode(vec![
                    0xa5;
                    MAX_MCP_CONTENT_TRANSFER_BYTES
                ])
            },
            "destination": "load:loadx"
        }),
    };
    let encoded = encode_ipc_request(&request, MAX_MCP_BRIDGE_REQUEST_BYTES).unwrap();
    assert!(encoded.len() > 1024 * 1024);
    assert!(encoded.len() <= MAX_MCP_BRIDGE_REQUEST_BYTES);
    assert!(
        encode_ipc_request(&request, encoded.len().saturating_sub(1))
            .unwrap_err()
            .to_string()
            .contains("byte limit")
    );
}

#[test]
fn content_upload_rejects_oversized_chunks_and_declared_size_quota() {
    let root = std::env::temp_dir().join(format!("portmate-content-quota-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let server = content_upload_server(&root, "quota-owner");
    let begin = |size_bytes: u64, file_name: &str| {
        server.begin_content_upload(&json!({
            "sessionId": "refresh-session",
            "protocol": "sftp",
            "fileName": file_name,
            "sizeBytes": size_bytes,
            "sha256": "0".repeat(64),
            "destination": "remote:/tmp/firmware.bin"
        }))
    };
    begin(MAX_MCP_CONTENT_UPLOAD_BYTES, "first.bin").unwrap();
    begin(MAX_MCP_CONTENT_UPLOAD_BYTES, "second.bin").unwrap();
    assert!(begin(1, "over-quota.bin")
        .unwrap_err()
        .to_string()
        .contains("quota exceeded"));

    let upload_id = fs::read_dir(
        root.join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)
            .join(MCP_CONTENT_UPLOADS_DIRECTORY),
    )
    .unwrap()
    .next()
    .unwrap()
    .unwrap()
    .file_name()
    .to_string_lossy()
    .to_string();
    let oversized = BASE64_STANDARD.encode(vec![0u8; MAX_MCP_CONTENT_TRANSFER_BYTES + 1]);
    assert!(server
        .append_content_upload(&json!({
            "uploadId": upload_id,
            "offset": 0,
            "contentBase64": oversized
        }))
        .unwrap_err()
        .to_string()
        .contains("decoded chunk"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn concurrent_content_upload_appends_serialize_the_expected_offset() {
    let root = std::env::temp_dir().join(format!("portmate-content-race-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let server = content_upload_server(&root, "race-owner");
    let payload = b"one-of-two-concurrent-chunks";
    let begin = server
        .begin_content_upload(&json!({
            "sessionId": "refresh-session",
            "protocol": "sftp",
            "fileName": "race.bin",
            "sizeBytes": payload.len() * 2,
            "sha256": "0".repeat(64),
            "destination": "remote:/tmp/race.bin"
        }))
        .unwrap();
    let upload_id = begin["uploadId"].as_str().unwrap().to_string();
    let barrier = Arc::new(Barrier::new(3));
    let results = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for _ in 0..2 {
            let root = root.clone();
            let upload_id = upload_id.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(scope.spawn(move || {
                let server = content_upload_server(&root, "race-owner");
                barrier.wait();
                server.append_content_upload(&json!({
                    "uploadId": upload_id,
                    "offset": 0,
                    "contentBase64": BASE64_STANDARD.encode(payload)
                }))
            }));
        }
        barrier.wait();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| result
                .as_ref()
                .is_err_and(|error| error.to_string().contains("offset mismatch")))
            .count(),
        1
    );
    let upload_dir = root
        .join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)
        .join(MCP_CONTENT_UPLOADS_DIRECTORY)
        .join(upload_id);
    assert_eq!(
        fs::metadata(upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE))
            .unwrap()
            .len(),
        payload.len() as u64
    );
    let _ = fs::remove_dir_all(root);
}

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

    let calls = std::cell::Cell::new(0_u32);
    let error = initialize_persistent_native_keyring_with(|| {
        calls.set(calls.get() + 1);
        Err(anyhow!("persistent store unavailable"))
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "persistent store unavailable");
    assert_eq!(calls.get(), 1);
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

#[test]
fn http_listener_requires_explicit_permission_outside_loopback() {
    let loopback: SocketAddr = "127.0.0.1:8787".parse().unwrap();
    let wildcard_v4: SocketAddr = "0.0.0.0:8787".parse().unwrap();
    let wildcard_v6: SocketAddr = "[::]:8787".parse().unwrap();

    validate_http_bind_addr(loopback, false).unwrap();
    assert!(validate_http_bind_addr(wildcard_v4, false)
        .unwrap_err()
        .to_string()
        .contains("PORTMATE_MCP_HTTP_ALLOW_REMOTE=1"));
    validate_http_bind_addr(wildcard_v4, true).unwrap();
    validate_http_bind_addr(wildcard_v6, true).unwrap();
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

    assert!(prepare_loaded_store(store).is_err());
}

#[test]
fn initial_store_loading_distinguishes_unconfigured_and_invalid_paths() {
    assert!(load_initial_store(None).unwrap().profiles.is_empty());

    let root = std::env::temp_dir().join(format!("portmate-mcp-initial-store-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let missing_path = root.join("missing.sqlite3");
    let missing_error = load_initial_store(Some(&missing_path))
        .unwrap_err()
        .to_string();
    assert!(missing_error.contains("PORTMATE_STORE_PATH"));
    assert!(missing_error.contains("not a readable PortMate Store"));
    assert!(!missing_path.exists());

    let corrupt_path = root.join("corrupt.json");
    fs::write(&corrupt_path, b"{not-json").unwrap();
    let corrupt_error = load_initial_store(Some(&corrupt_path))
        .unwrap_err()
        .to_string();
    assert!(corrupt_error.contains("PORTMATE_STORE_PATH"));
    assert!(corrupt_error.contains("not a readable PortMate Store"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn standalone_sqlite_store_loading_never_creates_or_migrates_a_store() {
    let root = std::env::temp_dir().join(format!("portmate-mcp-read-only-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let missing_path = root.join("missing.sqlite3");
    assert!(load_store_from_path(&missing_path).is_err());
    assert!(!missing_path.exists());

    let empty_path = root.join("empty.sqlite3");
    drop(SqliteConnection::open(&empty_path).unwrap());
    assert!(load_store_from_path(&empty_path).is_err());
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
        .record_command_history(
            "deploy --password command-history-secret".to_string(),
            100,
            30,
            diagnostic_ts.timestamp_millis(),
        )
        .unwrap();
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
fn serial_break_fails_closed_without_desktop_ipc() {
    let mut server = PortMateMcp {
        store: test_snapshot_store("serial break"),
        store_path: None,
        ipc: None,
        client_id: "serial-operator".to_string(),
        allow_write: true,
    };

    let response = server
        .tool_call(&json!({
            "name": "serial_send_break",
            "arguments": { "sessionId": "refresh-session" }
        }))
        .unwrap();
    assert_eq!(response["isError"], true);
    let text = response["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("NOT executed"));
    assert!(text.contains("desktop IPC is not available"));
    assert!(text.contains("no Break was sent"));
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
        "command-history-secret",
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
include!("tests_custom_scripts.rs");

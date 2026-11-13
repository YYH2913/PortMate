struct RecoveryRoot(PathBuf);

impl RecoveryRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("portmate-ipc-recovery-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for RecoveryRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn write_recovery_endpoint(store: &std::path::Path, listener: &TcpListener) -> IpcEndpointFile {
    let endpoint = IpcEndpointFile {
        addr: listener.local_addr().unwrap().to_string(),
        token: Some("test-only-ipc-token".into()),
        token_ref: None,
        store_path: store.display().to_string(),
    };
    let path = store.with_file_name("portmate-ipc.json");
    let staged = path.with_extension("pending");
    fs::write(&staged, serde_json::to_vec(&endpoint).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::rename(staged, path).unwrap();
    endpoint
}

#[test]
fn ipc_discovers_a_new_endpoint_after_a_temporary_startup_gap() {
    let root = RecoveryRoot::new();
    let store = root.0.join("store.sqlite3");
    fs::write(&store, "store").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let server_store = store.clone();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        write_recovery_endpoint(&server_store, &listener);
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let request: IpcRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request.command, "run_command");
        stream
            .write_all(br#"{"ok":true,"value":{"sent":true}}"#)
            .unwrap();
    });
    let result = call_desktop_ipc_value(
        None,
        &store,
        "client",
        false,
        "run_command",
        json!({"command":"echo test"}),
    )
    .unwrap();
    assert_eq!(result, json!({"sent":true}));
    worker.join().unwrap();
}

#[test]
fn ipc_reloads_the_port_and_token_after_a_failed_connection() {
    let root = RecoveryRoot::new();
    let store = root.0.join("store.sqlite3");
    fs::write(&store, "store").unwrap();
    let old_listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let old = write_recovery_endpoint(&store, &old_listener);
    drop(old_listener);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mut endpoint = write_recovery_endpoint(&store, &listener);
    endpoint.token = Some("rotated-test-only-token".into());
    fs::write(
        store.with_file_name("portmate-ipc.json"),
        serde_json::to_vec(&endpoint).unwrap(),
    )
    .unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let request: IpcRequest = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(request.token, endpoint.token.unwrap());
        stream
            .write_all(br#"{"ok":true,"value":"recovered"}"#)
            .unwrap();
    });
    assert_eq!(
        call_desktop_ipc_value(
            Some(&old),
            &store,
            "client",
            false,
            "run_command",
            json!({})
        )
        .unwrap(),
        json!("recovered")
    );
    worker.join().unwrap();
}

#[test]
fn ipc_never_replays_a_write_after_a_lost_response() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let root = RecoveryRoot::new();
    let store = root.0.join("store.sqlite3");
    fs::write(&store, "store").unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = write_recovery_endpoint(&store, &listener);
    listener.set_nonblocking(true).unwrap();
    let done = Arc::new(AtomicBool::new(false));
    let count = Arc::new(AtomicUsize::new(0));
    let (finished, requests) = (Arc::clone(&done), Arc::clone(&count));
    let worker = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(4);
        while !finished.load(Ordering::SeqCst) && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    // Accepted sockets can inherit nonblocking mode on Windows.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut bytes = Vec::new();
                    stream.read_to_end(&mut bytes).unwrap();
                    assert!(!bytes.is_empty());
                    requests.fetch_add(1, Ordering::SeqCst);
                    // Model a command that ran but whose response was lost.
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("{error}"),
            }
        }
    });
    let result = call_desktop_ipc_value(
        Some(&endpoint),
        &store,
        "client",
        false,
        "run_command",
        json!({"command":"echo once"}),
    );
    done.store(true, Ordering::SeqCst);
    worker.join().unwrap();
    let error = result.unwrap_err();
    assert!(error.to_string().contains("execution status is unknown"));
    assert!(!error.is::<desktop_ipc::IpcUnavailable>());
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn ipc_diagnostics_distinguish_missing_invalid_and_refused_endpoints_without_tokens() {
    let root = RecoveryRoot::new();
    let store = root.0.join("store.sqlite3");
    fs::write(&store, "store").unwrap();
    let inspect = |endpoint| {
        call_desktop_ipc_value(endpoint, &store, "client", false, "run_command", json!({}))
            .unwrap_err()
    };
    let missing = inspect(None);
    assert_eq!(
        missing
            .downcast_ref::<desktop_ipc::IpcUnavailable>()
            .unwrap()
            .code,
        "endpoint-missing"
    );
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let endpoint = write_recovery_endpoint(&store, &listener);
    drop(listener);
    let refused = inspect(Some(&endpoint));
    assert_eq!(
        refused
            .downcast_ref::<desktop_ipc::IpcUnavailable>()
            .unwrap()
            .code,
        "connect-failed"
    );
    assert!(refused.to_string().contains("loopback listener"));
    let path = store.with_file_name("portmate-ipc.json");
    fs::write(path, br#"{"token":"do-not-echo-this-secret","addr":false}"#).unwrap();
    let invalid = inspect(None);
    assert_eq!(
        invalid
            .downcast_ref::<desktop_ipc::IpcUnavailable>()
            .unwrap()
            .code,
        "endpoint-invalid"
    );
    for error in [missing, refused, invalid] {
        assert!(!error.to_string().contains("test-only-ipc-token"));
        assert!(!error.to_string().contains("do-not-echo-this-secret"));
    }
}

#[test]
fn bridge_status_and_failed_writes_expose_the_ipc_failure_reason() {
    let root = RecoveryRoot::new();
    let path = root.0.join("store.sqlite3");
    let mut store = test_snapshot_store("IPC diagnostics");
    grant_all_read_scopes(&mut store, "diagnostic-client");
    let mut server = PortMateMcp {
        store,
        store_path: Some(path),
        ipc: None,
        client_id: "diagnostic-client".into(),
        allow_write: false,
    };
    let status = server
        .tool_call(&json!({"name":"mcp_bridge_status","arguments":{}}))
        .unwrap();
    let status: Value =
        serde_json::from_str(status["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(status["desktopIpcAvailable"], false);
    assert_eq!(status["desktopIpcIssue"]["code"], "endpoint-missing");
    assert_eq!(status["bridgeVersion"], env!("CARGO_PKG_VERSION"));
    assert_eq!(status["bridgePid"], std::process::id());
    assert!(status["desktopIpcEndpointPath"]
        .as_str()
        .unwrap()
        .ends_with("portmate-ipc.json"));
    let result = server.tool_call(&json!({"name":"run_command","arguments":{"sessionId":"refresh-session","command":"echo test"}})).unwrap();
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("NOT executed"));
    assert!(text.contains("endpoint-missing"));
}

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
fn stdio_reader_accepts_a_maximum_inline_content_envelope() {
    let request = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "large-stdio-content",
        "method": "tools/call",
        "params": {
            "name": "start_transfer",
            "arguments": {
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
            }
        }
    }))
    .unwrap();
    assert!(request.len() > 1024 * 1024);
    assert!(request.len() <= MAX_MCP_BRIDGE_REQUEST_BYTES);
    let mut input = request.clone();
    input.push(b'\n');
    let mut reader = io::Cursor::new(input);
    assert_eq!(
        read_stdio_message(&mut reader, MAX_MCP_BRIDGE_REQUEST_BYTES).unwrap(),
        StdioMessage::Message(request)
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
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\nextra".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\nContent-Length: 2\r\n\r\n".as_slice(),
        b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\nDigest: bad\0value\r\n\r\n".as_slice(),
    ] {
        assert!(parse_http_request_bytes(request).is_err());
    }

    let oversized = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n{:X}\r\n",
        MAX_MCP_BRIDGE_REQUEST_BYTES + 1,
    );
    assert!(parse_http_request_bytes(oversized.as_bytes())
        .unwrap_err()
        .to_string()
        .contains("HTTP body is too large"));
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
fn http_parser_accepts_a_maximum_inline_content_envelope() {
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "large-http-content",
        "method": "tools/call",
        "params": {
            "name": "start_transfer",
            "arguments": {
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
            }
        }
    }))
    .unwrap();
    assert!(body.len() > 1024 * 1024);
    assert!(body.len() <= MAX_MCP_BRIDGE_REQUEST_BYTES);
    let mut request = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    request.extend_from_slice(&body);
    assert_eq!(parse_http_request_bytes(&request).unwrap().body, body);
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
    assert!(validate_origin(None, &config.security).is_ok());
    assert!(validate_origin(Some("http://127.0.0.1:8787"), &config.security).is_ok());
    assert!(validate_origin(Some("http://evil.example"), &config.security).is_err());
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

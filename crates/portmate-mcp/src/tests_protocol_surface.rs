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
fn tools_list_advertises_bridge_management_surface() {
    let response = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": {}
    }))
    .unwrap()
    .unwrap();
    let tools = response["result"]["tools"].as_array().unwrap();
    for (name, read_only) in [
        ("mcp_bridge_status", true),
        ("reload_mcp", true),
        ("restart_mcp", false),
    ] {
        let tool = tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("tools/list omitted {name}"));
        assert_eq!(tool["annotations"]["readOnlyHint"], read_only, "{name}");
    }
}

#[test]
fn tools_list_advertises_tftp_and_session_independent_routes() {
    let response = handle_http_json_rpc(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }))
    .unwrap()
    .unwrap();
    let tools = response["result"]["tools"].as_array().unwrap();
    let transfer = tools
        .iter()
        .find(|tool| tool["name"] == "start_transfer")
        .unwrap();
    assert!(transfer["inputSchema"]["properties"]["protocol"]["enum"]
        .as_array()
        .is_some_and(|protocols| protocols.iter().any(|protocol| protocol == "tftp")));
    let raw_bytes = tools
        .iter()
        .find(|tool| tool["name"] == "send_bytes")
        .expect("tools/list omitted send_bytes");
    assert_eq!(raw_bytes["inputSchema"]["properties"]["encoding"]["enum"],
        json!(["base64", "hex"]));
    assert!(transfer["inputSchema"]["oneOf"]
        .as_array()
        .is_some_and(|variants| variants.len() == 3));
    let tunnel = tools
        .iter()
        .find(|tool| tool["name"] == "create_tunnel")
        .unwrap();
    assert!(tunnel["description"]
        .as_str()
        .is_some_and(|description| description.contains("independent of terminal sessions")));
    assert!(tunnel["inputSchema"]["properties"]["egress"]["enum"]
        .as_array()
        .is_some_and(|egresses| egresses.iter().any(|egress| egress == "portmate-host")));
    assert!(tunnel["inputSchema"]["required"]
        .as_array()
        .is_some_and(|required| !required.iter().any(|value| value == "sessionId")));
    let list = tools
        .iter()
        .find(|tool| tool["name"] == "list_tunnels")
        .unwrap();
    assert!(list["inputSchema"]["required"].is_null());
}

#[test]
fn removed_transfer_tool_names_are_not_callable() {
    for name in [
        concat!("tf", "tp"),
        concat!("start_content", "_transfer"),
        concat!("start_content_upload", "_transfer"),
    ] {
        let mut server = PortMateMcp {
            store: test_snapshot_store("removed tool"),
            store_path: None,
            ipc: None,
            client_id: "removed-tool-client".to_string(),
            allow_write: true,
        };
        let error = server
            .tool_call(&json!({ "name": name, "arguments": {} }))
            .unwrap_err()
            .to_string();
        assert_eq!(error, format!("unknown tool: {name}"));
    }
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
    let log_resource = resources
        .iter()
        .find(|resource| resource["title"] == "opaque session Log")
        .unwrap();
    let log_uri = log_resource["uri"].as_str().unwrap();
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
    let screen = server.resource_read(&json!({ "uri": screen_uri })).unwrap();
    assert_eq!(screen["contents"][0]["mimeType"], "text/plain");
    assert!(
        screen["contents"][0]["text"]
            .as_str()
            .unwrap()
            .contains("opaque resource content")
    );
    assert_eq!(log_resource["mimeType"], "application/jsonl");
    let log = server.resource_read(&json!({ "uri": log_uri })).unwrap();
    assert_eq!(log["contents"][0]["mimeType"], "application/jsonl");
    assert!(log["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("opaque resource content"));
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
fn mcp_transfer_query_limit_matches_declared_schema_bounds() {
    assert_eq!(bounded_transfer_query_limit(None), 100);
    assert_eq!(bounded_transfer_query_limit(Some(0)), 1);
    assert_eq!(bounded_transfer_query_limit(Some(600)), 600);
    assert_eq!(bounded_transfer_query_limit(Some(u64::MAX)), 1000);
}

#[test]
fn transfer_tools_filter_by_scope_session_and_limit_while_redacting_paths() {
    let mut store = test_snapshot_store("visible transfer session");
    let mut hidden = store.profiles[0].clone();
    hidden.id = "hidden-session".to_string();
    hidden.name = "hidden transfer session".to_string();
    store.upsert_profile(hidden);
    let transfer = |id: &str, session_id: &str, path: &str| portmate_core::TransferTask {
        id: id.to_string(),
        session_id: session_id.to_string(),
        protocol: portmate_core::TransferProtocol::Sftp,
        source: path.to_string(),
        destination: format!("remote:/srv/{id}"),
        bytes_total: 1,
        bytes_done: 1,
        status: portmate_core::TransferStatus::Completed,
        message: Some(format!("token={id}-secret")),
        started_at: None,
        finished_at: None,
        average_bytes_per_second: None,
    };
    store.record_transfer(transfer(
        "visible-old",
        "refresh-session",
        "/home/operator/visible-old",
    ));
    store.record_transfer(transfer(
        "hidden",
        "hidden-session",
        "/home/operator/hidden",
    ));
    store.record_transfer(transfer(
        "visible-new",
        "refresh-session",
        "/home/operator/visible-new",
    ));
    store.grants.push(portmate_core::McpGrant {
        client_id: "transfer-reader".to_string(),
        name: "Transfer reader".to_string(),
        scopes: vec![McpScope::ReadTransfers],
        allowed_sessions: vec!["refresh-session".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    let mut server = PortMateMcp {
        store,
        store_path: None,
        ipc: None,
        client_id: "transfer-reader".to_string(),
        allow_write: false,
    };

    let listed = server
        .tool_call(&json!({
            "name": "list_transfers",
            "arguments": { "limit": 1 }
        }))
        .unwrap();
    let listed = listed["content"][0]["text"].as_str().unwrap();
    assert!(listed.contains("visible-new"));
    assert!(!listed.contains("visible-old"));
    assert!(!listed.contains("hidden"));
    assert!(listed.contains("<redacted-path>"));
    assert!(!listed.contains("/home/operator"));
    assert!(!listed.contains("visible-new-secret"));

    let one = server
        .tool_call(&json!({
            "name": "get_transfer",
            "arguments": { "transferId": "visible-new" }
        }))
        .unwrap();
    let one = one["content"][0]["text"].as_str().unwrap();
    assert!(one.contains("visible-new"));
    assert!(one.contains("<redacted-path>"));
    assert!(!one.contains("/home/operator"));
    assert!(server
        .tool_call(&json!({
            "name": "get_transfer",
            "arguments": { "transferId": "hidden" }
        }))
        .unwrap_err()
        .to_string()
        .contains("ReadTransfers"));

    server.store.grants[0].scopes = vec![McpScope::Transfer];
    assert!(server
        .tool_call(&json!({
            "name": "list_transfers",
            "arguments": { "sessionId": "refresh-session" }
        }))
        .is_ok());
    assert_eq!(
        server
            .tool_call(&json!({
                "name": "list_tunnels",
                "arguments": { "sessionId": "refresh-session" }
            }))
            .unwrap_err()
            .to_string(),
        "MCP read grant does not permit ReadTunnels for the requested session"
    );
}

#[test]
fn tunnel_read_scope_returns_a_stable_empty_list_without_desktop_ipc() {
    let mut store = test_snapshot_store("route session");
    store.grants.push(portmate_core::McpGrant {
        client_id: "route-reader".to_string(),
        name: "Route reader".to_string(),
        scopes: vec![McpScope::ReadTunnels],
        allowed_sessions: vec!["refresh-session".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    let mut server = PortMateMcp {
        store,
        store_path: None,
        ipc: None,
        client_id: "route-reader".to_string(),
        allow_write: false,
    };

    let response = server
        .tool_call(&json!({
            "name": "list_tunnels",
            "arguments": { "sessionId": "refresh-session" }
        }))
        .unwrap();
    assert_eq!(response["content"][0]["text"], "[]");
    let host_routes = server
        .tool_call(&json!({
            "name": "list_host_routes",
            "arguments": {}
        }))
        .unwrap();
    assert_eq!(host_routes["content"][0]["text"], "[]");

    server.store.grants[0].scopes = vec![McpScope::Tunnel];
    assert!(server
        .tool_call(&json!({
            "name": "list_tunnels",
            "arguments": { "sessionId": "refresh-session" }
        }))
        .is_ok());
    assert!(server
        .tool_call(&json!({
            "name": "list_host_routes",
            "arguments": {}
        }))
        .is_ok());
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

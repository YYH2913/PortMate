#[test]
fn host_scripts_never_fall_back_to_snapshot_execution() {
    let mut store = test_snapshot_store("script session");
    store.grants.push(portmate_core::McpGrant {
        client_id: "script-client".into(), name: "Script client".into(),
        scopes: vec![McpScope::RunScripts], allowed_sessions: vec![],
        confirm_writes: false, expires_at: None, revoked_at: None,
    });
    let mut server = PortMateMcp { store, store_path: None, ipc: None,
        client_id: "script-client".into(), allow_write: false };
    assert!(server.host_script_definitions().unwrap().is_empty());
    let run = server.tool_call(&json!({"name":"run_custom_script", "arguments":{
        "scriptId":"69c06a07-dc48-4d4e-9498-6f42b6deab21","parameters":{}
    }})).unwrap_err();
    assert!(run.to_string().contains("NOT executed"));
    assert!(server.tool_call(&json!({"name":"host_script_69c06a07dc484d4e94986f42b6deab21","arguments":{}})).is_err());
    server.store.grants[0].revoked_at = Some("2026-01-01T00:00:00Z".parse().unwrap());
    assert!(server.tool_call(&json!({"name":"list_custom_scripts","arguments":{}})).is_err());
}

#[test]
fn dynamic_host_tools_discover_and_call_through_authoritative_desktop_ipc() {
    let root = std::env::temp_dir().join(format!("portmate-host-tools-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let tool_name = "host_script_69c06a07dc484d4e94986f42b6deab21";
    let thread = thread::spawn(move || {
        for index in 0..3 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut raw = Vec::new(); stream.read_to_end(&mut raw).unwrap();
            let request: IpcRequest = serde_json::from_slice(&raw).unwrap();
            assert_eq!(request.client_id, "script-client");
            let value = if index < 2 {
                assert_eq!(request.command, "list_custom_scripts");
                json!([{"name":tool_name,"title":"Host echo","description":"Host skill",
                    "inputSchema":{"type":"object","properties":{"message":{"type":"string"}},"additionalProperties":false}, "readOnly":false}])
            } else {
                assert_eq!(request.command, "run_custom_script");
                assert_eq!(request.args, json!({"scriptId":"69c06a07-dc48-4d4e-9498-6f42b6deab21","parameters":{"message":"literal; $(data)"}}));
                json!({"runId":"run","exitCode":7,"stdout":"","stderr":"failure detail","failure":"exit 7"})
            };
            stream.write_all(&serde_json::to_vec(&json!({"ok":true,"value":value,"error":null})).unwrap()).unwrap();
        }
    });
    let mut store = test_snapshot_store("host tool");
    store.grants.push(portmate_core::McpGrant { client_id: "script-client".into(), name: "client".into(),
        scopes: vec![McpScope::RunScripts], allowed_sessions: vec!["__none__".into()], confirm_writes: false, expires_at: None, revoked_at: None });
    let mut server = PortMateMcp { store, store_path: Some(store_path.clone()),
        ipc: Some(IpcEndpointFile { addr: address.to_string(), token: Some("host-test-token".into()), token_ref: None, store_path: store_path.display().to_string() }),
        client_id: "script-client".into(), allow_write: false };
    let response = server.handle(JsonRpcRequest { jsonrpc:"2.0".into(), id:Some(json!(1)), method:"tools/list".into(), params:json!({}) }).unwrap().unwrap();
    assert!(response.result.unwrap()["tools"].as_array().unwrap().iter().any(|tool| tool["name"] == tool_name && tool["inputSchema"]["additionalProperties"] == false));
    let result = server.tool_call(&json!({"name":tool_name,"arguments":{"message":"literal; $(data)"}})).unwrap();
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["exitCode"], 7);
    thread.join().unwrap();
    fs::remove_dir_all(root).unwrap();
}

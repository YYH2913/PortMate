use super::*;

fn definition(name: &str) -> McpToolDefinition {
    tool_definitions()
        .into_iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("missing MCP tool definition: {name}"))
}

#[test]
fn bridge_management_tools_are_advertised_with_safe_schemas() {
    assert_eq!(tool_definitions().len(), 31);
    for name in ["mcp_bridge_status", "reload_mcp", "restart_mcp"] {
        let definition = definition(name);
        assert_eq!(definition.input_schema["type"], "object", "{name}");
        assert_eq!(
            definition.input_schema["additionalProperties"], false,
            "{name}"
        );
    }
    assert!(definition("mcp_bridge_status").read_only);
    assert!(definition("reload_mcp").read_only);
    assert!(!definition("restart_mcp").read_only);
}

#[test]
fn serial_break_is_a_bounded_write_tool() {
    let serial_break = definition("serial_send_break");
    assert!(!serial_break.read_only);
    assert_eq!(serial_break.input_schema["type"], "object");
    assert_eq!(serial_break.input_schema["required"], json!(["sessionId"]));
    assert_eq!(
        serial_break.input_schema["properties"]["sessionId"]["maxLength"],
        128
    );
    assert!(serial_break
        .description
        .contains("connected serial session"));
}

#[test]
fn raw_bytes_tool_exposes_binary_encodings_without_payload_echo() {
    let bytes = definition("send_bytes");
    assert!(!bytes.read_only);
    assert_eq!(
        bytes.input_schema["required"],
        json!(["sessionId", "encoding", "data"])
    );
    assert_eq!(
        bytes.input_schema["properties"]["encoding"]["enum"],
        json!(["base64", "hex"])
    );
    assert!(bytes.description.contains("without adding a newline"));
    assert!(bytes.description.contains("redacted byte summary"));
}

#[test]
fn transfer_and_route_lifecycle_tools_expose_bounded_schemas() {
    for name in [
        "list_transfers",
        "get_transfer",
        "cancel_transfer",
        "retry_transfer",
        "list_tunnels",
        "stop_tunnel",
    ] {
        let tool = definition(name);
        assert_eq!(tool.input_schema["type"], "object", "{name}");
    }
    for name in ["list_transfers", "get_transfer", "list_tunnels"] {
        assert!(definition(name).read_only, "{name}");
    }
    for name in [
        "start_transfer",
        "begin_content_upload",
        "append_content_upload",
        "cancel_content_upload",
        "cancel_transfer",
        "retry_transfer",
        "create_tunnel",
        "stop_tunnel",
    ] {
        assert!(!definition(name).read_only, "{name}");
    }

    let transfer = definition("start_transfer");
    assert_eq!(
        transfer.input_schema["properties"]["source"]["oneOf"][0]["maxLength"],
        32_768
    );
    assert_eq!(
        transfer.input_schema["properties"]["source"]["oneOf"][1]["properties"]["kind"]["const"],
        "mcp"
    );
    assert_eq!(
        transfer.input_schema["properties"]["source"]["oneOf"][1]["properties"]["contentBase64"]
            ["maxLength"],
        MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
    );
    let transfer_protocols = transfer.input_schema["properties"]["protocol"]["enum"]
        .as_array()
        .expect("start_transfer protocol enum");
    assert!(transfer_protocols.contains(&json!("scp")));
    assert!(transfer_protocols.contains(&json!("tftp")));
    assert_eq!(
        transfer.input_schema["oneOf"].as_array().map(Vec::len),
        Some(3)
    );
    assert_eq!(
        transfer.input_schema["properties"]["contentBase64"]["maxLength"],
        MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
    );
    assert_eq!(
        transfer.input_schema["properties"]["uploadId"]["format"],
        "uuid"
    );
    let tftp_destination = &transfer.input_schema["properties"]["destination"]["oneOf"][1];
    assert_eq!(tftp_destination["properties"]["kind"]["const"], "tftpboot");
    assert_eq!(tftp_destination["required"], json!(["kind", "deviceIp"]));
    assert_eq!(tftp_destination["additionalProperties"], false);
    assert_eq!(
        definition("begin_content_upload").input_schema["properties"]["destination"]["oneOf"][1],
        *tftp_destination
    );
    let tunnel_request = definition("tunnel_request");
    assert!(!tunnel_request.read_only);
    assert_eq!(
        tunnel_request.input_schema["properties"]["data"]["maxLength"],
        MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH
    );
    assert_eq!(
        tunnel_request.input_schema["properties"]["timeoutMs"]["maximum"],
        MAX_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS
    );
    assert_eq!(
        tunnel_request.input_schema["properties"]["maxResponseBytes"]["maximum"],
        MAX_MCP_TUNNEL_EXCHANGE_BYTES
    );
    assert!(transfer.description.contains("exactly one source"));
    assert!(transfer.description.contains("limited to 4 MiB"));
    assert!(transfer.description.contains("deviceIp is required"));
    assert!(definition("begin_content_upload")
        .description
        .contains("validated before content is uploaded"));
    let begin_upload = definition("begin_content_upload");
    let upload_protocols = begin_upload.input_schema["properties"]["protocol"]["enum"]
        .as_array()
        .expect("begin_content_upload protocol enum");
    assert!(upload_protocols.contains(&json!("scp")));
    assert!(upload_protocols.contains(&json!("tftp")));
    assert_eq!(
        definition("begin_content_upload").input_schema["properties"]["sizeBytes"]["maximum"],
        MAX_MCP_CONTENT_UPLOAD_BYTES
    );
    assert_eq!(
        definition("append_content_upload").input_schema["properties"]["contentBase64"]
            ["maxLength"],
        MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
    );
    let list = definition("list_transfers");
    assert_eq!(list.input_schema["properties"]["limit"]["maximum"], 1_000);
    assert_eq!(
        list.input_schema["properties"]["sessionId"]["maxLength"],
        128
    );
}

#[test]
fn route_schema_distinguishes_fixed_forwards_from_dynamic_socks() {
    let schema = definition("create_tunnel").input_schema;
    assert_eq!(
        schema["properties"]["mode"]["enum"],
        json!(["local", "remote", "dynamic"])
    );
    assert_eq!(schema["properties"]["bindPort"]["minimum"], 0);
    assert_eq!(schema["properties"]["bindPort"]["maximum"], 65_535);
    assert_eq!(
        schema["properties"]["egress"]["enum"],
        json!(["ssh", "portmate-host"])
    );
    assert!(schema["required"].as_array().is_some_and(|required| {
        !required
            .iter()
            .any(|value| value.as_str() == Some("sessionId"))
    }));
    assert!(schema["properties"]["allowRemoteBind"]
        .get("const")
        .is_none());

    let clauses = schema["allOf"].as_array().expect("route schema clauses");
    let target_clause = &clauses[1];
    assert_eq!(
        target_clause["if"]["properties"]["mode"]["const"],
        "dynamic"
    );
    assert_eq!(
        target_clause["then"]["properties"]["targetPort"]["const"],
        0
    );
    assert_eq!(schema["properties"]["routeRules"]["maxItems"], 64);
    assert_eq!(
        schema["properties"]["routeRules"]["items"]["properties"]["port"]["type"],
        json!(["integer", "null"])
    );
    assert_eq!(
        target_clause["else"]["properties"]["routeRules"]["maxItems"],
        0
    );
    assert_eq!(
        target_clause["else"]["required"],
        json!(["targetHost", "targetPort"])
    );
    assert_eq!(
        target_clause["else"]["properties"]["targetPort"]["minimum"],
        1
    );
    assert_eq!(clauses.len(), 4);
    assert_eq!(
        clauses[2]["oneOf"][1]["properties"]["egress"]["const"],
        "portmate-host"
    );
}

#[test]
fn custom_script_tools_select_saved_scripts_without_accepting_script_bodies() {
    let list = definition("list_custom_scripts");
    assert!(list.read_only);
    assert!(list.description.contains("never returned"));
    assert!(list.input_schema["properties"].get("sessionId").is_none());
    assert!(list.input_schema["properties"].get("content").is_none());

    let run = definition("run_custom_script");
    assert!(!run.read_only);
    assert_eq!(run.input_schema["additionalProperties"], false);
    assert_eq!(run.input_schema["required"], json!(["scriptId"]));
    assert_eq!(run.input_schema["properties"]["scriptId"]["format"], "uuid");
    assert!(run.input_schema["properties"].get("content").is_none());
}

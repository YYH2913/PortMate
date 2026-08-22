use super::*;

#[derive(Debug)]
pub(super) enum NormalizedMcpStartTransferRequest {
    Path(StartTransferRequest),
    Inline(StartMcpContentTransferRequest),
    Upload(StartMcpContentUploadTransferRequest),
}

fn normalize_mcp_virtual_content_source(
    args: &serde_json::Value,
) -> Result<StartMcpContentTransferRequest, String> {
    let object = args
        .as_object()
        .ok_or_else(|| "start_transfer arguments must be a JSON object".to_string())?;
    let source = object
        .get("source")
        .cloned()
        .ok_or_else(|| "virtual MCP source is missing".to_string())?;
    let source: McpVirtualFileSource = serde_json::from_value(source)
        .map_err(|error| format!("invalid virtual MCP source: {error}"))?;
    let McpVirtualFileSource::Mcp {
        file_name,
        content_base64,
    } = source;
    let session_id = object
        .get("sessionId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "virtual MCP source requires sessionId".to_string())?
        .to_string();
    let protocol = object
        .get("protocol")
        .cloned()
        .ok_or_else(|| "virtual MCP source requires protocol".to_string())?;
    let protocol: TransferProtocol = serde_json::from_value(protocol)
        .map_err(|error| format!("invalid virtual MCP source protocol: {error}"))?;
    let destination = object
        .get("destination")
        .ok_or_else(|| "virtual MCP source requires destination".to_string())?
        .clone();
    let destination: McpTransferDestination = serde_json::from_value(destination)
        .map_err(|error| format!("invalid virtual MCP destination: {error}"))?;
    let destination = destination.normalize(&protocol)?;
    Ok(StartMcpContentTransferRequest {
        session_id,
        protocol,
        file_name,
        content_base64,
        destination,
    })
}

pub(super) fn normalize_mcp_start_transfer_args(
    args: &serde_json::Value,
) -> Result<NormalizedMcpStartTransferRequest, String> {
    let object = args
        .as_object()
        .ok_or_else(|| "start_transfer arguments must be a JSON object".to_string())?;
    let source_mode = classify_mcp_start_transfer_source(object)?;
    match source_mode {
        McpStartTransferSource::Source
            if object
                .get("source")
                .is_some_and(serde_json::Value::is_object) =>
        {
            normalize_mcp_virtual_content_source(args)
                .map(NormalizedMcpStartTransferRequest::Inline)
        }
        McpStartTransferSource::Source => {
            let normalized = normalize_mcp_start_transfer_destination(args)?;
            serde_json::from_value(normalized)
                .map(NormalizedMcpStartTransferRequest::Path)
                .map_err(|error| format!("invalid path transfer request: {error}"))
        }
        McpStartTransferSource::Inline => {
            let normalized = normalize_mcp_start_transfer_destination(args)?;
            serde_json::from_value(normalized)
                .map(NormalizedMcpStartTransferRequest::Inline)
                .map_err(|error| format!("invalid inline transfer request: {error}"))
        }
        McpStartTransferSource::Upload => serde_json::from_value(args.clone())
            .map(NormalizedMcpStartTransferRequest::Upload)
            .map_err(|error| format!("invalid uploaded transfer request: {error}")),
    }
}

fn normalize_mcp_start_transfer_destination(
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut normalized = args.clone();
    let object = normalized
        .as_object_mut()
        .ok_or_else(|| "start_transfer arguments must be a JSON object".to_string())?;
    let protocol = object
        .get("protocol")
        .cloned()
        .ok_or_else(|| "start_transfer requires protocol".to_string())?;
    let protocol: TransferProtocol = serde_json::from_value(protocol)
        .map_err(|error| format!("invalid start_transfer protocol: {error}"))?;
    let destination = object
        .remove("destination")
        .ok_or_else(|| "start_transfer requires destination".to_string())?;
    let destination: McpTransferDestination = serde_json::from_value(destination)
        .map_err(|error| format!("invalid start_transfer destination: {error}"))?;
    object.insert(
        "destination".to_string(),
        serde_json::Value::String(destination.normalize(&protocol)?),
    );
    Ok(normalized)
}

pub(super) fn decode_mcp_direct_bytes(
    args: &serde_json::Value,
) -> Result<Vec<u8>, String> {
    let object = args
        .as_object()
        .ok_or_else(|| "send_bytes arguments must be a JSON object".to_string())?;
    let encoding = object
        .get("encoding")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "send_bytes encoding must be `base64` or `hex`".to_string())?;
    let data = object
        .get("data")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "send_bytes data must be an encoded string".to_string())?;
    use portmate_core::{MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH, MAX_MCP_CONTENT_TRANSFER_BYTES};
    if data.is_empty() || data.len() > MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH {
        return Err(format!(
            "send_bytes data must be non-empty and at most {MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH} bytes"
        ));
    }
    let decoded = match encoding {
        "base64" => {
            let compact: String = data.chars().filter(|character| !character.is_ascii_whitespace()).collect();
            BASE64_STANDARD
                .decode(compact)
                .map_err(|_| "send_bytes data is not valid standard Base64".to_string())?
        }
        "hex" => decode_mcp_hex("send_bytes", data)?,
        _ => return Err("send_bytes encoding must be `base64` or `hex`".to_string()),
    };
    if decoded.is_empty() || decoded.len() > MAX_MCP_CONTENT_TRANSFER_BYTES {
        return Err(format!(
            "send_bytes payload must contain 1 to {MAX_MCP_CONTENT_TRANSFER_BYTES} decoded bytes"
        ));
    }
    Ok(decoded)
}

pub(super) fn decode_mcp_tunnel_exchange_payload(
    encoding: &str,
    data: &str,
) -> Result<Vec<u8>, String> {
    if data.is_empty() || data.len() > MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH {
        return Err(format!(
            "tunnel_request data must be non-empty and at most {MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH} bytes"
        ));
    }
    let decoded = match encoding {
        "base64" => {
            let compact: String = data
                .chars()
                .filter(|character| !character.is_ascii_whitespace())
                .collect();
            BASE64_STANDARD
                .decode(compact)
                .map_err(|_| "tunnel_request data is not valid standard Base64".to_string())?
        }
        "hex" => decode_mcp_hex("tunnel_request", data)?,
        _ => return Err("tunnel_request encoding must be `base64` or `hex`".to_string()),
    };
    if decoded.is_empty() || decoded.len() > MAX_MCP_TUNNEL_EXCHANGE_BYTES {
        return Err(format!(
            "tunnel_request payload must contain 1 to {MAX_MCP_TUNNEL_EXCHANGE_BYTES} decoded bytes"
        ));
    }
    Ok(decoded)
}

fn decode_mcp_hex(label: &str, data: &str) -> Result<Vec<u8>, String> {
    let compact: String = data.chars().filter(|character| !character.is_ascii_whitespace()).collect();
    if !compact.len().is_multiple_of(2) {
        return Err(format!(
            "{label} hex data must contain an even number of digits"
        ));
    }
    let mut bytes = Vec::with_capacity(compact.len() / 2);
    for pair in compact.as_bytes().chunks_exact(2) {
        let high = hex_digit(pair[0])
            .ok_or_else(|| format!("{label} hex data contains a non-hex digit"))?;
        let low = hex_digit(pair[1])
            .ok_or_else(|| format!("{label} hex data contains a non-hex digit"))?;
        bytes.push((high << 4) | low);
    }
    Ok(bytes)
}

pub(super) fn validate_mcp_tunnel_exchange_request(
    request: &McpTunnelExchangeRequest,
) -> Result<(), String> {
    validate_mcp_operation_id(&request.tunnel_id, "tunnel")?;
    decode_mcp_tunnel_exchange_payload(&request.encoding, &request.data)?;
    if let Some(timeout_ms) = request.timeout_ms {
        if !(100..=MAX_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS).contains(&timeout_ms) {
            return Err(format!(
                "tunnel_request timeoutMs must be between 100 and {MAX_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS}"
            ));
        }
    }
    if let Some(max_response_bytes) = request.max_response_bytes {
        if max_response_bytes == 0 || max_response_bytes > MAX_MCP_TUNNEL_EXCHANGE_BYTES {
            return Err(format!(
                "tunnel_request maxResponseBytes must be between 1 and {MAX_MCP_TUNNEL_EXCHANGE_BYTES}"
            ));
        }
    }
    if request.target_port == Some(0) {
        return Err("tunnel_request targetPort must be between 1 and 65535".to_string());
    }
    if let Some(target_host) = request.target_host.as_deref() {
        if target_host.trim().is_empty()
            || target_host.len() > MAX_TUNNEL_HOST_CHARACTERS
            || target_host
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(
                "tunnel_request targetHost must be a valid host without surrounding whitespace"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
pub(super) enum McpWriteExecutionContext {
    #[default]
    Generic,
    CustomScript {
        script_id: String,
        updated_at: DateTime<Utc>,
        approval_target: McpApprovalTarget,
    },
    Tunnel {
        approval_target: McpApprovalTarget,
    },
    TunnelExchange {
        tunnel_id: String,
        owner_id: String,
        runtime_id: String,
        approval_target: McpApprovalTarget,
    },
}

impl McpWriteExecutionContext {
    pub(super) fn approval_target(&self) -> Option<McpApprovalTarget> {
        match self {
            Self::Generic => None,
            Self::CustomScript { approval_target, .. }
            | Self::Tunnel { approval_target }
            | Self::TunnelExchange { approval_target, .. } => {
                Some(approval_target.clone())
            }
        }
    }

    pub(super) fn custom_script_updated_at(
        &self,
        script_id: &str,
    ) -> Result<DateTime<Utc>, String> {
        match self {
            Self::CustomScript {
                script_id: expected_id,
                updated_at,
                ..
            } if expected_id == script_id => Ok(*updated_at),
            _ => Err(
                "MCP custom script execution is missing its authorized script version"
                    .to_string(),
            ),
        }
    }

    pub(super) fn revalidate(
        &self,
        state: &AppState,
        request: &IpcRequest,
    ) -> Result<(), String> {
        if let Self::TunnelExchange {
            tunnel_id,
            owner_id,
            runtime_id,
            ..
        } = self
        {
            if request.command != "tunnel_request"
                || ipc_string_arg(&request.args, "tunnelId")? != tunnel_id
            {
                return Err("MCP tunnel target changed after authorization; request was not executed".to_string());
            }
            let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
            let current = tunnels
                .get(tunnel_id)
                .filter(|runtime| {
                    runtime.session_id == *owner_id
                        && runtime.ssh_runtime_id == *runtime_id
                        && runtime.spec.egress == TunnelEgress::PortmateHost
                        && !runtime.closed.load(Ordering::SeqCst)
                })
                .ok_or_else(|| "MCP tunnel was stopped or replaced after authorization".to_string())?;
            let target_host = request.args.get("targetHost").and_then(serde_json::Value::as_str);
            let target_port = request.args.get("targetPort").and_then(serde_json::Value::as_u64);
            if current.spec.mode == TunnelMode::Dynamic {
                if target_host.is_none() || target_port.is_none() {
                    return Err("dynamic MCP tunnel requests require targetHost and targetPort".to_string());
                }
            } else if target_host.is_some() || target_port.is_some() {
                return Err("fixed MCP tunnel requests must not override targetHost or targetPort".to_string());
            }
            return Ok(());
        }
        let Self::CustomScript {
            script_id,
            updated_at,
            ..
        } = self
        else {
            return Ok(());
        };
        let session_id = ipc_string_arg(&request.args, "sessionId")?;
        let current_script_id = ipc_string_arg(&request.args, "scriptId")?;
        if current_script_id != script_id {
            return Err(
                "MCP custom script target changed after authorization; request was not executed"
                    .to_string(),
            );
        }
        let store = state.store.lock().map_err(|error| error.to_string())?;
        let script = custom_script_for_session(&store, script_id, session_id, true)?;
        if script.updated_at != *updated_at {
            return Err(
                "MCP custom script changed after authorization; review and approve it again"
                    .to_string(),
            );
        }
        Ok(())
    }
}

pub(super) fn capture_mcp_write_execution_context(
    state: &AppState,
    request: &IpcRequest,
) -> Result<McpWriteExecutionContext, String> {
    if request.command == "tunnel_request" {
        let exchange: McpTunnelExchangeRequest = serde_json::from_value(request.args.clone())
            .map_err(|error| format!("invalid MCP tunnel request: {error}"))?;
        validate_mcp_tunnel_exchange_request(&exchange)?;
        let owner_id = mcp_host_route_owner_id(&request.client_id)?;
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        let runtime = tunnels
            .get(&exchange.tunnel_id)
            .filter(|runtime| {
                runtime.session_id == owner_id
                    && runtime.spec.egress == TunnelEgress::PortmateHost
                    && !runtime.closed.load(Ordering::SeqCst)
            })
            .ok_or_else(|| "host route not found or owned by another MCP client".to_string())?;
        let target = if runtime.spec.mode == TunnelMode::Dynamic {
            format!(
                "{}:{}",
                exchange.target_host.as_deref().unwrap_or("<missing>"),
                exchange.target_port.unwrap_or(0)
            )
        } else {
            format!("{}:{}", runtime.spec.target_host, runtime.spec.target_port)
        };
        return Ok(McpWriteExecutionContext::TunnelExchange {
            tunnel_id: exchange.tunnel_id,
            owner_id,
            runtime_id: runtime.ssh_runtime_id.clone(),
            approval_target: McpApprovalTarget {
                kind: "portmate-host-tunnel-request".to_string(),
                id: runtime.spec.id.clone(),
                label: format!("Request through PortMate host tunnel to {target}"),
            },
        });
    }
    let host_route = match request.command.as_str() {
        "create_tunnel" => match normalize_mcp_tunnel_request(
            serde_json::from_value::<CreateMcpTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?,
        )? {
            NormalizedMcpTunnelRequest::PortmateHost(route) => Some(route),
            NormalizedMcpTunnelRequest::Ssh(_) => None,
        },
        _ => None,
    };
    if let Some(route) = host_route {
        {
            let target = if route.mode == TunnelMode::Dynamic {
                let mut routes = route
                    .route_rules
                    .iter()
                    .take(2)
                    .map(|rule| match rule.port {
                        Some(port) => format!("{}:{port}", bounded_approval_host(&rule.host)),
                        None => bounded_approval_host(&rule.host),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                if route.route_rules.len() > 2 {
                    routes.push_str(&format!(" +{} more", route.route_rules.len() - 2));
                }
                routes
            } else {
                format!("{}:{}", route.target_host, route.target_port)
            };
            let proxy_kind = match route.mode {
                TunnelMode::Local => "TCP",
                TunnelMode::Dynamic => "SOCKS5",
                TunnelMode::Remote => unreachable!("host egress remote mode was validated"),
            };
            return Ok(McpWriteExecutionContext::Tunnel {
                approval_target: McpApprovalTarget {
                    kind: "portmate-host-proxy".to_string(),
                    id: format!("{}:{}", route.bind_host, route.bind_port),
                    label: format!(
                        "PortMate host {proxy_kind} proxy to {target}{}",
                        if route.allow_remote_bind {
                            " (remote listener allowed)"
                        } else {
                            ""
                        }
                    ),
                },
            });
        }
    }
    if request.command != "run_custom_script" {
        return Ok(McpWriteExecutionContext::Generic);
    }
    let session_id = ipc_string_arg(&request.args, "sessionId")?;
    let script_id = ipc_string_arg(&request.args, "scriptId")?;
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let script = custom_script_for_session(&store, script_id, session_id, true)?;
    Ok(McpWriteExecutionContext::CustomScript {
        script_id: script.id.clone(),
        updated_at: script.updated_at,
        approval_target: McpApprovalTarget {
            kind: "custom-script".to_string(),
            id: script.id,
            label: script.name,
        },
    })
}

fn bounded_approval_host(host: &str) -> String {
    const MAX_APPROVAL_HOST_CHARACTERS: usize = 80;
    let mut value = host
        .chars()
        .take(MAX_APPROVAL_HOST_CHARACTERS)
        .collect::<String>();
    if host.chars().count() > MAX_APPROVAL_HOST_CHARACTERS {
        value.push_str("...");
    }
    value
}

pub(super) fn ipc_write_scope(command: &str) -> Option<McpScope> {
    match command {
        "send_text" | "send_key" | "send_bytes" | "serial_send_break" | "run_command" | "run_local_command" | "attach_tmux" => {
            Some(McpScope::WriteInput)
        }
        "start_transfer" | "cancel_transfer" | "retry_transfer" => Some(McpScope::Transfer),
        "create_tunnel" | "stop_tunnel" | "tunnel_request" => Some(McpScope::Tunnel),
        "run_custom_script" => Some(McpScope::RunScripts),
        "restart_mcp_http" => Some(McpScope::ManageMcp),
        _ => None,
    }
}

pub(super) fn ipc_read_scope(command: &str) -> Option<McpScope> {
    match command {
        "list_sessions" => Some(McpScope::ReadSessions),
        "read_screen"
        | "tail_log"
        | "search_logs"
        | "list_tmux_state"
        | "export_session_bundle" => Some(McpScope::ReadLogs),
        "list_transfers" | "get_transfer" => Some(McpScope::ReadTransfers),
        "list_tunnels" => Some(McpScope::ReadTunnels),
        "list_custom_scripts" => Some(McpScope::ReadScripts),
        "mcp_http_runtime_status" => Some(McpScope::ReadMcp),
        _ => None,
    }
}

pub(super) fn validate_mcp_session_id(session_id: &str) -> Result<(), String> {
    if session_id.is_empty()
        || session_id.len() > MAX_MCP_GRANT_SESSION_ID_BYTES
        || session_id.chars().any(char::is_control)
    {
        return Err(format!(
            "MCP session ID must be non-empty, printable, and at most {MAX_MCP_GRANT_SESSION_ID_BYTES} bytes"
        ));
    }
    Ok(())
}

pub(super) fn ensure_shell_profile(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<(), String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profile(session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    if !matches!(profile.connection, ConnectionConfig::Shell(_)) {
        return Err("run_local_command requires a local Shell session".to_string());
    }
    Ok(())
}

pub(super) fn validate_ipc_write_args(
    state: &AppState,
    request: &IpcRequest,
) -> Result<(), String> {
    match request.command.as_str() {
        "tunnel_request" => {
            let exchange: McpTunnelExchangeRequest = serde_json::from_value(request.args.clone())
                .map_err(|error| format!("invalid MCP tunnel request: {error}"))?;
            validate_mcp_tunnel_exchange_request(&exchange)?;
            let owner_id = mcp_host_route_owner_id(&request.client_id)?;
            let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
            let runtime = tunnels
                .get(&exchange.tunnel_id)
                .filter(|runtime| {
                    runtime.session_id == owner_id
                        && runtime.spec.egress == TunnelEgress::PortmateHost
                        && !runtime.closed.load(Ordering::SeqCst)
                })
                .ok_or_else(|| "host route not found or owned by another MCP client".to_string())?;
            if runtime.spec.mode == TunnelMode::Dynamic {
                let host = exchange
                    .target_host
                    .as_deref()
                    .ok_or_else(|| "dynamic MCP tunnel requests require targetHost and targetPort".to_string())?;
                let port = exchange
                    .target_port
                    .ok_or_else(|| "dynamic MCP tunnel requests require targetHost and targetPort".to_string())?;
                if !tunnel_route_allowed(&runtime.spec.route_rules, host, port) {
                    return Err(format!("MCP tunnel target denied by route rules: {host}:{port}"));
                }
            } else if exchange.target_host.is_some() || exchange.target_port.is_some() {
                return Err("fixed MCP tunnel requests must not override targetHost or targetPort".to_string());
            }
        }
        "send_text" => {
            ipc_string_arg(&request.args, "text")?;
        }
        "send_key" => {
            ipc_string_arg(&request.args, "key")?;
        }
        "send_bytes" => {
            decode_mcp_direct_bytes(&request.args)?;
        }
        "serial_send_break" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?;
            ensure_serial_profile(&state.store, session_id)?;
        }
        "run_command" => {
            ipc_string_arg(&request.args, "command")?;
        }
        "run_local_command" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?;
            ensure_shell_profile(&state.store, session_id)?;
            ipc_string_arg(&request.args, "command")?;
        }
        "run_custom_script" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?;
            let script_id = ipc_string_arg(&request.args, "scriptId")?;
            let store = state.store.lock().map_err(|error| error.to_string())?;
            custom_script_for_session(&store, script_id, session_id, true)?;
        }
        "start_transfer" => {
            match normalize_mcp_start_transfer_args(&request.args)? {
                NormalizedMcpStartTransferRequest::Path(transfer) => {
                    validate_mcp_transfer_route(&transfer)?;
                }
                NormalizedMcpStartTransferRequest::Inline(transfer) => {
                    validate_mcp_content_transfer_request(&transfer)?;
                }
                NormalizedMcpStartTransferRequest::Upload(transfer) => {
                    let metadata = load_mcp_content_upload_metadata(
                        state,
                        &request.client_id,
                        &transfer.upload_id,
                    )?;
                    validate_mcp_uploaded_content_route(&metadata)?;
                }
            }
        }
        "cancel_transfer" => {
            validate_mcp_operation_id(ipc_string_arg(&request.args, "transferId")?, "transfer")?;
        }
        "retry_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?;
            validate_mcp_operation_id(transfer_id, "transfer")?;
            let transfer = state
                .store
                .lock()
                .map_err(|error| error.to_string())?
                .transfer_by_id(transfer_id)
                .ok_or_else(|| "unknown or unavailable transfer".to_string())?;
            validate_mcp_transfer_route(&StartTransferRequest {
                session_id: transfer.session_id,
                protocol: transfer.protocol,
                source: transfer.source,
                destination: transfer.destination,
            })?;
        }
        "create_tunnel" => {
            let tunnel = serde_json::from_value::<CreateMcpTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
            normalize_mcp_tunnel_request(tunnel)?;
        }
        "stop_tunnel" => {
            validate_mcp_operation_id(ipc_string_arg(&request.args, "tunnelId")?, "tunnel")?;
        }
        "attach_tmux" => {
            ipc_string_arg(&request.args, "target")?;
        }
        "restart_mcp_http" => {}
        _ => {
            return Err(format!(
                "unsupported IPC write command: {}",
                request.command
            ))
        }
    }
    Ok(())
}

fn validate_mcp_operation_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(format!(
            "MCP {label} ID must be non-empty, printable, and at most 128 bytes"
        ));
    }
    Ok(())
}

pub(super) fn ipc_write_session_id(
    state: &AppState,
    request: &IpcRequest,
) -> Result<Option<String>, String> {
    match request.command.as_str() {
        "tunnel_request" => {
            validate_mcp_operation_id(ipc_string_arg(&request.args, "tunnelId")?, "tunnel")?;
            Ok(None)
        }
        "restart_mcp_http" => Ok(None),
        "create_tunnel" => {
            let tunnel = serde_json::from_value::<CreateMcpTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
            match normalize_mcp_tunnel_request(tunnel)? {
                NormalizedMcpTunnelRequest::Ssh(tunnel) => Ok(Some(tunnel.session_id)),
                NormalizedMcpTunnelRequest::PortmateHost(_) => Ok(None),
            }
        }
        "start_transfer" => match normalize_mcp_start_transfer_args(&request.args)? {
            NormalizedMcpStartTransferRequest::Path(transfer) => Ok(Some(transfer.session_id)),
            NormalizedMcpStartTransferRequest::Inline(transfer) => {
                Ok(Some(transfer.session_id))
            }
            NormalizedMcpStartTransferRequest::Upload(transfer) => {
                Ok(Some(load_mcp_content_upload_metadata(
                    state,
                    &request.client_id,
                    &transfer.upload_id,
                )?
                .session_id))
            }
        },
        "cancel_transfer" | "retry_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?;
            validate_mcp_operation_id(transfer_id, "transfer")?;
            state
                .store
                .lock()
                .map_err(|error| error.to_string())?
                .transfer_by_id(transfer_id)
                .map(|transfer| transfer.session_id)
                .map(Some)
                .ok_or_else(|| "unknown or unavailable transfer".to_string())
        }
        "stop_tunnel" => {
            let tunnel_id = ipc_string_arg(&request.args, "tunnelId")?;
            validate_mcp_operation_id(tunnel_id, "tunnel")?;
            let tunnels = state
                .tunnels
                .lock()
                .map_err(|error| error.to_string())?;
            let runtime = tunnels
                .get(tunnel_id)
                .filter(|runtime| !runtime.closed.load(Ordering::SeqCst))
                .ok_or_else(|| "unknown or unavailable tunnel".to_string())?;
            if runtime.spec.egress == TunnelEgress::PortmateHost {
                let owner = mcp_host_route_owner_id(&request.client_id)?;
                if runtime.session_id != owner {
                    return Err("host route not found or owned by another MCP client".to_string());
                }
                Ok(None)
            } else {
                Ok(Some(runtime.session_id.clone()))
            }
        }
        _ => Ok(Some(
            ipc_string_arg(&request.args, "sessionId")?.to_string(),
        )),
    }
}

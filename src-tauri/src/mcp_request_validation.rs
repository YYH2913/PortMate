use super::*;

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
}

impl McpWriteExecutionContext {
    pub(super) fn approval_target(&self) -> Option<McpApprovalTarget> {
        match self {
            Self::Generic => None,
            Self::CustomScript { approval_target, .. } | Self::Tunnel { approval_target } => {
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
    if request.command == "create_host_route" {
        let route = normalize_host_route_request(
            serde_json::from_value::<CreateHostRouteRequest>(request.args.clone())
                .map_err(|error| format!("invalid host route request: {error}"))?,
        )?;
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
        "send_text" | "send_key" | "run_command" | "attach_tmux" => Some(McpScope::WriteInput),
        "open_session" | "close_session" => Some(McpScope::ManageSessions),
        "start_transfer"
        | "start_content_transfer"
        | "start_content_upload_transfer"
        | "cancel_transfer"
        | "retry_transfer" => Some(McpScope::Transfer),
        "create_tunnel" | "stop_tunnel" | "create_host_route" | "stop_host_route" => {
            Some(McpScope::Tunnel)
        }
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
        "list_tunnels" | "list_host_routes" => Some(McpScope::ReadTunnels),
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

pub(super) fn validate_ipc_write_args(
    state: &AppState,
    request: &IpcRequest,
) -> Result<(), String> {
    match request.command.as_str() {
        "send_text" => {
            ipc_string_arg(&request.args, "text")?;
        }
        "send_key" => {
            ipc_string_arg(&request.args, "key")?;
        }
        "run_command" => {
            ipc_string_arg(&request.args, "command")?;
        }
        "run_custom_script" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?;
            let script_id = ipc_string_arg(&request.args, "scriptId")?;
            let store = state.store.lock().map_err(|error| error.to_string())?;
            custom_script_for_session(&store, script_id, session_id, true)?;
        }
        "start_transfer" => {
            let transfer = serde_json::from_value::<StartTransferRequest>(request.args.clone())
                .map_err(|error| format!("invalid transfer request: {error}"))?;
            validate_mcp_transfer_route(&transfer)?;
        }
        "start_content_transfer" => {
            let transfer = serde_json::from_value::<StartMcpContentTransferRequest>(
                request.args.clone(),
            )
            .map_err(|error| format!("invalid content transfer request: {error}"))?;
            validate_mcp_content_transfer_request(&transfer)?;
        }
        "start_content_upload_transfer" => {
            let transfer = serde_json::from_value::<StartMcpContentUploadTransferRequest>(
                request.args.clone(),
            )
            .map_err(|error| format!("invalid uploaded content transfer request: {error}"))?;
            let metadata = load_mcp_content_upload_metadata(
                state,
                &request.client_id,
                &transfer.upload_id,
            )?;
            validate_mcp_uploaded_content_route(&metadata)?;
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
            let tunnel = serde_json::from_value::<CreateTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
            let tunnel = normalize_tunnel_request(tunnel)?;
            if tunnel.egress != TunnelEgress::Ssh {
                return Err(
                    "create_tunnel only supports SSH/Tmux routes; use create_host_route for PortMate host routes"
                        .to_string(),
                );
            }
        }
        "create_host_route" => {
            let route = serde_json::from_value::<CreateHostRouteRequest>(request.args.clone())
                .map_err(|error| format!("invalid host route request: {error}"))?;
            normalize_host_route_request(route)?;
        }
        "stop_tunnel" | "stop_host_route" => {
            validate_mcp_operation_id(ipc_string_arg(&request.args, "tunnelId")?, "tunnel")?;
        }
        "attach_tmux" => {
            ipc_string_arg(&request.args, "target")?;
        }
        "open_session" | "close_session" | "restart_mcp_http" => {}
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
        "create_host_route" | "stop_host_route" | "restart_mcp_http" => Ok(None),
        "start_content_upload_transfer" => {
            let transfer = serde_json::from_value::<StartMcpContentUploadTransferRequest>(
                request.args.clone(),
            )
            .map_err(|error| format!("invalid uploaded content transfer request: {error}"))?;
            Ok(Some(load_mcp_content_upload_metadata(
                state,
                &request.client_id,
                &transfer.upload_id,
            )?
            .session_id))
        }
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
            state
                .tunnels
                .lock()
                .map_err(|error| error.to_string())?
                .get(tunnel_id)
                .filter(|runtime| !runtime.closed.load(Ordering::SeqCst))
                .map(|runtime| runtime.session_id.clone())
                .map(Some)
                .ok_or_else(|| "unknown or unavailable tunnel".to_string())
        }
        _ => Ok(Some(
            ipc_string_arg(&request.args, "sessionId")?.to_string(),
        )),
    }
}

use super::*;

const DEFAULT_MCP_TRANSFER_QUERY_LIMIT: u64 = 100;
const MAX_MCP_TRANSFER_QUERY_LIMIT: u64 = 1000;

fn bounded_mcp_transfer_query_limit(limit: Option<u64>) -> usize {
    limit
        .unwrap_or(DEFAULT_MCP_TRANSFER_QUERY_LIMIT)
        .clamp(1, MAX_MCP_TRANSFER_QUERY_LIMIT) as usize
}

pub(super) async fn execute_ipc_request(
    state: AppState,
    request: IpcRequest,
) -> Result<serde_json::Value, String> {
    execute_ipc_request_inner(state, request, None, None).await
}

pub(super) async fn execute_ipc_request_with_context(
    state: AppState,
    request: IpcRequest,
    execution_context: &McpWriteExecutionContext,
    authorization_context: &McpWriteAuthorizationContext,
) -> Result<serde_json::Value, String> {
    execute_ipc_request_inner(
        state,
        request,
        Some(execution_context),
        Some(authorization_context),
    )
    .await
}

async fn execute_ipc_request_inner(
    state: AppState,
    request: IpcRequest,
    execution_context: Option<&McpWriteExecutionContext>,
    authorization_context: Option<&McpWriteAuthorizationContext>,
) -> Result<serde_json::Value, String> {
    match request.command.as_str() {
        "mcp_http_runtime_status" => {
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(&store, &request, McpScope::ReadMcp, None)?;
            drop(store);
            serde_json::to_value(mcp_http_runtime_status_inner(&state)?)
                .map_err(|error| error.to_string())
        }
        "list_sessions" => {
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(&store, &request, McpScope::ReadSessions, None)?;
            let summaries = store
                .summaries()
                .into_iter()
                .filter(|summary| {
                    store.mcp_can_read(
                        &request.client_id,
                        McpScope::ReadSessions,
                        Some(&summary.profile.id),
                    )
                })
                .map(redact_session_summary)
                .collect::<Vec<_>>();
            serde_json::to_value(summaries).map_err(|error| error.to_string())
        }
        "read_screen" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(&store, &request, McpScope::ReadLogs, Some(&session_id))?;
            let screen = redact_secrets(&store.screen(&session_id).unwrap_or_default());
            Ok(serde_json::json!(screen))
        }
        "tail_log" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let limit = request
                .args
                .get("limit")
                .and_then(serde_json::Value::as_u64);
            let limit = bounded_log_query_limit(limit);
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(&store, &request, McpScope::ReadLogs, Some(&session_id))?;
            serde_json::to_value(redact_session_events(store.tail_log(&session_id, limit)))
                .map_err(|error| error.to_string())
        }
        "search_logs" => {
            let query = ipc_string_arg(&request.args, "query")?.to_string();
            let session_id = request
                .args
                .get("sessionId")
                .and_then(serde_json::Value::as_str);
            let limit = request
                .args
                .get("limit")
                .and_then(serde_json::Value::as_u64);
            let limit = bounded_log_query_limit(limit);
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(&store, &request, McpScope::ReadLogs, session_id)?;
            let events = store
                .search_logs(&query, session_id, limit)
                .into_iter()
                .filter(|event| {
                    store.mcp_can_read(
                        &request.client_id,
                        McpScope::ReadLogs,
                        Some(&event.session_id),
                    )
                })
                .collect();
            serde_json::to_value(redact_session_events(events)).map_err(|error| error.to_string())
        }
        "list_transfers" => {
            let session_id = request
                .args
                .get("sessionId")
                .and_then(serde_json::Value::as_str);
            let limit = bounded_mcp_transfer_query_limit(
                request.args.get("limit").and_then(serde_json::Value::as_u64),
            );
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(&store, &request, McpScope::ReadTransfers, session_id)?;
            let mut transfers = store
                .transfers
                .iter()
                .filter(|transfer| {
                    session_id.is_none_or(|session_id| transfer.session_id == session_id)
                        && store.mcp_can_read(
                            &request.client_id,
                            McpScope::ReadTransfers,
                            Some(&transfer.session_id),
                        )
                })
                .cloned()
                .map(redact_transfer_task)
                .collect::<Vec<_>>();
            if transfers.len() > limit {
                transfers.drain(..transfers.len() - limit);
            }
            serde_json::to_value(transfers).map_err(|error| error.to_string())
        }
        "get_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?;
            let store = state.store.lock().map_err(|error| error.to_string())?;
            let transfer = store
                .transfer_by_id(transfer_id)
                .ok_or_else(|| "unknown or unavailable transfer".to_string())?;
            require_mcp_read_scope(
                &store,
                &request,
                McpScope::ReadTransfers,
                Some(&transfer.session_id),
            )?;
            serde_json::to_value(redact_transfer_task(transfer)).map_err(|error| error.to_string())
        }
        "list_custom_scripts" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(
                &store,
                &request,
                McpScope::ReadScripts,
                Some(&session_id),
            )?;
            let scripts = store
                .custom_scripts
                .iter()
                .filter(|script| script.mcp_enabled && script.allows_session(&session_id))
                .map(CustomScript::summary)
                .collect::<Vec<_>>();
            serde_json::to_value(scripts).map_err(|error| error.to_string())
        }
        "send_bytes" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let bytes = decode_mcp_direct_bytes(&request.args)?;
            let actor = mcp_audit_actor(&request.client_id);
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let event = send_bytes_inner_with_context(
                state.session_io(),
                session_id,
                bytes,
                &actor,
                Some("send_bytes"),
                Some(validation),
            )
            .await?;
            serde_json::to_value(redact_session_event(event)).map_err(|error| error.to_string())
        }
        "send_text" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let text = ipc_string_arg(&request.args, "text")?.to_string();
            let actor = mcp_audit_actor(&request.client_id);
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let event = send_text_inner_with_context_and_validation(
                state.session_io(),
                session_id,
                text,
                &actor,
                None,
                Some(validation),
            )
            .await?;
            serde_json::to_value(redact_session_event(event)).map_err(|error| error.to_string())
        }
        "send_key" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let key = ipc_string_arg(&request.args, "key")?.to_string();
            let text = terminal_key_sequence_for_protocol(
                &key,
                is_telnet_session(&state.store, &session_id)?,
            )?;
            let actor = mcp_audit_actor(&request.client_id);
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let event = send_text_inner_with_context_and_validation(
                state.session_io(),
                session_id,
                text,
                &actor,
                None,
                Some(validation),
            )
            .await?;
            serde_json::to_value(redact_session_event(event)).map_err(|error| error.to_string())
        }
        "serial_send_break" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let break_state = state.clone();
            let break_session_id = session_id.clone();
            tauri::async_runtime::spawn_blocking(move || {
                serial_send_break_inner_with_validation(
                    &break_state,
                    &break_session_id,
                    Some(validation),
                )
            })
            .await
            .map_err(|error| format!("serial Break task failed: {error}"))??;
            Ok(serde_json::json!({
                "sessionId": session_id,
                "sent": true,
            }))
        }
        "run_command" | "run_local_command" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            if request.command == "run_local_command" {
                ensure_shell_profile(&state.store, &session_id)?;
            }
            let command = ipc_string_arg(&request.args, "command")?.to_string();
            let text = terminate_command_for_session(command, &state.store, &session_id)?;
            let actor = mcp_audit_actor(&request.client_id);
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let event = run_command_inner_with_context_and_validation(
                state.session_io(),
                session_id,
                text,
                &actor,
                None,
                Some(validation),
            )
            .await?;
            serde_json::to_value(redact_session_event(event)).map_err(|error| error.to_string())
        }
        "run_custom_script" => {
            let script_id = ipc_string_arg(&request.args, "scriptId")?.to_string();
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let expected_updated_at = execution_context
                .ok_or_else(|| {
                    "MCP custom script execution is missing its authorization context"
                        .to_string()
                })?
                .custom_script_updated_at(&script_id)?;
            let actor = mcp_audit_actor(&request.client_id);
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let event = run_custom_script_inner(
                &state,
                RunCustomScriptRequest {
                    script_id,
                    session_id,
                    expected_updated_at,
                },
                &actor,
                None,
                true,
                Some(validation),
            )
            .await?;
            serde_json::to_value(redact_session_event(event)).map_err(|error| error.to_string())
        }
        "start_transfer" => {
            let transfer = normalize_mcp_start_transfer_args(&request.args)
                .map_err(|error| format!("invalid transfer request: {error}"))?;
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let task = execute_mcp_start_transfer(
                &state,
                &request.client_id,
                transfer,
                validation,
            )
            .await?;
            serde_json::to_value(redact_transfer_task(task)).map_err(|error| error.to_string())
        }
        "cancel_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?.to_string();
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let task =
                cancel_transfer_inner_with_validation(&state, &transfer_id, Some(validation))?;
            serde_json::to_value(redact_transfer_task(task)).map_err(|error| error.to_string())
        }
        "retry_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?.to_string();
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let task = retry_transfer_inner_with_validation(
                &state,
                &transfer_id,
                Some(validation),
            )
            .await?;
            serde_json::to_value(redact_transfer_task(task)).map_err(|error| error.to_string())
        }
        "create_tunnel" => {
            let tunnel = serde_json::from_value::<CreateMcpTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let spec = match normalize_mcp_tunnel_request(tunnel)? {
                NormalizedMcpTunnelRequest::Ssh(tunnel) => {
                    create_tunnel_inner_with_validation(&state, tunnel, Some(validation)).await?
                }
                NormalizedMcpTunnelRequest::PortmateHost(route) => {
                    create_host_route_inner_with_validation(
                        &state,
                        &request.client_id,
                        route,
                        Some(validation),
                    )
                    .await?
                }
            };
            serde_json::to_value(redact_mcp_tunnel_spec(spec)).map_err(|error| error.to_string())
        }
        "list_tunnels" => {
            let session_id = request
                .args
                .get("sessionId")
                .and_then(serde_json::Value::as_str);
            let egress = request
                .args
                .get("egress")
                .and_then(serde_json::Value::as_str);
            if egress == Some("portmate-host") && session_id.is_some() {
                return Err(
                    "PortMate host route listing is session-independent; omit sessionId".to_string(),
                );
            }
            if egress == Some("ssh") && session_id.is_none() {
                return Err(
                    "SSH tunnel listing requires sessionId; use egress `portmate-host` for host routes"
                        .to_string(),
                );
            }
            let statuses = if let Some(session_id) = session_id {
                {
                    let store = state.store.lock().map_err(|error| error.to_string())?;
                    require_mcp_read_scope(
                        &store,
                        &request,
                        McpScope::ReadTunnels,
                        Some(session_id),
                    )?;
                }
                list_tunnels_inner(&state, Some(session_id))?
            } else {
                {
                    let store = state.store.lock().map_err(|error| error.to_string())?;
                    require_mcp_read_scope(&store, &request, McpScope::ReadTunnels, None)?;
                }
                list_host_routes_inner(&state, &request.client_id)?
            }
            .into_iter()
            .map(redact_mcp_tunnel_status)
            .collect::<Vec<_>>();
            serde_json::to_value(statuses).map_err(|error| error.to_string())
        }
        "stop_tunnel" => {
            let tunnel_id = ipc_string_arg(&request.args, "tunnelId")?.to_string();
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let host_route = {
                let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
                tunnels.get(&tunnel_id).is_some_and(|runtime| {
                    runtime.spec.egress == TunnelEgress::PortmateHost
                })
            };
            let status = if host_route {
                stop_host_route_inner_with_validation(
                    &state,
                    &request.client_id,
                    &tunnel_id,
                    Some(validation),
                )
                .await?
            } else {
                stop_tunnel_inner_with_validation(&state, &tunnel_id, Some(validation)).await?
            };
            serde_json::to_value(redact_mcp_tunnel_status(status))
                .map_err(|error| error.to_string())
        }
        "tunnel_request" => {
            let exchange = serde_json::from_value::<McpTunnelExchangeRequest>(request.args.clone())
                .map_err(|error| format!("invalid MCP tunnel request: {error}"))?;
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let result = execute_tunnel_request_inner(
                &state,
                &request.client_id,
                exchange,
                Some(validation),
            )
            .await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "udp_request" => {
            let exchange = serde_json::from_value::<McpUdpExchangeRequest>(request.args.clone())
                .map_err(|error| format!("invalid UDP tunnel request: {error}"))?;
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let result = execute_udp_tunnel_request_inner(
                &state,
                &request.client_id,
                exchange,
                Some(validation),
            )
            .await?;
            serde_json::to_value(result).map_err(|error| error.to_string())
        }
        "restart_mcp_http" => {
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            validation()?;
            serde_json::to_value(restart_mcp_http_runtime_inner(&state).await?)
                .map_err(|error| error.to_string())
        }
        "list_tmux_state" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            {
                let store = state.store.lock().map_err(|error| error.to_string())?;
                require_mcp_read_scope(&store, &request, McpScope::ReadLogs, Some(&session_id))?;
            }
            let tmux = redact_mcp_tmux_state(list_tmux_state_inner(&state, &session_id).await?);
            serde_json::to_value(tmux).map_err(|error| error.to_string())
        }
        "attach_tmux" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let target = ipc_string_arg(&request.args, "target")?.to_string();
            let command = tmux_attach_command(&target)?;
            let actor = mcp_audit_actor(&request.client_id);
            let validation = mcp_commit_validation(
                &state,
                &request,
                execution_context,
                authorization_context,
            )?;
            let event = send_text_inner_with_context_and_validation(
                state.session_io(),
                session_id,
                command,
                &actor,
                None,
                Some(validation),
            )
            .await?;
            serde_json::to_value(redact_session_event(event)).map_err(|error| error.to_string())
        }
        "export_session_bundle" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(&store, &request, McpScope::ReadLogs, Some(&session_id))?;
            Ok(store.export_session_bundle_redacted(&session_id))
        }
        other => Err(format!("unsupported IPC command: {other}")),
    }
}

async fn execute_mcp_start_transfer(
    state: &AppState,
    client_id: &str,
    request: NormalizedMcpStartTransferRequest,
    validation: CommitValidation,
) -> Result<TransferTask, String> {
    match request {
        NormalizedMcpStartTransferRequest::Path(transfer) => {
            start_transfer_inner_with_validation(state, transfer, Some(validation)).await
        }
        NormalizedMcpStartTransferRequest::Inline(content_request) => {
            let (source, staging_path) = stage_mcp_content_transfer(state, &content_request)?;
            let transfer = StartTransferRequest {
                session_id: content_request.session_id,
                protocol: content_request.protocol,
                source,
                destination: content_request.destination,
            };
            match start_transfer_inner_with_staging(
                state,
                transfer,
                Some(staging_path.clone()),
                Some(validation),
            )
            .await
            {
                Ok(task) => Ok(task),
                Err(error) => {
                    cleanup_staged_mcp_content_path(&staging_path);
                    Err(error)
                }
            }
        }
        NormalizedMcpStartTransferRequest::Upload(upload) => {
            let metadata =
                load_mcp_content_upload_metadata(state, client_id, &upload.upload_id)?;
            let staging_state = state.clone();
            let staging_metadata = metadata.clone();
            let (source, staging_path) = tauri::async_runtime::spawn_blocking(move || {
                stage_mcp_content_upload(&staging_state, &staging_metadata)
            })
            .await
            .map_err(|error| format!("MCP content upload staging task failed: {error}"))??;
            let transfer = StartTransferRequest {
                session_id: metadata.session_id,
                protocol: metadata.protocol,
                source,
                destination: metadata.destination,
            };
            match start_transfer_inner_with_staging(
                state,
                transfer,
                Some(staging_path.clone()),
                Some(validation),
            )
            .await
            {
                Ok(task) => Ok(task),
                Err(error) => {
                    cleanup_staged_mcp_content_path(&staging_path);
                    Err(error)
                }
            }
        }
    }
}

fn mcp_commit_validation(
    state: &AppState,
    request: &IpcRequest,
    execution_context: Option<&McpWriteExecutionContext>,
    authorization_context: Option<&McpWriteAuthorizationContext>,
) -> Result<CommitValidation, String> {
    let execution_context = execution_context
        .cloned()
        .ok_or_else(|| "MCP input is missing its execution context".to_string())?;
    let authorization_context = authorization_context
        .cloned()
        .ok_or_else(|| "MCP input is missing its authorization context".to_string())?;
    let state = state.clone();
    let request = request.clone();
    Ok(Box::new(move || {
        authorization_context.revalidate(&state, &request, &execution_context)
    }))
}

pub(super) fn ipc_string_arg<'a>(
    value: &'a serde_json::Value,
    key: &str,
) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("missing string argument `{key}`"))
}

fn redact_mcp_tmux_state(mut state: TmuxState) -> TmuxState {
    for session in &mut state.sessions {
        session.name = redact_secrets(&session.name);
    }
    for window in &mut state.windows {
        window.session = redact_secrets(&window.session);
        window.name = redact_secrets(&window.name);
    }
    for pane in &mut state.panes {
        pane.session = redact_secrets(&pane.session);
        pane.command = redact_secrets(&pane.command);
        pane.title = redact_secrets(&pane.title);
    }
    state
}

fn redact_mcp_tunnel_status(mut status: TunnelStatus) -> TunnelStatus {
    status.spec = redact_mcp_tunnel_spec(status.spec);
    status.last_error = status.last_error.take().map(|error| redact_secrets(&error));
    status
}

fn redact_mcp_tunnel_spec(mut spec: TunnelSpec) -> TunnelSpec {
    spec.label = redact_secrets(&spec.label);
    spec
}

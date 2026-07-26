use super::*;

pub(super) async fn execute_ipc_request(
    state: AppState,
    request: IpcRequest,
) -> Result<serde_json::Value, String> {
    match request.command.as_str() {
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
        "send_text" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let text = ipc_string_arg(&request.args, "text")?.to_string();
            let actor = mcp_audit_actor(&request.client_id);
            let event =
                send_text_inner_with_context(state.session_io(), session_id, text, &actor, None)
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
            let event =
                send_text_inner_with_context(state.session_io(), session_id, text, &actor, None)
                    .await?;
            serde_json::to_value(redact_session_event(event)).map_err(|error| error.to_string())
        }
        "run_command" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let command = ipc_string_arg(&request.args, "command")?.to_string();
            let text = terminate_command_for_protocol(
                command,
                is_telnet_session(&state.store, &session_id)?,
            );
            let actor = mcp_audit_actor(&request.client_id);
            let event =
                run_command_inner_with_context(state.session_io(), session_id, text, &actor, None)
                    .await?;
            serde_json::to_value(redact_session_event(event)).map_err(|error| error.to_string())
        }
        "open_session" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let password = request
                .args
                .get("password")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let passphrase = request
                .args
                .get("passphrase")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            let summary = open_session_inner(
                state.clone(),
                session_id,
                SessionOpenCredentials {
                    password,
                    passphrase,
                    ..Default::default()
                },
            )
            .await?;
            serde_json::to_value(redact_session_summary(summary)).map_err(|error| error.to_string())
        }
        "close_session" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            let summary = close_session_inner(&state, session_id).await?;
            serde_json::to_value(redact_session_summary(summary)).map_err(|error| error.to_string())
        }
        "start_transfer" => {
            let transfer = serde_json::from_value::<StartTransferRequest>(request.args.clone())
                .map_err(|error| format!("invalid transfer request: {error}"))?;
            let task = start_transfer_inner(&state, transfer).await?;
            serde_json::to_value(redact_transfer_task(task)).map_err(|error| error.to_string())
        }
        "create_tunnel" => {
            let tunnel = serde_json::from_value::<CreateTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
            let spec = create_tunnel_inner(&state, tunnel).await?;
            serde_json::to_value(spec).map_err(|error| error.to_string())
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
            let event =
                send_text_inner_with_context(state.session_io(), session_id, command, &actor, None)
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

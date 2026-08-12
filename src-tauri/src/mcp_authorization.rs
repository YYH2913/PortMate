use super::*;

pub(super) fn mcp_scope_allowed(
    store: &SessionStore,
    client_id: &str,
    trusted_write: bool,
    scope: McpScope,
    session_id: &str,
) -> bool {
    let Ok(client_id) = normalize_mcp_client_id(client_id) else {
        return false;
    };
    store.mcp_can(&client_id, scope, Some(session_id)) || (trusted_write && store.grants.is_empty())
}

pub(super) fn mcp_write_confirmation_required(
    store: &SessionStore,
    client_id: &str,
    scope: McpScope,
    session_id: &str,
) -> bool {
    let client_id = client_id.trim();
    !client_id.is_empty()
        && store.grants.iter().any(|grant| {
            grant.client_id == client_id
                && grant.confirm_writes
                && grant.allows(scope, Some(session_id), Utc::now())
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum McpWriteAuditStart {
    Authorized {
        audit_id: String,
        approval_required: bool,
        trusted_bootstrap: bool,
    },
    Denied(String),
}

fn ipc_write_scope(command: &str) -> Option<McpScope> {
    match command {
        "send_text" | "send_key" | "run_command" | "attach_tmux" => Some(McpScope::WriteInput),
        "open_session" | "close_session" => Some(McpScope::ManageSessions),
        "start_transfer" | "cancel_transfer" | "retry_transfer" => Some(McpScope::Transfer),
        "create_tunnel" | "stop_tunnel" => Some(McpScope::Tunnel),
        _ => None,
    }
}

fn ipc_read_scope(command: &str) -> Option<McpScope> {
    match command {
        "list_sessions" => Some(McpScope::ReadSessions),
        "read_screen"
        | "tail_log"
        | "search_logs"
        | "list_tmux_state"
        | "export_session_bundle" => Some(McpScope::ReadLogs),
        "list_transfers" | "get_transfer" => Some(McpScope::ReadTransfers),
        "list_tunnels" => Some(McpScope::ReadTunnels),
        _ => None,
    }
}

pub(super) fn mcp_scope_label(scope: McpScope) -> &'static str {
    match scope {
        McpScope::ReadSessions => "read-sessions",
        McpScope::ReadLogs => "read-logs",
        McpScope::ReadTransfers => "read-transfers",
        McpScope::ReadTunnels => "read-tunnels",
        McpScope::WriteInput => "write-input",
        McpScope::Transfer => "transfer",
        McpScope::Tunnel => "tunnel",
        McpScope::ManageSessions => "manage-sessions",
    }
}

pub(super) fn mcp_audit_actor(client_id: &str) -> String {
    let client_id = client_id.trim();
    if client_id.is_empty() {
        "<missing-client-id>".to_string()
    } else if client_id.len() > MAX_MCP_GRANT_CLIENT_ID_BYTES
        || client_id.chars().any(char::is_control)
    {
        "<invalid-client-id>".to_string()
    } else {
        client_id.to_string()
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

fn mcp_audit_details(
    scope: McpScope,
    trusted_bootstrap: bool,
    approval_required: bool,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("scope".to_string(), mcp_scope_label(scope).to_string()),
        (
            "trustedBootstrap".to_string(),
            trusted_bootstrap.to_string(),
        ),
        (
            "approvalRequired".to_string(),
            approval_required.to_string(),
        ),
    ])
}

fn validate_ipc_write_args(state: &AppState, request: &IpcRequest) -> Result<(), String> {
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
        "start_transfer" => {
            let transfer = serde_json::from_value::<StartTransferRequest>(request.args.clone())
                .map_err(|error| format!("invalid transfer request: {error}"))?;
            validate_mcp_transfer_route(&transfer)?;
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
            normalize_tunnel_request(tunnel)?;
        }
        "stop_tunnel" => {
            validate_mcp_operation_id(ipc_string_arg(&request.args, "tunnelId")?, "tunnel")?;
        }
        "attach_tmux" => {
            ipc_string_arg(&request.args, "target")?;
        }
        "open_session" | "close_session" => {}
        _ => {
            return Err(format!(
                "unsupported IPC write command: {}",
                request.command
            ))
        }
    }
    Ok(())
}

pub(super) fn validate_mcp_transfer_route(request: &StartTransferRequest) -> Result<(), String> {
    const MAX_MCP_TRANSFER_PATH_BYTES: usize = 32 * 1024;
    for path in [&request.source, &request.destination] {
        if path.is_empty()
            || path.len() > MAX_MCP_TRANSFER_PATH_BYTES
            || path.chars().any(|character| character == '\0')
        {
            return Err(format!(
                "MCP transfer paths must be non-empty, NUL-free, and at most {MAX_MCP_TRANSFER_PATH_BYTES} bytes"
            ));
        }
    }
    if has_load_receiver_prefix(&request.source) {
        return Err("MCP load: endpoint is only permitted as a Modem upload destination".to_string());
    }
    let load_receiver = parse_load_receiver_endpoint(&request.destination, &request.protocol)?;
    if load_receiver.is_some() && has_remote_transfer_prefix(&request.source) {
        return Err("MCP load: transfer source must be a local desktop file".to_string());
    }
    let source_remote = is_nonlocal_transfer_endpoint(&request.source);
    let destination_remote = is_nonlocal_transfer_endpoint(&request.destination);
    if !source_remote && !destination_remote {
        return Err(
            "MCP file transfer requires at least one remote:/ssh:/load: endpoint; local-to-local copy is not exposed"
                .to_string(),
        );
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

fn ipc_write_session_id(state: &AppState, request: &IpcRequest) -> Result<String, String> {
    match request.command.as_str() {
        "cancel_transfer" | "retry_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?;
            validate_mcp_operation_id(transfer_id, "transfer")?;
            state
                .store
                .lock()
                .map_err(|error| error.to_string())?
                .transfer_by_id(transfer_id)
                .map(|transfer| transfer.session_id)
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
                .ok_or_else(|| "unknown or unavailable tunnel".to_string())
        }
        _ => Ok(ipc_string_arg(&request.args, "sessionId")?.to_string()),
    }
}

pub(super) fn revalidate_ipc_write_target(
    state: &AppState,
    request: &IpcRequest,
    scope: McpScope,
    authorized_session_id: &str,
    trusted_bootstrap: bool,
) -> Result<(), String> {
    let current_session_id = ipc_write_session_id(state, request)?;
    if current_session_id != authorized_session_id {
        return Err(
            "MCP write target changed after authorization; request was not executed".to_string(),
        );
    }
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let still_allowed = if trusted_bootstrap {
        request.trusted_write && store.grants.is_empty()
    } else {
        store.mcp_can(&request.client_id, scope, Some(authorized_session_id))
    };
    if !still_allowed {
        return Err("MCP grant changed after authorization; request was not executed".to_string());
    }
    drop(store);
    validate_ipc_write_args(state, request)
}

fn append_and_save_mcp_audit(
    store_path: &Path,
    store: &mut SessionStore,
    record: AuditRecord,
) -> Result<(), String> {
    commit_store_mutation(store, store_path, |next_store| {
        next_store.record_audit(record);
        Ok(())
    })
}

fn record_invalid_mcp_write(
    state: &AppState,
    request: &IpcRequest,
    scope: McpScope,
    session_id: Option<&str>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let trusted_bootstrap = request.trusted_write && store.grants.is_empty();
    let record = AuditRecord {
        id: Uuid::new_v4().to_string(),
        ts: Utc::now(),
        actor: mcp_audit_actor(&request.client_id),
        action: request.command.clone(),
        session_id: session_id.map(str::to_string),
        decision: "invalid".to_string(),
        details: mcp_audit_details(scope, trusted_bootstrap, false),
    };
    append_and_save_mcp_audit(&state.store_path, &mut store, record)
}

fn begin_mcp_write_audit(
    state: &AppState,
    request: &IpcRequest,
    scope: McpScope,
    session_id: &str,
) -> Result<McpWriteAuditStart, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let trusted_bootstrap = request.trusted_write && store.grants.is_empty();
    let allowed = mcp_scope_allowed(
        &store,
        &request.client_id,
        request.trusted_write,
        scope,
        session_id,
    );
    let approval_required =
        allowed && mcp_write_confirmation_required(&store, &request.client_id, scope, session_id);
    let id = Uuid::new_v4().to_string();
    let record = AuditRecord {
        id: id.clone(),
        ts: Utc::now(),
        actor: mcp_audit_actor(&request.client_id),
        action: request.command.clone(),
        session_id: Some(session_id.to_string()),
        decision: if approval_required {
            "pending-approval"
        } else if allowed {
            "authorized"
        } else {
            "denied"
        }
        .to_string(),
        details: mcp_audit_details(scope, trusted_bootstrap, approval_required),
    };
    append_and_save_mcp_audit(&state.store_path, &mut store, record).map_err(|error| {
        format!("MCP write was not executed because its audit record could not be saved: {error}")
    })?;
    if allowed {
        Ok(McpWriteAuditStart::Authorized {
            audit_id: id,
            approval_required,
            trusted_bootstrap,
        })
    } else {
        Ok(McpWriteAuditStart::Denied(format!(
            "MCP grant does not permit {scope:?} for client `{}` on session `{session_id}`",
            request.client_id
        )))
    }
}

fn finish_mcp_write_audit(
    state: &AppState,
    audit_id: &str,
    decision: &str,
    approval: Option<&str>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        update_mcp_write_audit(next_store, audit_id, decision, approval)
    })
}

fn finish_applied_mcp_write_audit(
    state: &AppState,
    audit_id: &str,
    decision: &str,
    approval: Option<&str>,
) -> Result<(), String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    finish_applied_mcp_write_audit_with(
        &mut store,
        audit_id,
        decision,
        approval,
        |next_store| save_store(&state.store_path, next_store),
        |next_store| verify_persisted_store_commit(&state.store_path, next_store),
    )
}

pub(super) fn finish_applied_mcp_write_audit_with<Persist, VerifyAfterError>(
    store: &mut SessionStore,
    audit_id: &str,
    decision: &str,
    approval: Option<&str>,
    persist: Persist,
    verify_after_error: VerifyAfterError,
) -> Result<(), String>
where
    Persist: FnOnce(&SessionStore) -> Result<(), String>,
    VerifyAfterError: FnOnce(&SessionStore) -> Result<bool, String>,
{
    update_mcp_write_audit(store, audit_id, decision, approval)?;
    if let Err(error) =
        persist_applied_store_with(store, "MCP final audit", persist, verify_after_error)
    {
        if let Some(record) = store.audit.iter_mut().find(|record| record.id == audit_id) {
            record.details.insert(
                "finalizationPersistence".to_string(),
                "degraded".to_string(),
            );
        }
        return Err(error);
    }
    Ok(())
}

fn update_mcp_write_audit(
    store: &mut SessionStore,
    audit_id: &str,
    decision: &str,
    approval: Option<&str>,
) -> Result<(), String> {
    let record = store
        .audit
        .iter_mut()
        .find(|record| record.id == audit_id)
        .ok_or_else(|| format!("MCP audit record disappeared before completion: {audit_id}"))?;
    record.decision = decision.to_string();
    if let Some(approval) = approval {
        record
            .details
            .insert("approval".to_string(), approval.to_string());
    }
    Ok(())
}

pub(super) async fn handle_ipc_request(
    state: AppState,
    request: IpcRequest,
) -> Result<serde_json::Value, String> {
    let Some(scope) = ipc_write_scope(&request.command) else {
        if ipc_read_scope(&request.command).is_some() {
            return execute_ipc_request(state, request).await;
        }
        return Err(format!("unsupported IPC command: {}", request.command));
    };
    let session_id = match ipc_write_session_id(&state, &request) {
        Ok(session_id) => {
            if let Err(error) = validate_mcp_session_id(&session_id) {
                record_invalid_mcp_write(&state, &request, scope, None).map_err(|audit_error| {
                    format!("{error}; failed to save MCP invalid-request audit: {audit_error}")
                })?;
                return Err(error);
            }
            session_id
        }
        Err(error) => {
            record_invalid_mcp_write(&state, &request, scope, None).map_err(|audit_error| {
                format!("{error}; failed to save MCP invalid-request audit: {audit_error}")
            })?;
            return Err(error);
        }
    };
    if let Err(error) = validate_ipc_write_args(&state, &request) {
        record_invalid_mcp_write(&state, &request, scope, Some(&session_id)).map_err(
            |audit_error| {
                format!("{error}; failed to save MCP invalid-request audit: {audit_error}")
            },
        )?;
        return Err(error);
    }
    let (audit_id, approval_required, trusted_bootstrap) =
        match begin_mcp_write_audit(&state, &request, scope, &session_id)? {
            McpWriteAuditStart::Authorized {
                audit_id,
                approval_required,
                trusted_bootstrap,
            } => (audit_id, approval_required, trusted_bootstrap),
            McpWriteAuditStart::Denied(error) => return Err(error),
        };
    let approval = if approval_required {
        match request_mcp_approval(
            &state,
            &request.client_id,
            &request.command,
            &session_id,
            scope,
        )
        .await
        {
            Ok(McpApprovalOutcome::Approved) => {
                finish_mcp_write_audit(&state, &audit_id, "authorized", Some("approved"))
                    .map_err(|error| {
                        format!(
                            "MCP write was approved but not executed because its approval audit could not be saved: {error}"
                        )
                    })?;
                Some("approved")
            }
            Ok(McpApprovalOutcome::Denied) => {
                finish_mcp_write_audit(&state, &audit_id, "denied", Some("user-denied"))
                    .map_err(|error| {
                        format!(
                            "MCP write was not executed and its denial audit could not be saved: {error}"
                        )
                    })?;
                return Err("MCP write was denied by the desktop user".to_string());
            }
            Ok(McpApprovalOutcome::TimedOut) => {
                finish_mcp_write_audit(&state, &audit_id, "denied", Some("timed-out"))
                    .map_err(|error| {
                        format!(
                            "MCP write timed out without execution and its audit could not be saved: {error}"
                        )
                    })?;
                return Err("MCP write approval timed out without execution".to_string());
            }
            Err(approval_error) => {
                finish_mcp_write_audit(&state, &audit_id, "denied", Some("unavailable"))
                    .map_err(|audit_error| {
                        format!(
                            "MCP write was not executed because approval was unavailable: {approval_error}; failed to save denial audit: {audit_error}"
                        )
                    })?;
                return Err(format!(
                    "MCP write was not executed because approval was unavailable: {approval_error}"
                ));
            }
        }
    } else {
        None
    };
    let result = match revalidate_ipc_write_target(
        &state,
        &request,
        scope,
        &session_id,
        trusted_bootstrap,
    ) {
        Ok(()) => execute_ipc_request(state.clone(), request).await,
        Err(error) => Err(error),
    };
    let decision = if result.is_ok() {
        "succeeded"
    } else {
        "failed"
    };
    if let Err(error) = finish_applied_mcp_write_audit(&state, &audit_id, decision, approval) {
        eprintln!("PortMate: failed to finalize MCP audit {audit_id} as {decision}: {error}");
    }
    result
}

pub(super) fn require_mcp_read_scope(
    store: &SessionStore,
    request: &IpcRequest,
    scope: McpScope,
    session_id: Option<&str>,
) -> Result<(), String> {
    if let Some(session_id) = session_id {
        validate_mcp_session_id(session_id)?;
    }
    if store.mcp_can_read(&request.client_id, scope, session_id) {
        Ok(())
    } else {
        Err(format!(
            "MCP read grant does not permit {scope:?} for the requested session"
        ))
    }
}

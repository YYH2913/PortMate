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
    },
    Denied(String),
}

fn ipc_write_scope(command: &str) -> Option<McpScope> {
    match command {
        "send_text" | "send_key" | "run_command" | "attach_tmux" => Some(McpScope::WriteInput),
        "open_session" | "close_session" => Some(McpScope::ManageSessions),
        "start_transfer" => Some(McpScope::Transfer),
        "create_tunnel" => Some(McpScope::Tunnel),
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
        _ => None,
    }
}

pub(super) fn mcp_scope_label(scope: McpScope) -> &'static str {
    match scope {
        McpScope::ReadSessions => "read-sessions",
        McpScope::ReadLogs => "read-logs",
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

fn validate_ipc_write_args(request: &IpcRequest) -> Result<(), String> {
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
            serde_json::from_value::<StartTransferRequest>(request.args.clone())
                .map_err(|error| format!("invalid transfer request: {error}"))?;
        }
        "create_tunnel" => {
            serde_json::from_value::<CreateTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
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
    let session_id = match ipc_string_arg(&request.args, "sessionId") {
        Ok(session_id) => {
            if let Err(error) = validate_mcp_session_id(session_id) {
                record_invalid_mcp_write(&state, &request, scope, None).map_err(|audit_error| {
                    format!("{error}; failed to save MCP invalid-request audit: {audit_error}")
                })?;
                return Err(error);
            }
            session_id.to_string()
        }
        Err(error) => {
            record_invalid_mcp_write(&state, &request, scope, None).map_err(|audit_error| {
                format!("{error}; failed to save MCP invalid-request audit: {audit_error}")
            })?;
            return Err(error);
        }
    };
    if let Err(error) = validate_ipc_write_args(&request) {
        record_invalid_mcp_write(&state, &request, scope, Some(&session_id)).map_err(
            |audit_error| {
                format!("{error}; failed to save MCP invalid-request audit: {audit_error}")
            },
        )?;
        return Err(error);
    }
    let (audit_id, approval_required) =
        match begin_mcp_write_audit(&state, &request, scope, &session_id)? {
            McpWriteAuditStart::Authorized {
                audit_id,
                approval_required,
            } => (audit_id, approval_required),
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
    let result = execute_ipc_request(state.clone(), request).await;
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

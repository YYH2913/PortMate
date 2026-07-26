use super::*;

#[tauri::command]
pub(crate) fn list_mcp_audit(
    state: State<'_, AppState>,
) -> Result<Vec<portmate_core::AuditRecord>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.audit.clone())
}

#[tauri::command]
pub(crate) fn export_mcp_audit(
    state: State<'_, AppState>,
    request: ExportMcpAuditRequest,
) -> Result<ExportMcpAuditResult, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    export_mcp_audit_inner(&state.store_path, &store.audit, request)
}

#[tauri::command]
pub(crate) fn list_mcp_grants(state: State<'_, AppState>) -> Result<Vec<McpGrant>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.grants.clone())
}

#[tauri::command]
pub(crate) fn list_mcp_approvals(
    state: State<'_, AppState>,
) -> Result<Vec<McpApprovalRequest>, String> {
    list_mcp_approvals_inner(state.inner())
}

#[tauri::command]
pub(crate) fn respond_mcp_approval(
    state: State<'_, AppState>,
    approval_id: String,
    approved: bool,
) -> Result<(), String> {
    respond_mcp_approval_inner(state.inner(), &approval_id, approved)
}

#[tauri::command]
pub(crate) fn save_mcp_grant(
    state: State<'_, AppState>,
    grant: McpGrant,
) -> Result<Vec<McpGrant>, String> {
    let grant = normalize_mcp_grant(grant)?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        upsert_mcp_grant_in_store(next_store, grant)
    })
}

#[tauri::command]
pub(crate) fn revoke_mcp_grant(
    state: State<'_, AppState>,
    client_id: String,
) -> Result<Vec<McpGrant>, String> {
    let client_id = normalize_mcp_client_id(&client_id)?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        Ok(revoke_mcp_grant_from_store(next_store, &client_id))
    })
}

#[tauri::command]
pub(crate) fn mcp_http_config(state: State<'_, AppState>) -> McpHttpConfig {
    build_mcp_http_config(
        has_secret_ref(MCP_HTTP_TOKEN_REF),
        &mcp_sidecar_executable_path(),
        &state.store_path,
    )
}

#[tauri::command]
pub(crate) fn rotate_mcp_http_token(
    state: State<'_, AppState>,
) -> Result<McpHttpTokenResponse, String> {
    let token = Uuid::new_v4().to_string();
    write_secret_to_keyring(MCP_HTTP_TOKEN_REF, &token)?;
    Ok(McpHttpTokenResponse {
        config: build_mcp_http_config(true, &mcp_sidecar_executable_path(), &state.store_path),
        token,
    })
}

#[tauri::command]
pub(crate) fn mcp_manifest() -> serde_json::Value {
    serde_json::json!({
        "protocolVersion": "2025-06-18",
        "tools": tool_definitions(),
        "resources": resource_templates(),
        "prompts": prompt_templates(),
    })
}

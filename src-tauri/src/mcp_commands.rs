use super::*;

const MAX_MCP_AUDIT_EXPORT_RECORDS: usize = 5_000;
const MAX_MCP_AUDIT_EXPORT_RECORD_BYTES: usize = 64 * 1024;
const MAX_MCP_AUDIT_EXPORT_BYTES: usize = 16 * 1024 * 1024;

pub(super) fn export_mcp_audit_inner(
    store_path: &Path,
    audit: &[AuditRecord],
    request: ExportMcpAuditRequest,
) -> Result<ExportMcpAuditResult, String> {
    if request.record_ids.is_empty() {
        return Err("select at least one MCP audit record to export".to_string());
    }
    if request.record_ids.len() > MAX_MCP_AUDIT_EXPORT_RECORDS {
        return Err(format!(
            "MCP audit export record limit exceeded ({MAX_MCP_AUDIT_EXPORT_RECORDS})"
        ));
    }
    let mut requested = HashSet::new();
    for id in &request.record_ids {
        if id.trim().is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
            return Err("MCP audit export contains an invalid record ID".to_string());
        }
        if !requested.insert(id.as_str()) {
            return Err("MCP audit export contains duplicate record IDs".to_string());
        }
    }

    let by_id = audit
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect::<HashMap<_, _>>();
    let selected = request
        .record_ids
        .iter()
        .map(|id| {
            by_id
                .get(id.as_str())
                .copied()
                .ok_or_else(|| "MCP audit changed; refresh before exporting".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;

    let created_at = Utc::now();
    let mut output = Vec::new();
    append_mcp_audit_jsonl(
        &mut output,
        &serde_json::json!({
            "type": "metadata",
            "format": "portmate-mcp-audit",
            "version": 1,
            "createdAt": created_at.to_rfc3339(),
            "recordCount": selected.len(),
            "containsSecretBodies": false,
        }),
    )?;
    for record in &selected {
        append_mcp_audit_jsonl(
            &mut output,
            &serde_json::json!({ "type": "record", "record": record }),
        )?;
    }

    let export_dir = prepare_export_directory(store_path, "MCP audit")?;
    let timestamp = created_at.format("%Y%m%dT%H%M%SZ");
    let name = format!(
        "portmate-mcp-audit-{timestamp}-{}.jsonl",
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let final_path = export_dir.join(name);
    let finalized = write_atomic_export_with_checksum(&final_path, &output, "MCP audit export")?;
    Ok(ExportMcpAuditResult {
        path: final_path.display().to_string(),
        checksum_path: finalized.checksum_path.display().to_string(),
        sha256: finalized.sha256,
        size: finalized.size,
        records: selected.len(),
    })
}

fn append_mcp_audit_jsonl<T: Serialize>(output: &mut Vec<u8>, value: &T) -> Result<(), String> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| format!("failed to encode MCP audit export: {error}"))?;
    if encoded.len() > MAX_MCP_AUDIT_EXPORT_RECORD_BYTES {
        return Err(format!(
            "MCP audit export record exceeds {MAX_MCP_AUDIT_EXPORT_RECORD_BYTES} bytes"
        ));
    }
    if encoded.len().saturating_add(1) > MAX_MCP_AUDIT_EXPORT_BYTES.saturating_sub(output.len()) {
        return Err(format!(
            "MCP audit export exceeds {MAX_MCP_AUDIT_EXPORT_BYTES} bytes"
        ));
    }
    output.extend_from_slice(&encoded);
    output.push(b'\n');
    Ok(())
}

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
pub(crate) fn mcp_http_config(state: State<'_, AppState>) -> Result<McpHttpConfig, String> {
    let settings = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .mcp_http_settings
        .clone();
    build_mcp_http_config_for_request(
        has_secret_ref(MCP_HTTP_TOKEN_REF),
        &mcp_sidecar_executable_path(),
        &state.store_path,
        settings,
    )
}

#[tauri::command]
pub(crate) fn preview_mcp_http_config(
    state: State<'_, AppState>,
    settings: McpHttpSettings,
) -> Result<McpHttpConfig, String> {
    build_mcp_http_config_for_request(
        has_secret_ref(MCP_HTTP_TOKEN_REF),
        &mcp_sidecar_executable_path(),
        &state.store_path,
        settings,
    )
}

#[tauri::command]
pub(crate) fn save_mcp_http_settings(
    state: State<'_, AppState>,
    settings: McpHttpSettings,
) -> Result<McpHttpConfig, String> {
    let (settings, _) = normalize_mcp_http_settings(settings)?;
    let config = build_mcp_http_config_for_request(
        has_secret_ref(MCP_HTTP_TOKEN_REF),
        &mcp_sidecar_executable_path(),
        &state.store_path,
        settings.clone(),
    )?;
    {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        commit_store_mutation(&mut store, &state.store_path, |next_store| {
            Ok(set_mcp_http_settings_in_store(next_store, settings))
        })?;
    }
    Ok(config)
}

#[tauri::command]
pub(crate) fn rotate_mcp_http_token(
    state: State<'_, AppState>,
) -> Result<McpHttpTokenResponse, String> {
    let settings = state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .mcp_http_settings
        .clone();
    let config = build_mcp_http_config_for_request(
        true,
        &mcp_sidecar_executable_path(),
        &state.store_path,
        settings,
    )?;
    let token = Uuid::new_v4().to_string();
    write_secret_to_keyring(MCP_HTTP_TOKEN_REF, &token)?;
    Ok(McpHttpTokenResponse { config, token })
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

use super::*;

pub(super) const MCP_HTTP_TOKEN_REF: &str = "keychain:mcp-http-token";
const MCP_HTTP_DEFAULT_ADDR: &str = "127.0.0.1:8787";
pub(super) const MAX_MCP_GRANTS: usize = 512;
pub(super) const MAX_MCP_GRANT_CLIENT_ID_BYTES: usize = 128;
pub(super) const MAX_MCP_GRANT_NAME_BYTES: usize = 256;
pub(super) const MAX_MCP_GRANT_SESSIONS: usize = 1_024;
pub(super) const MAX_MCP_GRANT_SESSION_ID_BYTES: usize = 128;
pub(super) const MAX_PENDING_MCP_APPROVALS: usize = 32;
pub(super) const MCP_APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) type PendingMcpApprovalMap = Arc<Mutex<HashMap<String, PendingMcpApproval>>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpApprovalRequest {
    pub id: String,
    pub client_id: String,
    pub action: String,
    pub session_id: String,
    pub scope: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

pub(super) struct PendingMcpApproval {
    pub(super) request: McpApprovalRequest,
    pub(super) response: tokio::sync::oneshot::Sender<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum McpApprovalOutcome {
    Approved,
    Denied,
    TimedOut,
}

pub(super) fn upsert_mcp_grant_in_store(
    store: &mut SessionStore,
    grant: McpGrant,
) -> Result<Vec<McpGrant>, String> {
    if let Some(existing) = store
        .grants
        .iter_mut()
        .find(|existing| existing.client_id == grant.client_id)
    {
        *existing = grant;
    } else {
        if store.grants.len() >= MAX_MCP_GRANTS {
            return Err(format!("MCP grant limit exceeded ({MAX_MCP_GRANTS})"));
        }
        store.grants.push(grant);
    }
    Ok(store.grants.clone())
}

pub(super) fn revoke_mcp_grant_from_store(
    store: &mut SessionStore,
    client_id: &str,
) -> Vec<McpGrant> {
    store.grants.retain(|grant| grant.client_id != client_id);
    store.grants.clone()
}

pub(super) fn normalize_mcp_grant(mut grant: McpGrant) -> Result<McpGrant, String> {
    grant.client_id = normalize_mcp_client_id(&grant.client_id)?;
    grant.name = grant.name.trim().to_string();
    if grant.name.len() > MAX_MCP_GRANT_NAME_BYTES || grant.name.chars().any(char::is_control) {
        return Err(format!(
            "MCP grant name must not contain control characters or exceed {MAX_MCP_GRANT_NAME_BYTES} bytes"
        ));
    }
    if grant.scopes.len() > 6 {
        return Err("MCP grant contains too many scopes".to_string());
    }
    for (index, scope) in grant.scopes.iter().enumerate() {
        if grant.scopes[..index].contains(scope) {
            return Err("MCP grant contains duplicate scopes".to_string());
        }
    }
    if grant.allowed_sessions.len() > MAX_MCP_GRANT_SESSIONS {
        return Err(format!(
            "MCP grant session limit exceeded ({MAX_MCP_GRANT_SESSIONS})"
        ));
    }
    let mut allowed_sessions = Vec::with_capacity(grant.allowed_sessions.len());
    for session_id in grant.allowed_sessions {
        let session_id = session_id.trim();
        if session_id.is_empty()
            || session_id.len() > MAX_MCP_GRANT_SESSION_ID_BYTES
            || session_id.chars().any(char::is_control)
        {
            return Err(format!(
                "MCP grant session IDs must be non-empty, printable, and at most {MAX_MCP_GRANT_SESSION_ID_BYTES} bytes"
            ));
        }
        if allowed_sessions
            .iter()
            .any(|existing| existing == session_id)
        {
            return Err("MCP grant contains duplicate session IDs".to_string());
        }
        allowed_sessions.push(session_id.to_string());
    }
    grant.allowed_sessions = allowed_sessions;
    Ok(grant)
}

pub(super) fn normalize_mcp_client_id(client_id: &str) -> Result<String, String> {
    let client_id = client_id.trim();
    if client_id.is_empty()
        || client_id.len() > MAX_MCP_GRANT_CLIENT_ID_BYTES
        || client_id.chars().any(char::is_control)
    {
        return Err(format!(
            "MCP client ID must be non-empty, printable, and at most {MAX_MCP_GRANT_CLIENT_ID_BYTES} bytes"
        ));
    }
    Ok(client_id.to_string())
}

pub(super) fn list_mcp_approvals_inner(
    state: &AppState,
) -> Result<Vec<McpApprovalRequest>, String> {
    let now = Utc::now();
    let mut requests = state
        .pending_mcp_approvals
        .lock()
        .map_err(|error| error.to_string())?
        .values()
        .filter(|pending| pending.request.expires_at > now)
        .map(|pending| pending.request.clone())
        .collect::<Vec<_>>();
    requests.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(requests)
}

pub(super) fn respond_mcp_approval_inner(
    state: &AppState,
    approval_id: &str,
    approved: bool,
) -> Result<(), String> {
    let approval_id = Uuid::parse_str(approval_id)
        .map_err(|_| "invalid MCP approval ID".to_string())?
        .to_string();
    let pending = state
        .pending_mcp_approvals
        .lock()
        .map_err(|error| error.to_string())?
        .remove(&approval_id)
        .ok_or_else(|| "MCP approval is no longer pending".to_string())?;
    pending
        .response
        .send(approved)
        .map_err(|_| "MCP approval requester is no longer waiting".to_string())
}

pub(super) async fn await_mcp_approval_with_emitter<F>(
    state: &AppState,
    request: McpApprovalRequest,
    timeout: Duration,
    emit: F,
) -> Result<McpApprovalOutcome, String>
where
    F: FnOnce(&McpApprovalRequest) -> Result<(), String>,
{
    let (response, receiver) = tokio::sync::oneshot::channel();
    {
        let mut pending = state
            .pending_mcp_approvals
            .lock()
            .map_err(|error| error.to_string())?;
        if pending.len() >= MAX_PENDING_MCP_APPROVALS {
            return Err(format!(
                "MCP approval queue limit exceeded ({MAX_PENDING_MCP_APPROVALS})"
            ));
        }
        pending.insert(
            request.id.clone(),
            PendingMcpApproval {
                request: request.clone(),
                response,
            },
        );
    }
    if let Err(error) = emit(&request) {
        state
            .pending_mcp_approvals
            .lock()
            .map_err(|lock_error| lock_error.to_string())?
            .remove(&request.id);
        return Err(error);
    }
    match tokio::time::timeout(timeout, receiver).await {
        Ok(Ok(true)) => Ok(McpApprovalOutcome::Approved),
        Ok(Ok(false)) => Ok(McpApprovalOutcome::Denied),
        Ok(Err(_)) => Err("MCP approval response channel closed".to_string()),
        Err(_) => {
            state
                .pending_mcp_approvals
                .lock()
                .map_err(|error| error.to_string())?
                .remove(&request.id);
            Ok(McpApprovalOutcome::TimedOut)
        }
    }
}

pub(super) async fn request_mcp_approval(
    state: &AppState,
    client_id: &str,
    action: &str,
    session_id: &str,
    scope: McpScope,
) -> Result<McpApprovalOutcome, String> {
    let app_handle = state
        .app_handle
        .as_ref()
        .ok_or_else(|| "MCP approval UI is unavailable".to_string())?;
    let request = build_mcp_approval_request(client_id, action, session_id, scope)?;
    await_mcp_approval_with_emitter(state, request, MCP_APPROVAL_TIMEOUT, |request| {
        app_handle
            .emit("portmate-mcp-approval", request)
            .map_err(|error| format!("failed to show MCP approval: {error}"))
    })
    .await
}

pub(super) fn build_mcp_approval_request(
    client_id: &str,
    action: &str,
    session_id: &str,
    scope: McpScope,
) -> Result<McpApprovalRequest, String> {
    validate_mcp_session_id(session_id)?;
    let created_at = Utc::now();
    Ok(McpApprovalRequest {
        id: Uuid::new_v4().to_string(),
        client_id: mcp_audit_actor(client_id),
        action: action.to_string(),
        session_id: session_id.to_string(),
        scope: mcp_scope_label(scope).to_string(),
        created_at,
        expires_at: created_at
            + chrono::Duration::from_std(MCP_APPROVAL_TIMEOUT)
                .map_err(|error| format!("invalid MCP approval timeout: {error}"))?,
    })
}

pub(super) fn mcp_sidecar_executable_path() -> PathBuf {
    let file_name = if cfg!(windows) {
        "portmate-mcp.exe"
    } else {
        "portmate-mcp"
    };
    if let Ok(current_exe) = std::env::current_exe() {
        let mut directories = current_exe
            .parent()
            .into_iter()
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        if let Some(parent) = current_exe.parent().and_then(Path::parent) {
            directories.push(parent.to_path_buf());
        }
        if let Some(candidate) = directories
            .into_iter()
            .map(|directory| directory.join(file_name))
            .find(|candidate| candidate.is_file())
        {
            return candidate;
        }
    }
    PathBuf::from(file_name)
}

fn shell_command_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if cfg!(windows) {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

pub(super) fn build_mcp_http_config(
    token_available: bool,
    executable: &Path,
    store_path: &Path,
) -> McpHttpConfig {
    let executable_text = executable.to_string_lossy().to_string();
    let store_path_text = store_path.to_string_lossy().to_string();
    let executable_command = shell_command_path(executable);
    let store_path_command = shell_command_path(store_path);
    let start_command = if cfg!(windows) {
        format!(
            "$env:PORTMATE_STORE_PATH={store_path_command}; $env:PORTMATE_MCP_HTTP='1'; $env:PORTMATE_MCP_HTTP_ADDR='{MCP_HTTP_DEFAULT_ADDR}'; $env:PORTMATE_MCP_HTTP_ORIGINS='http://{MCP_HTTP_DEFAULT_ADDR}'; & {executable_command} --http"
        )
    } else {
        format!(
            "PORTMATE_STORE_PATH={store_path_command} PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR={MCP_HTTP_DEFAULT_ADDR} PORTMATE_MCP_HTTP_ORIGINS=http://{MCP_HTTP_DEFAULT_ADDR} {executable_command} --http"
        )
    };
    McpHttpConfig {
        endpoint: format!("http://{MCP_HTTP_DEFAULT_ADDR}/mcp"),
        token_ref: MCP_HTTP_TOKEN_REF.to_string(),
        token_available,
        default_origin: format!("http://{MCP_HTTP_DEFAULT_ADDR}"),
        executable: executable_text,
        store_path: store_path_text,
        start_command,
    }
}

use super::*;

pub(super) const MCP_HTTP_TOKEN_REF: &str = "keychain:mcp-http-token";
const MCP_HTTP_DEFAULT_ADDR: &str = "127.0.0.1:8787";
pub(super) const MAX_MCP_GRANTS: usize = 512;
pub(super) const MAX_MCP_GRANT_CLIENT_ID_BYTES: usize = 128;
pub(super) const MAX_MCP_GRANT_NAME_BYTES: usize = 256;
pub(super) const MAX_MCP_GRANT_SESSIONS: usize = 1_024;
pub(super) const MAX_MCP_GRANT_SESSION_ID_BYTES: usize = 128;

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

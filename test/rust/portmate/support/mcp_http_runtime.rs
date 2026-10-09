#[cfg(test)]
pub(super) fn install_test_mcp_http_process(
    state: &AppState,
    command: &mut Command,
    endpoint: String,
    connect_address: std::net::SocketAddr,
) -> Result<McpHttpProcessOwner, String> {
    // These transport/process fixtures intentionally have no grant/token.
    // The separate expiry regression installs a normally bound instance.
    state
        .store
        .lock()
        .map_err(|error| error.to_string())?
        .mcp_http_settings
        .client_id
        .clear();
    let mut registry = state
        .mcp_http_process
        .lock()
        .map_err(|error| error.to_string())?;
    reap_mcp_http_process(&mut registry)?;
    if registry.process.is_some() {
        return Err("MCP HTTP test process is already running".to_string());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let child = command
        .spawn()
        .map_err(|error| format!("failed to start MCP HTTP test process: {error}"))?;
    Ok(install_mcp_http_process(
        &mut registry,
        child,
        endpoint,
        connect_address,
    ))
}

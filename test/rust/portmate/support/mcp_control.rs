#[cfg(test)]
pub(super) fn build_mcp_approval_request(
    client_id: &str,
    action: &str,
    session_id: &str,
    scope: McpScope,
) -> Result<McpApprovalRequest, String> {
    build_mcp_approval_request_with_target(client_id, action, session_id, scope, None)
}

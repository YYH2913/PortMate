use super::*;

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
    let load_receiver = validate_load_receiver_endpoint(&request.destination, &request.protocol)?;
    if load_receiver && has_remote_transfer_prefix(&request.source) {
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

pub(super) fn mcp_transfer_uses_host_path(request: &StartTransferRequest) -> bool {
    !is_nonlocal_transfer_endpoint(&request.source)
        || !is_nonlocal_transfer_endpoint(&request.destination)
}

pub(super) fn validate_mcp_content_transfer_request(
    request: &StartMcpContentTransferRequest,
) -> Result<(), String> {
    use portmate_core::{MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH, MAX_MCP_CONTENT_TRANSFER_BYTES};

    if request.file_name.len() > 255
        || request.file_name.is_empty()
        || request.file_name.chars().any(|character| {
            character == '\0'
                || character.is_control()
                || character == '/'
                || character == '\\'
                || character == ':'
        })
        || matches!(request.file_name.as_str(), "." | "..")
    {
        return Err(
            "MCP content transfer fileName must be a single printable file name without path separators"
                .to_string(),
        );
    }
    if request.protocol == TransferProtocol::Tftp {
        validate_tftp_file_name(&request.file_name)?;
    }
    if request.content_base64.is_empty()
        || request.content_base64.len() > MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
    {
        return Err(format!(
            "MCP contentBase64 exceeds the {MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH}-byte encoded limit"
        ));
    }
    let content = BASE64_STANDARD
        .decode(&request.content_base64)
        .map_err(|_| "MCP contentBase64 is not valid standard Base64".to_string())?;
    if content.len() > MAX_MCP_CONTENT_TRANSFER_BYTES {
        return Err(format!(
            "MCP content exceeds the {MAX_MCP_CONTENT_TRANSFER_BYTES}-byte decoded limit"
        ));
    }
    validate_mcp_transfer_route(&StartTransferRequest {
        session_id: request.session_id.clone(),
        protocol: request.protocol.clone(),
        source: request.file_name.clone(),
        destination: request.destination.clone(),
    })
}

pub(super) fn validate_mcp_uploaded_content_route(
    metadata: &McpContentUploadMetadata,
) -> Result<(), String> {
    if metadata.protocol == TransferProtocol::Tftp {
        validate_tftp_file_name(&metadata.file_name)?;
    }
    validate_mcp_transfer_route(&StartTransferRequest {
        session_id: metadata.session_id.clone(),
        protocol: metadata.protocol.clone(),
        source: metadata.file_name.clone(),
        destination: metadata.destination.clone(),
    })
}

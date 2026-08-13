use super::*;

const DEFAULT_MCP_TRANSFER_QUERY_LIMIT: u64 = 100;
const MAX_MCP_TRANSFER_QUERY_LIMIT: u64 = 1000;

fn bounded_mcp_transfer_query_limit(limit: Option<u64>) -> usize {
    limit
        .unwrap_or(DEFAULT_MCP_TRANSFER_QUERY_LIMIT)
        .clamp(1, MAX_MCP_TRANSFER_QUERY_LIMIT) as usize
}

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
        "list_transfers" => {
            let session_id = request
                .args
                .get("sessionId")
                .and_then(serde_json::Value::as_str);
            let limit = bounded_mcp_transfer_query_limit(
                request.args.get("limit").and_then(serde_json::Value::as_u64),
            );
            let store = state.store.lock().map_err(|error| error.to_string())?;
            require_mcp_read_scope(&store, &request, McpScope::ReadTransfers, session_id)?;
            let mut transfers = store
                .transfers
                .iter()
                .filter(|transfer| {
                    session_id.is_none_or(|session_id| transfer.session_id == session_id)
                        && store.mcp_can_read(
                            &request.client_id,
                            McpScope::ReadTransfers,
                            Some(&transfer.session_id),
                        )
                })
                .cloned()
                .map(redact_transfer_task)
                .collect::<Vec<_>>();
            if transfers.len() > limit {
                transfers.drain(..transfers.len() - limit);
            }
            serde_json::to_value(transfers).map_err(|error| error.to_string())
        }
        "get_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?;
            let store = state.store.lock().map_err(|error| error.to_string())?;
            let transfer = store
                .transfer_by_id(transfer_id)
                .ok_or_else(|| "unknown or unavailable transfer".to_string())?;
            require_mcp_read_scope(
                &store,
                &request,
                McpScope::ReadTransfers,
                Some(&transfer.session_id),
            )?;
            serde_json::to_value(redact_transfer_task(transfer)).map_err(|error| error.to_string())
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
            if ["password", "passphrase", "credentialHandle"]
                .iter()
                .any(|field| request.args.get(*field).is_some())
            {
                return Err(
                    "MCP open_session 不接受内联凭据或桌面凭据句柄；请使用已保存的 Profile 凭据"
                        .to_string(),
                );
            }
            let summary = open_session_inner(
                state.clone(),
                session_id,
                SessionOpenCredentials::default(),
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
        "start_content_transfer" => {
            let content_request = serde_json::from_value::<StartMcpContentTransferRequest>(
                request.args.clone(),
            )
            .map_err(|error| format!("invalid content transfer request: {error}"))?;
            let (source, staging_path) = stage_mcp_content_transfer(&state, &content_request)?;
            let transfer = StartTransferRequest {
                session_id: content_request.session_id,
                protocol: content_request.protocol,
                source,
                destination: content_request.destination,
            };
            match start_transfer_inner_with_staging(&state, transfer, Some(staging_path.clone()))
                .await
            {
                Ok(task) => serde_json::to_value(redact_transfer_task(task))
                    .map_err(|error| error.to_string()),
                Err(error) => {
                    let _ = fs::remove_file(&staging_path);
                    let _ = staging_path
                        .parent()
                        .filter(|parent| parent.file_name().is_some())
                        .map(fs::remove_dir);
                    Err(error)
                }
            }
        }
        "start_content_upload_transfer" => {
            let transfer = serde_json::from_value::<StartMcpContentUploadTransferRequest>(
                request.args.clone(),
            )
            .map_err(|error| format!("invalid uploaded content transfer request: {error}"))?;
            let metadata = load_mcp_content_upload_metadata(
                &state,
                &request.client_id,
                &transfer.upload_id,
            )?;
            let staging_state = state.clone();
            let staging_metadata = metadata.clone();
            let (source, staging_path) = tauri::async_runtime::spawn_blocking(move || {
                stage_mcp_content_upload(&staging_state, &staging_metadata)
            })
            .await
            .map_err(|error| format!("MCP content upload staging task failed: {error}"))??;
            let transfer = StartTransferRequest {
                session_id: metadata.session_id,
                protocol: metadata.protocol,
                source,
                destination: metadata.destination,
            };
            match start_transfer_inner_with_staging(&state, transfer, Some(staging_path.clone()))
                .await
            {
                Ok(task) => serde_json::to_value(redact_transfer_task(task))
                    .map_err(|error| error.to_string()),
                Err(error) => {
                    cleanup_staged_mcp_content_path(&staging_path);
                    Err(error)
                }
            }
        }
        "cancel_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?.to_string();
            let task = cancel_transfer_inner(&state, &transfer_id)?;
            serde_json::to_value(redact_transfer_task(task)).map_err(|error| error.to_string())
        }
        "retry_transfer" => {
            let transfer_id = ipc_string_arg(&request.args, "transferId")?.to_string();
            let task = retry_transfer_inner(&state, &transfer_id).await?;
            serde_json::to_value(redact_transfer_task(task)).map_err(|error| error.to_string())
        }
        "create_tunnel" => {
            let tunnel = serde_json::from_value::<CreateTunnelRequest>(request.args.clone())
                .map_err(|error| format!("invalid tunnel request: {error}"))?;
            let spec = create_tunnel_inner(&state, tunnel).await?;
            serde_json::to_value(redact_mcp_tunnel_spec(spec)).map_err(|error| error.to_string())
        }
        "list_tunnels" => {
            let session_id = ipc_string_arg(&request.args, "sessionId")?.to_string();
            {
                let store = state.store.lock().map_err(|error| error.to_string())?;
                require_mcp_read_scope(
                    &store,
                    &request,
                    McpScope::ReadTunnels,
                    Some(&session_id),
                )?;
            }
            let statuses = list_tunnels_inner(&state, Some(&session_id))?
                .into_iter()
                .map(redact_mcp_tunnel_status)
                .collect::<Vec<_>>();
            serde_json::to_value(statuses).map_err(|error| error.to_string())
        }
        "stop_tunnel" => {
            let tunnel_id = ipc_string_arg(&request.args, "tunnelId")?.to_string();
            let status = stop_tunnel_inner(&state, &tunnel_id).await?;
            serde_json::to_value(redact_mcp_tunnel_status(status))
                .map_err(|error| error.to_string())
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

pub(super) fn stage_mcp_content_transfer(
    state: &AppState,
    request: &StartMcpContentTransferRequest,
) -> Result<(String, PathBuf), String> {
    validate_mcp_content_transfer_request(request)?;
    let content = BASE64_STANDARD
        .decode(&request.content_base64)
        .map_err(|_| "MCP contentBase64 is not valid standard Base64".to_string())?;
    let parent = state
        .store_path
        .parent()
        .ok_or_else(|| "MCP content staging directory is unavailable".to_string())?;
    let staging_dir = parent.join(".mcp-transfer-staging");
    fs::create_dir_all(&staging_dir)
        .map_err(|error| format!("failed to create MCP content staging directory: {error}"))?;
    #[cfg(unix)]
    fs::set_permissions(&staging_dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
        format!("failed to secure MCP content staging directory: {error}")
    })?;
    let task_dir = staging_dir.join(Uuid::new_v4().to_string());
    fs::create_dir(&task_dir)
        .map_err(|error| format!("failed to create MCP content staging task directory: {error}"))?;
    #[cfg(unix)]
    if let Err(error) = fs::set_permissions(&task_dir, fs::Permissions::from_mode(0o700)) {
        let _ = fs::remove_dir(&task_dir);
        return Err(format!(
            "failed to secure MCP content staging task directory: {error}"
        ));
    }
    let path = task_dir.join(&request.file_name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .map_err(|error| {
            let _ = fs::remove_dir(&task_dir);
            format!("failed to create MCP content staging file: {error}")
        })?;
    if let Err(error) = file.write_all(&content).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&task_dir);
        return Err(format!("failed to write MCP content staging file: {error}"));
    }
    Ok((path.display().to_string(), path))
}

pub(super) fn load_mcp_content_upload_metadata(
    state: &AppState,
    client_id: &str,
    upload_id: &str,
) -> Result<McpContentUploadMetadata, String> {
    Uuid::parse_str(upload_id).map_err(|_| "MCP uploadId must be a UUID".to_string())?;
    let upload_dir = mcp_content_upload_root(state)?.join(upload_id);
    require_regular_directory(&upload_dir, "MCP content upload")?;
    let metadata_path = upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE);
    let metadata_file = open_mcp_upload_file(&metadata_path, "metadata")
        .map_err(|_| "unknown or unavailable MCP content upload".to_string())?;
    if metadata_file
        .metadata()
        .map_err(|error| format!("failed to inspect MCP content upload metadata: {error}"))?
        .len()
        > 64 * 1024
    {
        return Err("invalid MCP content upload metadata file".to_string());
    }
    let mut metadata_bytes = Vec::new();
    metadata_file
        .take(64 * 1024 + 1)
        .read_to_end(&mut metadata_bytes)
        .map_err(|error| format!("failed to read MCP content upload metadata: {error}"))?;
    if metadata_bytes.len() > 64 * 1024 {
        return Err("invalid MCP content upload metadata file".to_string());
    }
    let metadata: McpContentUploadMetadata = serde_json::from_slice(&metadata_bytes)
    .map_err(|error| format!("invalid MCP content upload metadata: {error}"))?;
    if metadata.version != MCP_CONTENT_UPLOAD_METADATA_VERSION
        || metadata.upload_id != upload_id
        || metadata.client_id != client_id
    {
        return Err("unknown or unavailable MCP content upload".to_string());
    }
    validate_mcp_content_upload_metadata(&metadata)?;
    Ok(metadata)
}

pub(super) fn validate_mcp_uploaded_content_route(
    metadata: &McpContentUploadMetadata,
) -> Result<(), String> {
    validate_mcp_transfer_route(&StartTransferRequest {
        session_id: metadata.session_id.clone(),
        protocol: metadata.protocol.clone(),
        source: metadata.file_name.clone(),
        destination: metadata.destination.clone(),
    })
}

pub(super) fn stage_mcp_content_upload(
    state: &AppState,
    metadata: &McpContentUploadMetadata,
) -> Result<(String, PathBuf), String> {
    validate_mcp_content_upload_metadata(metadata)?;
    validate_mcp_uploaded_content_route(metadata)?;
    let upload_dir = mcp_content_upload_root(state)?.join(&metadata.upload_id);
    require_regular_directory(&upload_dir, "MCP content upload")?;
    let payload_path = upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE);
    let mut payload = open_mcp_upload_file(&payload_path, "payload")?;
    let opened_metadata = payload
        .metadata()
        .map_err(|error| format!("failed to inspect MCP content upload payload: {error}"))?;
    if !opened_metadata.is_file() || opened_metadata.len() != metadata.size_bytes {
        return Err(format!(
            "MCP content upload is incomplete: expected {} bytes, received {}",
            metadata.size_bytes,
            opened_metadata.len()
        ));
    }

    let staging_dir = mcp_content_staging_root(state)?;
    require_regular_directory(&staging_dir, "MCP content staging")?;
    let task_dir = staging_dir.join(Uuid::new_v4().to_string());
    create_private_mcp_directory(&task_dir)?;
    let staged_path = task_dir.join(&metadata.file_name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut staged = match options.open(&staged_path) {
        Ok(file) => file,
        Err(error) => {
            let _ = fs::remove_dir(&task_dir);
            return Err(format!("failed to create MCP content staging file: {error}"));
        }
    };

    let result = (|| -> Result<(), String> {
        let mut digest = Sha256::new();
        let mut copied = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = payload
                .read(&mut buffer)
                .map_err(|error| format!("failed to read MCP content upload: {error}"))?;
            if read == 0 {
                break;
            }
            copied = copied
                .checked_add(read as u64)
                .ok_or_else(|| "MCP content upload size overflow".to_string())?;
            if copied > metadata.size_bytes {
                return Err("MCP content upload exceeds its declared size".to_string());
            }
            digest.update(&buffer[..read]);
            staged
                .write_all(&buffer[..read])
                .map_err(|error| format!("failed to stage MCP content upload: {error}"))?;
        }
        if copied != metadata.size_bytes {
            return Err(format!(
                "MCP content upload is incomplete: expected {} bytes, received {copied}",
                metadata.size_bytes
            ));
        }
        let actual = format!("{:x}", digest.finalize());
        if actual != metadata.sha256 {
            return Err("MCP content upload SHA-256 mismatch".to_string());
        }
        staged
            .sync_all()
            .map_err(|error| format!("failed to sync MCP content staging file: {error}"))?;
        Ok(())
    })();
    if let Err(error) = result {
        cleanup_staged_mcp_content_path(&staged_path);
        return Err(error);
    }
    Ok((staged_path.display().to_string(), staged_path))
}

fn validate_mcp_content_upload_metadata(
    metadata: &McpContentUploadMetadata,
) -> Result<(), String> {
    if metadata.version != MCP_CONTENT_UPLOAD_METADATA_VERSION
        || Uuid::parse_str(&metadata.upload_id).is_err()
        || metadata.size_bytes == 0
        || metadata.size_bytes > MAX_MCP_CONTENT_UPLOAD_BYTES
    {
        return Err("invalid MCP content upload metadata".to_string());
    }
    if metadata.session_id.is_empty()
        || metadata.session_id.len() > 128
        || metadata.session_id.chars().any(char::is_control)
    {
        return Err("invalid MCP content upload session".to_string());
    }
    if metadata.file_name.is_empty()
        || metadata.file_name.len() > 255
        || matches!(metadata.file_name.as_str(), "." | "..")
        || metadata.file_name.chars().any(|character| {
            character.is_control() || matches!(character, '\0' | '/' | '\\' | ':')
        })
    {
        return Err("invalid MCP content upload file name".to_string());
    }
    if metadata.sha256.len() != 64
        || !metadata
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("invalid MCP content upload SHA-256".to_string());
    }
    Ok(())
}

fn mcp_content_staging_root(state: &AppState) -> Result<PathBuf, String> {
    state
        .store_path
        .parent()
        .map(|parent| parent.join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY))
        .ok_or_else(|| "MCP content staging directory is unavailable".to_string())
}

fn mcp_content_upload_root(state: &AppState) -> Result<PathBuf, String> {
    let staging_root = mcp_content_staging_root(state)?;
    require_regular_directory(&staging_root, "MCP content staging")?;
    let uploads_root = staging_root.join(MCP_CONTENT_UPLOADS_DIRECTORY);
    require_regular_directory(&uploads_root, "MCP content uploads")?;
    Ok(uploads_root)
}

fn require_regular_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("{label} directory is unavailable: {error}"))?;
    if !metadata.is_dir() || mcp_upload_metadata_is_link(&metadata) {
        return Err(format!("invalid {label} directory"));
    }
    Ok(())
}

fn open_mcp_upload_file(path: &Path, label: &str) -> Result<fs::File, String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect MCP content upload {label}: {error}"))?;
    if !metadata.is_file() || mcp_upload_metadata_is_link(&metadata) {
        return Err(format!("invalid MCP content upload {label}"));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    #[cfg(windows)]
    options.custom_flags(0x0020_0000);
    let file = options
        .open(path)
        .map_err(|error| format!("failed to open MCP content upload {label}: {error}"))?;
    let opened = file
        .metadata()
        .map_err(|error| format!("failed to inspect opened MCP content upload {label}: {error}"))?;
    if !opened.is_file() || mcp_upload_metadata_is_link(&opened) {
        return Err(format!("invalid MCP content upload {label}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.nlink() != 1 {
            return Err(format!("MCP content upload {label} must not have hard links"));
        }
    }
    Ok(file)
}

fn mcp_upload_metadata_is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        metadata.file_attributes() & 0x0400 != 0
    }
    #[cfg(not(windows))]
    false
}

fn create_private_mcp_directory(path: &Path) -> Result<(), String> {
    fs::create_dir(path)
        .map_err(|error| format!("failed to create MCP content staging directory: {error}"))?;
    #[cfg(unix)]
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o700)) {
        let _ = fs::remove_dir(path);
        return Err(format!("failed to secure MCP content staging directory: {error}"));
    }
    Ok(())
}

fn cleanup_staged_mcp_content_path(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = path
        .parent()
        .filter(|parent| parent.file_name().is_some())
        .map(fs::remove_dir);
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

fn redact_mcp_tunnel_status(mut status: TunnelStatus) -> TunnelStatus {
    status.spec = redact_mcp_tunnel_spec(status.spec);
    status.last_error = status.last_error.take().map(|error| redact_secrets(&error));
    status
}

fn redact_mcp_tunnel_spec(mut spec: TunnelSpec) -> TunnelSpec {
    spec.label = redact_secrets(&spec.label);
    spec
}

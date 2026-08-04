use super::*;

#[cfg(unix)]
const SSH_AGENT_FAILURE: u8 = 5;
#[cfg(unix)]
const SSH_AGENT_REQUEST_IDENTITIES: u8 = 11;
#[cfg(unix)]
const SSH_AGENT_IDENTITIES_ANSWER: u8 = 12;
#[cfg(unix)]
const SSH_AGENT_SIGN_REQUEST: u8 = 13;
#[cfg(unix)]
const MAX_FILTERED_AGENT_FRAME_BYTES: usize = 256 * 1024;
#[cfg(unix)]
const MAX_FILTERED_AGENT_CONNECTIONS: usize = 8;

#[cfg(unix)]
pub(super) struct FilteredAgentProxy {
    socket_path: PathBuf,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    finished: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
    _directory: tempfile::TempDir,
}

#[cfg(unix)]
impl FilteredAgentProxy {
    pub(super) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub(super) async fn stop(mut self) -> Result<(), String> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let Some(mut finished) = self.finished.take() else {
            return Ok(());
        };
        tokio::time::timeout(Duration::from_secs(2), &mut finished)
            .await
            .map_err(|_| "停止 libssh SSH agent 过滤代理超时".to_string())?
            .map_err(|_| "libssh SSH agent 过滤代理未返回清理结果".to_string())?
    }
}

#[cfg(unix)]
impl Drop for FilteredAgentProxy {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

#[cfg(unix)]
pub(super) async fn start_filtered_agent_proxy(
    upstream_socket: PathBuf,
    identity_refs: &[IdentityRef],
) -> Result<FilteredAgentProxy, String> {
    let directory = tempfile::Builder::new()
        .prefix("portmate-agent-")
        .tempdir()
        .map_err(|error| format!("创建 libssh SSH agent 过滤目录失败: {error}"))?;
    let socket_path = directory.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path)
        .map_err(|error| format!("创建 libssh SSH agent 过滤 socket 失败: {error}"))?;
    let identity_refs = Arc::new(
        identity_refs
            .iter()
            .filter(|identity| identity.source == IdentitySource::Agent)
            .cloned()
            .collect::<Vec<_>>(),
    );
    let allowed_key_blobs = Arc::new(Mutex::new(HashSet::new()));
    let (shutdown_sender, shutdown_receiver) = tokio::sync::oneshot::channel();
    let (finished_sender, finished_receiver) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let result = run_filtered_agent_proxy(
            listener,
            upstream_socket,
            identity_refs,
            allowed_key_blobs,
            shutdown_receiver,
        )
        .await;
        let _ = finished_sender.send(result);
    });
    Ok(FilteredAgentProxy {
        socket_path,
        shutdown: Some(shutdown_sender),
        finished: Some(finished_receiver),
        _directory: directory,
    })
}

#[cfg(unix)]
async fn run_filtered_agent_proxy(
    listener: tokio::net::UnixListener,
    upstream_socket: PathBuf,
    identity_refs: Arc<Vec<IdentityRef>>,
    allowed_key_blobs: Arc<Mutex<HashSet<Vec<u8>>>>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    let mut connections = tokio::task::JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        tokio::select! {
            _ = &mut shutdown => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted
                    .map_err(|error| format!("接受 libssh SSH agent 过滤连接失败: {error}"))?;
                if connections.len() >= MAX_FILTERED_AGENT_CONNECTIONS {
                    drop(stream);
                    continue;
                }
                let upstream_socket = upstream_socket.clone();
                let identity_refs = Arc::clone(&identity_refs);
                let allowed_key_blobs = Arc::clone(&allowed_key_blobs);
                connections.spawn(async move {
                    handle_filtered_agent_connection(
                        stream,
                        upstream_socket,
                        identity_refs,
                        allowed_key_blobs,
                    ).await
                });
            }
        }
    }
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

#[cfg(unix)]
async fn handle_filtered_agent_connection(
    mut client: tokio::net::UnixStream,
    upstream_socket: PathBuf,
    identity_refs: Arc<Vec<IdentityRef>>,
    allowed_key_blobs: Arc<Mutex<HashSet<Vec<u8>>>>,
) -> Result<(), String> {
    let mut upstream = tokio::net::UnixStream::connect(&upstream_socket)
        .await
        .map_err(|error| {
            format!(
                "连接上游 SSH agent {} 失败: {error}",
                upstream_socket.display()
            )
        })?;
    loop {
        let request = read_agent_frame(&mut client).await?;
        let response = match request.first().copied() {
            Some(SSH_AGENT_REQUEST_IDENTITIES) => {
                write_agent_frame(&mut upstream, &request).await?;
                let response = read_agent_frame(&mut upstream).await?;
                let (response, selected) = filter_identity_response(&response, &identity_refs)?;
                *allowed_key_blobs
                    .lock()
                    .map_err(|error| error.to_string())? = selected;
                response
            }
            Some(SSH_AGENT_SIGN_REQUEST)
                if sign_request_key_blob(&request).is_some_and(|key| {
                    allowed_key_blobs
                        .lock()
                        .is_ok_and(|allowed| allowed.contains(key))
                }) =>
            {
                write_agent_frame(&mut upstream, &request).await?;
                read_agent_frame(&mut upstream).await?
            }
            _ => vec![SSH_AGENT_FAILURE],
        };
        write_agent_frame(&mut client, &response).await?;
    }
}

#[cfg(unix)]
async fn read_agent_frame(stream: &mut tokio::net::UnixStream) -> Result<Vec<u8>, String> {
    let mut length = [0_u8; 4];
    stream
        .read_exact(&mut length)
        .await
        .map_err(|error| format!("读取 SSH agent 帧长度失败: {error}"))?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FILTERED_AGENT_FRAME_BYTES {
        return Err(format!("SSH agent 帧长度超出边界: {length}"));
    }
    let mut payload = vec![0_u8; length];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|error| format!("读取 SSH agent 帧失败: {error}"))?;
    Ok(payload)
}

#[cfg(unix)]
async fn write_agent_frame(
    stream: &mut tokio::net::UnixStream,
    payload: &[u8],
) -> Result<(), String> {
    let length =
        u32::try_from(payload.len()).map_err(|_| "SSH agent 响应长度超出协议边界".to_string())?;
    stream
        .write_all(&length.to_be_bytes())
        .await
        .map_err(|error| format!("写入 SSH agent 帧长度失败: {error}"))?;
    stream
        .write_all(payload)
        .await
        .map_err(|error| format!("写入 SSH agent 帧失败: {error}"))?;
    stream
        .flush()
        .await
        .map_err(|error| format!("刷新 SSH agent 帧失败: {error}"))
}

#[cfg(unix)]
fn sign_request_key_blob(request: &[u8]) -> Option<&[u8]> {
    if request.first().copied() != Some(SSH_AGENT_SIGN_REQUEST) {
        return None;
    }
    let mut offset = 1_usize;
    read_ssh_string(request, &mut offset)
}

#[cfg(unix)]
fn filter_identity_response(
    response: &[u8],
    identity_refs: &[IdentityRef],
) -> Result<(Vec<u8>, HashSet<Vec<u8>>), String> {
    if response.first().copied() != Some(SSH_AGENT_IDENTITIES_ANSWER) {
        return Ok((response.to_vec(), HashSet::new()));
    }
    let mut offset = 1_usize;
    let count = read_u32(response, &mut offset)
        .ok_or_else(|| "SSH agent identities 响应缺少数量".to_string())?;
    let mut selected = Vec::new();
    for _ in 0..count {
        let key = read_ssh_string(response, &mut offset)
            .ok_or_else(|| "SSH agent identities 响应包含损坏的公钥".to_string())?;
        let comment = read_ssh_string(response, &mut offset)
            .ok_or_else(|| "SSH agent identities 响应包含损坏的注释".to_string())?;
        if agent_identity_blob_matches(key, comment, identity_refs) {
            selected.push((key, comment));
        }
    }
    if offset != response.len() {
        return Err("SSH agent identities 响应包含尾随数据".to_string());
    }
    let mut filtered = Vec::with_capacity(response.len());
    filtered.push(SSH_AGENT_IDENTITIES_ANSWER);
    filtered.extend_from_slice(
        &u32::try_from(selected.len())
            .map_err(|_| "SSH agent identity 数量超出协议边界".to_string())?
            .to_be_bytes(),
    );
    let selected_blobs = selected.iter().map(|(key, _)| key.to_vec()).collect();
    for (key, comment) in selected {
        push_ssh_string(&mut filtered, key)?;
        push_ssh_string(&mut filtered, comment)?;
    }
    Ok((filtered, selected_blobs))
}

#[cfg(unix)]
fn agent_identity_blob_matches(key: &[u8], comment: &[u8], identity_refs: &[IdentityRef]) -> bool {
    let fingerprint = agent_identity_blob_fingerprint(key);
    identity_refs.iter().any(|identity_ref| {
        if let Some(expected) = identity_ref
            .fingerprint_sha256
            .as_deref()
            .map(str::trim)
            .filter(|expected| !expected.is_empty())
        {
            return fingerprint.as_deref() == Some(expected);
        }
        if let Some(path) = identity_ref
            .path
            .as_deref()
            .filter(|path| !path.trim().is_empty())
        {
            return path.as_bytes() == comment;
        }
        !identity_ref.label.trim().is_empty() && identity_ref.label.as_bytes() == comment
    })
}

#[cfg(unix)]
fn agent_identity_blob_fingerprint(key: &[u8]) -> Option<String> {
    let public_key = ssh_key::PublicKey::from_bytes(key)
        .or_else(|_| {
            ssh_key::Certificate::from_bytes(key)
                .map(|certificate| ssh_key::PublicKey::new(certificate.public_key().clone(), ""))
        })
        .ok()?;
    compute_ssh_sha256_fingerprint(&public_key.public_key_base64()).ok()
}

#[cfg(unix)]
fn read_u32(value: &[u8], offset: &mut usize) -> Option<u32> {
    let bytes: [u8; 4] = value
        .get(*offset..(*offset).checked_add(4)?)?
        .try_into()
        .ok()?;
    *offset += 4;
    Some(u32::from_be_bytes(bytes))
}

#[cfg(unix)]
fn read_ssh_string<'a>(value: &'a [u8], offset: &mut usize) -> Option<&'a [u8]> {
    let length = usize::try_from(read_u32(value, offset)?).ok()?;
    let end = (*offset).checked_add(length)?;
    let result = value.get(*offset..end)?;
    *offset = end;
    Some(result)
}

#[cfg(unix)]
fn push_ssh_string(target: &mut Vec<u8>, value: &[u8]) -> Result<(), String> {
    target.extend_from_slice(
        &u32::try_from(value.len())
            .map_err(|_| "SSH agent 字符串超出协议边界".to_string())?
            .to_be_bytes(),
    );
    target.extend_from_slice(value);
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn filtered_agent_response_exposes_only_allowed_identity_blobs() {
        let mut response = vec![SSH_AGENT_IDENTITIES_ANSWER];
        response.extend_from_slice(&2_u32.to_be_bytes());
        push_ssh_string(&mut response, b"rejected-key").unwrap();
        push_ssh_string(&mut response, b"rejected-comment").unwrap();
        push_ssh_string(&mut response, b"accepted-key").unwrap();
        push_ssh_string(&mut response, b"accepted-comment").unwrap();
        let refs = [IdentityRef {
            id: "accepted".to_string(),
            label: "accepted-comment".to_string(),
            source: IdentitySource::Agent,
            fingerprint_sha256: None,
            path: None,
            secret_ref: None,
        }];

        let (filtered, allowed) = filter_identity_response(&response, &refs).unwrap();
        assert_eq!(allowed, HashSet::from([b"accepted-key".to_vec()]));
        let mut offset = 1_usize;
        assert_eq!(filtered[0], SSH_AGENT_IDENTITIES_ANSWER);
        assert_eq!(read_u32(&filtered, &mut offset), Some(1));
        assert_eq!(
            read_ssh_string(&filtered, &mut offset),
            Some(&b"accepted-key"[..])
        );
        assert_eq!(
            read_ssh_string(&filtered, &mut offset),
            Some(&b"accepted-comment"[..])
        );
        assert_eq!(offset, filtered.len());
    }

    #[test]
    fn filtered_agent_path_matches_exact_comment_bytes() {
        let mut response = vec![SSH_AGENT_IDENTITIES_ANSWER];
        response.extend_from_slice(&2_u32.to_be_bytes());
        push_ssh_string(&mut response, b"rejected-key").unwrap();
        push_ssh_string(&mut response, b"accepted-comment").unwrap();
        push_ssh_string(&mut response, b"accepted-key").unwrap();
        push_ssh_string(&mut response, b"accepted-comment ").unwrap();
        let refs = [IdentityRef {
            id: "accepted".to_string(),
            label: "agent key".to_string(),
            source: IdentitySource::Agent,
            fingerprint_sha256: None,
            path: Some("accepted-comment ".to_string()),
            secret_ref: None,
        }];

        let (filtered, allowed) = filter_identity_response(&response, &refs).unwrap();
        assert_eq!(allowed, HashSet::from([b"accepted-key".to_vec()]));
        let mut offset = 1_usize;
        assert_eq!(filtered[0], SSH_AGENT_IDENTITIES_ANSWER);
        assert_eq!(read_u32(&filtered, &mut offset), Some(1));
        assert_eq!(
            read_ssh_string(&filtered, &mut offset),
            Some(&b"accepted-key"[..])
        );
        assert_eq!(
            read_ssh_string(&filtered, &mut offset),
            Some(&b"accepted-comment "[..])
        );
        assert_eq!(offset, filtered.len());
    }

    #[test]
    fn filtered_agent_parser_rejects_truncated_and_trailing_identity_data() {
        assert!(filter_identity_response(&[SSH_AGENT_IDENTITIES_ANSWER], &[]).is_err());

        let mut response = vec![SSH_AGENT_IDENTITIES_ANSWER];
        response.extend_from_slice(&0_u32.to_be_bytes());
        response.push(0);
        assert!(filter_identity_response(&response, &[]).is_err());
    }

    #[test]
    fn filtered_agent_sign_request_reads_the_exact_key_blob() {
        let mut request = vec![SSH_AGENT_SIGN_REQUEST];
        push_ssh_string(&mut request, b"selected-key").unwrap();
        push_ssh_string(&mut request, b"payload").unwrap();
        request.extend_from_slice(&0_u32.to_be_bytes());
        assert_eq!(sign_request_key_blob(&request), Some(&b"selected-key"[..]));
        assert!(sign_request_key_blob(&[SSH_AGENT_SIGN_REQUEST, 0, 0]).is_none());
    }
}

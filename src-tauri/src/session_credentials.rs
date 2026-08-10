use super::*;

pub(super) const SESSION_CREDENTIAL_TTL: Duration = Duration::from_secs(30);
const SESSION_CREDENTIAL_HANDLE_PREFIX: &str = "session-credential:";
const MAX_SESSION_CREDENTIAL_BYTES: usize = 32 * 1024;
const MAX_PENDING_SESSION_CREDENTIALS: usize = 128;

#[derive(Default)]
pub(super) struct SessionCredentialRegistry {
    entries: HashMap<String, PendingSessionCredentials>,
}

pub(super) struct SessionCredentialBinding {
    session_id: String,
    connection_sha256: [u8; 32],
}

pub(super) struct ConsumedSessionCredentials {
    pub(super) password: Option<String>,
    pub(super) passphrase: Option<String>,
    pub(super) binding: SessionCredentialBinding,
}

struct PendingSessionCredentials {
    owner_window: String,
    binding: SessionCredentialBinding,
    expires_at: Instant,
    password: Option<Zeroizing<String>>,
    passphrase: Option<Zeroizing<String>>,
}

fn session_credential_binding(
    store: &SessionStore,
    session_id: &str,
) -> Result<SessionCredentialBinding, String> {
    let session_id = session_id.trim();
    if session_id.is_empty() || session_id.contains('\0') {
        return Err("会话 ID 无效".to_string());
    }
    let profile = store
        .profile(session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    let profile = normalize_session_profile(profile);
    if !matches!(
        profile.connection,
        ConnectionConfig::Ssh(_) | ConnectionConfig::Tmux(_)
    ) {
        return Err("临时凭据句柄只支持 SSH/Tmux 会话".to_string());
    }
    let encoded = serde_json::to_vec(&profile.connection)
        .map_err(|error| format!("无法绑定 SSH 会话配置: {error}"))?;
    Ok(SessionCredentialBinding {
        session_id: session_id.to_string(),
        connection_sha256: Sha256::digest(encoded).into(),
    })
}

fn prepare_runtime_secret(
    value: Option<String>,
    label: &str,
) -> Result<Option<Zeroizing<String>>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_SESSION_CREDENTIAL_BYTES || value.contains('\0') {
        return Err(format!(
            "{label} 必须小于等于 {MAX_SESSION_CREDENTIAL_BYTES} 字节且不能包含 NUL"
        ));
    }
    Ok(Some(Zeroizing::new(value)))
}

fn prune_expired_session_credentials(
    registry: &mut SessionCredentialRegistry,
    now: Instant,
) {
    registry
        .entries
        .retain(|_, credentials| credentials.expires_at > now);
}

pub(super) fn stage_session_credentials_for_owner(
    registry: &Mutex<SessionCredentialRegistry>,
    store: &SessionStore,
    owner_window: &str,
    request: StageSessionCredentialsRequest,
    now: Instant,
) -> Result<SessionCredentialHandleResponse, String> {
    let owner_window = owner_window.trim();
    if owner_window.is_empty() || owner_window.contains('\0') {
        return Err("凭据调用窗口无效".to_string());
    }
    let binding = session_credential_binding(store, &request.session_id)?;
    let password = prepare_runtime_secret(request.password, "SSH 密码")?;
    let passphrase = prepare_runtime_secret(request.passphrase, "SSH 私钥口令")?;
    if password.is_none() && passphrase.is_none() {
        return Err("没有可暂存的 SSH 凭据".to_string());
    }

    let mut registry = registry.lock().map_err(|error| error.to_string())?;
    prune_expired_session_credentials(&mut registry, now);
    registry.entries.retain(|_, credentials| {
        credentials.owner_window != owner_window
            || credentials.binding.session_id != binding.session_id
    });
    if registry.entries.len() >= MAX_PENDING_SESSION_CREDENTIALS {
        return Err(format!(
            "临时 SSH 凭据数量已达上限 ({MAX_PENDING_SESSION_CREDENTIALS})"
        ));
    }

    let credential_handle = format!("{SESSION_CREDENTIAL_HANDLE_PREFIX}{}", Uuid::new_v4());
    registry.entries.insert(
        credential_handle.clone(),
        PendingSessionCredentials {
            owner_window: owner_window.to_string(),
            binding,
            expires_at: now + SESSION_CREDENTIAL_TTL,
            password,
            passphrase,
        },
    );
    Ok(SessionCredentialHandleResponse {
        credential_handle,
        expires_in_ms: SESSION_CREDENTIAL_TTL.as_millis() as u64,
    })
}

fn valid_session_credential_handle(handle: &str) -> bool {
    handle
        .strip_prefix(SESSION_CREDENTIAL_HANDLE_PREFIX)
        .is_some_and(|id| Uuid::parse_str(id).is_ok())
}

pub(super) fn consume_session_credentials_for_owner(
    registry: &Mutex<SessionCredentialRegistry>,
    owner_window: &str,
    session_id: &str,
    credential_handle: &str,
    now: Instant,
) -> Result<ConsumedSessionCredentials, String> {
    let owner_window = owner_window.trim();
    let session_id = session_id.trim();
    if owner_window.is_empty()
        || session_id.is_empty()
        || credential_handle.trim() != credential_handle
        || !valid_session_credential_handle(credential_handle)
    {
        return Err("临时 SSH 凭据句柄无效".to_string());
    }

    let mut registry = registry.lock().map_err(|error| error.to_string())?;
    prune_expired_session_credentials(&mut registry, now);
    let credentials = registry
        .entries
        .get(credential_handle)
        .ok_or_else(|| "临时 SSH 凭据句柄已过期或已使用".to_string())?;
    if credentials.owner_window != owner_window || credentials.binding.session_id != session_id {
        return Err("临时 SSH 凭据句柄与当前窗口或会话不匹配".to_string());
    }
    let mut credentials = registry
        .entries
        .remove(credential_handle)
        .expect("validated credential handle must still exist");
    let password = credentials
        .password
        .take()
        .map(|mut secret| std::mem::take(&mut *secret));
    let passphrase = credentials
        .passphrase
        .take()
        .map(|mut secret| std::mem::take(&mut *secret));
    Ok(ConsumedSessionCredentials {
        password,
        passphrase,
        binding: credentials.binding,
    })
}

pub(super) fn validate_session_credential_binding(
    profile: &SessionProfile,
    binding: &SessionCredentialBinding,
) -> Result<(), String> {
    let mut store = SessionStore::default();
    store.upsert_profile(profile.clone());
    let current = session_credential_binding(&store, &profile.id)?;
    if current.session_id != binding.session_id
        || current.connection_sha256 != binding.connection_sha256
    {
        return Err("SSH 会话配置已在凭据暂存后改变，请重新输入凭据".to_string());
    }
    Ok(())
}

pub(super) fn clear_session_credentials(state: &AppState, session_id: &str) {
    state
        .session_credentials
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entries
        .retain(|_, credentials| credentials.binding.session_id != session_id);
}

pub(super) fn clear_all_session_credentials(state: &AppState) {
    state
        .session_credentials
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .entries
        .clear();
}

#[tauri::command]
pub(crate) fn stage_session_credentials(
    state: State<'_, AppState>,
    window: WebviewWindow,
    request: StageSessionCredentialsRequest,
) -> Result<SessionCredentialHandleResponse, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    stage_session_credentials_for_owner(
        &state.session_credentials,
        &store,
        window.label(),
        request,
        Instant::now(),
    )
}

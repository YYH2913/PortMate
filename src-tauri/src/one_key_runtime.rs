use super::*;

pub(super) const MAX_ONE_KEYS: usize = 64;
pub(super) const MAX_ONE_KEY_LABEL_CHARACTERS: usize = 64;
pub(super) const MAX_ONE_KEY_USERNAME_CHARACTERS: usize = 256;
pub(super) const MAX_ONE_KEY_SESSIONS: usize = 64;
pub(super) const MAX_ONE_KEY_SECRET_BYTES: usize = 32 * 1024;

pub(super) fn truncate_one_key_text(value: &str, max_characters: usize) -> String {
    value.chars().take(max_characters).collect()
}

fn one_key_summary(one_key: &OneKeyCredential) -> OneKeySummary {
    OneKeySummary {
        id: one_key.id.clone(),
        label: one_key.label.clone(),
        kind: one_key.kind,
        username: one_key.username.clone(),
        has_password: one_key.password_secret_ref.is_some(),
        has_passphrase: one_key.passphrase_secret_ref.is_some(),
        identity: one_key
            .identity
            .as_ref()
            .map(|selected| OneKeyIdentitySummary {
                source_profile_id: selected.source_profile_id.clone(),
                id: selected.identity.id.clone(),
                label: selected.identity.label.clone(),
                source: selected.identity.source,
                fingerprint_sha256: selected.identity.fingerprint_sha256.clone(),
            }),
        session_ids: one_key.session_ids.clone(),
        created_at: one_key.created_at,
        updated_at: one_key.updated_at,
    }
}

pub(super) fn one_key_summaries(store: &SessionStore) -> Vec<OneKeySummary> {
    store.one_keys.iter().map(one_key_summary).collect()
}

pub(super) fn one_key_secret_refs(one_key: &OneKeyCredential) -> Vec<String> {
    [
        one_key.password_secret_ref.as_deref(),
        one_key.passphrase_secret_ref.as_deref(),
        one_key
            .identity
            .as_ref()
            .and_then(|selected| selected.identity.secret_ref.as_deref()),
    ]
    .into_iter()
    .flatten()
    .filter_map(canonical_secret_ref)
    .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct OneKeyLoginCredentials {
    pub(super) username: String,
    pub(super) password: Option<String>,
    pub(super) passphrase: Option<String>,
    pub(super) identity: Option<IdentityRef>,
}

fn read_one_key_login_secret<ReadSecret>(
    secret_ref: Option<&str>,
    field: &str,
    read_secret: &mut ReadSecret,
) -> Result<Option<String>, String>
where
    ReadSecret: FnMut(&str) -> Result<String, String>,
{
    let Some(secret_ref) = secret_ref else {
        return Ok(None);
    };
    let secret_ref = canonical_secret_ref(secret_ref)
        .ok_or_else(|| format!("OneKey {field} Secret 引用无效"))?;
    let secret =
        read_secret(&secret_ref).map_err(|error| format!("读取 OneKey {field} 失败: {error}"))?;
    if secret.is_empty() || secret.len() > MAX_ONE_KEY_SECRET_BYTES || secret.contains('\0') {
        return Err(format!("OneKey {field} 内容无效"));
    }
    Ok(Some(secret))
}

pub(super) fn resolve_one_key_login_credentials_with<ReadSecret>(
    store: &SessionStore,
    session_id: &str,
    one_key_id: &str,
    mut read_secret: ReadSecret,
) -> Result<OneKeyLoginCredentials, String>
where
    ReadSecret: FnMut(&str) -> Result<String, String>,
{
    let profile = store
        .profiles
        .iter()
        .find(|profile| profile.id == session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    if !matches!(profile.kind, SessionKind::Ssh | SessionKind::Tmux)
        || !matches!(
            &profile.connection,
            ConnectionConfig::Ssh(_) | ConnectionConfig::Tmux(_)
        )
    {
        return Err("OneKey 登录只支持 SSH/Tmux 会话".to_string());
    }
    let one_key = store
        .one_keys
        .iter()
        .find(|one_key| one_key.id == one_key_id)
        .ok_or_else(|| "OneKey 已被删除，请刷新后重试".to_string())?;
    if one_key.kind != OneKeyKind::Ssh {
        return Err("SSH 登录向导只能使用 SSH OneKey".to_string());
    }
    if !one_key
        .session_ids
        .iter()
        .any(|bound_session_id| bound_session_id == session_id)
    {
        return Err("OneKey 未绑定当前会话".to_string());
    }
    if one_key.username.trim().is_empty() {
        return Err("OneKey 用户名无效".to_string());
    }
    let password = read_one_key_login_secret(
        one_key.password_secret_ref.as_deref(),
        "密码",
        &mut read_secret,
    )?;
    let passphrase = read_one_key_login_secret(
        one_key.passphrase_secret_ref.as_deref(),
        "私钥口令",
        &mut read_secret,
    )?;
    let identity = one_key
        .identity
        .as_ref()
        .map(|selected| selected.identity.clone());
    if password.is_none() && passphrase.is_none() && identity.is_none() {
        return Err("OneKey 没有可用于 SSH 登录的凭据".to_string());
    }
    Ok(OneKeyLoginCredentials {
        username: one_key.username.clone(),
        password,
        passphrase,
        identity,
    })
}

pub(super) fn resolve_one_key_login_credentials(
    state: &AppState,
    session_id: &str,
    one_key_id: &str,
) -> Result<OneKeyLoginCredentials, String> {
    let _credential_guard = lock_credential_operations(state)?;
    let store = state.store.lock().map_err(|error| error.to_string())?;
    resolve_one_key_login_credentials_with(&store, session_id, one_key_id, read_secret_from_store)
}

pub(super) fn normalize_one_key_sessions(
    store: &SessionStore,
    kind: OneKeyKind,
    session_ids: Vec<String>,
) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for session_id in session_ids {
        let session_id = session_id.trim();
        if session_id.is_empty() || normalized.iter().any(|existing| existing == session_id) {
            continue;
        }
        if normalized.len() >= MAX_ONE_KEY_SESSIONS {
            return Err(format!("OneKey 最多绑定 {MAX_ONE_KEY_SESSIONS} 个会话"));
        }
        let profile = store
            .profiles
            .iter()
            .find(|profile| profile.id == session_id)
            .ok_or_else(|| format!("OneKey 绑定了不存在的会话: {session_id}"))?;
        if kind == OneKeyKind::Ssh && !matches!(profile.kind, SessionKind::Ssh | SessionKind::Tmux)
        {
            return Err(format!(
                "SSH OneKey 只能绑定 SSH/Tmux 会话: {}",
                profile.name
            ));
        }
        normalized.push(session_id.to_string());
    }
    if normalized.is_empty() {
        return Err("OneKey 至少需要绑定一个会话".to_string());
    }
    Ok(normalized)
}

pub(super) fn normalize_one_key_identity(
    source_profile_id: String,
    identity: IdentityRef,
) -> Result<OneKeyIdentity, String> {
    if identity.source == IdentitySource::PublicKeyOnly {
        return Err("OneKey 公钥身份必须包含可用于认证的私钥或 ssh-agent 身份".to_string());
    }
    let identity_id = identity.id.clone();
    let mut identity = normalize_client_identity(&identity_id, identity, |secret_ref| {
        canonical_secret_ref(secret_ref)
            .map(|_| ())
            .ok_or_else(|| "OneKey identity Secret 引用无效".to_string())
    })?;
    identity.secret_ref = identity
        .secret_ref
        .as_deref()
        .and_then(canonical_secret_ref);
    Ok(OneKeyIdentity {
        source_profile_id,
        identity,
    })
}

pub(super) fn apply_one_key_identity_update(
    store: &SessionStore,
    kind: OneKeyKind,
    session_ids: &[String],
    current: Option<OneKeyIdentity>,
    update: OneKeyIdentityUpdate,
) -> Result<Option<OneKeyIdentity>, String> {
    if kind == OneKeyKind::Account {
        return Ok(None);
    }
    match update {
        OneKeyIdentityUpdate::Preserve => Ok(current.filter(|selected| {
            session_ids
                .iter()
                .any(|session_id| session_id == &selected.source_profile_id)
        })),
        OneKeyIdentityUpdate::Clear => Ok(None),
        OneKeyIdentityUpdate::Set {
            source_profile_id,
            identity_id,
        } => {
            if !session_ids
                .iter()
                .any(|session_id| session_id == &source_profile_id)
            {
                return Err("OneKey 公钥身份必须来自已绑定的 SSH/Tmux 会话".to_string());
            }
            let identity = find_client_identity(store, &source_profile_id, &identity_id)?;
            normalize_one_key_identity(source_profile_id, identity).map(Some)
        }
    }
}

pub(super) fn apply_one_key_secret_update(
    current: Option<String>,
    update: OneKeySecretUpdate,
    generated: &mut Vec<String>,
) -> Result<Option<String>, String> {
    match update {
        OneKeySecretUpdate::Preserve => Ok(current),
        OneKeySecretUpdate::Clear => Ok(None),
        OneKeySecretUpdate::Set { secret, storage } => {
            let secret = Zeroizing::new(secret.trim_end_matches(['\r', '\n']).to_string());
            if secret.is_empty() {
                return Err("OneKey Secret 不能为空".to_string());
            }
            if secret.len() > MAX_ONE_KEY_SECRET_BYTES {
                return Err(format!(
                    "OneKey Secret 不能超过 {MAX_ONE_KEY_SECRET_BYTES} bytes"
                ));
            }
            if secret.contains('\0') {
                return Err("OneKey Secret 不能包含 NUL".to_string());
            }
            let secret_ref = write_new_secret(storage, secret.as_str())?;
            generated.push(secret_ref.clone());
            Ok(Some(secret_ref))
        }
    }
}

pub(super) fn cleanup_generated_one_key_secrets(secret_refs: &[String]) {
    for secret_ref in secret_refs {
        if let Err(error) = delete_secret_from_store(secret_ref) {
            eprintln!("PortMate: failed to clean up generated OneKey secret: {error}");
        }
    }
}

pub(super) fn cleanup_replaced_one_key_secrets(
    store: &SessionStore,
    old_refs: impl IntoIterator<Item = String>,
    retained_refs: &HashSet<String>,
) {
    for secret_ref in old_refs {
        if !retained_refs.contains(&secret_ref) && secret_ref_usage_count(store, &secret_ref) == 0 {
            if let Err(error) = delete_secret_from_store(&secret_ref) {
                eprintln!("PortMate: OneKey saved but old secret cleanup failed: {error}");
            }
        }
    }
}

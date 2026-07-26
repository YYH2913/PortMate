use super::*;

pub(super) fn ssh_connection(profile: &SessionProfile) -> Result<&SshConnection, String> {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => Ok(ssh),
        _ => Err(format!("Profile {} 不是 SSH/Tmux 会话", profile.id)),
    }
}

pub(super) fn ssh_connection_mut(
    profile: &mut SessionProfile,
) -> Result<&mut SshConnection, String> {
    match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => Ok(ssh),
        _ => Err(format!("Profile {} 不是 SSH/Tmux 会话", profile.id)),
    }
}

pub(super) fn profile_proxy(profile: &SessionProfile) -> Option<&ProxyConfig> {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => Some(&ssh.proxy),
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => Some(&tcp.proxy),
        ConnectionConfig::Serial(_) | ConnectionConfig::Shell(_) => None,
    }
}

pub(super) fn profile_proxy_mut(profile: &mut SessionProfile) -> Option<&mut ProxyConfig> {
    match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => Some(&mut ssh.proxy),
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => Some(&mut tcp.proxy),
        ConnectionConfig::Serial(_) | ConnectionConfig::Shell(_) => None,
    }
}

pub(super) fn find_client_identity(
    store: &SessionStore,
    profile_id: &str,
    identity_id: &str,
) -> Result<IdentityRef, String> {
    let profile = store
        .profiles
        .iter()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("unknown session: {profile_id}"))?;
    let ssh = ssh_connection(profile)?;
    let matches = ssh
        .identity_refs
        .iter()
        .filter(|identity| identity.id == identity_id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [identity] => Ok((*identity).clone()),
        [] => Err(format!("unknown client identity: {identity_id}")),
        _ => Err(format!(
            "Profile {profile_id} 中存在重复 identity id: {identity_id}"
        )),
    }
}

pub(super) fn validate_profile_client_identity_ids(profile: &SessionProfile) -> Result<(), String> {
    let Ok(ssh) = ssh_connection(profile) else {
        return Ok(());
    };
    let mut ids = HashSet::new();
    for identity in &ssh.identity_refs {
        if identity.id.trim().is_empty() {
            return Err(format!("Profile {} 包含空 identity id", profile.id));
        }
        if !ids.insert(identity.id.as_str()) {
            return Err(format!(
                "Profile {} 中存在重复 identity id: {}",
                profile.id, identity.id
            ));
        }
    }
    Ok(())
}

pub(super) fn normalize_client_identity<F>(
    expected_id: &str,
    mut identity: IdentityRef,
    check_secret: F,
) -> Result<IdentityRef, String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    if identity.id != expected_id || identity.id.trim().is_empty() {
        return Err("identity id 不可修改且不能为空".to_string());
    }
    identity.label = identity.label.trim().to_string();
    if identity.label.is_empty() {
        return Err("identity label 不能为空".to_string());
    }
    identity.fingerprint_sha256 = identity
        .fingerprint_sha256
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    identity.path = identity
        .path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    identity.secret_ref = identity
        .secret_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    match identity.source {
        IdentitySource::ProfileVault => {
            let secret_ref = identity
                .secret_ref
                .as_deref()
                .ok_or_else(|| "Profile Vault identity 必须包含 secretRef".to_string())?;
            check_secret(secret_ref)?;
            identity.path = None;
        }
        IdentitySource::SystemFile => {
            if identity.path.is_none() {
                return Err("System File identity 必须包含私钥路径".to_string());
            }
            identity.secret_ref = None;
        }
        IdentitySource::Agent | IdentitySource::PublicKeyOnly => {
            identity.secret_ref = None;
        }
    }
    Ok(identity)
}

pub(super) fn merge_expected_client_identity_update(
    current_identity: &IdentityRef,
    expected_identity: &IdentityRef,
    incoming_identity: IdentityRef,
) -> Result<IdentityRef, String> {
    if current_identity.id != incoming_identity.id || expected_identity.id != incoming_identity.id {
        return Err("expectedIdentity 与更新目标不是同一个 identity".to_string());
    }
    let expected = serde_json::to_value(expected_identity)
        .map_err(|error| format!("序列化 expectedIdentity 失败: {error}"))?;
    let current = serde_json::to_value(current_identity)
        .map_err(|error| format!("序列化当前 identity 失败: {error}"))?;
    let incoming = serde_json::to_value(&incoming_identity)
        .map_err(|error| format!("序列化待更新 identity 失败: {error}"))?;
    let merged = merge_expected_json_value(
        "Client Identity",
        "identity",
        &expected,
        &current,
        &incoming,
    )?;
    serde_json::from_value(merged)
        .map_err(|error| format!("反序列化合并后的 identity 失败: {error}"))
}

pub(super) fn replace_client_identity(
    store: &mut SessionStore,
    profile_id: &str,
    identity_id: &str,
    identity: IdentityRef,
) -> Result<(SessionSummary, Option<String>), String> {
    let profile = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("unknown session: {profile_id}"))?;
    let ssh = ssh_connection_mut(profile)?;
    let matching = ssh
        .identity_refs
        .iter()
        .enumerate()
        .filter(|(_, identity)| identity.id == identity_id)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let index = match matching.as_slice() {
        [index] => *index,
        [] => return Err(format!("unknown client identity: {identity_id}")),
        _ => {
            return Err(format!(
                "Profile {profile_id} 中存在重复 identity id: {identity_id}"
            ));
        }
    };
    let old_secret_ref = ssh.identity_refs[index].secret_ref.clone();
    ssh.identity_refs[index] = identity;
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == profile_id)
        .ok_or_else(|| format!("session summary is missing: {profile_id}"))?;
    Ok((summary, old_secret_ref))
}

pub(super) fn remove_client_identity(
    store: &mut SessionStore,
    profile_id: &str,
    identity_id: &str,
) -> Result<(SessionSummary, Option<String>), String> {
    let current = find_client_identity(store, profile_id, identity_id)?;
    let profile = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
        .ok_or_else(|| format!("unknown session: {profile_id}"))?;
    let ssh = ssh_connection_mut(profile)?;
    if ssh
        .jumps
        .iter()
        .any(|jump| jump.identity_ref.as_deref() == Some(identity_id))
    {
        return Err("identity 正被 Jump Host 使用，无法移除".to_string());
    }
    ssh.identity_refs
        .retain(|identity| identity.id != identity_id);
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == profile_id)
        .ok_or_else(|| format!("session summary is missing: {profile_id}"))?;
    Ok((summary, current.secret_ref))
}

pub(super) fn canonical_secret_ref(secret_ref: &str) -> Option<String> {
    let secret_ref = secret_ref.trim();
    if secret_ref.is_empty() || secret_ref.contains('\0') {
        return None;
    }
    if let Some(account) = secret_ref.strip_prefix("stronghold:") {
        return (!account.is_empty()).then(|| format!("stronghold:{account}"));
    }
    let account = secret_ref.strip_prefix("keychain:").unwrap_or(secret_ref);
    (!account.is_empty()).then(|| format!("keychain:{account}"))
}

pub(super) fn secret_ref_usage_count(store: &SessionStore, secret_ref: &str) -> usize {
    let Some(expected) = canonical_secret_ref(secret_ref) else {
        return 0;
    };
    let profile_count = store
        .profiles
        .iter()
        .flat_map(profile_secret_ref_occurrences)
        .filter(|secret_ref| secret_ref == &expected)
        .count();
    let one_key_count = store
        .one_keys
        .iter()
        .flat_map(one_key_secret_refs)
        .filter(|secret_ref| secret_ref == &expected)
        .count();
    profile_count + one_key_count
}

pub(super) fn profile_secret_refs(profile: &SessionProfile) -> HashSet<String> {
    profile_secret_ref_occurrences(profile)
        .into_iter()
        .collect()
}

pub(super) fn client_identity_mutation_response<F>(
    store: &SessionStore,
    summary: SessionSummary,
    old_secret_ref: Option<&str>,
    delete_orphan: bool,
    delete_secret: F,
) -> ClientIdentityMutationResponse
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let Some(old_secret_ref) = old_secret_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return ClientIdentityMutationResponse {
            summary,
            old_secret_deleted: false,
            old_secret_shared: false,
            cleanup_warning: None,
        };
    };
    let old_secret_shared = secret_ref_usage_count(store, old_secret_ref) > 0;
    if !delete_orphan || old_secret_shared {
        return ClientIdentityMutationResponse {
            summary,
            old_secret_deleted: false,
            old_secret_shared,
            cleanup_warning: None,
        };
    }
    match delete_secret(old_secret_ref) {
        Ok(()) => ClientIdentityMutationResponse {
            summary,
            old_secret_deleted: true,
            old_secret_shared: false,
            cleanup_warning: None,
        },
        Err(error) => ClientIdentityMutationResponse {
            summary,
            old_secret_deleted: false,
            old_secret_shared: false,
            cleanup_warning: Some(format!("Profile 已保存，但旧 secret 清理失败: {error}")),
        },
    }
}

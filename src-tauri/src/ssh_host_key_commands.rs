use super::*;

#[tauri::command]
pub(crate) fn list_host_keys(state: State<'_, AppState>) -> Result<HostKeyStore, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.host_keys.clone())
}

#[tauri::command]
pub(crate) fn evaluate_host_key(
    state: State<'_, AppState>,
    profile_id: String,
    observation: HostKeyObservation,
) -> Result<HostKeyEvaluation, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    store.evaluate_host_key(&profile_id, &observation)
}

#[tauri::command]
pub(crate) fn apply_host_key_decision(
    state: State<'_, AppState>,
    request: HostKeyDecisionRequest,
) -> Result<Option<portmate_core::TrustedHostKey>, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    if request.decision == HostKeyDecision::TrustOnce {
        let trusted =
            temporary_trusted_host_key(&store, &request.profile_id, &request.observation)?;
        drop(store);
        remember_one_time_host_key(state.inner(), &request.profile_id, trusted.clone())?;
        return Ok(Some(trusted));
    }
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        apply_persistent_host_key_decision(
            next_store,
            &request.profile_id,
            &request.observation,
            request.decision,
        )
    })
}

#[tauri::command]
pub(crate) async fn scan_ssh_host_key(
    state: State<'_, AppState>,
    window: WebviewWindow,
    request: HostKeyScanRequest,
) -> Result<HostKeyScanResult, String> {
    let profile = normalize_session_profile(request.profile);
    let credentials = request
        .credential_handle
        .as_deref()
        .map(|credential_handle| {
            consume_session_credentials_for_owner(
                &state.session_credentials,
                window.label(),
                &profile.id,
                credential_handle,
                Instant::now(),
            )
        })
        .transpose()?;
    if let Some(credentials) = credentials.as_ref() {
        validate_session_credential_binding(&profile, &credentials.binding)?;
    }
    scan_ssh_host_key_inner(
        state.inner(),
        profile,
        credentials
            .as_ref()
            .and_then(|credentials| credentials.password.as_deref()),
        credentials
            .as_ref()
            .and_then(|credentials| credentials.passphrase.as_deref()),
    )
    .await
}

#[tauri::command]
pub(crate) fn trust_scanned_host_key(
    state: State<'_, AppState>,
    request: TrustScannedHostKeyRequest,
) -> Result<Option<portmate_core::TrustedHostKey>, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let profile = normalize_session_profile(request.profile);
    let profile_id = profile.id.clone();
    let policy = validate_scanned_host_key_profile_snapshot(
        &store,
        &profile,
        &request.observation,
    )?;
    if request.decision == HostKeyDecision::TrustOnce {
        let trusted =
            temporary_trusted_host_key_for_policy(&profile_id, &policy, &request.observation)?;
        drop(store);
        remember_one_time_host_key(state.inner(), &profile_id, trusted.clone())?;
        return Ok(Some(trusted));
    }
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        apply_persistent_host_key_decision_with_policy(
            next_store,
            &profile_id,
            &policy,
            &request.observation,
            request.decision,
        )
    })
}

pub(super) fn validate_scanned_host_key_profile_snapshot(
    store: &SessionStore,
    scanned_profile: &SessionProfile,
    observation: &HostKeyObservation,
) -> Result<portmate_core::HostKeyPolicy, String> {
    let current_profile = store
        .profile(&scanned_profile.id)
        .ok_or_else(|| "Host Key 扫描对应的 Profile 已删除，请重新扫描".to_string())?;
    let current_profile = normalize_session_profile(current_profile);
    if host_key_scan_connection_snapshot(&current_profile)?
        != host_key_scan_connection_snapshot(scanned_profile)?
    {
        return Err("SSH 配置已在 Host Key 扫描后变化，请重新扫描".to_string());
    }

    host_key_scan_policy_for_observation(scanned_profile, observation)
}

fn host_key_scan_connection_snapshot(profile: &SessionProfile) -> Result<ConnectionConfig, String> {
    let mut connection = profile.connection.clone();
    match &mut connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            // Persistent host-key mirrors may legitimately change after a scan. They do not
            // alter the endpoint or route that produced the observation.
            ssh.trusted_host_keys.clear();
        }
        _ => return Err(format!("profile is not SSH-backed: {}", profile.id)),
    }
    Ok(connection)
}

fn host_key_scan_policy_for_observation(
    profile: &SessionProfile,
    observation: &HostKeyObservation,
) -> Result<portmate_core::HostKeyPolicy, String> {
    let ssh = ssh_connection(profile)?;
    let target_alias = ssh
        .host_key_policy
        .alias
        .as_deref()
        .unwrap_or(profile.id.as_str());
    if observation.host == ssh.endpoint.host
        && observation.port == ssh.endpoint.port
        && observation.alias.as_deref() == Some(target_alias)
    {
        return Ok(ssh.host_key_policy.clone());
    }
    for jump in &ssh.jumps {
        let policy = jump_host_key_policy(ssh, jump);
        if observation.host == jump.host
            && observation.port == jump.port
            && observation.alias.as_deref() == policy.alias.as_deref()
        {
            return Ok(policy);
        }
    }
    Err("Host Key 扫描结果与当前 SSH 目标或 Jump Host 不匹配，请重新扫描".to_string())
}

#[tauri::command]
pub(crate) fn import_known_hosts(
    state: State<'_, AppState>,
    request: KnownHostsImportRequest,
) -> Result<HostKeyStore, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        if next_store.profile(&request.profile_id).is_none() {
            return Err(format!("unknown session: {}", request.profile_id));
        }
        let previous_count = next_store.host_keys.keys.len();
        next_store
            .host_keys
            .import_known_hosts(&request.profile_id, &request.contents);
        let imported = next_store.host_keys.keys[previous_count..].to_vec();
        mirror_persistent_host_keys(next_store, &imported)?;
        Ok(next_store.host_keys.clone())
    })
}

#[tauri::command]
pub(crate) fn export_known_hosts(state: State<'_, AppState>) -> Result<String, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.host_keys.export_known_hosts())
}

#[tauri::command]
pub(crate) fn delete_host_key(
    state: State<'_, AppState>,
    key_id: String,
) -> Result<HostKeyStore, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        Ok(delete_host_keys_from_store(next_store, &[key_id]))
    })
}

#[tauri::command]
pub(crate) fn delete_host_keys(
    state: State<'_, AppState>,
    key_ids: Vec<String>,
) -> Result<HostKeyStore, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        Ok(delete_host_keys_from_store(next_store, &key_ids))
    })
}

pub(super) fn delete_host_keys_from_store(
    store: &mut SessionStore,
    key_ids: &[String],
) -> HostKeyStore {
    store
        .host_keys
        .keys
        .retain(|key| !key_ids.contains(&key.id));
    for profile in &mut store.profiles {
        if let ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) = &mut profile.connection {
            ssh.trusted_host_keys
                .retain(|key| !key_ids.contains(&key.id));
        }
    }
    store.host_keys.clone()
}

#[tauri::command]
pub(crate) fn update_host_key(
    state: State<'_, AppState>,
    request: HostKeyUpdateRequest,
) -> Result<HostKeyStore, String> {
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        update_host_key_in_store(next_store, request)
    })
}

pub(super) fn update_host_key_in_store(
    store: &mut SessionStore,
    request: HostKeyUpdateRequest,
) -> Result<HostKeyStore, String> {
    if request.expected_key.id != request.key_id {
        return Err("expectedKey 与更新目标不是同一个 host key".to_string());
    }
    let alias = request.alias.trim().to_string();
    if alias.is_empty() {
        return Err("host key alias 不能为空".to_string());
    }
    let host = request.host.trim().to_string();
    if host.is_empty() {
        return Err("host key host 不能为空".to_string());
    }
    if request.port == 0 {
        return Err("host key 端口必须在 1-65535 之间".to_string());
    }
    let profile_id = request
        .profile_id
        .as_deref()
        .map(str::trim)
        .filter(|profile_id| !profile_id.is_empty())
        .map(str::to_string);
    let label = request
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string);

    let current_key = store
        .host_keys
        .keys
        .iter()
        .find(|key| key.id == request.key_id)
        .cloned()
        .ok_or_else(|| format!("unknown host key: {}", request.key_id))?;
    let mut incoming_key = request.expected_key.clone();
    incoming_key.profile_id = profile_id;
    incoming_key.alias = alias;
    incoming_key.host = host;
    incoming_key.port = request.port;
    incoming_key.scope = request.scope;
    incoming_key.label = label;
    let merged_key =
        merge_expected_host_key_update(&current_key, &request.expected_key, incoming_key)?;
    if merged_key.scope == HostKeyScope::Profile {
        let Some(profile_id) = merged_key.profile_id.as_deref() else {
            return Err("Profile scope host key 必须选择 Profile".to_string());
        };
        if store.profile(profile_id).is_none() {
            return Err(format!("unknown session: {profile_id}"));
        }
    } else if let Some(profile_id) = merged_key.profile_id.as_deref() {
        if store.profile(profile_id).is_none() {
            return Err(format!("unknown session: {profile_id}"));
        }
    }

    let Some(key) = store
        .host_keys
        .keys
        .iter_mut()
        .find(|key| key.id == request.key_id)
    else {
        return Err(format!("unknown host key: {}", request.key_id));
    };
    apply_host_key_editable_fields(key, &merged_key);

    for profile in &mut store.profiles {
        if let ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) = &mut profile.connection {
            for profile_key in &mut ssh.trusted_host_keys {
                if profile_key.id == request.key_id {
                    apply_host_key_editable_fields(profile_key, &merged_key);
                }
            }
        }
    }

    Ok(store.host_keys.clone())
}

pub(super) fn merge_expected_host_key_update(
    current_key: &TrustedHostKey,
    expected_key: &TrustedHostKey,
    incoming_key: TrustedHostKey,
) -> Result<TrustedHostKey, String> {
    if current_key.id != incoming_key.id || expected_key.id != incoming_key.id {
        return Err("expectedKey 与更新目标不是同一个 host key".to_string());
    }
    let expected = serde_json::to_value(expected_key)
        .map_err(|error| format!("序列化 expectedKey 失败: {error}"))?;
    let current = serde_json::to_value(current_key)
        .map_err(|error| format!("序列化当前 host key 失败: {error}"))?;
    let incoming = serde_json::to_value(&incoming_key)
        .map_err(|error| format!("序列化待更新 host key 失败: {error}"))?;
    let merged = merge_expected_json_value("Host Key", "hostKey", &expected, &current, &incoming)?;
    serde_json::from_value(merged)
        .map_err(|error| format!("反序列化合并后的 host key 失败: {error}"))
}

fn apply_host_key_editable_fields(target: &mut TrustedHostKey, source: &TrustedHostKey) {
    target.profile_id.clone_from(&source.profile_id);
    target.alias.clone_from(&source.alias);
    target.host.clone_from(&source.host);
    target.port = source.port;
    target.scope = source.scope;
    target.label.clone_from(&source.label);
}

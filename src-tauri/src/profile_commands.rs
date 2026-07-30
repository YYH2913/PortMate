use super::*;

pub(super) fn validate_profile_tunnels(profile: &SessionProfile) -> Result<(), String> {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => validate_tunnels(&ssh.tunnels),
        _ => Ok(()),
    }
}

pub(super) fn apply_proxy_password_update_with_io<WriteSecret>(
    profile: &mut SessionProfile,
    update: Option<ProxyPasswordUpdate>,
    mut write_secret: WriteSecret,
) -> Result<Option<String>, String>
where
    WriteSecret: FnMut(Option<SecretStorage>, &str) -> Result<String, String>,
{
    let Some(update) = update else {
        return Ok(None);
    };
    let proxy =
        profile_proxy_mut(profile).ok_or_else(|| "当前会话协议不支持代理密码".to_string())?;
    match update {
        ProxyPasswordUpdate::Set { password, storage } => {
            let password = Zeroizing::new(password);
            validate_proxy_credentials(proxy.kind, &proxy.username, password.as_str())?;
            let secret_ref = write_secret(storage, password.as_str())?;
            proxy.password_secret_ref = Some(secret_ref.clone());
            Ok(Some(secret_ref))
        }
        ProxyPasswordUpdate::Clear => {
            proxy.password_secret_ref = None;
            Ok(None)
        }
    }
}

pub(super) fn validate_profile_transport_change(
    current_profile: Option<&SessionProfile>,
    next_profile: &SessionProfile,
    runtime_status: Option<SessionStatus>,
) -> Result<(), String> {
    let Some(current_profile) = current_profile else {
        return Ok(());
    };
    let current_kind = current_profile.connection.kind();
    let next_kind = next_profile.connection.kind();
    if current_kind == next_kind {
        return Ok(());
    }
    if matches!(
        runtime_status,
        Some(SessionStatus::Connecting | SessionStatus::Connected | SessionStatus::Reconnecting)
    ) {
        let status = runtime_status.expect("active runtime status was matched above");
        return Err(format!(
            "会话仍在运行（{status:?}，当前协议 {current_kind:?}）；切换到 {next_kind:?} 前请先关闭会话"
        ));
    }
    Ok(())
}

pub(super) fn merge_expected_profile_update(
    current_profile: Option<&SessionProfile>,
    expected_profile: Option<&SessionProfile>,
    incoming_profile: SessionProfile,
) -> Result<SessionProfile, String> {
    match (current_profile, expected_profile) {
        (Some(current), Some(expected)) => {
            if current.id != incoming_profile.id || expected.id != incoming_profile.id {
                return Err("expectedProfile 与保存目标不是同一个 Profile".to_string());
            }
            let expected = serde_json::to_value(expected)
                .map_err(|error| format!("序列化 expectedProfile 失败: {error}"))?;
            let current = serde_json::to_value(current)
                .map_err(|error| format!("序列化当前 Profile 失败: {error}"))?;
            let incoming = serde_json::to_value(&incoming_profile)
                .map_err(|error| format!("序列化待保存 Profile 失败: {error}"))?;
            let merged =
                merge_expected_json_value("Profile", "profile", &expected, &current, &incoming)?;
            serde_json::from_value(merged)
                .map_err(|error| format!("反序列化合并后的 Profile 失败: {error}"))
        }
        (Some(_), None) => Err("保存现有 Profile 必须提供 expectedProfile 版本".to_string()),
        (None, Some(_)) => Err("Profile 已被其他操作删除，请刷新会话列表".to_string()),
        (None, None) => Ok(incoming_profile),
    }
}

pub(super) fn validate_expected_proxy_password(
    current_profile: Option<&SessionProfile>,
    expected_profile: Option<&SessionProfile>,
) -> Result<(), String> {
    let (Some(current), Some(expected)) = (current_profile, expected_profile) else {
        return Ok(());
    };
    let current_ref = profile_proxy(current).and_then(|proxy| proxy.password_secret_ref.as_deref());
    let expected_ref =
        profile_proxy(expected).and_then(|proxy| proxy.password_secret_ref.as_deref());
    if current_ref != expected_ref {
        return Err("代理密码已在其他操作中更新，请重新打开设置后再保存".to_string());
    }
    Ok(())
}

pub(super) fn merge_expected_json_value(
    entity: &str,
    path: &str,
    expected: &serde_json::Value,
    current: &serde_json::Value,
    incoming: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    if incoming == expected {
        return Ok(current.clone());
    }
    if current == expected || incoming == current {
        return Ok(incoming.clone());
    }

    let (
        serde_json::Value::Object(expected),
        serde_json::Value::Object(current),
        serde_json::Value::Object(incoming),
    ) = (expected, current, incoming)
    else {
        return Err(format!(
            "{entity} 字段已被其他操作修改，请刷新后重试: {path}"
        ));
    };
    if expected.len() != current.len()
        || expected.len() != incoming.len()
        || !expected
            .keys()
            .all(|key| current.contains_key(key) && incoming.contains_key(key))
    {
        return Err(format!(
            "{entity} 结构已被其他操作修改，请刷新后重试: {path}"
        ));
    }

    let mut merged = serde_json::Map::with_capacity(expected.len());
    for (key, expected_value) in expected {
        let current_value = current
            .get(key)
            .expect("merged JSON key sets were checked above");
        let incoming_value = incoming
            .get(key)
            .expect("merged JSON key sets were checked above");
        let child_path = format!("{path}.{key}");
        merged.insert(
            key.clone(),
            merge_expected_json_value(
                entity,
                &child_path,
                expected_value,
                current_value,
                incoming_value,
            )?,
        );
    }
    Ok(serde_json::Value::Object(merged))
}

#[tauri::command]
pub(crate) fn save_session_profile(
    state: State<'_, AppState>,
    profile: SessionProfile,
    expected_profile: Option<SessionProfile>,
    proxy_password_update: Option<ProxyPasswordUpdate>,
) -> Result<SessionSummary, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    validate_triggers(&profile.triggers)?;
    validate_profile_tunnels(&profile)?;
    let mut profile = normalize_session_profile(profile);
    let expected_profile = expected_profile.map(normalize_session_profile);
    validate_profile_client_identity_ids(&profile)?;
    validate_logging_retention(&profile)?;
    validate_transfer_default_local_dir(&profile)?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let current_profile = store.profile(&profile.id);
    store.validate_profile_capacity(&profile.id)?;
    if proxy_password_update.is_some() {
        validate_expected_proxy_password(current_profile.as_ref(), expected_profile.as_ref())?;
    }
    profile = merge_expected_profile_update(
        current_profile.as_ref(),
        expected_profile.as_ref(),
        profile,
    )?;
    validate_profile_client_identity_ids(&profile)?;
    validate_logging_retention(&profile)?;
    validate_transfer_default_local_dir(&profile)?;
    validate_triggers(&profile.triggers)?;
    validate_profile_tunnels(&profile)?;
    let runtime_status = store
        .runtimes
        .iter()
        .find(|runtime| runtime.session_id == profile.id)
        .map(|runtime| runtime.status);
    validate_profile_transport_change(current_profile.as_ref(), &profile, runtime_status)?;
    let old_secret_refs = current_profile
        .as_ref()
        .map(profile_secret_refs)
        .unwrap_or_default();
    let generated_proxy_secret_ref =
        apply_proxy_password_update_with_io(&mut profile, proxy_password_update, write_new_secret)?;
    let new_secret_refs = profile_secret_refs(&profile);
    let save_result = (|| {
        for secret_ref in new_secret_refs.difference(&old_secret_refs) {
            if is_reserved_internal_secret_ref(secret_ref) {
                return Err("内部保留 secretRef 不能用作 Profile 凭据".to_string());
            }
            read_secret_from_store(secret_ref).map_err(|error| {
                format!("新增 Profile secretRef 无法读取 ({secret_ref}): {error}")
            })?;
        }
        commit_store_mutation(&mut store, &state.store_path, |next_store| {
            next_store.validate_profile_capacity(&profile.id)?;
            Ok(next_store.upsert_profile(profile))
        })
    })();
    let summary = match save_result {
        Ok(saved) => saved,
        Err(error) => {
            if let Some(secret_ref) = generated_proxy_secret_ref.as_deref() {
                if let Err(cleanup_error) = delete_secret_from_store(secret_ref) {
                    return Err(format!(
                        "{error}；新代理密码 secret 回收失败，已保留孤立副本: {cleanup_error}"
                    ));
                }
            }
            return Err(error);
        }
    };
    for secret_ref in old_secret_refs {
        if secret_ref_usage_count(&store, &secret_ref) == 0 {
            if let Err(error) = delete_secret_from_store(&secret_ref) {
                eprintln!("PortMate: profile saved but orphan secret cleanup failed: {error}");
            }
        }
    }
    drop(store);
    clear_log_retention_check(&state.store_path, &summary.profile.id);
    if let Some(app_handle) = &state.app_handle {
        let _ = app_handle.emit("portmate-session-profile-updated", summary.clone());
    }
    Ok(summary)
}

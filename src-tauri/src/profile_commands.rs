use super::*;

pub(super) fn validate_profile_tunnels(profile: &SessionProfile) -> Result<(), String> {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => validate_tunnels(&ssh.tunnels),
        _ => Ok(()),
    }
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

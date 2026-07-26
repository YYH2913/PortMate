use super::*;

#[tauri::command]
pub(crate) fn save_secret(
    state: State<'_, AppState>,
    request: SecretWriteRequest,
) -> Result<SecretWriteResponse, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let secret = request.secret.trim_end_matches(['\r', '\n']).to_string();
    if secret.trim().is_empty() {
        return Err("密钥内容不能为空".to_string());
    }
    let secret_ref =
        if let Some(secret_ref) = request.secret_ref.filter(|value| !value.trim().is_empty()) {
            if is_reserved_internal_secret_ref(&secret_ref) {
                return Err("内部保留 secretRef 不能通过通用凭据接口写入".to_string());
            }
            write_secret_to_store(&secret_ref, &secret)?;
            secret_ref
        } else {
            write_new_secret(request.storage, &secret)?
        };
    Ok(SecretWriteResponse { secret_ref })
}

#[tauri::command]
pub(crate) fn delete_secret(state: State<'_, AppState>, secret_ref: String) -> Result<(), String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    if is_reserved_internal_secret_ref(&secret_ref) {
        return Err("内部保留 secretRef 不能通过通用凭据接口删除".to_string());
    }
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let usage_count = secret_ref_usage_count(&store, &secret_ref);
    if usage_count > 0 {
        return Err(format!(
            "secretRef 仍被 {usage_count} 个凭据字段引用，无法删除"
        ));
    }
    delete_secret_from_store(&secret_ref)
}

#[tauri::command]
pub(crate) fn has_secret(state: State<'_, AppState>, secret_ref: String) -> Result<bool, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    if is_reserved_internal_secret_ref(&secret_ref) {
        return Err("内部保留 secretRef 不能通过通用凭据接口读取".to_string());
    }
    match read_secret_from_store(&secret_ref) {
        Ok(_) => Ok(true),
        Err(error)
            if error.contains("NoEntry")
                || error.contains("No credential")
                || error.contains("不存在该 secretRef") =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

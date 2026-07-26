use super::*;

#[tauri::command]
pub(crate) async fn send_one_key(
    state: State<'_, AppState>,
    request: SendOneKeyRequest,
) -> Result<SessionEvent, String> {
    let (value, origin, prompt_event_id, prompt_validation) = {
        let _credential_guard = lock_credential_operations(state.inner())?;
        let store = state.store.lock().map_err(|error| error.to_string())?;
        let one_key = store
            .one_keys
            .iter()
            .find(|one_key| one_key.id == request.id)
            .ok_or_else(|| "OneKey 已被删除，请刷新后重试".to_string())?;
        if !one_key
            .session_ids
            .iter()
            .any(|session_id| session_id == &request.session_id)
        {
            return Err("OneKey 未绑定当前会话".to_string());
        }
        let status = store
            .runtimes
            .iter()
            .find(|runtime| runtime.session_id == request.session_id)
            .map(|runtime| runtime.status)
            .ok_or_else(|| format!("unknown session: {}", request.session_id))?;
        if status != SessionStatus::Connected {
            return Err("OneKey 只能发送到已连接会话".to_string());
        }
        let (prompt_event_id, prompt_validation) = match request.source {
            OneKeySendSource::Manual => (None, None),
            OneKeySendSource::PromptCompletion => {
                let prompt_event_id = request
                    .prompt_event_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|event_id| !event_id.is_empty())
                    .ok_or_else(|| "OneKey 提示补全缺少 promptEventId".to_string())?;
                validate_one_key_prompt_completion(
                    &store,
                    one_key,
                    &request.session_id,
                    request.field,
                    prompt_event_id,
                )?;
                let prompt_event_id = prompt_event_id.to_string();
                (
                    Some(prompt_event_id.clone()),
                    Some(OneKeyPromptValidation {
                        one_key_id: request.id.clone(),
                        one_key_updated_at: one_key.updated_at,
                        field: request.field,
                        prompt_event_id,
                    }),
                )
            }
        };
        let value = match request.field {
            OneKeyField::Username => Zeroizing::new(one_key.username.clone()),
            OneKeyField::Password => Zeroizing::new(
                read_optional_secret_ref(
                    one_key.password_secret_ref.as_deref(),
                    "OneKey password",
                )?
                .ok_or_else(|| "OneKey 没有保存密码".to_string())?,
            ),
            OneKeyField::Passphrase => Zeroizing::new(
                read_optional_secret_ref(
                    one_key.passphrase_secret_ref.as_deref(),
                    "OneKey passphrase",
                )?
                .ok_or_else(|| "OneKey 没有保存私钥口令".to_string())?,
            ),
        };
        let origin = match request.source {
            OneKeySendSource::Manual => "one-key",
            OneKeySendSource::PromptCompletion => "one-key-completion",
        };
        (value, origin, prompt_event_id, prompt_validation)
    };
    send_one_key_value(
        state.inner().session_io(),
        &request.session_id,
        value.as_str(),
        origin,
        prompt_event_id.as_deref(),
        prompt_validation.as_ref(),
    )
    .await
}

#[tauri::command]
pub(crate) fn list_one_keys(state: State<'_, AppState>) -> Result<Vec<OneKeySummary>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(one_key_summaries(&store))
}

#[tauri::command]
pub(crate) fn save_one_key(
    state: State<'_, AppState>,
    request: SaveOneKeyRequest,
) -> Result<OneKeyMutationResponse, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let existing = request.id.as_deref().and_then(|id| {
        store
            .one_keys
            .iter()
            .find(|one_key| one_key.id == id)
            .cloned()
    });
    if request.id.is_some() && existing.is_none() {
        return Err("OneKey 已被删除，请刷新后重试".to_string());
    }
    if existing.is_none() && store.one_keys.len() >= MAX_ONE_KEYS {
        return Err(format!("OneKey 最多保存 {MAX_ONE_KEYS} 条"));
    }

    let label = truncate_one_key_text(request.label.trim(), MAX_ONE_KEY_LABEL_CHARACTERS);
    if label.is_empty() || label.contains('\0') {
        return Err("OneKey 名称不能为空".to_string());
    }
    let username = truncate_one_key_text(request.username.trim(), MAX_ONE_KEY_USERNAME_CHARACTERS);
    if username.is_empty() || username.contains(['\0', '\r', '\n']) {
        return Err("OneKey 用户名不能为空且不能包含换行或 NUL".to_string());
    }
    let session_ids = normalize_one_key_sessions(&store, request.kind, request.session_ids)?;
    let now = Utc::now();
    let current_password = existing
        .as_ref()
        .and_then(|one_key| one_key.password_secret_ref.clone());
    let current_passphrase = existing
        .as_ref()
        .and_then(|one_key| one_key.passphrase_secret_ref.clone());
    let current_identity = existing
        .as_ref()
        .and_then(|one_key| one_key.identity.clone());
    let old_refs = existing
        .as_ref()
        .map(one_key_secret_refs)
        .unwrap_or_default();
    let identity = apply_one_key_identity_update(
        &store,
        request.kind,
        &session_ids,
        current_identity,
        request.identity_update,
    )?;
    let mut generated = Vec::new();
    let password_secret_ref = match apply_one_key_secret_update(
        current_password,
        request.password_update,
        &mut generated,
    ) {
        Ok(secret_ref) => secret_ref,
        Err(error) => {
            cleanup_generated_one_key_secrets(&generated);
            return Err(error);
        }
    };
    let passphrase_update = if request.kind == OneKeyKind::Account {
        OneKeySecretUpdate::Clear
    } else {
        request.passphrase_update
    };
    let passphrase_secret_ref =
        match apply_one_key_secret_update(current_passphrase, passphrase_update, &mut generated) {
            Ok(secret_ref) => secret_ref,
            Err(error) => {
                cleanup_generated_one_key_secrets(&generated);
                return Err(error);
            }
        };
    if password_secret_ref.is_none() && passphrase_secret_ref.is_none() && identity.is_none() {
        cleanup_generated_one_key_secrets(&generated);
        return Err("OneKey 至少需要保存密码、私钥口令或公钥身份".to_string());
    }

    let one_key = OneKeyCredential {
        id: existing
            .as_ref()
            .map(|one_key| one_key.id.clone())
            .unwrap_or_else(|| format!("onekey:{}", Uuid::new_v4())),
        label,
        kind: request.kind,
        username,
        password_secret_ref,
        passphrase_secret_ref,
        identity,
        session_ids,
        created_at: existing
            .as_ref()
            .map(|one_key| one_key.created_at)
            .unwrap_or(now),
        updated_at: now,
    };
    let saved_id = one_key.id.clone();
    let retained_refs = one_key_secret_refs(&one_key)
        .into_iter()
        .collect::<HashSet<_>>();
    if let Err(error) = commit_store_mutation(&mut store, &state.store_path, |next_store| {
        if let Some(index) = next_store
            .one_keys
            .iter()
            .position(|candidate| candidate.id == one_key.id)
        {
            next_store.one_keys[index] = one_key;
        } else {
            next_store.one_keys.push(one_key);
        }
        Ok(())
    }) {
        cleanup_generated_one_key_secrets(&generated);
        return Err(error);
    }
    cleanup_replaced_one_key_secrets(&store, old_refs, &retained_refs);
    Ok(OneKeyMutationResponse {
        items: one_key_summaries(&store),
        saved_id,
    })
}

#[tauri::command]
pub(crate) fn delete_one_key(
    state: State<'_, AppState>,
    request: DeleteOneKeyRequest,
) -> Result<Vec<OneKeySummary>, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let one_key = store
        .one_keys
        .iter()
        .find(|one_key| one_key.id == request.id)
        .cloned()
        .ok_or_else(|| "OneKey 已被删除，请刷新后重试".to_string())?;
    commit_store_mutation(&mut store, &state.store_path, |next_store| {
        next_store
            .one_keys
            .retain(|one_key| one_key.id != request.id);
        Ok(())
    })?;
    let retained = HashSet::new();
    cleanup_replaced_one_key_secrets(&store, one_key_secret_refs(&one_key), &retained);
    Ok(one_key_summaries(&store))
}

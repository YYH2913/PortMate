use super::*;

#[tauri::command]
pub(crate) async fn list_ssh_agent_identities() -> Result<Vec<IdentityRef>, String> {
    let identities = list_ssh_agent_identities_on_thread(None).await?;
    Ok(identities
        .into_iter()
        .enumerate()
        .map(|(index, identity)| {
            let public_key = identity.public_key();
            IdentityRef {
                id: format!("agent-{index}"),
                label: if identity.comment().trim().is_empty() {
                    format!("agent key {}", index + 1)
                } else {
                    identity.comment().to_string()
                },
                source: IdentitySource::Agent,
                fingerprint_sha256: compute_ssh_sha256_fingerprint(&public_key.public_key_base64())
                    .ok(),
                path: (!identity.comment().trim().is_empty())
                    .then(|| identity.comment().to_string()),
                secret_ref: None,
            }
        })
        .collect())
}

#[tauri::command]
pub(crate) fn update_client_identity(
    state: State<'_, AppState>,
    request: ClientIdentityUpdateRequest,
) -> Result<ClientIdentityMutationResponse, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let expected_identity =
        normalize_client_identity(&request.identity_id, request.expected_identity, |_| Ok(()))?;
    let incoming_identity = normalize_client_identity(
        &request.identity_id,
        IdentityRef {
            id: request.identity_id.clone(),
            label: request.label,
            source: request.source,
            fingerprint_sha256: request.fingerprint_sha256,
            path: request.path,
            secret_ref: request.secret_ref,
        },
        |_| Ok(()),
    )?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let current_identity = find_client_identity(&store, &request.profile_id, &request.identity_id)?;
    let identity = merge_expected_client_identity_update(
        &current_identity,
        &expected_identity,
        incoming_identity,
    )?;
    let identity = normalize_client_identity(&request.identity_id, identity, |secret_ref| {
        read_secret_from_store(secret_ref).map(|_| ())
    })?;
    let new_secret_ref = identity.secret_ref.clone();
    let (summary, old_secret_ref) =
        commit_store_mutation(&mut store, &state.store_path, |next_store| {
            replace_client_identity(
                next_store,
                &request.profile_id,
                &request.identity_id,
                identity,
            )
        })?;
    let cleanup_secret_ref = old_secret_ref.filter(|old_secret_ref| {
        new_secret_ref.as_deref().map(str::trim) != Some(old_secret_ref.trim())
    });
    Ok(client_identity_mutation_response(
        &store,
        summary,
        cleanup_secret_ref.as_deref(),
        true,
        delete_secret_from_store,
    ))
}

#[tauri::command]
pub(crate) fn rotate_client_identity(
    state: State<'_, AppState>,
    request: ClientIdentityRotateRequest,
) -> Result<ClientIdentityMutationResponse, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let private_key = request
        .private_key
        .trim_end_matches(['\r', '\n'])
        .to_string();
    if private_key.trim().is_empty() {
        return Err("私钥内容不能为空".to_string());
    }
    let (saved_passphrase_ref, current_secret_ref) = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        let current = find_client_identity(&store, &request.profile_id, &request.identity_id)?;
        if current.source != IdentitySource::ProfileVault {
            return Err("只有 Profile Vault identity 可以轮换私钥".to_string());
        }
        let profile = store
            .profiles
            .iter()
            .find(|profile| profile.id == request.profile_id)
            .ok_or_else(|| format!("unknown session: {}", request.profile_id))?;
        (
            ssh_connection(profile)?.passphrase_secret_ref.clone(),
            current.secret_ref,
        )
    };
    let saved_passphrase = saved_passphrase_ref
        .as_deref()
        .map(|secret_ref| read_optional_secret_ref(Some(secret_ref), "SSH private-key passphrase"))
        .transpose()?
        .flatten();
    let validation_passphrase = saved_passphrase
        .as_deref()
        .or(request.passphrase.as_deref());
    let decoded = decode_secret_key(&private_key, validation_passphrase).map_err(|error| {
        if saved_passphrase.is_some() {
            format!("新私钥无法使用 Profile 已保存的私钥口令解析: {error}")
        } else {
            format!("新私钥无法解析: {error}")
        }
    })?;
    let fingerprint_sha256 =
        compute_ssh_sha256_fingerprint(&decoded.public_key().public_key_base64())
            .map_err(|error| format!("无法计算新私钥指纹: {error}"))?;
    let storage = request.storage.or_else(|| {
        current_secret_ref
            .as_deref()
            .is_some_and(|secret_ref| secret_ref.trim().starts_with("stronghold:"))
            .then_some(SecretStorage::Portable)
    });
    let new_secret_ref = write_new_secret(storage, &private_key)?;

    let result = (|| {
        let mut store = state.store.lock().map_err(|error| error.to_string())?;
        let (summary, old_secret_ref) =
            commit_store_mutation(&mut store, &state.store_path, |next_store| {
                let current =
                    find_client_identity(next_store, &request.profile_id, &request.identity_id)?;
                if current.source != IdentitySource::ProfileVault {
                    return Err("只有 Profile Vault identity 可以轮换私钥".to_string());
                }
                let identity = IdentityRef {
                    fingerprint_sha256: Some(fingerprint_sha256),
                    secret_ref: Some(new_secret_ref.clone()),
                    path: None,
                    ..current
                };
                replace_client_identity(
                    next_store,
                    &request.profile_id,
                    &request.identity_id,
                    identity,
                )
            })?;
        Ok(client_identity_mutation_response(
            &store,
            summary,
            old_secret_ref.as_deref(),
            true,
            delete_secret_from_store,
        ))
    })();

    if result.is_err() {
        let _ = delete_secret_from_store(&new_secret_ref);
    }
    result
}

#[tauri::command]
pub(crate) fn delete_client_identity(
    state: State<'_, AppState>,
    request: ClientIdentityDeleteRequest,
) -> Result<ClientIdentityMutationResponse, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    let (summary, old_secret_ref) =
        commit_store_mutation(&mut store, &state.store_path, |next_store| {
            remove_client_identity(next_store, &request.profile_id, &request.identity_id)
        })?;
    Ok(client_identity_mutation_response(
        &store,
        summary,
        old_secret_ref.as_deref(),
        request.delete_secret,
        delete_secret_from_store,
    ))
}

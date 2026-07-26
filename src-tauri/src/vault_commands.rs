use super::*;

#[tauri::command]
pub(crate) fn portable_vault_status(
    state: State<'_, AppState>,
) -> Result<PortableVaultStatus, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    portable_vault_status_inner()
}

#[tauri::command]
pub(crate) fn unlock_portable_vault(
    state: State<'_, AppState>,
    request: PortableVaultUnlockRequest,
) -> Result<PortableVaultStatus, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    let password = Zeroizing::new(request.password);
    let context = portable_vault_context()?;
    unlock_portable_vault_in(context, password.as_str())?;
    portable_vault_status_inner()
}

#[tauri::command]
pub(crate) fn rotate_portable_vault_password(
    state: State<'_, AppState>,
    request: PortableVaultRotatePasswordRequest,
) -> Result<PortableVaultStatus, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let current_password = Zeroizing::new(request.current_password);
    let new_password = Zeroizing::new(request.new_password);
    let context = portable_vault_context()?;
    rotate_portable_vault_password_in(context, current_password.as_str(), new_password.as_str())?;
    portable_vault_status_inner()
}

#[tauri::command]
pub(crate) fn lock_portable_vault(
    state: State<'_, AppState>,
) -> Result<PortableVaultStatus, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    let context = portable_vault_context()?;
    context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?
        .take();
    portable_vault_status_inner()
}

#[tauri::command]
pub(crate) fn preview_profile_secret_migration(
    state: State<'_, AppState>,
    request: ProfileSecretMigrationRequest,
) -> Result<ProfileSecretMigrationPreview, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    let store = state.store.lock().map_err(|error| error.to_string())?;
    verify_store_snapshot_is_current(&state.store_path)?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let mut plan = build_profile_secret_migration_plan(&store, &request)?;
    let plan_token = profile_secret_migration_plan_token(&plan, &request);
    plan.preview.plan_token = plan_token;
    if plan.preview.eligible_secret_count > 0 {
        ensure_portable_vault_ready_for_migration()?;
    }
    Ok(plan.preview)
}

#[tauri::command]
pub(crate) fn migrate_profile_secrets(
    state: State<'_, AppState>,
    request: ProfileSecretMigrationRequest,
    expected_plan_token: String,
) -> Result<ProfileSecretMigrationResponse, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    verify_store_snapshot_is_current(&state.store_path)?;
    ensure_no_pending_profile_secret_migration(&state.store_path)?;
    let plan = build_profile_secret_migration_plan(&store, &request)?;
    let current_plan_token = profile_secret_migration_plan_token(&plan, &request);
    if expected_plan_token.trim().is_empty() || expected_plan_token != current_plan_token {
        return Err("凭据迁移预检已过期，请重新预检后再执行".to_string());
    }
    if plan.preview.eligible_secret_count > 0 {
        ensure_portable_vault_ready_for_migration()?;
    }
    migrate_profile_secrets_with_journal_io(
        &mut store,
        &request,
        read_secret_from_store,
        write_profile_secret_migration_batch,
        delete_profile_secret_migration_batch,
        |next_store, affected_profile_ids, target_refs, migration_id| {
            persist_profile_secret_migration(
                &state.store_path,
                next_store,
                affected_profile_ids,
                target_refs,
                migration_id,
            )
        },
        |event| persist_profile_secret_migration_journal_event(&state.store_path, event),
    )
}

#[tauri::command]
pub(crate) fn get_profile_secret_migration_recovery(
    state: State<'_, AppState>,
) -> Result<Option<ProfileSecretMigrationRecoverySummary>, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    let store = state.store.lock().map_err(|error| error.to_string())?;
    verify_store_snapshot_is_current(&state.store_path)?;
    let Some(journal) = load_profile_secret_migration_journal(&state.store_path)? else {
        return Ok(None);
    };
    Ok(Some(profile_secret_migration_recovery_summary(
        &store,
        &journal,
        portable_vault_recovery_ready()?,
    )))
}

#[tauri::command]
pub(crate) fn export_profile_secret_migration_diagnostics(
    state: State<'_, AppState>,
) -> Result<ProfileSecretMigrationDiagnosticExportResult, String> {
    let _credential_guard = lock_credential_operations(state.inner())?;
    let snapshot_lock = lock_store_snapshot(&state.store_path)?;
    let persisted_store = read_persisted_store_for_migration(&state.store_path)?;
    let result = export_profile_secret_migration_diagnostics_with_io(
        &state.store_path,
        &persisted_store,
        probe_secret_from_store,
        profile_secret_migration_diagnostic_vault_status(),
    );
    drop(snapshot_lock);
    result
}

#[tauri::command]
pub(crate) fn recover_profile_secret_migration(
    state: State<'_, AppState>,
    request: ProfileSecretMigrationRecoveryRequest,
) -> Result<ProfileSecretMigrationRecoveryResponse, String> {
    let migration_id = request.migration_id.trim().to_string();
    Uuid::parse_str(&migration_id).map_err(|_| "凭据迁移恢复 ID 无效".to_string())?;
    let _credential_guard = lock_credential_operations(state.inner())?;
    let store = state.store.lock().map_err(|error| error.to_string())?;
    verify_store_snapshot_is_current(&state.store_path)?;
    let Some(journal) = load_profile_secret_migration_journal(&state.store_path)? else {
        return Ok(ProfileSecretMigrationRecoveryResponse {
            migration_id,
            resolved: true,
            action: "already-resolved".to_string(),
            warnings: Vec::new(),
            pending: None,
        });
    };
    if journal.payload.migration_id != migration_id {
        return Err(format!(
            "当前待恢复迁移为 {}，与请求 ID 不一致",
            journal.payload.migration_id
        ));
    }
    let outcome = recover_profile_secret_migration_with_io(
        &store,
        &journal,
        probe_secret_from_store,
        delete_profile_secret_migration_batch,
        |event| persist_profile_secret_migration_journal_event(&state.store_path, event),
    )?;
    let pending = load_profile_secret_migration_journal(&state.store_path)?
        .map(|journal| {
            Ok::<_, String>(profile_secret_migration_recovery_summary(
                &store,
                &journal,
                portable_vault_recovery_ready()?,
            ))
        })
        .transpose()?;
    Ok(ProfileSecretMigrationRecoveryResponse {
        migration_id,
        resolved: outcome.resolved,
        action: outcome.action,
        warnings: outcome.warnings,
        pending,
    })
}

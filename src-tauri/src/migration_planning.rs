use super::*;

pub(super) fn secret_ref_storage(secret_ref: &str) -> SecretStorage {
    if canonical_secret_ref(secret_ref)
        .is_some_and(|secret_ref| secret_ref.starts_with("stronghold:"))
    {
        SecretStorage::Portable
    } else {
        SecretStorage::Native
    }
}

pub(super) fn is_reserved_internal_secret_ref(secret_ref: &str) -> bool {
    canonical_secret_ref(secret_ref).is_some_and(|secret_ref| {
        secret_ref == MCP_HTTP_TOKEN_REF
            || secret_ref == BUNDLE_SIGNING_KEY_REF
            || secret_ref == BUNDLE_SIGNING_KEY_PORTABLE_REF
            || secret_ref.starts_with("keychain:ipc-")
    })
}

pub(super) fn new_secret_ref(storage: SecretStorage) -> String {
    match storage {
        SecretStorage::Native => format!("keychain:{}", Uuid::new_v4()),
        SecretStorage::Portable => format!("stronghold:{}", Uuid::new_v4()),
    }
}

pub(super) fn profile_secret_ref_occurrences(profile: &SessionProfile) -> Vec<String> {
    let mut refs = Vec::new();
    if let Some(secret_ref) = profile_proxy(profile)
        .and_then(|proxy| proxy.password_secret_ref.as_deref())
        .and_then(canonical_secret_ref)
    {
        refs.push(secret_ref);
    }
    let Ok(ssh) = ssh_connection(profile) else {
        return refs;
    };
    for secret_ref in [
        ssh.password_secret_ref.as_deref(),
        ssh.passphrase_secret_ref.as_deref(),
    ] {
        if let Some(secret_ref) = secret_ref.and_then(canonical_secret_ref) {
            refs.push(secret_ref);
        }
    }
    for identity in &ssh.identity_refs {
        if identity.source != IdentitySource::ProfileVault {
            continue;
        }
        if let Some(secret_ref) = identity
            .secret_ref
            .as_deref()
            .and_then(canonical_secret_ref)
        {
            refs.push(secret_ref);
        }
    }
    for jump in &ssh.jumps {
        for secret_ref in [
            jump.password_secret_ref.as_deref(),
            jump.passphrase_secret_ref.as_deref(),
        ] {
            if let Some(secret_ref) = secret_ref.and_then(canonical_secret_ref) {
                refs.push(secret_ref);
            }
        }
    }
    refs
}

pub(super) fn build_profile_secret_migration_plan(
    store: &SessionStore,
    request: &ProfileSecretMigrationRequest,
) -> Result<ProfileSecretMigrationPlan, String> {
    if request.profile_ids.is_empty() {
        return Err("凭据迁移必须显式选择至少一个支持凭据的 Profile".to_string());
    }
    let mut requested = HashSet::new();
    let mut requested_ids = Vec::new();
    for profile_id in &request.profile_ids {
        let profile_id = profile_id.trim();
        if profile_id.is_empty() {
            return Err("凭据迁移包含空 Profile ID".to_string());
        }
        if requested.insert(profile_id.to_string()) {
            requested_ids.push(profile_id.to_string());
        }
    }
    for profile_id in &requested_ids {
        let profile = store
            .profiles
            .iter()
            .find(|profile| profile.id == *profile_id)
            .ok_or_else(|| format!("unknown session: {profile_id}"))?;
        if profile_proxy(profile).is_none() {
            return Err(format!("Profile {} 不支持 Profile 凭据迁移", profile.id));
        }
    }

    let selected_profile_ids = store
        .profiles
        .iter()
        .filter(|profile| requested.contains(&profile.id))
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    let mut source_ref_counts = BTreeMap::<String, usize>::new();
    let mut affected_profile_ids = Vec::new();
    let mut in_flight_source_refs = HashSet::new();
    let mut already_target_reference_count = 0;
    let mut excluded_reserved_reference_count = 0;
    for profile in store
        .profiles
        .iter()
        .filter(|profile| requested.contains(&profile.id))
    {
        let mut affected = false;
        let connection_in_flight = store.runtimes.iter().any(|runtime| {
            runtime.session_id == profile.id
                && matches!(
                    runtime.status,
                    SessionStatus::Connecting | SessionStatus::Reconnecting
                )
        });
        for secret_ref in profile_secret_ref_occurrences(profile) {
            if is_reserved_internal_secret_ref(&secret_ref) {
                excluded_reserved_reference_count += 1;
            } else if secret_ref_storage(&secret_ref) == request.target_storage {
                already_target_reference_count += 1;
            } else {
                *source_ref_counts.entry(secret_ref.clone()).or_default() += 1;
                if connection_in_flight {
                    in_flight_source_refs.insert(secret_ref);
                }
                affected = true;
            }
        }
        if affected {
            affected_profile_ids.push(profile.id.clone());
        }
    }
    let retained_shared_secret_count = source_ref_counts
        .iter()
        .filter(|(secret_ref, selected_count)| {
            secret_ref_usage_count(store, secret_ref) > **selected_count
        })
        .count();
    let eligible_reference_count = source_ref_counts.values().sum();
    let preview = ProfileSecretMigrationPreview {
        plan_token: String::new(),
        target_storage: request.target_storage,
        selected_profile_count: selected_profile_ids.len(),
        affected_profile_count: affected_profile_ids.len(),
        eligible_reference_count,
        eligible_secret_count: source_ref_counts.len(),
        retained_shared_secret_count,
        retained_in_flight_secret_count: in_flight_source_refs.len(),
        already_target_reference_count,
        excluded_reserved_reference_count,
    };
    Ok(ProfileSecretMigrationPlan {
        preview,
        selected_profile_ids,
        affected_profile_ids,
        source_ref_counts,
        in_flight_source_refs,
    })
}

fn update_migration_token_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}

pub(super) fn profile_secret_migration_plan_token(
    plan: &ProfileSecretMigrationPlan,
    request: &ProfileSecretMigrationRequest,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"portmate-profile-secret-migration-v1\0");
    digest.update([match request.target_storage {
        SecretStorage::Native => 0,
        SecretStorage::Portable => 1,
    }]);
    digest.update([u8::from(request.cleanup_source)]);
    for profile_id in &plan.selected_profile_ids {
        update_migration_token_field(&mut digest, profile_id.as_bytes());
    }
    digest.update([0xff]);
    for profile_id in &plan.affected_profile_ids {
        update_migration_token_field(&mut digest, profile_id.as_bytes());
    }
    digest.update([0xfe]);
    for (secret_ref, count) in &plan.source_ref_counts {
        update_migration_token_field(&mut digest, secret_ref.as_bytes());
        digest.update((*count as u64).to_le_bytes());
    }
    let mut in_flight_source_refs = plan.in_flight_source_refs.iter().collect::<Vec<_>>();
    in_flight_source_refs.sort_unstable();
    digest.update([0xfd]);
    for secret_ref in in_flight_source_refs {
        update_migration_token_field(&mut digest, secret_ref.as_bytes());
    }
    for count in [
        plan.preview.retained_shared_secret_count,
        plan.preview.already_target_reference_count,
        plan.preview.excluded_reserved_reference_count,
    ] {
        digest.update((count as u64).to_le_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn journal_optional_secret_ref(
    secret_ref: Option<&str>,
    label: &str,
) -> Result<Option<String>, String> {
    match secret_ref {
        Some(secret_ref) => canonical_secret_ref(secret_ref)
            .map(Some)
            .ok_or_else(|| format!("凭据迁移恢复记录包含无效的 {label} secretRef")),
        None => Ok(None),
    }
}

pub(super) fn profile_secret_migration_projection(
    profile: &SessionProfile,
) -> Result<ProfileSecretMigrationJournalProjection, String> {
    let proxy = profile_proxy(profile)
        .ok_or_else(|| format!("Profile {} 不支持 Profile 凭据迁移", profile.id))?;
    let proxy_password_secret_ref =
        journal_optional_secret_ref(proxy.password_secret_ref.as_deref(), "proxy password")?;
    let mut identity_secret_refs = BTreeMap::new();
    let (password_secret_ref, passphrase_secret_ref, jumps) = if let Ok(ssh) =
        ssh_connection(profile)
    {
        for identity in &ssh.identity_refs {
            if identity.source != IdentitySource::ProfileVault {
                continue;
            }
            let identity_id = identity.id.trim();
            if identity_id.is_empty() || identity_id != identity.id {
                return Err(format!(
                    "Profile {} 包含无效的 Vault identity ID",
                    profile.id
                ));
            }
            let secret_ref =
                journal_optional_secret_ref(identity.secret_ref.as_deref(), "Vault identity")?
                    .ok_or_else(|| {
                        format!(
                            "Profile {} 的 Vault identity {} 缺少 secretRef",
                            profile.id, identity.id
                        )
                    })?;
            if identity_secret_refs
                .insert(identity.id.clone(), secret_ref)
                .is_some()
            {
                return Err(format!(
                    "Profile {} 包含重复的 Vault identity ID: {}",
                    profile.id, identity.id
                ));
            }
        }
        let jumps = ssh
            .jumps
            .iter()
            .enumerate()
            .map(|(index, jump)| {
                Ok(ProfileSecretMigrationJournalJumpProjection {
                    password_secret_ref: journal_optional_secret_ref(
                        jump.password_secret_ref.as_deref(),
                        &format!("Jump Host #{} password", index + 1),
                    )?,
                    passphrase_secret_ref: journal_optional_secret_ref(
                        jump.passphrase_secret_ref.as_deref(),
                        &format!("Jump Host #{} passphrase", index + 1),
                    )?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        (
            journal_optional_secret_ref(ssh.password_secret_ref.as_deref(), "SSH password")?,
            journal_optional_secret_ref(ssh.passphrase_secret_ref.as_deref(), "SSH passphrase")?,
            jumps,
        )
    } else {
        (None, None, Vec::new())
    };
    Ok(ProfileSecretMigrationJournalProjection {
        proxy_password_secret_ref,
        password_secret_ref,
        passphrase_secret_ref,
        identity_secret_refs,
        jumps,
    })
}

pub(super) fn profile_secret_projection_ref_counts(
    projection: &ProfileSecretMigrationJournalProjection,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    let mut secret_refs = Vec::new();
    secret_refs.extend(projection.proxy_password_secret_ref.iter());
    secret_refs.extend(projection.password_secret_ref.iter());
    secret_refs.extend(projection.passphrase_secret_ref.iter());
    secret_refs.extend(projection.identity_secret_refs.values());
    for jump in &projection.jumps {
        secret_refs.extend(jump.password_secret_ref.iter());
        secret_refs.extend(jump.passphrase_secret_ref.iter());
    }
    for secret_ref in secret_refs {
        let secret_ref = secret_ref.clone();
        *counts.entry(secret_ref).or_default() += 1;
    }
    counts
}

fn replace_journal_projection_ref(
    secret_ref: &mut Option<String>,
    replacements: &HashMap<&str, &str>,
) -> usize {
    let Some(current) = secret_ref.as_deref() else {
        return 0;
    };
    let Some(replacement) = replacements.get(current) else {
        return 0;
    };
    *secret_ref = Some((*replacement).to_string());
    1
}

pub(super) fn replace_journal_projection_refs(
    projection: &mut ProfileSecretMigrationJournalProjection,
    replacements: &HashMap<&str, &str>,
) -> usize {
    let mut replaced =
        replace_journal_projection_ref(&mut projection.proxy_password_secret_ref, replacements)
            + replace_journal_projection_ref(&mut projection.password_secret_ref, replacements)
            + replace_journal_projection_ref(&mut projection.passphrase_secret_ref, replacements);
    for secret_ref in projection.identity_secret_refs.values_mut() {
        if let Some(replacement) = replacements.get(secret_ref.as_str()) {
            *secret_ref = (*replacement).to_string();
            replaced += 1;
        }
    }
    for jump in &mut projection.jumps {
        replaced += replace_journal_projection_ref(&mut jump.password_secret_ref, replacements);
        replaced += replace_journal_projection_ref(&mut jump.passphrase_secret_ref, replacements);
    }
    replaced
}

pub(super) fn build_profile_secret_migration_journal(
    store: &SessionStore,
    next_store: &SessionStore,
    plan: &ProfileSecretMigrationPlan,
    request: &ProfileSecretMigrationRequest,
    prepared: &[PreparedProfileSecretMigration],
) -> Result<ProfileSecretMigrationJournalPayload, String> {
    let mut profiles = Vec::with_capacity(plan.affected_profile_ids.len());
    for profile_id in &plan.affected_profile_ids {
        let before = store
            .profiles
            .iter()
            .find(|profile| profile.id == *profile_id)
            .ok_or_else(|| format!("迁移恢复记录缺少原 Profile: {profile_id}"))?;
        let after = next_store
            .profiles
            .iter()
            .find(|profile| profile.id == *profile_id)
            .ok_or_else(|| format!("迁移恢复记录缺少目标 Profile: {profile_id}"))?;
        profiles.push(ProfileSecretMigrationJournalProfile {
            profile_id: profile_id.clone(),
            before: profile_secret_migration_projection(before)?,
            after: profile_secret_migration_projection(after)?,
        });
    }
    let target_by_source = prepared
        .iter()
        .map(|item| (item.source_ref.as_str(), item.target_ref.as_str()))
        .collect::<HashMap<_, _>>();
    let items = plan
        .source_ref_counts
        .iter()
        .map(|(source_ref, reference_count)| {
            let target_ref = target_by_source
                .get(source_ref.as_str())
                .ok_or_else(|| format!("迁移恢复记录缺少目标引用: {source_ref}"))?;
            Ok(ProfileSecretMigrationJournalItem {
                source_ref: source_ref.clone(),
                target_ref: (*target_ref).to_string(),
                reference_count: *reference_count,
                in_flight_at_start: plan.in_flight_source_refs.contains(source_ref),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(ProfileSecretMigrationJournalPayload {
        version: PROFILE_SECRET_MIGRATION_JOURNAL_VERSION,
        migration_id: Uuid::new_v4().to_string(),
        target_storage: request.target_storage,
        cleanup_source: request.cleanup_source,
        plan_token: profile_secret_migration_plan_token(plan, request),
        selected_profile_ids: plan.selected_profile_ids.clone(),
        profiles,
        items,
    })
}

fn replace_optional_profile_secret_ref(
    secret_ref: &mut Option<String>,
    replacements: &HashMap<String, String>,
) -> usize {
    let Some(current) = secret_ref.as_deref().and_then(canonical_secret_ref) else {
        return 0;
    };
    let Some(replacement) = replacements.get(&current) else {
        return 0;
    };
    *secret_ref = Some(replacement.clone());
    1
}

pub(super) fn replace_profile_secret_refs(
    profile: &mut SessionProfile,
    replacements: &HashMap<String, String>,
) -> usize {
    let mut replaced = profile_proxy_mut(profile)
        .map(|proxy| {
            replace_optional_profile_secret_ref(&mut proxy.password_secret_ref, replacements)
        })
        .unwrap_or_default();
    let Ok(ssh) = ssh_connection_mut(profile) else {
        return replaced;
    };
    replaced += replace_optional_profile_secret_ref(&mut ssh.password_secret_ref, replacements)
        + replace_optional_profile_secret_ref(&mut ssh.passphrase_secret_ref, replacements);
    for identity in &mut ssh.identity_refs {
        if identity.source == IdentitySource::ProfileVault {
            replaced += replace_optional_profile_secret_ref(&mut identity.secret_ref, replacements);
        }
    }
    for jump in &mut ssh.jumps {
        replaced +=
            replace_optional_profile_secret_ref(&mut jump.password_secret_ref, replacements);
        replaced +=
            replace_optional_profile_secret_ref(&mut jump.passphrase_secret_ref, replacements);
    }
    replaced
}

pub(super) fn migration_error_with_cleanup(
    message: impl Into<String>,
    cleanup: &SecretBatchDeleteOutcome,
) -> String {
    let failures = cleanup
        .results
        .iter()
        .filter_map(|(secret_ref, result)| {
            result
                .as_ref()
                .err()
                .map(|error| format!("{secret_ref}: {error}"))
        })
        .collect::<Vec<_>>();
    if failures.is_empty() {
        message.into()
    } else {
        format!(
            "{}；新目标 secret 回收失败，已保留孤立副本: {}",
            message.into(),
            failures.join(" | ")
        )
    }
}

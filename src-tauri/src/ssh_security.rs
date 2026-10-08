use super::*;

pub(super) fn describe_host_key_rejection(evaluation: &HostKeyEvaluation) -> String {
    match evaluation {
        HostKeyEvaluation::Trusted { .. } => "SSH host key 已受信任".to_string(),
        HostKeyEvaluation::Unknown {
            alias,
            port,
            algorithm,
            fingerprint_sha256,
            ..
        } => format!(
            "SSH host key 未受信任: alias={alias}:{port}, algorithm={algorithm}, fingerprint={fingerprint_sha256}"
        ),
        HostKeyEvaluation::Mismatch {
            alias,
            port,
            algorithm,
            expected,
            observed_fingerprint_sha256,
            ..
        } => {
            let expected = expected
                .iter()
                .map(|key| key.fingerprint_sha256.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "SSH host key 已变化，已阻断: alias={alias}:{port}, algorithm={algorithm}, observed={observed_fingerprint_sha256}, expected=[{expected}]"
            )
        }
    }
}

pub(super) fn persist_observed_host_key(
    store: &Arc<Mutex<SessionStore>>,
    store_path: &Path,
    guard: HostKeyPersistenceGuard<'_>,
    observed_key: &Arc<Mutex<Option<HostKeyObservation>>>,
    one_time_host_keys: &[TrustedHostKey],
) -> Result<(), String> {
    let profile_id = guard.profile_id;
    let observation = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| "SSH 未收到服务器 host key".to_string())?;
    let mut store = store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profile(profile_id)
        .ok_or_else(|| format!("unknown session: {profile_id}"))?;
    if let Some(expected_profile) = guard.expected_profile {
        if !ssh_establishment_profile_matches_with_store_mirrors(expected_profile, &profile, &store)
        {
            return Err(format!(
                "SSH profile changed while establishing session: {profile_id}"
            ));
        }
    }
    let policy = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.host_key_policy,
        _ => return Err(format!("profile is not SSH-backed: {profile_id}")),
    };
    commit_tracked_store_mutation(&mut store, store_path, |next_store| {
        let message = if let Some(message) = reconnect_host_key_change_message(
            next_store,
            guard,
            &policy,
            &observation,
            one_time_host_keys,
            "SSH",
        )? {
            message
        } else if one_time_trusts_observation(one_time_host_keys, profile_id, &policy, &observation)
        {
            let fingerprint = observation
                .fingerprint_sha256()
                .map_err(|error| error.to_string())?;
            format!(
                "PortMate: SSH host key trusted for this connection only ({}, {})",
                observation.algorithm, fingerprint
            )
        } else if profile_trusts_observation(next_store, profile_id, &observation) {
            let fingerprint = observation
                .fingerprint_sha256()
                .map_err(|error| error.to_string())?;
            touch_observed_host_key(next_store, profile_id, &policy, &observation, Utc::now())?;
            format!(
                "PortMate: SSH host key verified by profile trust ({}, {})",
                observation.algorithm, fingerprint
            )
        } else {
            match next_store.evaluate_host_key(profile_id, &observation)? {
                HostKeyEvaluation::Trusted {
                    fingerprint_sha256, ..
                } => {
                    touch_observed_host_key(
                        next_store,
                        profile_id,
                        &policy,
                        &observation,
                        Utc::now(),
                    )?;
                    format!(
                        "PortMate: SSH host key verified ({}, {})",
                        observation.algorithm, fingerprint_sha256
                    )
                }
                HostKeyEvaluation::Unknown {
                    fingerprint_sha256, ..
                } => {
                    if policy.mode != HostKeyMode::TrustOnFirstUse {
                        return Err(format!(
                            "SSH host key 未受信任: {} {}",
                            observation.algorithm, fingerprint_sha256
                        ));
                    }
                    apply_persistent_host_key_decision_with_policy(
                        next_store,
                        profile_id,
                        &policy,
                        &observation,
                        HostKeyDecision::AppendToProfile,
                    )?;
                    format!(
                        "PortMate: SSH host key trusted for this profile ({}, {})",
                        observation.algorithm, fingerprint_sha256
                    )
                }
                mismatch @ HostKeyEvaluation::Mismatch { .. } => {
                    return Err(describe_host_key_rejection(&mismatch));
                }
            }
        };
        let event_ids = next_store
            .record_system_event_tracked(profile_id, message)
            .into_iter()
            .collect();
        Ok(((), event_ids))
    })
}

pub(super) fn persist_observed_host_key_with_policy(
    store: &Arc<Mutex<SessionStore>>,
    store_path: &Path,
    guard: HostKeyPersistenceGuard<'_>,
    policy: &portmate_core::HostKeyPolicy,
    observed_key: &Arc<Mutex<Option<HostKeyObservation>>>,
    one_time_host_keys: &[TrustedHostKey],
    label: &str,
) -> Result<(), String> {
    let profile_id = guard.profile_id;
    let observation = observed_key
        .lock()
        .map_err(|error| error.to_string())?
        .clone()
        .ok_or_else(|| format!("{label} 未收到服务器 host key"))?;
    let mut store = store.lock().map_err(|error| error.to_string())?;
    if let Some(expected_profile) = guard.expected_profile {
        let latest_profile = store
            .profile(profile_id)
            .ok_or_else(|| format!("unknown session: {profile_id}"))?;
        if !ssh_establishment_profile_matches_with_store_mirrors(
            expected_profile,
            &latest_profile,
            &store,
        ) {
            return Err(format!(
                "SSH profile changed while establishing session: {profile_id}"
            ));
        }
    }
    commit_tracked_store_mutation(&mut store, store_path, |next_store| {
        let message = if let Some(message) = reconnect_host_key_change_message(
            next_store,
            guard,
            policy,
            &observation,
            one_time_host_keys,
            label,
        )? {
            message
        } else if one_time_trusts_observation(one_time_host_keys, profile_id, policy, &observation)
        {
            let fingerprint = observation
                .fingerprint_sha256()
                .map_err(|error| error.to_string())?;
            format!(
                "PortMate: {label} host key trusted for this connection only ({}, {})",
                observation.algorithm, fingerprint
            )
        } else {
            match next_store
                .host_keys
                .evaluate(profile_id, policy, &observation)
            {
                Ok(HostKeyEvaluation::Trusted {
                    fingerprint_sha256, ..
                }) => {
                    touch_observed_host_key(
                        next_store,
                        profile_id,
                        policy,
                        &observation,
                        Utc::now(),
                    )?;
                    format!(
                        "PortMate: {label} host key verified ({}, {})",
                        observation.algorithm, fingerprint_sha256
                    )
                }
                Ok(HostKeyEvaluation::Unknown {
                    fingerprint_sha256, ..
                }) if policy.mode == HostKeyMode::TrustOnFirstUse => {
                    apply_persistent_host_key_decision_with_policy(
                        next_store,
                        profile_id,
                        policy,
                        &observation,
                        HostKeyDecision::AppendToProfile,
                    )?;
                    format!(
                        "PortMate: {label} host key trusted for this profile ({}, {})",
                        observation.algorithm, fingerprint_sha256
                    )
                }
                Ok(other) => return Err(describe_host_key_rejection(&other)),
                Err(error) => return Err(error.to_string()),
            }
        };
        let event_ids = next_store
            .record_system_event_tracked(profile_id, message)
            .into_iter()
            .collect();
        Ok(((), event_ids))
    })
}

fn reconnect_host_key_change_message(
    store: &SessionStore,
    guard: HostKeyPersistenceGuard<'_>,
    policy: &portmate_core::HostKeyPolicy,
    observation: &HostKeyObservation,
    one_time_host_keys: &[TrustedHostKey],
    label: &str,
) -> Result<Option<String>, String> {
    if guard.host_key_verification != SshHostKeyVerification::ReconnectIgnoreChanges {
        return Ok(None);
    }
    // Reevaluate the current trust set after authentication. The snapshot guard
    // above has already rejected policy edits made while the handshake was in flight.
    let mut host_keys = store.host_keys.clone();
    if let Some(profile) = store.profile(guard.profile_id) {
        if let ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) = &profile.connection {
            host_keys.keys.extend(ssh.trusted_host_keys.clone());
        }
    }
    host_keys.keys.extend_from_slice(one_time_host_keys);
    let evaluation = host_keys
        .evaluate(guard.profile_id, policy, observation)
        .map_err(|error| error.to_string())?;
    if !guard.host_key_verification.ignores_change(
        guard.profile_id,
        policy,
        &host_keys,
        observation,
        &evaluation,
    ) {
        return Ok(None);
    }
    let fingerprint = observation
        .fingerprint_sha256()
        .map_err(|error| error.to_string())?;
    // Never append, replace or touch trusted keys when a change was ignored.
    Ok(Some(format!(
        "PortMate: {label} 自动重连临时放行变化的主机密钥（{}，{}）；已保存的信任记录未变更",
        observation.algorithm, fingerprint
    )))
}

pub(super) fn touch_observed_host_key(
    store: &mut SessionStore,
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    observation: &HostKeyObservation,
    seen_at: DateTime<Utc>,
) -> Result<bool, String> {
    let fingerprint = observation
        .fingerprint_sha256()
        .map_err(|error| error.to_string())?;
    let alias = observation.target_alias(policy);
    let mut touched_key_ids = HashSet::new();
    for key in &mut store.host_keys.keys {
        if persistent_host_key_matches_observation(
            key,
            profile_id,
            policy,
            observation,
            alias,
            &fingerprint,
        ) {
            key.last_seen = seen_at;
            touched_key_ids.insert(key.id.clone());
        }
    }

    let mut touched = !touched_key_ids.is_empty();
    if let Some(profile) = store
        .profiles
        .iter_mut()
        .find(|profile| profile.id == profile_id)
    {
        if let ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) = &mut profile.connection {
            for key in &mut ssh.trusted_host_keys {
                if touched_key_ids.contains(&key.id)
                    || persistent_host_key_matches_observation(
                        key,
                        profile_id,
                        policy,
                        observation,
                        alias,
                        &fingerprint,
                    )
                {
                    key.last_seen = seen_at;
                    touched = true;
                }
            }
        }
    }
    let mirrors = store.host_keys.keys.iter().filter(|key| touched_key_ids.contains(&key.id)).cloned().collect::<Vec<_>>();
    mirror_persistent_host_keys(store, &mirrors)?;
    Ok(touched)
}

pub(super) fn apply_persistent_host_key_decision_with_policy(
    store: &mut SessionStore,
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    observation: &HostKeyObservation,
    decision: HostKeyDecision,
) -> Result<Option<TrustedHostKey>, String> {
    let previous_ids = store
        .host_keys
        .keys
        .iter()
        .map(|key| key.id.clone())
        .collect::<HashSet<_>>();
    let trusted = store
        .host_keys
        .apply_decision(profile_id, policy, observation, decision)
        .map_err(|error| error.to_string())?;
    let retained_ids = store
        .host_keys
        .keys
        .iter()
        .map(|key| key.id.as_str())
        .collect::<HashSet<_>>();
    let removed_ids = previous_ids
        .iter()
        .filter(|id| !retained_ids.contains(id.as_str()))
        .cloned()
        .collect::<HashSet<_>>();
    if !removed_ids.is_empty() {
        for profile in &mut store.profiles {
            if let ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) =
                &mut profile.connection
            {
                ssh.trusted_host_keys
                    .retain(|key| !removed_ids.contains(&key.id));
            }
        }
    }
    if let Some(key) = trusted.as_ref() {
        mirror_persistent_host_keys(store, std::slice::from_ref(key))?;
    }
    Ok(trusted)
}

pub(super) fn mirror_persistent_host_keys(
    store: &mut SessionStore,
    keys: &[TrustedHostKey],
) -> Result<(), String> {
    for key in keys {
        let profile_id = key
            .profile_id
            .as_deref()
            .ok_or_else(|| format!("host key {} has no source Profile", key.id))?;
        let profile = store
            .profiles
            .iter_mut()
            .find(|profile| profile.id == profile_id)
            .ok_or_else(|| format!("unknown session: {profile_id}"))?;
        let ssh = match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh,
            _ => return Err(format!("profile is not SSH-backed: {profile_id}")),
        };
        if let Some(existing) = ssh
            .trusted_host_keys
            .iter_mut()
            .find(|existing| existing.id == key.id)
        {
            existing.clone_from(key);
        } else {
            ssh.trusted_host_keys.push(key.clone());
        }
    }
    Ok(())
}

fn persistent_host_key_matches_observation(
    key: &TrustedHostKey,
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    observation: &HostKeyObservation,
    alias: &str,
    fingerprint: &str,
) -> bool {
    key.alias == alias
        && key.port == observation.port
        && (!policy.check_ip || key.host == observation.host)
        && key.algorithm == observation.algorithm
        && key.fingerprint_sha256 == fingerprint
        && match key.scope {
            HostKeyScope::Profile => key.profile_id.as_deref() == Some(profile_id),
            HostKeyScope::Project => matches!(
                policy.trust_scope,
                HostKeyScope::Project | HostKeyScope::Profile
            ),
            HostKeyScope::User => true,
        }
}

fn profile_trusts_observation(
    store: &SessionStore,
    profile_id: &str,
    observation: &HostKeyObservation,
) -> bool {
    let Some(profile) = store.profile(profile_id) else {
        return false;
    };
    let (policy, trusted_host_keys) = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            (ssh.host_key_policy, ssh.trusted_host_keys)
        }
        _ => return false,
    };
    let Ok(fingerprint) = observation.fingerprint_sha256() else {
        return false;
    };
    let alias = observation.target_alias(&policy);
    trusted_host_keys.iter().any(|key| {
        persistent_host_key_matches_observation(
            key,
            profile_id,
            &policy,
            observation,
            alias,
            &fingerprint,
        )
    })
}

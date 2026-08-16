use super::*;

pub(super) fn temporary_trusted_host_key_for_policy(
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    observation: &HostKeyObservation,
) -> Result<portmate_core::TrustedHostKey, String> {
    Ok(portmate_core::TrustedHostKey {
        id: Uuid::new_v4().to_string(),
        profile_id: Some(profile_id.to_string()),
        alias: observation.target_alias(policy).to_string(),
        host: observation.host.clone(),
        port: observation.port,
        algorithm: observation.algorithm.clone(),
        fingerprint_sha256: observation
            .fingerprint_sha256()
            .map_err(|error| error.to_string())?,
        public_key_base64: observation.public_key_base64.clone(),
        scope: HostKeyScope::Profile,
        label: Some("trust once".to_string()),
        first_seen: Utc::now(),
        last_seen: Utc::now(),
    })
}

pub(super) fn remember_one_time_host_key(
    state: &AppState,
    profile_id: &str,
    key: portmate_core::TrustedHostKey,
) -> Result<(), String> {
    remember_one_time_host_key_in(&state.one_time_host_keys, profile_id, key)
}

pub(super) fn take_one_time_host_keys(
    state: &AppState,
    profile_id: &str,
) -> Result<Vec<portmate_core::TrustedHostKey>, String> {
    take_one_time_host_keys_from(&state.one_time_host_keys, profile_id)
}

pub(super) fn one_time_host_keys_snapshot(
    state: &AppState,
    profile_id: &str,
) -> Result<Vec<portmate_core::TrustedHostKey>, String> {
    one_time_host_keys_snapshot_from(&state.one_time_host_keys, profile_id)
}

pub(super) fn remember_one_time_host_key_in(
    one_time: &Arc<Mutex<HashMap<String, Vec<portmate_core::TrustedHostKey>>>>,
    profile_id: &str,
    key: portmate_core::TrustedHostKey,
) -> Result<(), String> {
    let mut one_time = one_time.lock().map_err(|error| error.to_string())?;
    one_time
        .entry(profile_id.to_string())
        .or_default()
        .push(key);
    Ok(())
}

pub(super) fn take_one_time_host_keys_from(
    one_time: &Arc<Mutex<HashMap<String, Vec<portmate_core::TrustedHostKey>>>>,
    profile_id: &str,
) -> Result<Vec<portmate_core::TrustedHostKey>, String> {
    let mut one_time = one_time.lock().map_err(|error| error.to_string())?;
    Ok(one_time.remove(profile_id).unwrap_or_default())
}

pub(super) fn one_time_host_keys_snapshot_from(
    one_time: &Arc<Mutex<HashMap<String, Vec<portmate_core::TrustedHostKey>>>>,
    profile_id: &str,
) -> Result<Vec<portmate_core::TrustedHostKey>, String> {
    let one_time = one_time.lock().map_err(|error| error.to_string())?;
    Ok(one_time.get(profile_id).cloned().unwrap_or_default())
}

pub(super) fn one_time_trusts_observation(
    one_time_host_keys: &[TrustedHostKey],
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    observation: &HostKeyObservation,
) -> bool {
    let Ok(fingerprint) = observation.fingerprint_sha256() else {
        return false;
    };
    let alias = observation.target_alias(policy);
    one_time_host_keys.iter().any(|key| {
        key.profile_id.as_deref() == Some(profile_id)
            && key.alias == alias
            && key.host == observation.host
            && key.port == observation.port
            && key.algorithm == observation.algorithm
            && key.fingerprint_sha256 == fingerprint
    })
}

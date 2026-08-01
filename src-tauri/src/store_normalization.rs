use super::*;

mod mirror;
mod one_key;

pub(super) use mirror::normalize_loaded_mirror_keys;
#[cfg(test)]
pub(super) use mirror::normalize_loaded_record_ids;
pub(super) use one_key::normalize_loaded_one_keys;

pub(super) fn load_store_json(path: &Path) -> Result<SessionStore, String> {
    let raw = fs::read_to_string(path)
        .map_err(|error| format!("failed to read store {}: {error}", path.display()))?;
    let store = serde_json::from_str::<SessionStore>(&raw)
        .map_err(|error| format!("failed to parse store {}: {error}", path.display()))?;
    normalize_loaded_store_checked(store)
        .map_err(|error| format!("failed to load store {}: {error}", path.display()))
}

fn prune_orphaned_loaded_session_state(store: &mut SessionStore) {
    let session_ids = store
        .profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<HashSet<_>>();
    let session_exists = |session_id: &str| session_ids.contains(session_id);

    store
        .events
        .retain(|event| session_exists(&event.session_id));
    store
        .transfers
        .retain(|transfer| session_exists(&transfer.session_id));
    store
        .timeline
        .retain(|mark| session_exists(&mark.session_id));
    store
        .sysmon
        .retain(|snapshot| session_exists(&snapshot.session_id));
    store.host_keys.keys.retain(|key| {
        key.scope != HostKeyScope::Profile || key.profile_id.as_deref().is_some_and(&session_exists)
    });
    for key in &mut store.host_keys.keys {
        if key
            .profile_id
            .as_deref()
            .is_some_and(|profile_id| !session_exists(profile_id))
        {
            key.profile_id = None;
        }
    }
}

fn normalize_loaded_mcp_grants(store: &mut SessionStore) {
    let grants = std::mem::take(&mut store.grants);
    let revoked_at = Utc::now();
    let session_ids = store
        .profiles
        .iter()
        .map(|profile| profile.id.as_str())
        .collect::<HashSet<_>>();
    let mut normalized = Vec::<McpGrant>::new();
    let mut indices = HashMap::<String, usize>::new();
    let mut needs_review = false;

    for grant in grants {
        let Ok(mut grant) = normalize_mcp_grant(grant) else {
            needs_review = true;
            continue;
        };
        let was_session_scoped = !grant.allowed_sessions.is_empty();
        grant
            .allowed_sessions
            .retain(|session_id| session_ids.contains(session_id.as_str()));
        if was_session_scoped && grant.allowed_sessions.is_empty() && grant.revoked_at.is_none() {
            grant.revoked_at = Some(revoked_at);
        }
        if let Some(index) = indices.get(&grant.client_id).copied() {
            if normalized[index] != grant {
                needs_review = true;
            }
            continue;
        }
        if normalized.len() >= MAX_MCP_GRANTS {
            needs_review = true;
            continue;
        }
        indices.insert(grant.client_id.clone(), normalized.len());
        normalized.push(grant);
    }

    if needs_review {
        if normalized.len() >= MAX_MCP_GRANTS {
            normalized.truncate(MAX_MCP_GRANTS - 1);
        }
        let mut suffix = 1_usize;
        let client_id = loop {
            let candidate = format!("portmate:invalid-loaded-grant:{suffix}");
            if !normalized.iter().any(|grant| grant.client_id == candidate) {
                break candidate;
            }
            suffix += 1;
        };
        normalized.push(McpGrant {
            client_id,
            name: "Invalid loaded grant - review required".to_string(),
            scopes: Vec::new(),
            allowed_sessions: Vec::new(),
            confirm_writes: true,
            expires_at: None,
            revoked_at: Some(Utc::now()),
        });
    }

    store.grants = normalized;
}

fn normalize_interrupted_transfers(store: &mut SessionStore, now: DateTime<Utc>) {
    for task in &mut store.transfers {
        if !transfer_task_is_active(&task.status) {
            continue;
        }
        task.status = TransferStatus::Failed;
        task.message = Some("interrupted by previous PortMate shutdown".to_string());
        task.finished_at = Some(now);
        task.average_bytes_per_second = transfer_average_bps(task);
    }
}

#[derive(Default)]
struct LoadedSessionIdRemap {
    exact: HashMap<String, String>,
    normalized: HashMap<String, Option<String>>,
}

impl LoadedSessionIdRemap {
    fn insert(&mut self, original_id: &str, normalized_id: &str) {
        self.exact
            .entry(original_id.to_string())
            .or_insert_with(|| normalized_id.to_string());

        let original_id = normalized_session_profile_id(original_id);
        match self.normalized.entry(original_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Some(normalized_id.to_string()));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().as_deref() != Some(normalized_id) {
                    entry.insert(None);
                }
            }
        }
    }

    fn resolve(&self, session_id: &str) -> String {
        if let Some(session_id) = self.exact.get(session_id) {
            return session_id.clone();
        }
        let normalized = normalized_session_profile_id(session_id);
        self.normalized
            .get(&normalized)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_else(|| session_id.to_string())
    }
}

fn reserve_unique_loaded_session_id(
    session_id: &str,
    profile_position: usize,
    used_session_ids: &mut HashSet<String>,
) -> String {
    if used_session_ids.insert(session_id.to_string()) {
        return session_id.to_string();
    }

    let mut suffix = profile_position.saturating_add(1);
    loop {
        let suffix_text = format!(":loaded:{suffix}");
        let base_limit = MAX_SESSION_PROFILE_ID_CHARACTERS.saturating_sub(suffix_text.len());
        let base = session_id.chars().take(base_limit).collect::<String>();
        let candidate = format!("{base}{suffix_text}");
        if used_session_ids.insert(candidate.clone()) {
            return candidate;
        }
        suffix = suffix.saturating_add(1);
    }
}

fn assign_loaded_profile_id(
    profile: &mut SessionProfile,
    normalized_id: &str,
    assigned_id: String,
) {
    profile.id = assigned_id.clone();
    let ssh = match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh,
        _ => return,
    };
    for key in &mut ssh.trusted_host_keys {
        if key.scope != HostKeyScope::Profile {
            continue;
        }
        let owner_id = key
            .profile_id
            .as_deref()
            .map(normalized_session_profile_id)
            .unwrap_or_default();
        if owner_id.is_empty() || owner_id == normalized_id {
            key.profile_id = Some(assigned_id.clone());
        }
    }
}

pub(super) fn normalize_loaded_store(store: SessionStore) -> SessionStore {
    normalize_loaded_store_at(store, Utc::now())
}

pub(super) fn normalize_loaded_store_checked(store: SessionStore) -> Result<SessionStore, String> {
    store.validate_profile_count()?;
    Ok(normalize_loaded_store(store))
}

pub(super) fn normalize_loaded_store_at(
    mut store: SessionStore,
    loaded_at: DateTime<Utc>,
) -> SessionStore {
    let profiles = std::mem::take(&mut store.profiles);
    let mut normalized_profiles = Vec::with_capacity(profiles.len());
    let mut session_id_remap = LoadedSessionIdRemap::default();
    let mut used_session_ids = HashSet::with_capacity(profiles.len());
    for (profile_index, mut profile) in profiles.into_iter().enumerate() {
        let original_id = profile.id.clone();
        let mut normalized_id = normalized_session_profile_id(&profile.id);
        if normalized_id.is_empty() {
            normalized_id = format!("session-loaded-{}", profile_index + 1);
        }
        let assigned_id =
            reserve_unique_loaded_session_id(&normalized_id, profile_index, &mut used_session_ids);
        assign_loaded_profile_id(&mut profile, &normalized_id, assigned_id);
        let profile = normalize_session_profile(profile);
        session_id_remap.insert(&original_id, &profile.id);
        normalized_profiles.push(profile);
    }

    let saved_runtimes = std::mem::take(&mut store.runtimes)
        .into_iter()
        .map(|mut runtime| {
            let original_id = runtime.session_id.clone();
            runtime.session_id = remap_loaded_session_id(&original_id, &session_id_remap);
            if runtime.pane_id == format!("{original_id}:main") {
                runtime.pane_id = format!("{}:main", runtime.session_id);
            }
            (runtime.session_id.clone(), runtime)
        })
        .collect::<HashMap<_, _>>();

    for event in &mut store.events {
        let original_id = event.session_id.clone();
        event.session_id = remap_loaded_session_id(&original_id, &session_id_remap);
        if event.pane_id == format!("{original_id}:main") {
            event.pane_id = format!("{}:main", event.session_id);
        }
    }
    for transfer in &mut store.transfers {
        transfer.session_id = remap_loaded_session_id(&transfer.session_id, &session_id_remap);
    }
    for record in &mut store.audit {
        if let Some(session_id) = &mut record.session_id {
            *session_id = remap_loaded_session_id(session_id, &session_id_remap);
        }
    }
    for mark in &mut store.timeline {
        mark.session_id = remap_loaded_session_id(&mark.session_id, &session_id_remap);
    }
    for snapshot in &mut store.sysmon {
        snapshot.session_id = remap_loaded_session_id(&snapshot.session_id, &session_id_remap);
    }
    for key in &mut store.host_keys.keys {
        if let Some(profile_id) = &mut key.profile_id {
            *profile_id = remap_loaded_session_id(profile_id, &session_id_remap);
        }
        key.alias = key.alias.trim().to_string();
    }
    for grant in &mut store.grants {
        for session_id in &mut grant.allowed_sessions {
            *session_id = remap_loaded_session_id(session_id, &session_id_remap);
        }
    }
    for one_key in &mut store.one_keys {
        for session_id in &mut one_key.session_ids {
            *session_id = remap_loaded_session_id(session_id, &session_id_remap);
        }
        if let Some(identity) = &mut one_key.identity {
            identity.source_profile_id =
                remap_loaded_session_id(&identity.source_profile_id, &session_id_remap);
        }
    }

    for profile in normalized_profiles {
        let _ = store.upsert_profile(profile);
    }
    prune_orphaned_loaded_session_state(&mut store);
    normalize_loaded_mirror_keys(&mut store);
    normalize_loaded_mcp_grants(&mut store);
    normalize_loaded_one_keys(&mut store);
    for runtime in &mut store.runtimes {
        if let Some(saved) = saved_runtimes.get(&runtime.session_id) {
            runtime.pane_id = saved.pane_id.clone();
            runtime.title = saved.title.clone();
            runtime.cwd = saved.cwd.clone();
            runtime.last_activity = saved.last_activity;
            runtime.last_disconnect = saved.last_disconnect;
            runtime.last_disconnect_reason = saved
                .last_disconnect_reason
                .as_deref()
                .and_then(portmate_core::normalize_session_disconnect_reason);
            match saved.status {
                SessionStatus::Connected => {
                    runtime.last_disconnect = Some(loaded_at);
                    runtime.last_disconnect_reason =
                        Some("connection interrupted by previous PortMate shutdown".to_string());
                }
                SessionStatus::Connecting => {
                    runtime.last_disconnect = Some(loaded_at);
                    runtime.last_disconnect_reason = Some(
                        "connection attempt interrupted by previous PortMate shutdown".to_string(),
                    );
                }
                SessionStatus::Reconnecting => {
                    runtime.last_disconnect.get_or_insert(loaded_at);
                    runtime.last_disconnect_reason =
                        Some("reconnect interrupted by previous PortMate shutdown".to_string());
                }
                SessionStatus::Disconnected | SessionStatus::Blocked | SessionStatus::Error => {}
            }
        }
        runtime.status = SessionStatus::Disconnected;
        runtime.connected_since = None;
    }
    normalize_interrupted_transfers(&mut store, loaded_at);
    store.normalize_bounded_histories();
    store
}

fn remap_loaded_session_id(session_id: &str, remap: &LoadedSessionIdRemap) -> String {
    remap.resolve(session_id)
}

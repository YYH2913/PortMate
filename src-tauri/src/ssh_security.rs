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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[derive(Clone)]
struct AgentIdentityFilter {
    label: String,
    fingerprint_sha256: Option<String>,
    path: Option<String>,
}

#[derive(Clone, Default)]
struct PortMateAgentSigner {
    socket_path: Option<PathBuf>,
}

#[derive(Debug)]
enum PortMateAgentAuthError {
    Send(russh::SendError),
    Agent(String),
}

impl From<russh::SendError> for PortMateAgentAuthError {
    fn from(error: russh::SendError) -> Self {
        Self::Send(error)
    }
}

impl std::fmt::Display for PortMateAgentAuthError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Send(error) => write!(formatter, "{error}"),
            Self::Agent(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for PortMateAgentAuthError {}

impl russh::Signer for PortMateAgentSigner {
    type Error = PortMateAgentAuthError;

    fn auth_sign(
        &mut self,
        key: &AgentIdentity,
        hash_alg: Option<HashAlg>,
        to_sign: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let key = key.clone();
        let socket_path = self.socket_path.clone();
        async move {
            sign_with_ssh_agent_on_thread(key, hash_alg, to_sign, socket_path)
                .await
                .map_err(PortMateAgentAuthError::Agent)
        }
    }
}

pub(super) async fn authenticate_with_agent<H: client::Handler>(
    session: &mut client::Handle<H>,
    username: String,
    identities_only: bool,
    offer_mode: portmate_core::AgentOfferMode,
    identity_refs: Vec<IdentityRef>,
    agent_socket_path: Option<PathBuf>,
) -> Result<bool, String> {
    if offer_mode == portmate_core::AgentOfferMode::Disabled {
        return Ok(false);
    }

    let agent_refs = identity_refs
        .into_iter()
        .filter(|identity| identity.source == IdentitySource::Agent)
        .map(|identity| AgentIdentityFilter {
            label: identity.label,
            fingerprint_sha256: identity.fingerprint_sha256,
            path: identity.path,
        })
        .collect::<Vec<_>>();
    let allow_unfiltered_agent = !identities_only && agent_refs.is_empty();
    let identities = list_ssh_agent_identities_on_thread(agent_socket_path.clone()).await?;
    if identities.is_empty() {
        return Ok(false);
    }

    let rsa_hash = session
        .best_supported_rsa_hash()
        .await
        .map_err(|error| format!("SSH 查询 RSA 签名算法失败: {error}"))?
        .flatten();
    let mut tried = 0_usize;
    let max_agent_attempts = if allow_unfiltered_agent {
        6
    } else {
        usize::MAX
    };
    let mut signer = PortMateAgentSigner {
        socket_path: agent_socket_path,
    };

    for identity in identities {
        if !allow_unfiltered_agent && !agent_identity_matches(&identity, &agent_refs) {
            continue;
        }
        if tried >= max_agent_attempts {
            break;
        }
        tried += 1;
        let public_key = identity.public_key().into_owned();
        let result = session
            .authenticate_publickey_with(username.clone(), public_key, rsa_hash, &mut signer)
            .await
            .map_err(|error| format!("ssh-agent 认证失败: {error}"))?;
        if result.success() {
            return Ok(true);
        }
    }

    Ok(false)
}

pub(super) async fn list_ssh_agent_identities_on_thread(
    socket_path: Option<PathBuf>,
) -> Result<Vec<AgentIdentity>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("portmate-ssh-agent-list".to_string())
        .spawn(move || {
            let result = run_agent_runtime(async {
                let mut agent = connect_ssh_agent(socket_path.as_deref()).await?;
                agent
                    .request_identities()
                    .await
                    .map_err(|error| format!("读取 ssh-agent identities 失败: {error}"))
            });
            let _ = sender.send(result);
        })
        .map_err(|error| format!("启动 ssh-agent 查询线程失败: {error}"))?;
    receiver
        .await
        .map_err(|error| format!("ssh-agent 查询线程未返回: {error}"))?
}

async fn sign_with_ssh_agent_on_thread(
    identity: AgentIdentity,
    hash_alg: Option<HashAlg>,
    data: Vec<u8>,
    socket_path: Option<PathBuf>,
) -> Result<Vec<u8>, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("portmate-ssh-agent-sign".to_string())
        .spawn(move || {
            let result = run_agent_runtime(async {
                let mut agent = connect_ssh_agent(socket_path.as_deref()).await?;
                agent
                    .sign_request(&identity, hash_alg, data)
                    .await
                    .map_err(|error| format!("ssh-agent 签名失败: {error}"))
            });
            let _ = sender.send(result);
        })
        .map_err(|error| format!("启动 ssh-agent 签名线程失败: {error}"))?;
    receiver
        .await
        .map_err(|error| format!("ssh-agent 签名线程未返回: {error}"))?
}

fn run_agent_runtime<T>(
    future: impl std::future::Future<Output = Result<T, String>>,
) -> Result<T, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("创建 ssh-agent runtime 失败: {error}"))?
        .block_on(future)
}

fn agent_identity_matches(identity: &AgentIdentity, refs: &[AgentIdentityFilter]) -> bool {
    let comment = identity.comment();
    let public_key = identity.public_key();
    let fingerprint = compute_ssh_sha256_fingerprint(&public_key.public_key_base64()).ok();
    refs.iter().any(|identity_ref| {
        if let Some(expected) = identity_ref
            .fingerprint_sha256
            .as_deref()
            .map(str::trim)
            .filter(|expected| !expected.is_empty())
        {
            return fingerprint.as_deref() == Some(expected);
        }
        if let Some(path) = identity_ref
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            return path == comment;
        }
        !identity_ref.label.trim().is_empty() && identity_ref.label == comment
    })
}

async fn connect_ssh_agent(
    socket_path: Option<&Path>,
) -> Result<AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>, String> {
    #[cfg(unix)]
    {
        if let Some(path) = socket_path {
            return AgentClient::connect_uds(path)
                .await
                .map(|client| client.dynamic())
                .map_err(|error| format!("无法连接 SSH agent socket {}: {error}", path.display()));
        }
        AgentClient::connect_env()
            .await
            .map(|client| client.dynamic())
            .map_err(|error| format!("无法连接 SSH_AUTH_SOCK: {error}"))
    }

    #[cfg(windows)]
    {
        if let Some(path) = socket_path {
            return AgentClient::connect_named_pipe(path)
                .await
                .map(|client| client.dynamic())
                .map_err(|error| {
                    format!("无法连接 Windows OpenSSH agent {}: {error}", path.display())
                });
        }
        if let Ok(path) = std::env::var("SSH_AUTH_SOCK") {
            if !path.trim().is_empty() {
                return AgentClient::connect_named_pipe(path)
                    .await
                    .map(|client| client.dynamic())
                    .map_err(|error| format!("无法连接 Windows OpenSSH agent: {error}"));
            }
        }
        AgentClient::connect_pageant()
            .await
            .map(|client| client.dynamic())
            .map_err(|error| format!("无法连接 Pageant: {error}"))
    }

    #[cfg(not(any(unix, windows)))]
    {
        Err("当前平台不支持 ssh-agent".to_string())
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
        if !ssh_establishment_profile_matches(expected_profile, &profile) {
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
        let message =
            if one_time_trusts_observation(one_time_host_keys, profile_id, &policy, &observation) {
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

pub(super) fn temporary_trusted_host_key(
    store: &SessionStore,
    profile_id: &str,
    observation: &HostKeyObservation,
) -> Result<portmate_core::TrustedHostKey, String> {
    let profile = store
        .profile(profile_id)
        .ok_or_else(|| format!("unknown session: {profile_id}"))?;
    let policy = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.host_key_policy,
        _ => return Err(format!("profile is not SSH-backed: {profile_id}")),
    };
    temporary_trusted_host_key_for_policy(profile_id, &policy, observation)
}

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
        if !ssh_establishment_profile_matches(expected_profile, &latest_profile) {
            return Err(format!(
                "SSH profile changed while establishing session: {profile_id}"
            ));
        }
    }
    commit_tracked_store_mutation(&mut store, store_path, |next_store| {
        let message =
            if one_time_trusts_observation(one_time_host_keys, profile_id, policy, &observation) {
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
    Ok(touched)
}

pub(super) fn apply_persistent_host_key_decision(
    store: &mut SessionStore,
    profile_id: &str,
    observation: &HostKeyObservation,
    decision: HostKeyDecision,
) -> Result<Option<TrustedHostKey>, String> {
    let policy = match store
        .profile(profile_id)
        .ok_or_else(|| format!("unknown session: {profile_id}"))?
        .connection
    {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.host_key_policy,
        _ => return Err(format!("profile is not SSH-backed: {profile_id}")),
    };
    apply_persistent_host_key_decision_with_policy(
        store,
        profile_id,
        &policy,
        observation,
        decision,
    )
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

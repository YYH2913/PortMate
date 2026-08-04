use super::*;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::agent::AgentIdentity;
use russh::keys::HashAlg;

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
            .filter(|path| !path.trim().is_empty())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_identity_path_matches_exact_comment_bytes() {
        let key = russh::keys::PublicKey::from_openssh(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAhLsO2cjvuaNmiRlw4TJjIL+yzlPke9KgoXSfTiaqzQ",
        )
        .unwrap();
        let identity = AgentIdentity::PublicKey {
            key,
            comment: "accepted-comment ".to_string(),
        };
        let exact = AgentIdentityFilter {
            label: "agent key".to_string(),
            fingerprint_sha256: None,
            path: Some("accepted-comment ".to_string()),
        };
        let lossy = AgentIdentityFilter {
            path: Some("accepted-comment".to_string()),
            ..exact.clone()
        };

        assert!(agent_identity_matches(&identity, &[exact]));
        assert!(!agent_identity_matches(&identity, &[lossy]));
    }
}

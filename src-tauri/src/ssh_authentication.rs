use super::*;

// RUSTSEC-2023-0071 has no patched rsa release; local RSA authentication must
// use randomized PKCS#1 v1.5 signing so the private operation is blinded.
#[derive(Debug)]
enum BlindedRsaSignerError {
    Send(russh::SendError),
    KeyMismatch,
    Signing(String),
}

impl std::fmt::Display for BlindedRsaSignerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Send(error) => error.fmt(formatter),
            Self::KeyMismatch => formatter.write_str("SSH RSA signer received a different key"),
            Self::Signing(error) => formatter.write_str(error),
        }
    }
}

impl std::error::Error for BlindedRsaSignerError {}

impl From<russh::SendError> for BlindedRsaSignerError {
    fn from(error: russh::SendError) -> Self {
        Self::Send(error)
    }
}

struct BlindedRsaSigner {
    private_key: ssh_key::PrivateKey,
}

impl BlindedRsaSigner {
    fn new(private_key: ssh_key::PrivateKey) -> Result<Self, BlindedRsaSignerError> {
        if !matches!(
            private_key.key_data(),
            ssh_key::private::KeypairData::Rsa(_)
        ) {
            return Err(BlindedRsaSignerError::Signing(
                "SSH RSA signer requires an RSA private key".to_string(),
            ));
        }
        Ok(Self { private_key })
    }

    fn sign_with_rng<R: rsa::rand_core::TryCryptoRng + ?Sized>(
        &self,
        hash_alg: Option<ssh_key::HashAlg>,
        message: &[u8],
        rng: &mut R,
    ) -> Result<Vec<u8>, BlindedRsaSignerError> {
        let ssh_key::private::KeypairData::Rsa(keypair) = self.private_key.key_data() else {
            return Err(BlindedRsaSignerError::Signing(
                "SSH RSA signer lost its RSA private key".to_string(),
            ));
        };

        let raw_signature = match hash_alg {
            Some(ssh_key::HashAlg::Sha512) => {
                let signer = rsa::pkcs1v15::SigningKey::<rsa::sha2::Sha512>::try_from(keypair)
                    .map_err(|error| {
                        BlindedRsaSignerError::Signing(format!(
                            "SSH RSA-SHA512 signing key preparation failed: {error}"
                        ))
                    })?;
                let signature =
                    rsa::signature::RandomizedSigner::try_sign_with_rng(&signer, rng, message)
                        .map_err(|error| {
                            BlindedRsaSignerError::Signing(format!(
                                "SSH RSA-SHA512 blinded signing failed: {error}"
                            ))
                        })?;
                rsa::signature::SignatureEncoding::to_vec(&signature)
            }
            Some(ssh_key::HashAlg::Sha256) => {
                let signer = rsa::pkcs1v15::SigningKey::<rsa::sha2::Sha256>::try_from(keypair)
                    .map_err(|error| {
                        BlindedRsaSignerError::Signing(format!(
                            "SSH RSA-SHA256 signing key preparation failed: {error}"
                        ))
                    })?;
                let signature =
                    rsa::signature::RandomizedSigner::try_sign_with_rng(&signer, rng, message)
                        .map_err(|error| {
                            BlindedRsaSignerError::Signing(format!(
                                "SSH RSA-SHA256 blinded signing failed: {error}"
                            ))
                        })?;
                rsa::signature::SignatureEncoding::to_vec(&signature)
            }
            None => {
                let signer = rsa::pkcs1v15::SigningKey::<sha1_11::Sha1>::try_from(keypair)
                    .map_err(|error| {
                        BlindedRsaSignerError::Signing(format!(
                            "SSH legacy RSA signing key preparation failed: {error}"
                        ))
                    })?;
                let signature =
                    rsa::signature::RandomizedSigner::try_sign_with_rng(&signer, rng, message)
                        .map_err(|error| {
                            BlindedRsaSignerError::Signing(format!(
                                "SSH legacy RSA blinded signing failed: {error}"
                            ))
                        })?;
                rsa::signature::SignatureEncoding::to_vec(&signature)
            }
            Some(unsupported) => {
                return Err(BlindedRsaSignerError::Signing(format!(
                    "SSH RSA signer does not support hash algorithm {unsupported:?}"
                )));
            }
        };

        let signature =
            ssh_key::Signature::new(ssh_key::Algorithm::Rsa { hash: hash_alg }, raw_signature)
                .map_err(|error| {
                    BlindedRsaSignerError::Signing(format!(
                        "SSH RSA signature encoding preparation failed: {error}"
                    ))
                })?;
        let mut encoded = Vec::new();
        russh::keys::ssh_encoding::Encode::encode(&signature, &mut encoded).map_err(|error| {
            BlindedRsaSignerError::Signing(format!("SSH RSA signature encoding failed: {error}"))
        })?;
        Ok(encoded)
    }
}

impl russh::Signer for BlindedRsaSigner {
    type Error = BlindedRsaSignerError;

    fn auth_sign(
        &mut self,
        key: &russh::keys::agent::AgentIdentity,
        hash_alg: Option<ssh_key::HashAlg>,
        mut to_sign: Vec<u8>,
    ) -> impl std::future::Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        let result = if key.public_key().key_data() != self.private_key.public_key().key_data() {
            Err(BlindedRsaSignerError::KeyMismatch)
        } else {
            self.sign_with_rng(hash_alg, &to_sign, &mut getrandom::SysRng)
                .and_then(|signature| {
                    // Russh expects the original request buffer with an SSH
                    // string containing the signature appended to it.
                    russh::keys::ssh_encoding::Encode::encode(&signature, &mut to_sign).map_err(
                        |error| {
                            BlindedRsaSignerError::Signing(format!(
                                "SSH RSA authentication signature framing failed: {error}"
                            ))
                        },
                    )?;
                    Ok(to_sign)
                })
        };
        std::future::ready(result)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SshAuthenticationError {
    TimedOut {
        timeout_ms: u128,
        cleanup_warning: Option<String>,
    },
    Failed(String),
}

impl std::fmt::Display for SshAuthenticationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TimedOut {
                timeout_ms,
                cleanup_warning,
            } => {
                write!(formatter, "SSH 认证超时（{timeout_ms} ms）")?;
                if let Some(warning) = cleanup_warning {
                    write!(formatter, "; {warning}")?;
                }
                Ok(())
            }
            Self::Failed(error) => formatter.write_str(error),
        }
    }
}

pub(super) struct SshAuthenticationRequest<'a> {
    pub(super) ssh: SshConnection,
    pub(super) username: String,
    pub(super) password: Option<String>,
    pub(super) passphrase: Option<String>,
    pub(super) agent_socket_path: Option<PathBuf>,
    pub(super) timeout: Duration,
    pub(super) disconnect_description: &'a str,
}

pub(super) async fn authenticate_ssh_with_timeout<H: client::Handler>(
    session: &mut client::Handle<H>,
    request: SshAuthenticationRequest<'_>,
) -> Result<AuthMethod, SshAuthenticationError> {
    let SshAuthenticationRequest {
        ssh,
        username,
        password,
        passphrase,
        agent_socket_path,
        timeout,
        disconnect_description,
    } = request;
    match bounded_connection_step(
        authenticate_ssh_with_agent_socket(
            session,
            ssh,
            username,
            password,
            passphrase,
            agent_socket_path,
        ),
        timeout,
    )
    .await
    {
        Ok(method) => Ok(method),
        Err(BoundedConnectionStepError::Failed(error)) => {
            Err(SshAuthenticationError::Failed(error))
        }
        Err(BoundedConnectionStepError::TimedOut) => {
            let cleanup_warning =
                request_ssh_disconnect_with_timeout(session, disconnect_description).await;
            Err(SshAuthenticationError::TimedOut {
                timeout_ms: timeout.as_millis(),
                cleanup_warning,
            })
        }
    }
}

pub(super) async fn authenticate_ssh_with_agent_socket<H: client::Handler>(
    session: &mut client::Handle<H>,
    ssh: SshConnection,
    username: String,
    password: Option<String>,
    passphrase: Option<String>,
    agent_socket_path: Option<PathBuf>,
) -> Result<AuthMethod, String> {
    let auth_order = ordered_auth_methods(&ssh);
    let mut attempted = Vec::new();
    let mut key_errors = Vec::new();
    let saved_password = if password
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        read_optional_secret_ref(ssh.password_secret_ref.as_deref(), "SSH password")?
    } else {
        None
    };
    let saved_passphrase = if passphrase
        .as_deref()
        .filter(|value| !value.is_empty())
        .is_none()
    {
        read_optional_secret_ref(
            ssh.passphrase_secret_ref.as_deref(),
            "SSH private-key passphrase",
        )?
    } else {
        None
    };
    let effective_password = password
        .filter(|value| !value.is_empty())
        .or(saved_password);
    let effective_passphrase = passphrase
        .filter(|value| !value.is_empty())
        .or(saved_passphrase);
    let mut agent_attempted = false;

    for method in auth_order {
        match method {
            AuthMethod::PublicKey => {
                if ssh.agent_policy.enabled
                    && !agent_attempted
                    && !ssh.identity_policy.identities_only
                    && ssh.agent_policy.offer_mode
                        == portmate_core::AgentOfferMode::BeforeProfileKeys
                {
                    attempted.push("agent(before-profile-keys)");
                    agent_attempted = true;
                    match authenticate_with_agent(
                        session,
                        username.clone(),
                        ssh.identity_policy.identities_only,
                        ssh.agent_policy.offer_mode,
                        ssh.identity_refs.clone(),
                        agent_socket_path.clone(),
                    )
                    .await
                    {
                        Ok(true) => return Ok(AuthMethod::PublicKey),
                        Ok(false) => {}
                        Err(error) => key_errors.push(error),
                    }
                }

                let identities = ssh
                    .identity_refs
                    .iter()
                    .filter(|identity| {
                        matches!(
                            identity.source,
                            IdentitySource::SystemFile | IdentitySource::ProfileVault
                        )
                    })
                    .collect::<Vec<_>>();
                if !identities.is_empty() {
                    attempted.push("publickey");
                    let rsa_hash = session
                        .best_supported_rsa_hash()
                        .await
                        .map_err(|error| {
                            format!("SSH publickey 认证准备失败，无法查询 RSA 签名算法: {error}")
                        })?
                        .flatten();
                    for identity in identities {
                        let label = identity.label.clone();
                        let key = match load_identity_private_key(
                            identity,
                            effective_passphrase.as_deref(),
                        ) {
                            Ok(Some(key)) => key,
                            Ok(None) => continue,
                            Err(error) => {
                                key_errors.push(format!("{label}: {error}"));
                                continue;
                            }
                        };
                        let authentication = if key.algorithm().is_rsa() {
                            let public_key = key.public_key().clone();
                            let mut signer = match BlindedRsaSigner::new(key) {
                                Ok(signer) => signer,
                                Err(error) => {
                                    key_errors
                                        .push(format!("{label}: RSA signer 初始化失败: {error}"));
                                    continue;
                                }
                            };
                            session
                                .authenticate_publickey_with(
                                    username.clone(),
                                    public_key,
                                    rsa_hash,
                                    &mut signer,
                                )
                                .await
                                .map_err(|error| error.to_string())
                        } else {
                            session
                                .authenticate_publickey(
                                    username.clone(),
                                    PrivateKeyWithHashAlg::new(Arc::new(key), rsa_hash),
                                )
                                .await
                                .map_err(|error| error.to_string())
                        };
                        let result = match authentication {
                            Ok(result) => result,
                            Err(error) => {
                                key_errors.push(format!("{label}: 认证请求失败: {error}"));
                                break;
                            }
                        };
                        if result.success() {
                            return Ok(AuthMethod::PublicKey);
                        }
                        key_errors.push(format!("{label}: 被服务器拒绝"));
                    }
                }

                if ssh.agent_policy.enabled
                    && !agent_attempted
                    && (ssh.agent_policy.offer_mode
                        == portmate_core::AgentOfferMode::AfterProfileKeys
                        || ssh
                            .identity_refs
                            .iter()
                            .any(|identity| identity.source == IdentitySource::Agent))
                    && (!ssh.identity_policy.identities_only
                        || ssh
                            .identity_refs
                            .iter()
                            .any(|identity| identity.source == IdentitySource::Agent))
                {
                    attempted.push("agent(after-profile-keys)");
                    agent_attempted = true;
                    match authenticate_with_agent(
                        session,
                        username.clone(),
                        ssh.identity_policy.identities_only,
                        ssh.agent_policy.offer_mode,
                        ssh.identity_refs.clone(),
                        agent_socket_path.clone(),
                    )
                    .await
                    {
                        Ok(true) => return Ok(AuthMethod::PublicKey),
                        Ok(false) => {}
                        Err(error) => key_errors.push(error),
                    }
                }
            }
            AuthMethod::KeyboardInteractive => {
                let Some(password) = effective_password.clone() else {
                    continue;
                };
                attempted.push("keyboard-interactive");
                if authenticate_keyboard_interactive(session, username.clone(), password).await? {
                    return Ok(AuthMethod::KeyboardInteractive);
                }
            }
            AuthMethod::Password => {
                let Some(password) = effective_password.clone() else {
                    continue;
                };
                attempted.push("password");
                let result = session
                    .authenticate_password(username.clone(), password)
                    .await
                    .map_err(|error| format!("SSH password 认证失败: {error}"))?;
                if result.success() {
                    return Ok(AuthMethod::Password);
                }
            }
            AuthMethod::None => {
                attempted.push("none");
                let result = session
                    .authenticate_none(username.clone())
                    .await
                    .map_err(|error| format!("SSH none 认证失败: {error}"))?;
                if result.success() {
                    return Ok(AuthMethod::None);
                }
            }
            AuthMethod::GssapiWithMic => {
                attempted.push("gssapi-with-mic(unsupported)");
            }
        }
    }

    let mut message = if attempted.is_empty() {
        "SSH 认证失败：没有可尝试的认证方式。请配置 identityRefs 或在连接时输入密码。".to_string()
    } else {
        format!("SSH 认证失败，已尝试: {}", attempted.join(", "))
    };
    if !key_errors.is_empty() {
        message.push_str(&format!("；密钥详情: {}", key_errors.join(" | ")));
    }
    if ssh.agent_policy.enabled && ssh.identity_policy.identities_only {
        message.push_str("；当前按 IdentitiesOnly 处理，不会遍历系统 ssh-agent 的全部密钥");
    }
    Err(message)
}

pub(super) async fn authenticate_keyboard_interactive<H: client::Handler>(
    session: &mut client::Handle<H>,
    username: String,
    password: String,
) -> Result<bool, String> {
    let mut response = session
        .authenticate_keyboard_interactive_start(username, None::<String>)
        .await
        .map_err(|error| format!("SSH keyboard-interactive 启动失败: {error}"))?;

    for _ in 0..8 {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(true),
            KeyboardInteractiveAuthResponse::Failure { .. } => return Ok(false),
            KeyboardInteractiveAuthResponse::InfoRequest { prompts, .. } => {
                let responses = prompts
                    .iter()
                    .map(|prompt| {
                        if prompt.echo {
                            String::new()
                        } else {
                            password.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                response = session
                    .authenticate_keyboard_interactive_respond(responses)
                    .await
                    .map_err(|error| format!("SSH keyboard-interactive 响应失败: {error}"))?;
            }
        }
    }

    Err("SSH keyboard-interactive 认证轮次过多，已中止".to_string())
}

pub(super) fn ordered_auth_methods(ssh: &SshConnection) -> Vec<AuthMethod> {
    let mut ordered = Vec::new();
    if let Some(last) = ssh.identity_policy.last_successful.filter(|method| {
        ssh.identity_policy.record_success && ssh.identity_policy.auth_order.contains(method)
    }) {
        ordered.push(last);
    }
    for method in &ssh.identity_policy.auth_order {
        if !ordered.contains(method) {
            ordered.push(*method);
        }
    }
    if ordered.is_empty() {
        ordered.extend([
            AuthMethod::PublicKey,
            AuthMethod::KeyboardInteractive,
            AuthMethod::Password,
        ]);
    }
    ordered
}

pub(super) fn load_identity_private_key(
    identity: &IdentityRef,
    passphrase: Option<&str>,
) -> Result<Option<ssh_key::PrivateKey>, String> {
    match identity.source {
        IdentitySource::SystemFile => {
            let Some(path) = identity
                .path
                .as_deref()
                .filter(|path| !path.trim().is_empty())
            else {
                return Ok(None);
            };
            load_secret_key(expand_identity_path(path), passphrase)
                .map(Some)
                .map_err(|error| format!("system-file {}: {error}", path))
        }
        IdentitySource::ProfileVault => {
            let Some(secret_ref) = identity
                .secret_ref
                .as_deref()
                .map(str::trim)
                .filter(|secret_ref| !secret_ref.is_empty())
            else {
                return Err("profile-vault identity 缺少 secretRef".to_string());
            };
            let private_key = read_secret_from_store(secret_ref)?;
            decode_secret_key(&private_key, passphrase)
                .map(Some)
                .map_err(|error| format!("profile-vault {secret_ref}: {error}"))
        }
        IdentitySource::Agent | IdentitySource::PublicKeyOnly => Ok(None),
    }
}

#[cfg(test)]
mod blinded_rsa_signer_tests {
    use super::*;

    #[derive(Default)]
    struct CountingSystemRng {
        requests: usize,
        requested_bytes: usize,
    }

    impl CountingSystemRng {
        fn record(&mut self, bytes: usize) {
            self.requests += 1;
            self.requested_bytes += bytes;
        }
    }

    impl rsa::rand_core::TryRng for CountingSystemRng {
        type Error = getrandom::Error;

        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            self.record(std::mem::size_of::<u32>());
            rsa::rand_core::TryRng::try_next_u32(&mut getrandom::SysRng)
        }

        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            self.record(std::mem::size_of::<u64>());
            rsa::rand_core::TryRng::try_next_u64(&mut getrandom::SysRng)
        }

        fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), Self::Error> {
            self.record(destination.len());
            rsa::rand_core::TryRng::try_fill_bytes(&mut getrandom::SysRng, destination)
        }
    }

    impl rsa::rand_core::TryCryptoRng for CountingSystemRng {}

    #[test]
    fn every_rsa_signature_requests_blinding_randomness() {
        let mut key_rng = rsa::rand_core::UnwrapErr(getrandom::SysRng);
        let private_key = rsa::RsaPrivateKey::new(&mut key_rng, 2048).unwrap();
        let private_key = ssh_key::private::RsaKeypair::try_from(private_key).unwrap();
        let signer = BlindedRsaSigner::new(private_key.into()).unwrap();
        let message = b"PortMate RSA authentication blinding regression";
        let mut rng = CountingSystemRng::default();

        for hash_alg in [
            Some(ssh_key::HashAlg::Sha512),
            Some(ssh_key::HashAlg::Sha256),
            None,
        ] {
            let previous_requests = rng.requests;
            let previous_bytes = rng.requested_bytes;
            let encoded = signer.sign_with_rng(hash_alg, message, &mut rng).unwrap();
            assert!(rng.requests > previous_requests);
            assert!(rng.requested_bytes > previous_bytes);

            let mut reader = encoded.as_slice();
            let signature =
                <ssh_key::Signature as russh::keys::ssh_encoding::Decode>::decode(&mut reader)
                    .unwrap();
            assert!(reader.is_empty());
            russh::keys::signature::Verifier::verify(
                signer.private_key.public_key(),
                message,
                &signature,
            )
            .unwrap();
        }
    }
}

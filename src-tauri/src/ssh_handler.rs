use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SshHostKeyVerification {
    Standard,
    ReconnectIgnoreChanges,
}

impl SshHostKeyVerification {
    pub(super) fn for_establishment(ssh: &SshConnection, automatic_reconnect: bool) -> Self {
        if automatic_reconnect && ssh.reconnect && ssh.reconnect_ignore_host_key_changes {
            Self::ReconnectIgnoreChanges
        } else {
            Self::Standard
        }
    }

    pub(super) fn ignores_change(
        self,
        profile_id: &str,
        policy: &portmate_core::HostKeyPolicy,
        host_keys: &HostKeyStore,
        observation: &HostKeyObservation,
        evaluation: &HostKeyEvaluation,
    ) -> bool {
        self == Self::ReconnectIgnoreChanges
            && !observation.algorithm.ends_with("-cert-v01@openssh.com")
            && !matches!(evaluation, HostKeyEvaluation::Trusted { .. })
            && host_keys.has_trusted_endpoint(profile_id, policy, observation)
    }
}

#[derive(Debug)]
pub(super) struct PortMateSshHandler {
    pub(super) profile_id: String,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) alias: Option<String>,
    pub(super) policy: portmate_core::HostKeyPolicy,
    pub(super) host_keys: HostKeyStore,
    pub(super) one_time_host_key_ids: Vec<String>,
    pub(super) host_key_verification: SshHostKeyVerification,
    pub(super) observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    pub(super) host_key_error: Arc<Mutex<Option<String>>>,
    pub(super) remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
}

pub(super) struct SshHandlerParams {
    pub(super) profile_id: String,
    pub(super) host: String,
    pub(super) port: u16,
    pub(super) alias: Option<String>,
    pub(super) policy: portmate_core::HostKeyPolicy,
    pub(super) host_keys: HostKeyStore,
    pub(super) one_time_host_key_ids: Vec<String>,
    pub(super) host_key_verification: SshHostKeyVerification,
    pub(super) observed_key: Arc<Mutex<Option<HostKeyObservation>>>,
    pub(super) host_key_error: Arc<Mutex<Option<String>>>,
    pub(super) remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
}

pub(super) fn ssh_handler_for_endpoint(params: SshHandlerParams) -> PortMateSshHandler {
    PortMateSshHandler {
        profile_id: params.profile_id,
        host: params.host,
        port: params.port,
        alias: params.alias,
        policy: params.policy,
        host_keys: params.host_keys,
        one_time_host_key_ids: params.one_time_host_key_ids,
        host_key_verification: params.host_key_verification,
        observed_key: params.observed_key,
        host_key_error: params.host_key_error,
        remote_forwards: params.remote_forwards,
    }
}

pub(super) fn lock_ssh_handler_state<'a, T>(
    state: &'a Mutex<T>,
    label: &str,
) -> Result<MutexGuard<'a, T>, russh::Error> {
    state.lock().map_err(|_| {
        russh::Error::IO(std::io::Error::other(format!(
            "PortMate SSH {label} lock is poisoned"
        )))
    })
}

impl client::Handler for PortMateSshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // CA/principal/expiry verification is not implemented by the saved
        // host-key policy. Do not silently treat a certificate as a raw key.
        let russh::keys::PublicKeyOrCertificate::PublicKey {
            key: server_public_key,
            ..
        } = server_public_key
        else {
            *lock_ssh_handler_state(&self.host_key_error, "host key error")? = Some(
                "SSH host certificates are not supported by the configured host-key policy".into(),
            );
            return Ok(false);
        };
        let observation = HostKeyObservation {
            host: self.host.clone(),
            port: self.port,
            alias: self.alias.clone(),
            algorithm: server_public_key.algorithm().to_string(),
            public_key_base64: server_public_key.public_key_base64(),
        };
        *lock_ssh_handler_state(&self.observed_key, "host key observation")? =
            Some(observation.clone());

        let verification = verify_ssh_host_key_observation(
            &self.profile_id,
            &self.policy,
            &self.host_keys,
            &self.one_time_host_key_ids,
            &observation,
            self.host_key_verification,
        );
        *lock_ssh_handler_state(&self.host_key_error, "host key error")? =
            verification.as_ref().err().cloned();

        Ok(verification.is_ok())
    }

    fn server_channel_open_forwarded_tcpip(
        &mut self,
        channel: Channel<client::Msg>,
        connected_address: &str,
        connected_port: u32,
        originator_address: &str,
        originator_port: u32,
        reply: client::ChannelOpenHandle,
        _session: &mut client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let forwards = Arc::clone(&self.remote_forwards);
        let connected_address = connected_address.to_string();
        let originator_address = originator_address.to_string();
        async move {
            let Some((connected_port, originator_port)) =
                forwarded_tcpip_ports(connected_port, originator_port)
            else {
                return Ok(());
            };
            let target = {
                let forwards = lock_ssh_handler_state(&forwards, "remote forward targets")?;
                let key = remote_forward_key(&connected_address, connected_port);
                forwards
                    .get(&key)
                    .or_else(|| forwards.get(&remote_forward_port_key(connected_port)))
                    .cloned()
            };
            if let Some(target) = target {
                let Some(permit) = try_acquire_tunnel_connection(
                    &target.connection_slots,
                    target.metrics.as_ref(),
                ) else {
                    return Ok(());
                };
                reply.accept().await;
                tauri::async_runtime::spawn(async move {
                    let _permit = permit;
                    target.metrics.connection_opened();
                    let result = handle_remote_tunnel_client(
                        SshBackendChannel::from_russh(channel),
                        target.spec.clone(),
                        Some((originator_address, originator_port)),
                        Arc::clone(&target.metrics),
                    )
                    .await;
                    match result {
                        Ok(()) => target.metrics.clear_error(),
                        Err(error) => {
                            target.metrics.record_error(&error);
                            eprintln!("PortMate: remote SSH tunnel client failed: {error}");
                        }
                    }
                    target.metrics.connection_closed();
                });
            }
            Ok(())
        }
    }
}

pub(super) fn verify_ssh_host_key_observation(
    profile_id: &str,
    policy: &portmate_core::HostKeyPolicy,
    host_keys: &HostKeyStore,
    one_time_host_key_ids: &[String],
    observation: &HostKeyObservation,
    verification: SshHostKeyVerification,
) -> Result<(), String> {
    if observation.algorithm.ends_with("-cert-v01@openssh.com") {
        return Err(
            "SSH host certificates are not supported by the configured host-key policy".into(),
        );
    }
    let evaluation = host_keys
        .evaluate(profile_id, policy, observation)
        .map_err(|error| format!("host key fingerprint 计算失败: {error}"))?;
    if verification.ignores_change(profile_id, policy, host_keys, observation, &evaluation) {
        return Ok(());
    }
    match evaluation {
        HostKeyEvaluation::Trusted { matched_key_id, .. }
            if trusted_host_key_allowed(policy, &matched_key_id, one_time_host_key_ids) =>
        {
            Ok(())
        }
        HostKeyEvaluation::Trusted {
            fingerprint_sha256, ..
        } => Err(format!(
            "SSH host key requires confirmation for this connection: {fingerprint_sha256}"
        )),
        HostKeyEvaluation::Unknown { .. } if policy.mode == HostKeyMode::TrustOnFirstUse => Ok(()),
        other => Err(describe_host_key_rejection(&other)),
    }
}

pub(super) fn trusted_host_key_allowed(
    policy: &portmate_core::HostKeyPolicy,
    matched_key_id: &str,
    one_time_host_key_ids: &[String],
) -> bool {
    policy.mode != HostKeyMode::AskEveryTime
        || one_time_host_key_ids
            .iter()
            .any(|key_id| key_id == matched_key_id)
}

use super::*;

const SSH_HEALTH_PING_TIMEOUT: Duration = Duration::from_secs(3);
const SSH_HEALTH_CHANNEL_TIMEOUT: Duration = Duration::from_secs(5);
const SSH_HEALTH_MARKER: &str = "PORTMATE_SSH_HEALTH_OK";
const MAX_SSH_HEALTH_ERROR_CHARACTERS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SshHealthStatus {
    Healthy,
    Degraded,
    Unresponsive,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SshHealthReport {
    pub session_id: String,
    pub runtime_id: String,
    pub checked_at: DateTime<Utc>,
    pub status: SshHealthStatus,
    pub backend: SshBackendKind,
    pub authentication_method: AuthMethod,
    pub terminal_channel_open: bool,
    pub transport_round_trip_ms: Option<u64>,
    pub channel_round_trip_ms: Option<u64>,
    pub sftp_round_trip_ms: Option<u64>,
    pub transport_error: Option<String>,
    pub terminal_error: Option<String>,
    pub channel_error: Option<String>,
    pub sftp_error: Option<String>,
    pub sftp_probed: bool,
}

enum SftpHealthProbeError {
    Failed(String),
    TimedOut,
}

#[tauri::command]
pub(crate) async fn check_ssh_health(
    state: State<'_, AppState>,
    session_id: String,
    probe_sftp: Option<bool>,
    expected_profile: Option<SessionProfile>,
) -> Result<SshHealthReport, String> {
    let probe_sftp = probe_sftp.unwrap_or(false);
    match expected_profile.as_ref() {
        Some(expected_profile) => {
            check_ssh_health_for_profile_inner(
                state.inner(),
                &session_id,
                probe_sftp,
                Some(expected_profile),
            )
            .await
        }
        None => check_ssh_health_inner(state.inner(), &session_id, probe_sftp).await,
    }
}

pub(super) async fn check_ssh_health_inner(
    state: &AppState,
    session_id: &str,
    probe_sftp: bool,
) -> Result<SshHealthReport, String> {
    check_ssh_health_for_profile_inner(state, session_id, probe_sftp, None).await
}

pub(super) async fn check_ssh_health_for_profile_inner(
    state: &AppState,
    session_id: &str,
    probe_sftp: bool,
    expected_profile: Option<&SessionProfile>,
) -> Result<SshHealthReport, String> {
    let ((runtime_id, backend, authentication_method, terminal_channel_open), auxiliary) =
        ssh_auxiliary_lease_with_runtime(state, session_id, |runtime| {
            if let Some(expected_profile) = expected_profile {
                validate_ssh_health_profile_snapshot(
                    session_id,
                    &runtime.profile_snapshot,
                    expected_profile,
                )?;
            }
            Ok((
                runtime.runtime_id.clone(),
                runtime.backend,
                runtime.auth_method,
                runtime.terminal_channel_open.load(Ordering::SeqCst),
            ))
        })?;
    let handle = auxiliary.handle();
    let mut report = SshHealthReport {
        session_id: session_id.to_string(),
        runtime_id: runtime_id.clone(),
        checked_at: Utc::now(),
        status: if terminal_channel_open {
            SshHealthStatus::Healthy
        } else {
            SshHealthStatus::Degraded
        },
        backend,
        authentication_method,
        terminal_channel_open,
        transport_round_trip_ms: None,
        channel_round_trip_ms: None,
        sftp_round_trip_ms: None,
        transport_error: None,
        terminal_error: (!terminal_channel_open)
            .then(|| "SSH 主终端 channel 已关闭或正在重连，交互输入不可用".to_string()),
        channel_error: None,
        sftp_error: None,
        sftp_probed: probe_sftp,
    };

    let ping_started = Instant::now();
    let ping = match tokio::time::timeout(SSH_HEALTH_PING_TIMEOUT, handle.lock()).await {
        Ok(handle) => {
            let remaining = SSH_HEALTH_PING_TIMEOUT
                .checked_sub(ping_started.elapsed())
                .filter(|remaining| !remaining.is_zero());
            match remaining {
                Some(remaining) => {
                    bounded_connection_step(
                        async {
                            handle
                                .send_ping(remaining)
                                .await
                                .map_err(|error| format!("SSH keepalive 往返失败: {error}"))
                        },
                        remaining,
                    )
                    .await
                }
                None => Err(BoundedConnectionStepError::TimedOut),
            }
        }
        Err(_) => Err(BoundedConnectionStepError::TimedOut),
    };
    match ping {
        Ok(()) => report.transport_round_trip_ms = Some(elapsed_millis(ping_started)),
        Err(BoundedConnectionStepError::Failed(error)) => {
            report.status = SshHealthStatus::Unresponsive;
            report.transport_error = Some(bounded_ssh_health_error(&error));
            return finish_health_report(state, runtime_id, report);
        }
        Err(BoundedConnectionStepError::TimedOut) => {
            report.status = SshHealthStatus::Unresponsive;
            report.transport_error = Some(format!(
                "SSH keepalive 往返超过 {} ms",
                SSH_HEALTH_PING_TIMEOUT.as_millis()
            ));
            return finish_health_report(state, runtime_id, report);
        }
    }

    let channel_started = Instant::now();
    match exec_ssh_command_capture(
        Arc::clone(&handle),
        "printf 'PORTMATE_SSH_HEALTH_OK\\n'",
        SSH_HEALTH_CHANNEL_TIMEOUT,
    )
    .await
    {
        Ok(output) if output.lines().any(|line| line.trim() == SSH_HEALTH_MARKER) => {
            report.channel_round_trip_ms = Some(elapsed_millis(channel_started));
        }
        Ok(_) => {
            report.status = SshHealthStatus::Degraded;
            report.channel_error = Some("SSH exec channel 未返回健康标记".to_string());
        }
        Err(error) => {
            report.status = SshHealthStatus::Degraded;
            report.channel_error = Some(bounded_ssh_health_error(&error));
        }
    }

    if probe_sftp {
        let sftp_started = Instant::now();
        match probe_sftp_health(&auxiliary, Arc::clone(&handle), SSH_HEALTH_CHANNEL_TIMEOUT).await {
            Ok(()) => {
                report.sftp_round_trip_ms = Some(elapsed_millis(sftp_started));
            }
            Err(SftpHealthProbeError::Failed(error)) => {
                report.status = SshHealthStatus::Degraded;
                report.sftp_error = Some(bounded_ssh_health_error(&format!(
                    "SFTP health probe failed: {error}"
                )));
            }
            Err(SftpHealthProbeError::TimedOut) => {
                report.status = SshHealthStatus::Degraded;
                report.sftp_error = Some(format!(
                    "SFTP health probe exceeded {} ms",
                    SSH_HEALTH_CHANNEL_TIMEOUT.as_millis()
                ));
            }
        }
    }

    finish_health_report(state, runtime_id, report)
}

pub(super) fn ssh_health_profile_snapshot(profile: &SessionProfile) -> Result<String, String> {
    let mut profile = normalize_session_profile(profile.clone());
    let ssh = match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh,
        _ => return Err("SSH 健康检查仅支持 SSH/Tmux Profile".to_string()),
    };
    ssh.trusted_host_keys.clear();
    ssh.identity_policy.last_successful = None;
    serde_json::to_string(&(&profile.connection, &profile.terminal))
        .map_err(|error| format!("无法创建 SSH 健康检查配置快照: {error}"))
}

pub(super) fn validate_ssh_health_profile_snapshot(
    session_id: &str,
    runtime_snapshot: &str,
    expected_profile: &SessionProfile,
) -> Result<(), String> {
    if expected_profile.id != session_id {
        return Err("SSH 健康检查 Profile 与会话不匹配".to_string());
    }
    let expected_snapshot = ssh_health_profile_snapshot(expected_profile)?;
    if expected_snapshot != runtime_snapshot {
        return Err("SSH 连接配置已更改，请保存并重新连接后再检查健康状态".to_string());
    }
    Ok(())
}

async fn probe_sftp_health(
    auxiliary: &SshAuxiliaryLease,
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    timeout: Duration,
) -> Result<(), SftpHealthProbeError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let libssh_backend = tokio::time::timeout_at(deadline, async {
        let handle = handle.lock().await;
        handle.is_libssh()
    })
    .await
    .map_err(|_| SftpHealthProbeError::TimedOut)?;

    if libssh_backend {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(SftpHealthProbeError::TimedOut);
        }
        return tokio::time::timeout_at(deadline, async {
            let handle = handle.lock().await;
            handle.probe_libssh_sftp(remaining).await
        })
        .await
        .map_err(|_| SftpHealthProbeError::TimedOut)?
        .map_err(SftpHealthProbeError::Failed);
    }

    let mut session = tokio::time::timeout_at(deadline, auxiliary.sftp())
        .await
        .map_err(|_| SftpHealthProbeError::TimedOut)?
        .map_err(SftpHealthProbeError::Failed)?;
    match tokio::time::timeout_at(deadline, async {
        session
            .canonicalize(".")
            .await
            .map_err(|error| format!("SFTP canonicalize failed: {error}"))?;
        session
            .read_dir(".")
            .await
            .map_err(|error| format!("SFTP read_dir failed: {error}"))?;
        Ok::<_, String>(())
    })
    .await
    {
        Ok(result) => result.map_err(SftpHealthProbeError::Failed),
        Err(_) => {
            session.invalidate();
            Err(SftpHealthProbeError::TimedOut)
        }
    }
}

fn finish_health_report(
    state: &AppState,
    expected_runtime_id: String,
    mut report: SshHealthReport,
) -> Result<SshHealthReport, String> {
    let (current_runtime_id, terminal_channel_open) = {
        let runtimes = state.ssh.lock().map_err(|error| error.to_string())?;
        runtimes
            .get(&report.session_id)
            .map(|runtime| {
                (
                    Some(runtime.runtime_id.clone()),
                    runtime.terminal_channel_open.load(Ordering::SeqCst),
                )
            })
            .unwrap_or((None, false))
    };
    if current_runtime_id.as_deref() != Some(expected_runtime_id.as_str()) {
        return Err("SSH runtime 在健康检查期间已变化，请重试".to_string());
    }
    if !terminal_channel_open {
        report.terminal_channel_open = false;
        report.terminal_error.get_or_insert_with(|| {
            "SSH 主终端 channel 已关闭或正在重连，交互输入不可用".to_string()
        });
        if report.status == SshHealthStatus::Healthy {
            report.status = SshHealthStatus::Degraded;
        }
    }
    Ok(report)
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn bounded_ssh_health_error(error: &str) -> String {
    let mut value = error
        .chars()
        .take(MAX_SSH_HEALTH_ERROR_CHARACTERS)
        .collect::<String>();
    if error.chars().count() > MAX_SSH_HEALTH_ERROR_CHARACTERS {
        value.push_str("...");
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_health_errors_are_bounded_on_character_boundaries() {
        let error = "故".repeat(MAX_SSH_HEALTH_ERROR_CHARACTERS + 1);
        let bounded = bounded_ssh_health_error(&error);
        assert_eq!(bounded.chars().count(), MAX_SSH_HEALTH_ERROR_CHARACTERS + 3);
        assert!(bounded.ends_with("..."));
    }

    #[test]
    fn ssh_health_status_serializes_as_stable_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SshHealthStatus::Unresponsive).unwrap(),
            "\"unresponsive\""
        );
    }

    #[test]
    fn ssh_health_backend_serializes_as_stable_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SshBackendKind::Libssh).unwrap(),
            "\"libssh\""
        );
    }
}

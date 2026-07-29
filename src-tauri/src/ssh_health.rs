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
    pub transport_round_trip_ms: Option<u64>,
    pub channel_round_trip_ms: Option<u64>,
    pub sftp_round_trip_ms: Option<u64>,
    pub transport_error: Option<String>,
    pub channel_error: Option<String>,
    pub sftp_error: Option<String>,
    pub sftp_probed: bool,
}

#[tauri::command]
pub(crate) async fn check_ssh_health(
    state: State<'_, AppState>,
    session_id: String,
    probe_sftp: Option<bool>,
) -> Result<SshHealthReport, String> {
    check_ssh_health_inner(state.inner(), &session_id, probe_sftp.unwrap_or(false)).await
}

pub(super) async fn check_ssh_health_inner(
    state: &AppState,
    session_id: &str,
    probe_sftp: bool,
) -> Result<SshHealthReport, String> {
    let runtime_id = {
        let runtimes = state.ssh.lock().map_err(|error| error.to_string())?;
        runtimes
            .get(session_id)
            .map(|runtime| runtime.runtime_id.clone())
            .ok_or_else(|| "需要先连接 SSH/Tmux 会话才能执行健康检查".to_string())?
    };
    let auxiliary = ssh_auxiliary_lease(state, session_id)?;
    let handle = auxiliary.handle();
    let mut report = SshHealthReport {
        session_id: session_id.to_string(),
        runtime_id: runtime_id.clone(),
        checked_at: Utc::now(),
        status: SshHealthStatus::Healthy,
        transport_round_trip_ms: None,
        channel_round_trip_ms: None,
        sftp_round_trip_ms: None,
        transport_error: None,
        channel_error: None,
        sftp_error: None,
        sftp_probed: probe_sftp,
    };

    let ping_started = Instant::now();
    let ping = tokio::time::timeout(SSH_HEALTH_PING_TIMEOUT, async {
        let handle = handle.lock().await;
        handle
            .send_ping()
            .await
            .map_err(|error| format!("SSH keepalive 往返失败: {error}"))
    })
    .await;
    match ping {
        Ok(Ok(())) => report.transport_round_trip_ms = Some(elapsed_millis(ping_started)),
        Ok(Err(error)) => {
            report.status = SshHealthStatus::Unresponsive;
            report.transport_error = Some(bounded_ssh_health_error(&error));
            return finish_health_report(state, runtime_id, report);
        }
        Err(_) => {
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
        let result = tokio::time::timeout(SSH_HEALTH_CHANNEL_TIMEOUT, async {
            let libssh_backend = {
                let handle = handle.lock().await;
                handle.is_libssh()
            };
            if libssh_backend {
                let handle = handle.lock().await;
                handle.probe_libssh_sftp().await
            } else {
                let session = auxiliary.sftp().await?;
                session
                    .canonicalize(".")
                    .await
                    .map_err(|error| format!("SFTP canonicalize failed: {error}"))?;
                session
                    .read_dir(".")
                    .await
                    .map_err(|error| format!("SFTP read_dir failed: {error}"))?;
                Ok(())
            }
        })
        .await;
        match result {
            Ok(Ok(())) => {
                report.sftp_round_trip_ms = Some(elapsed_millis(sftp_started));
            }
            Ok(Err(error)) => {
                report.status = SshHealthStatus::Degraded;
                report.sftp_error = Some(bounded_ssh_health_error(&format!(
                    "SFTP health probe failed: {error}"
                )));
            }
            Err(_) => {
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

fn finish_health_report(
    state: &AppState,
    expected_runtime_id: String,
    report: SshHealthReport,
) -> Result<SshHealthReport, String> {
    let current_runtime_id = state
        .ssh
        .lock()
        .map_err(|error| error.to_string())?
        .get(&report.session_id)
        .map(|runtime| runtime.runtime_id.clone());
    if current_runtime_id.as_deref() != Some(expected_runtime_id.as_str()) {
        return Err("SSH runtime 在健康检查期间已变化，请重试".to_string());
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
}

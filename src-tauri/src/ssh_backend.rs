use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SshBackendKind {
    Russh,
    Libssh,
}

pub(super) enum SshBackendSession<H = PortMateSshHandler>
where
    H: client::Handler,
{
    Russh(client::Handle<H>),
    Libssh(libssh_rs::Session),
}

fn libssh_operation_deadline(timeout: Duration, label: &str) -> Result<Instant, String> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| format!("{label} libssh deadline is outside the supported range"))
}

fn run_libssh_runtime_operation<T>(
    session: &libssh_rs::Session,
    deadline: Instant,
    label: &str,
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    session
        .set_timeout_until(deadline)
        .map_err(|error| format!("{label} libssh deadline setup failed: {error}"))?;
    let result = operation();
    let restored = session
        .set_option(libssh_rs::SshOption::Timeout(
            SSH_RUNTIME_OPERATION_TIMEOUT,
        ))
        .map_err(|error| format!("{label} libssh runtime timeout restore failed: {error}"));
    match (result, restored) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(restore_error)) => Err(format!("{error}; {restore_error}")),
    }
}

impl<H> SshBackendSession<H>
where
    H: client::Handler,
{
    pub(super) fn from_russh(handle: client::Handle<H>) -> Self {
        Self::Russh(handle)
    }

    pub(super) fn from_libssh(session: libssh_rs::Session) -> Self {
        Self::Libssh(session)
    }

    pub(super) fn is_libssh(&self) -> bool {
        matches!(self, Self::Libssh(_))
    }

    #[cfg(all(test, unix))]
    pub(super) fn russh_compat(&self) -> Result<&client::Handle<H>, String> {
        match self {
            Self::Russh(handle) => Ok(handle),
            Self::Libssh(_) => Err("该 SSH 操作尚未迁移到 libssh backend".to_string()),
        }
    }

    pub(super) async fn disconnect(&self, description: &str) -> Result<(), String> {
        match self {
            Self::Russh(handle) => handle
                .disconnect(Disconnect::ByApplication, description, "en")
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = session.clone();
                tokio::task::spawn_blocking(move || session.disconnect())
                    .await
                    .map_err(|error| format!("libssh disconnect worker failed: {error}"))?;
                Ok(())
            }
        }
    }

    pub(super) async fn send_ping(&self, timeout: Duration) -> Result<(), String> {
        match self {
            Self::Russh(handle) => handle.send_ping().await.map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = session.clone();
                let deadline = libssh_operation_deadline(timeout, "SSH keepalive")?;
                tokio::task::spawn_blocking(move || {
                    run_libssh_runtime_operation(
                        &session,
                        deadline,
                        "SSH keepalive",
                        || session.send_keepalive().map_err(|error| error.to_string()),
                    )
                })
                    .await
                    .map_err(|error| format!("libssh keepalive worker failed: {error}"))?
            }
        }
    }

    pub(super) async fn probe_libssh_sftp(&self, timeout: Duration) -> Result<(), String> {
        let Self::Libssh(session) = self else {
            return Err("SFTP libssh health probe requires a libssh backend".to_string());
        };
        let session = session.clone();
        let deadline = libssh_operation_deadline(timeout, "SFTP health probe")?;
        tokio::task::spawn_blocking(move || {
            run_libssh_runtime_operation(&session, deadline, "SFTP health probe", || {
                let sftp = session
                    .sftp()
                    .map_err(|error| format!("libssh SFTP initialization failed: {error}"))?;
                sftp.canonicalize(".")
                    .map_err(|error| format!("libssh SFTP canonicalize failed: {error}"))?;
                sftp.read_dir_bounded(".", MAX_FILE_DIRECTORY_ENTRIES)
                    .map_err(|error| format!("libssh SFTP read_dir failed: {error}"))?;
                Ok(())
            })
        })
        .await
        .map_err(|error| format!("libssh SFTP health worker failed: {error}"))?
    }

    pub(super) async fn open_libssh_sftp(
        &self,
        timeout: Duration,
    ) -> Result<SftpBackendSession, String> {
        let Self::Libssh(session) = self else {
            return Err("libssh SFTP setup requires a libssh backend".to_string());
        };
        let session = session.clone();
        let deadline = libssh_operation_deadline(timeout, "SFTP setup")?;
        tokio::task::spawn_blocking(move || {
            run_libssh_runtime_operation(&session, deadline, "SFTP setup", || {
                session
                    .sftp()
                    .map(SftpBackendSession::from_libssh)
                    .map_err(|error| format!("SFTP 初始化失败: {error}"))
            })
        })
        .await
        .map_err(|error| format!("libssh SFTP setup worker failed: {error}"))?
    }

    pub(super) async fn open_exec(
        &self,
        command: &str,
        label: &str,
        timeout: Duration,
    ) -> Result<SshBackendChannel, String> {
        match self {
            Self::Russh(handle) => {
                let channel = handle
                    .channel_open_session()
                    .await
                    .map_err(|error| format!("{label} 打开 SSH channel 失败: {error}"))?;
                channel
                    .exec(true, command)
                    .await
                    .map_err(|error| format!("{label} 启动 SSH exec 失败: {error}"))?;
                Ok(SshBackendChannel::Russh(channel))
            }
            Self::Libssh(session) => {
                let session = session.clone();
                let command = command.to_string();
                let deadline = libssh_operation_deadline(timeout, label)?;
                let worker_label = label.to_string();
                let channel = tokio::task::spawn_blocking(move || {
                    run_libssh_runtime_operation(&session, deadline, &worker_label, || {
                        let channel = session
                            .new_channel()
                            .map_err(|error| error.to_string())?;
                        channel
                            .open_session()
                            .map_err(|error| error.to_string())?;
                        channel
                            .request_exec(&command)
                            .map_err(|error| error.to_string())?;
                        Ok(channel)
                    })
                })
                .await
                .map_err(|error| format!("{label} libssh worker failed: {error}"))?
                .map_err(|error| format!("{label} libssh setup failed: {error}"))?;
                Ok(SshBackendChannel::from_libssh(channel))
            }
        }
    }

    pub(super) async fn open_direct_tcpip(
        &self,
        target_host: String,
        target_port: u16,
        originator_address: String,
        originator_port: u16,
        timeout: Duration,
    ) -> Result<SshBackendChannel, String> {
        match self {
            Self::Russh(handle) => handle
                .channel_open_direct_tcpip(
                    target_host,
                    u32::from(target_port),
                    originator_address,
                    u32::from(originator_port),
                )
                .await
                .map(SshBackendChannel::from_russh)
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = session.clone();
                let deadline = libssh_operation_deadline(timeout, "direct-tcpip open")?;
                let channel = tokio::task::spawn_blocking(move || {
                    run_libssh_runtime_operation(
                        &session,
                        deadline,
                        "direct-tcpip open",
                        || {
                            let channel = session
                                .new_channel()
                                .map_err(|error| error.to_string())?;
                            channel
                                .open_forward(
                                    &target_host,
                                    target_port,
                                    &originator_address,
                                    originator_port,
                                )
                                .map_err(|error| error.to_string())?;
                            Ok(channel)
                        },
                    )
                })
                .await
                .map_err(|error| format!("libssh direct-tcpip worker failed: {error}"))?
                .map_err(|error| format!("libssh direct-tcpip open failed: {error}"))?;
                Ok(SshBackendChannel::from_libssh_forward(channel))
            }
        }
    }

    pub(super) async fn listen_remote_forward(
        &self,
        bind_host: String,
        bind_port: u16,
        timeout: Duration,
    ) -> Result<u16, String> {
        match self {
            Self::Russh(handle) => {
                let returned_port = handle
                    .tcpip_forward(bind_host, u32::from(bind_port))
                    .await
                    .map_err(|error| error.to_string())?;
                if returned_port == 0 {
                    Ok(bind_port)
                } else {
                    u16::try_from(returned_port).map_err(|_| {
                        format!("remote forward returned invalid port {returned_port}")
                    })
                }
            }
            Self::Libssh(session) => {
                let session = session.clone();
                let deadline = libssh_operation_deadline(timeout, "remote forward request")?;
                tokio::task::spawn_blocking(move || {
                    run_libssh_runtime_operation(
                        &session,
                        deadline,
                        "remote forward request",
                        || {
                            session
                                .listen_forward(Some(&bind_host), bind_port)
                                .map_err(|error| error.to_string())
                        },
                    )
                })
                .await
                .map_err(|error| format!("libssh remote forward worker failed: {error}"))?
                .map_err(|error| format!("libssh remote forward request failed: {error}"))
            }
        }
    }

    pub(super) async fn cancel_remote_forward(
        &self,
        bind_host: String,
        bind_port: u16,
        timeout: Duration,
    ) -> Result<(), String> {
        match self {
            Self::Russh(handle) => handle
                .cancel_tcpip_forward(bind_host, u32::from(bind_port))
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = session.clone();
                let deadline = libssh_operation_deadline(timeout, "remote forward cancel")?;
                tokio::task::spawn_blocking(move || {
                    run_libssh_runtime_operation(
                        &session,
                        deadline,
                        "remote forward cancel",
                        || {
                            session
                                .cancel_forward(Some(&bind_host), bind_port)
                                .map_err(|error| error.to_string())
                        },
                    )
                })
                .await
                .map_err(|error| format!("libssh remote forward cancel worker failed: {error}"))?
            }
        }
    }

    pub(super) fn libssh_forward_session(&self) -> Option<libssh_rs::Session> {
        match self {
            Self::Russh(_) => None,
            Self::Libssh(session) => Some(session.clone()),
        }
    }
}

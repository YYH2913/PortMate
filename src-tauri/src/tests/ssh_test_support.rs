use super::*;

#[cfg(unix)]
pub(super) fn openssh_test_server_path() -> Option<&'static Path> {
    ["/usr/sbin/sshd", "/usr/local/sbin/sshd"]
        .into_iter()
        .map(Path::new)
        .find(|path| path.exists())
}

#[cfg(unix)]
pub(super) fn generate_ed25519_test_key(path: &Path) {
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success(), "ssh-keygen failed for {}", path.display());
}

#[cfg(unix)]
pub(super) fn openssh_test_username() -> String {
    std::env::var("USER").unwrap_or_else(|_| {
        String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout)
            .unwrap()
            .trim()
            .to_string()
    })
}

#[cfg(unix)]
pub(super) fn write_openssh_test_config(
    config_path: &Path,
    host_key: &Path,
    pid_file: &Path,
    authorized_keys: &Path,
    port: u16,
) {
    write_openssh_test_config_with_extra(
        config_path,
        host_key,
        pid_file,
        authorized_keys,
        port,
        "",
    );
}

#[cfg(unix)]
pub(super) fn write_openssh_test_config_with_extra(
    config_path: &Path,
    host_key: &Path,
    pid_file: &Path,
    authorized_keys: &Path,
    port: u16,
    extra_config: &str,
) {
    fs::write(
        config_path,
        format!(
            "AddressFamily inet\nListenAddress 127.0.0.1\nPort {port}\nHostKey {}\nPidFile {}\nAuthorizedKeysFile {}\nAuthenticationMethods publickey\nPubkeyAuthentication yes\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nPermitRootLogin prohibit-password\nStrictModes no\nAllowTcpForwarding yes\nLogLevel ERROR\nSubsystem sftp internal-sftp\n{extra_config}",
            host_key.display(),
            pid_file.display(),
            authorized_keys.display(),
        ),
    )
    .unwrap();
}

#[cfg(unix)]
pub(super) fn spawn_openssh_test_server(sshd_path: &Path, config_path: &Path) -> ChildGuard {
    let child = Command::new(sshd_path)
        .args(["-D", "-e", "-f"])
        .arg(config_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    ChildGuard(Some(child))
}

#[cfg(unix)]
pub(super) fn spawn_openssh_test_agent(socket_path: &Path) -> ChildGuard {
    let child = Command::new("ssh-agent")
        .args(["-D", "-a"])
        .arg(socket_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    ChildGuard(Some(child))
}

#[cfg(unix)]
pub(super) async fn wait_for_openssh_test_agent(
    agent: &mut ChildGuard,
    socket_path: &Path,
    label: &str,
) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if socket_path.exists() {
                let status = Command::new("ssh-add")
                    .arg("-l")
                    .env("SSH_AUTH_SOCK", socket_path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap();
                if status.success() || status.code() == Some(1) {
                    break;
                }
            }
            if let Some(status) = agent.0.as_mut().unwrap().try_wait().unwrap() {
                let mut stderr = String::new();
                agent
                    .0
                    .as_mut()
                    .unwrap()
                    .stderr
                    .as_mut()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                panic!("{label} exited early with {status}: {stderr}");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} did not start"));
}

#[cfg(unix)]
#[derive(Default)]
pub(super) struct MixedAuthTestCounters {
    pub(super) password_attempts: AtomicU64,
    pub(super) password_completions: AtomicU64,
    pub(super) password_successes: AtomicU64,
    pub(super) keyboard_interactive_successes: AtomicU64,
    pub(super) session_channel_attempts: AtomicU64,
    pub(super) session_channel_completions: AtomicU64,
    pub(super) direct_tcpip_attempts: AtomicU64,
    pub(super) direct_tcpip_completions: AtomicU64,
    pub(super) channel_closes: AtomicU64,
    pub(super) scp_upload_bytes: AtomicU64,
}

#[cfg(unix)]
#[derive(Clone)]
pub(super) struct MixedAuthTestServer {
    username: String,
    secret: String,
    counters: Arc<MixedAuthTestCounters>,
    password_auth_delay: Option<Duration>,
    session_channel_delay: Option<Duration>,
    direct_tcpip_delay: Option<Duration>,
    active_scp_uploads: Arc<Mutex<HashSet<russh::ChannelId>>>,
}

#[cfg(unix)]
impl russh::server::Handler for MixedAuthTestServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<russh::server::Auth, Self::Error> {
        self.counters
            .password_attempts
            .fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.password_auth_delay {
            tokio::time::sleep(delay).await;
        }
        self.counters
            .password_completions
            .fetch_add(1, Ordering::SeqCst);
        if user == self.username && password == self.secret {
            self.counters
                .password_successes
                .fetch_add(1, Ordering::SeqCst);
            Ok(russh::server::Auth::Accept)
        } else {
            Ok(russh::server::Auth::reject())
        }
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        user: &str,
        _submethods: &str,
        response: Option<russh::server::Response<'a>>,
    ) -> Result<russh::server::Auth, Self::Error> {
        let Some(mut response) = response else {
            return Ok(russh::server::Auth::Partial {
                name: std::borrow::Cow::Borrowed("PortMate integration"),
                instructions: std::borrow::Cow::Borrowed("Enter the test secret"),
                prompts: std::borrow::Cow::Owned(vec![(
                    std::borrow::Cow::Borrowed("Secret: "),
                    false,
                )]),
            });
        };
        let accepted = user == self.username
            && response
                .next()
                .is_some_and(|value| value.as_ref() == self.secret.as_bytes())
            && response.next().is_none();
        if accepted {
            self.counters
                .keyboard_interactive_successes
                .fetch_add(1, Ordering::SeqCst);
            Ok(russh::server::Auth::Accept)
        } else {
            Ok(russh::server::Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<russh::server::Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        self.counters
            .session_channel_attempts
            .fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.session_channel_delay {
            tokio::time::sleep(delay).await;
        }
        self.counters
            .session_channel_completions
            .fetch_add(1, Ordering::SeqCst);
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        match data {
            b"__PORTMATE_TEST_EXEC_SUCCESS__" => {
                session.data(channel, b"captured".to_vec())?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
            }
            b"__PORTMATE_TEST_EXEC_OVERFLOW__" => {
                session.extended_data(channel, 1, vec![b'x'; MAX_SSH_EXEC_STDERR_BYTES + 1])?;
                session.eof(channel)?;
            }
            b"__PORTMATE_TEST_EXEC_NONZERO__" => {
                session.data(channel, b"partial output".to_vec())?;
                session.extended_data(channel, 1, b"remote failure".to_vec())?;
                session.exit_status_request(channel, 7)?;
                session.eof(channel)?;
            }
            b"__PORTMATE_TEST_EXEC_EOF_BEFORE_NONZERO__" => {
                session.data(channel, b"early output".to_vec())?;
                session.extended_data(channel, 1, b"late status failure".to_vec())?;
                session.eof(channel)?;
                session.exit_status_request(channel, 9)?;
            }
            b"__PORTMATE_TEST_EXEC_TIMEOUT__" => {}
            command
                if command
                    .windows(b"__PORTMATE_TEST_SCP_UPLOAD_DATA_SUCCESS__".len())
                    .any(|window| window == b"__PORTMATE_TEST_SCP_UPLOAD_DATA_SUCCESS__") =>
            {
                self.active_scp_uploads.lock().unwrap().insert(channel);
                session.data(
                    channel,
                    b"__PORTMATE_SIZE__4\n__PORTMATE_RESUME__0\n__PORTMATE_PROGRESS__0\n".to_vec(),
                )?;
            }
            command
                if command
                    .windows(b"__PORTMATE_TEST_SCP_UPLOAD_SUCCESS__".len())
                    .any(|window| window == b"__PORTMATE_TEST_SCP_UPLOAD_SUCCESS__") =>
            {
                session.data(
                    channel,
                    b"__PORTMATE_SIZE__0\n__PORTMATE_RESUME__0\n__PORTMATE_DONE__0\n".to_vec(),
                )?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
            }
            command
                if command
                    .windows(b"__PORTMATE_TEST_SCP_UPLOAD_EOF_BEFORE_NONZERO__".len())
                    .any(|window| window == b"__PORTMATE_TEST_SCP_UPLOAD_EOF_BEFORE_NONZERO__") =>
            {
                session.data(
                    channel,
                    b"__PORTMATE_SIZE__0\n__PORTMATE_RESUME__0\n__PORTMATE_DONE__0\n".to_vec(),
                )?;
                session.extended_data(channel, 1, b"late SCP upload failure".to_vec())?;
                session.eof(channel)?;
                session.exit_status_request(channel, 12)?;
            }
            command
                if command
                    .windows(b"__PORTMATE_TEST_SCP_DOWNLOAD_SUCCESS__".len())
                    .any(|window| window == b"__PORTMATE_TEST_SCP_DOWNLOAD_SUCCESS__") =>
            {
                session.data(channel, b"C0644 4 file.bin\ndata\0".to_vec())?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
            }
            command
                if command
                    .windows(b"__PORTMATE_TEST_SCP_DOWNLOAD_STDERR_BEFORE_DATA__".len())
                    .any(|window| {
                        window == b"__PORTMATE_TEST_SCP_DOWNLOAD_STDERR_BEFORE_DATA__"
                    }) =>
            {
                session.extended_data(channel, 1, b"remote login warning".to_vec())?;
                session.data(channel, b"C0644 4 file.bin\ndata\0".to_vec())?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
            }
            command
                if command
                    .windows(b"__PORTMATE_TEST_SCP_DOWNLOAD_OVERSIZED_HEADER__".len())
                    .any(|window| window == b"__PORTMATE_TEST_SCP_DOWNLOAD_OVERSIZED_HEADER__") =>
            {
                let mut header = b"C0644 4 ".to_vec();
                header.extend(std::iter::repeat_n(b'x', MAX_SCP_PROTOCOL_LINE_BYTES + 1));
                session.data(channel, header)?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
            }
            command
                if command
                    .windows(b"__PORTMATE_TEST_SCP_DOWNLOAD_EOF_BEFORE_NONZERO__".len())
                    .any(|window| {
                        window == b"__PORTMATE_TEST_SCP_DOWNLOAD_EOF_BEFORE_NONZERO__"
                    }) =>
            {
                session.data(channel, b"C0644 4 file.bin\ndata\0".to_vec())?;
                session.extended_data(channel, 1, b"late SCP download failure".to_vec())?;
                session.eof(channel)?;
                session.exit_status_request(channel, 13)?;
            }
            command
                if command
                    .windows(b"__PORTMATE_TEST_REMOTE_COPY_SUCCESS__".len())
                    .any(|window| window == b"__PORTMATE_TEST_REMOTE_COPY_SUCCESS__") =>
            {
                session.data(
                    channel,
                    b"__PORTMATE_SIZE__4\n__PORTMATE_RESUME__0\n__PORTMATE_PROGRESS__4\n__PORTMATE_DONE__4\n"
                        .to_vec(),
                )?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
            }
            command
                if command
                    .windows(b"__PORTMATE_TEST_REMOTE_COPY_EOF_BEFORE_NONZERO__".len())
                    .any(|window| {
                        window == b"__PORTMATE_TEST_REMOTE_COPY_EOF_BEFORE_NONZERO__"
                    }) =>
            {
                session.data(
                    channel,
                    b"__PORTMATE_SIZE__4\n__PORTMATE_RESUME__0\n__PORTMATE_PROGRESS__4\n__PORTMATE_DONE__4\n"
                        .to_vec(),
                )?;
                session.extended_data(channel, 1, b"late remote-copy failure".to_vec())?;
                session.eof(channel)?;
                session.exit_status_request(channel, 11)?;
            }
            command if command.starts_with(b"tmux ") => {
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        _session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        if self.active_scp_uploads.lock().unwrap().contains(&channel) {
            self.counters
                .scp_upload_bytes
                .fetch_add(data.len() as u64, Ordering::SeqCst);
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: russh::ChannelId,
        session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        if self.active_scp_uploads.lock().unwrap().remove(&channel) {
            let received = self.counters.scp_upload_bytes.load(Ordering::SeqCst);
            session.data(
                channel,
                format!("__PORTMATE_PROGRESS__{received}\n__PORTMATE_DONE__{received}\n")
                    .into_bytes(),
            )?;
            session.exit_status_request(channel, u32::from(received != 4))?;
            session.eof(channel)?;
        }
        Ok(())
    }

    async fn channel_close(
        &mut self,
        _channel: russh::ChannelId,
        _session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        self.counters.channel_closes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<russh::server::Msg>,
        host_to_connect: &str,
        port_to_connect: u32,
        _originator_address: &str,
        _originator_port: u32,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        self.counters
            .direct_tcpip_attempts
            .fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.direct_tcpip_delay {
            tokio::time::sleep(delay).await;
        }
        self.counters
            .direct_tcpip_completions
            .fetch_add(1, Ordering::SeqCst);
        let Ok(mut socket) = TcpStream::connect((host_to_connect, port_to_connect as u16)).await
        else {
            return Ok(());
        };
        reply.accept().await;
        tokio::spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut socket).await;
        });
        Ok(())
    }
}

#[cfg(unix)]
#[derive(Default)]
pub(super) struct SilentSftpTestCounters {
    pub(super) session_channel_attempts: AtomicU64,
    pub(super) session_channel_completions: AtomicU64,
    pub(super) subsystem_requests: AtomicU64,
    pub(super) lstat_attempts: AtomicU64,
}

#[cfg(unix)]
#[derive(Clone)]
pub(super) struct SilentSftpSshTestServer {
    username: String,
    secret: String,
    counters: Arc<SilentSftpTestCounters>,
    channels: Arc<tokio::sync::Mutex<HashMap<russh::ChannelId, Channel<russh::server::Msg>>>>,
    response_delay: Duration,
}

#[cfg(unix)]
impl russh::server::Handler for SilentSftpSshTestServer {
    type Error = russh::Error;

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<russh::server::Auth, Self::Error> {
        if user == self.username && password == self.secret {
            Ok(russh::server::Auth::Accept)
        } else {
            Ok(russh::server::Auth::reject())
        }
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<russh::server::Msg>,
        reply: russh::server::ChannelOpenHandle,
        _session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        self.counters
            .session_channel_attempts
            .fetch_add(1, Ordering::SeqCst);
        self.channels.lock().await.insert(channel.id(), channel);
        self.counters
            .session_channel_completions
            .fetch_add(1, Ordering::SeqCst);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: russh::ChannelId,
        name: &str,
        session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        if name != "sftp" {
            session.channel_failure(channel_id)?;
            return Ok(());
        }
        let Some(channel) = self.channels.lock().await.remove(&channel_id) else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        self.counters
            .subsystem_requests
            .fetch_add(1, Ordering::SeqCst);
        session.channel_success(channel_id)?;
        russh_sftp::server::run(
            channel.into_stream(),
            SilentSftpProtocolTestServer {
                counters: Arc::clone(&self.counters),
                response_delay: self.response_delay,
            },
        )
        .await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: russh::ChannelId,
        _data: &[u8],
        session: &mut russh::server::Session,
    ) -> Result<(), Self::Error> {
        self.channels.lock().await.remove(&channel);
        session.channel_success(channel)?;
        Ok(())
    }
}

#[cfg(unix)]
pub(super) struct SilentSftpProtocolTestServer {
    counters: Arc<SilentSftpTestCounters>,
    response_delay: Duration,
}

#[cfg(unix)]
impl russh_sftp::server::Handler for SilentSftpProtocolTestServer {
    type Error = russh_sftp::protocol::StatusCode;

    fn unimplemented(&self) -> Self::Error {
        russh_sftp::protocol::StatusCode::OpUnsupported
    }

    async fn lstat(
        &mut self,
        id: u32,
        _path: String,
    ) -> Result<russh_sftp::protocol::Attrs, Self::Error> {
        self.counters.lstat_attempts.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.response_delay).await;
        Ok(russh_sftp::protocol::Attrs {
            id,
            attrs: russh_sftp::protocol::FileAttributes::default(),
        })
    }
}

#[cfg(unix)]
#[derive(Clone)]
pub(super) struct AcceptAnyTestSshClient;

#[cfg(unix)]
impl client::Handler for AcceptAnyTestSshClient {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[cfg(unix)]
pub(super) async fn spawn_mixed_auth_test_server(
    host_key_path: &Path,
    username: &str,
    secret: &str,
) -> (u16, Arc<MixedAuthTestCounters>, tokio::task::JoinHandle<()>) {
    spawn_mixed_auth_test_server_with_delays(host_key_path, username, secret, None, None, None)
        .await
}

#[cfg(unix)]
pub(super) async fn spawn_mixed_auth_test_server_with_delays(
    host_key_path: &Path,
    username: &str,
    secret: &str,
    password_auth_delay: Option<Duration>,
    session_channel_delay: Option<Duration>,
    direct_tcpip_delay: Option<Duration>,
) -> (u16, Arc<MixedAuthTestCounters>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let counters = Arc::new(MixedAuthTestCounters::default());
    let handler = MixedAuthTestServer {
        username: username.to_string(),
        secret: secret.to_string(),
        counters: Arc::clone(&counters),
        password_auth_delay,
        session_channel_delay,
        direct_tcpip_delay,
        active_scp_uploads: Arc::new(Mutex::new(HashSet::new())),
    };
    let config = Arc::new(russh::server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![load_secret_key(host_key_path, None).unwrap()],
        ..Default::default()
    });
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let config = Arc::clone(&config);
            let handler = handler.clone();
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    (port, counters, task)
}

#[cfg(unix)]
pub(super) async fn spawn_silent_sftp_test_server(
    host_key_path: &Path,
    username: &str,
    secret: &str,
    response_delay: Duration,
) -> (
    u16,
    Arc<SilentSftpTestCounters>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let counters = Arc::new(SilentSftpTestCounters::default());
    let handler = SilentSftpSshTestServer {
        username: username.to_string(),
        secret: secret.to_string(),
        counters: Arc::clone(&counters),
        channels: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        response_delay,
    };
    let config = Arc::new(russh::server::Config {
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        keys: vec![load_secret_key(host_key_path, None).unwrap()],
        ..Default::default()
    });
    let task = tokio::spawn(async move {
        loop {
            let Ok((socket, _)) = listener.accept().await else {
                break;
            };
            let config = Arc::clone(&config);
            let handler = handler.clone();
            tokio::spawn(async move {
                if let Ok(session) = russh::server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    (port, counters, task)
}

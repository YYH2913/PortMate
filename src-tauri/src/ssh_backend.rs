use super::*;

pub(super) enum SshBackendSession<H = PortMateSshHandler>
where
    H: client::Handler,
{
    Russh(client::Handle<H>),
    Libssh(libssh_rs::Session),
}

const MAX_LIBSSH_AGENT_FORWARD_CHANNELS: usize = 16;

pub(super) fn start_libssh_agent_forwarder(
    session: libssh_rs::Session,
    agent_socket_path: std::path::PathBuf,
    closed: Arc<AtomicBool>,
) -> tokio::sync::oneshot::Receiver<()> {
    let (finished_sender, finished_receiver) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn(async move {
        let mut bridges = tokio::task::JoinSet::new();
        while !closed.load(Ordering::SeqCst) {
            while let Some(result) = bridges.try_join_next() {
                report_libssh_agent_bridge_result(result);
            }
            let accept_session = session.clone();
            let accepted =
                tokio::task::spawn_blocking(move || accept_session.accept_agent_forward()).await;
            match accepted {
                Ok(Some(channel)) if bridges.len() >= MAX_LIBSSH_AGENT_FORWARD_CHANNELS => {
                    eprintln!(
                        "PortMate: rejected libssh agent forward channel at the {} channel limit",
                        MAX_LIBSSH_AGENT_FORWARD_CHANNELS
                    );
                    let _ = tokio::task::spawn_blocking(move || channel.close()).await;
                }
                Ok(Some(channel)) => {
                    let socket_path = agent_socket_path.clone();
                    let channel_closed = Arc::clone(&closed);
                    bridges.spawn_blocking(move || {
                        bridge_libssh_agent_channel(channel, socket_path, channel_closed)
                    });
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(20)).await,
                Err(error) => {
                    eprintln!("PortMate: libssh agent forward accept worker failed: {error}");
                    break;
                }
            }
        }
        session.enable_accept_agent_forward(false);
        while let Some(result) = bridges.join_next().await {
            report_libssh_agent_bridge_result(result);
        }
        let _ = finished_sender.send(());
    });
    finished_receiver
}

fn report_libssh_agent_bridge_result(result: Result<Result<(), String>, tokio::task::JoinError>) {
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => eprintln!("PortMate: libssh agent forward bridge failed: {error}"),
        Err(error) => eprintln!("PortMate: libssh agent forward worker failed: {error}"),
    }
}

#[cfg(unix)]
fn bridge_libssh_agent_channel(
    channel: libssh_rs::Channel,
    agent_socket_path: std::path::PathBuf,
    closed: Arc<AtomicBool>,
) -> Result<(), String> {
    use std::io::{ErrorKind, Read, Write};
    use std::os::unix::net::UnixStream;

    let mut socket = UnixStream::connect(&agent_socket_path).map_err(|error| {
        format!(
            "connect SSH agent socket {} failed: {error}",
            agent_socket_path.display()
        )
    })?;
    socket
        .set_read_timeout(Some(Duration::from_millis(50)))
        .map_err(|error| format!("set SSH agent read timeout failed: {error}"))?;
    socket
        .set_write_timeout(Some(Duration::from_millis(250)))
        .map_err(|error| format!("set SSH agent write timeout failed: {error}"))?;
    let mut socket_buffer = [0_u8; 64 * 1024];
    let mut channel_buffer = vec![0_u8; 64 * 1024];

    loop {
        if closed.load(Ordering::SeqCst) {
            break;
        }
        let mut channel_finished = false;
        match channel
            .poll_timeout(false, Some(Duration::ZERO))
            .map_err(|error| error.to_string())?
        {
            libssh_rs::PollStatus::AvailableBytes(available) if available > 0 => {
                let read_limit = (available as usize).min(channel_buffer.len());
                let read = channel
                    .read_timeout(
                        &mut channel_buffer[..read_limit],
                        false,
                        Some(Duration::from_millis(50)),
                    )
                    .map_err(|error| error.to_string())?;
                if read > 0 {
                    socket
                        .write_all(&channel_buffer[..read])
                        .map_err(|error| format!("write SSH agent request failed: {error}"))?;
                }
            }
            libssh_rs::PollStatus::AvailableBytes(_) => {}
            libssh_rs::PollStatus::EndOfFile => channel_finished = true,
        }
        if channel.is_closed() || channel.is_eof() || channel_finished {
            break;
        }

        match socket.read(&mut socket_buffer) {
            Ok(0) => {
                let _ = channel.send_eof();
                break;
            }
            Ok(read) => {
                let mut stdin = channel.stdin();
                stdin
                    .write_all(&socket_buffer[..read])
                    .map_err(|error| format!("write SSH agent response failed: {error}"))?;
                drop(stdin);
                channel
                    .flush(Some(Duration::from_millis(250)))
                    .map_err(|error| format!("flush SSH agent response failed: {error}"))?;
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(format!("read SSH agent response failed: {error}")),
        }
    }
    let _ = channel.close();
    Ok(())
}

#[cfg(not(unix))]
fn bridge_libssh_agent_channel(
    _channel: libssh_rs::Channel,
    _agent_socket_path: std::path::PathBuf,
    _closed: Arc<AtomicBool>,
) -> Result<(), String> {
    Err("libssh SSH agent forwarding requires a Unix-domain SSH agent socket".to_string())
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

    #[cfg(test)]
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

    pub(super) async fn send_ping(&self) -> Result<(), String> {
        match self {
            Self::Russh(handle) => handle.send_ping().await.map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = session.clone();
                tokio::task::spawn_blocking(move || session.send_keepalive())
                    .await
                    .map_err(|error| format!("libssh keepalive worker failed: {error}"))?
                    .map_err(|error| error.to_string())
            }
        }
    }

    pub(super) async fn probe_libssh_sftp(&self) -> Result<(), String> {
        let Self::Libssh(session) = self else {
            return Err("SFTP libssh health probe requires a libssh backend".to_string());
        };
        let session = session.clone();
        tokio::task::spawn_blocking(move || {
            let sftp = session
                .sftp()
                .map_err(|error| format!("libssh SFTP initialization failed: {error}"))?;
            sftp.canonicalize(".")
                .map_err(|error| format!("libssh SFTP canonicalize failed: {error}"))?;
            sftp.read_dir(".")
                .map_err(|error| format!("libssh SFTP read_dir failed: {error}"))?;
            Ok::<_, String>(())
        })
        .await
        .map_err(|error| format!("libssh SFTP health worker failed: {error}"))?
    }

    pub(super) async fn open_exec(
        &self,
        command: &str,
        label: &str,
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
                let channel = tokio::task::spawn_blocking(move || {
                    let channel = session.new_channel()?;
                    channel.open_session()?;
                    channel.request_exec(&command)?;
                    Ok::<_, libssh_rs::Error>(channel)
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
                let channel = tokio::task::spawn_blocking(move || {
                    let channel = session.new_channel()?;
                    channel.open_forward(
                        &target_host,
                        target_port,
                        &originator_address,
                        originator_port,
                    )?;
                    Ok::<_, libssh_rs::Error>(channel)
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
                tokio::task::spawn_blocking(move || {
                    session.listen_forward(Some(&bind_host), bind_port)
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
    ) -> Result<(), String> {
        match self {
            Self::Russh(handle) => handle
                .cancel_tcpip_forward(bind_host, u32::from(bind_port))
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = session.clone();
                tokio::task::spawn_blocking(move || {
                    session.cancel_forward(Some(&bind_host), bind_port)
                })
                .await
                .map_err(|error| format!("libssh remote forward cancel worker failed: {error}"))?
                .map_err(|error| error.to_string())
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

pub(super) enum SshBackendChannel {
    Russh(Channel<client::Msg>),
    Libssh(LibsshChannelReader),
}

impl std::fmt::Debug for SshBackendChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("SshBackendChannel")
            .field(&match self {
                Self::Russh(_) => "russh",
                Self::Libssh(_) => "libssh",
            })
            .finish()
    }
}

impl SshBackendChannel {
    pub(super) fn from_russh(channel: Channel<client::Msg>) -> Self {
        Self::Russh(channel)
    }

    pub(super) fn from_libssh(channel: libssh_rs::Channel) -> Self {
        Self::Libssh(LibsshChannelReader::new(channel, true))
    }

    pub(super) fn from_libssh_forward(channel: libssh_rs::Channel) -> Self {
        // Forwarding channels have no process exit status; querying one can block until close.
        Self::Libssh(LibsshChannelReader::new(channel, false))
    }

    pub(super) fn split(self) -> (SshBackendChannelReader, SshBackendChannelWriter) {
        match self {
            Self::Russh(channel) => {
                let (reader, writer) = channel.split();
                (
                    SshBackendChannelReader::Russh(reader),
                    SshBackendChannelWriter::Russh(writer),
                )
            }
            Self::Libssh(reader) => {
                let writer = SshBackendChannelWriter::Libssh(Arc::clone(&reader.channel));
                (SshBackendChannelReader::Libssh(reader), writer)
            }
        }
    }

    pub(super) async fn wait(&mut self) -> Option<SshBackendMessage> {
        match self {
            Self::Russh(channel) => channel.wait().await.map(SshBackendMessage::from),
            Self::Libssh(reader) => reader.wait().await,
        }
    }

    pub(super) async fn data(&self, data: &[u8]) -> Result<(), String> {
        match self {
            Self::Russh(channel) => channel.data(data).await.map_err(|error| error.to_string()),
            Self::Libssh(reader) => {
                let channel = Arc::clone(&reader.channel);
                let data = data.to_vec();
                tokio::task::spawn_blocking(move || {
                    let channel = channel.blocking_lock();
                    let mut stdin = channel.stdin();
                    stdin.write_all(&data)?;
                    stdin.flush()
                })
                .await
                .map_err(|error| format!("libssh write worker failed: {error}"))?
                .map_err(|error| error.to_string())
            }
        }
    }

    pub(super) async fn eof(&self) -> Result<(), String> {
        match self {
            Self::Russh(channel) => channel.eof().await.map_err(|error| error.to_string()),
            Self::Libssh(reader) => {
                let channel = Arc::clone(&reader.channel);
                tokio::task::spawn_blocking(move || channel.blocking_lock().send_eof())
                    .await
                    .map_err(|error| format!("libssh EOF worker failed: {error}"))?
                    .map_err(|error| error.to_string())
            }
        }
    }

    pub(super) async fn close(&self) -> Result<(), String> {
        match self {
            Self::Russh(channel) => channel.close().await.map_err(|error| error.to_string()),
            Self::Libssh(reader) => close_libssh_channel(Arc::clone(&reader.channel)).await,
        }
    }
}

pub(super) enum SshBackendChannelReader {
    Russh(ChannelReadHalf),
    Libssh(LibsshChannelReader),
}

impl SshBackendChannelReader {
    pub(super) async fn wait(&mut self) -> Option<SshBackendMessage> {
        match self {
            Self::Russh(reader) => reader.wait().await.map(SshBackendMessage::from),
            Self::Libssh(reader) => reader.wait().await,
        }
    }

    pub(super) async fn wait_until_closed(
        &mut self,
        closed: &AtomicBool,
    ) -> Option<SshBackendMessage> {
        match self {
            Self::Russh(reader) => reader.wait().await.map(SshBackendMessage::from),
            Self::Libssh(reader) => reader.wait_until_closed(closed).await,
        }
    }
}

pub(super) enum SshBackendChannelWriter {
    Russh(ChannelWriteHalf<client::Msg>),
    Libssh(Arc<tokio::sync::Mutex<libssh_rs::Channel>>),
}

impl SshBackendChannelWriter {
    pub(super) async fn data(&self, data: &[u8]) -> Result<(), String> {
        match self {
            Self::Russh(writer) => writer
                .data(data)
                .await
                .map_err(|_| "SSH channel 已关闭".to_string()),
            Self::Libssh(channel) => {
                let channel = Arc::clone(channel);
                let data = data.to_vec();
                tokio::task::spawn_blocking(move || {
                    let channel = channel.blocking_lock();
                    let mut stdin = channel.stdin();
                    stdin.write_all(&data)?;
                    stdin.flush()
                })
                .await
                .map_err(|error| format!("libssh write worker failed: {error}"))?
                .map_err(|error| error.to_string())
            }
        }
    }

    pub(super) async fn window_change(&self, cols: u32, rows: u32) -> Result<(), String> {
        match self {
            Self::Russh(writer) => writer
                .window_change(cols, rows, 0, 0)
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(channel) => {
                let channel = Arc::clone(channel);
                tokio::task::spawn_blocking(move || {
                    channel.blocking_lock().change_pty_size(cols, rows)
                })
                .await
                .map_err(|error| format!("libssh resize worker failed: {error}"))?
                .map_err(|error| error.to_string())
            }
        }
    }

    pub(super) async fn eof(&self) -> Result<(), String> {
        match self {
            Self::Russh(writer) => writer.eof().await.map_err(|error| error.to_string()),
            Self::Libssh(channel) => {
                let channel = Arc::clone(channel);
                tokio::task::spawn_blocking(move || channel.blocking_lock().send_eof())
                    .await
                    .map_err(|error| format!("libssh EOF worker failed: {error}"))?
                    .map_err(|error| error.to_string())
            }
        }
    }
}

pub(super) struct LibsshChannelReader {
    channel: Arc<tokio::sync::Mutex<libssh_rs::Channel>>,
    pending: VecDeque<SshBackendMessage>,
    collect_exit_metadata: bool,
    completed: bool,
}

impl LibsshChannelReader {
    fn new(channel: libssh_rs::Channel, collect_exit_metadata: bool) -> Self {
        Self {
            channel: Arc::new(tokio::sync::Mutex::new(channel)),
            pending: VecDeque::new(),
            collect_exit_metadata,
            completed: false,
        }
    }

    async fn wait(&mut self) -> Option<SshBackendMessage> {
        self.wait_inner(None).await
    }

    async fn wait_until_closed(&mut self, closed: &AtomicBool) -> Option<SshBackendMessage> {
        self.wait_inner(Some(closed)).await
    }

    async fn wait_inner(&mut self, closed: Option<&AtomicBool>) -> Option<SshBackendMessage> {
        loop {
            if closed.is_some_and(|closed| closed.load(Ordering::SeqCst)) {
                return None;
            }
            if let Some(message) = self.pending.pop_front() {
                return Some(message);
            }
            if self.completed {
                return None;
            }

            let channel = Arc::clone(&self.channel);
            let collect_exit_metadata = self.collect_exit_metadata;
            let polled = tokio::task::spawn_blocking(move || {
                poll_libssh_channel(channel, collect_exit_metadata)
            })
            .await
            .map_err(|error| format!("libssh read worker failed: {error}"))
            .and_then(|result| result);
            match polled {
                Ok(LibsshChannelPoll::Data(data)) => return Some(SshBackendMessage::Data(data)),
                Ok(LibsshChannelPoll::ExtendedData(data)) => {
                    return Some(SshBackendMessage::ExtendedData { data, ext: 1 });
                }
                Ok(LibsshChannelPoll::Pending) => continue,
                Ok(LibsshChannelPoll::Finished {
                    exit_status,
                    exit_signal,
                    closed,
                }) => {
                    if let Some(signal) = exit_signal {
                        self.pending.push_back(SshBackendMessage::ExitSignal {
                            signal_name: signal
                                .signal_name
                                .unwrap_or_else(|| "unknown".to_string()),
                            error_message: signal.error_message.unwrap_or_default(),
                        });
                    }
                    if let Some(status) = exit_status.and_then(|status| u32::try_from(status).ok())
                    {
                        self.pending
                            .push_back(SshBackendMessage::ExitStatus(status));
                    }
                    self.pending.push_back(if closed {
                        SshBackendMessage::Close
                    } else {
                        SshBackendMessage::Eof
                    });
                    self.completed = true;
                }
                Err(error) => {
                    self.completed = true;
                    return Some(SshBackendMessage::Error(error));
                }
            }
        }
    }
}

enum LibsshChannelPoll {
    Data(Vec<u8>),
    ExtendedData(Vec<u8>),
    Pending,
    Finished {
        exit_status: Option<i32>,
        exit_signal: Option<libssh_rs::SignalState>,
        closed: bool,
    },
}

fn poll_libssh_channel(
    channel: Arc<tokio::sync::Mutex<libssh_rs::Channel>>,
    collect_exit_metadata: bool,
) -> Result<LibsshChannelPoll, String> {
    const POLL_TIMEOUT: Duration = Duration::from_millis(50);
    const MAX_READ_BYTES: usize = 64 * 1024;

    let channel = channel.blocking_lock();
    for (is_stderr, timeout) in [(false, Some(POLL_TIMEOUT)), (true, Some(Duration::ZERO))] {
        match channel
            .poll_timeout(is_stderr, timeout)
            .map_err(|error| error.to_string())?
        {
            libssh_rs::PollStatus::AvailableBytes(available) if available > 0 => {
                let mut data = vec![0_u8; (available as usize).min(MAX_READ_BYTES)];
                let read = channel
                    .read_timeout(&mut data, is_stderr, Some(POLL_TIMEOUT))
                    .map_err(|error| error.to_string())?;
                if read == 0 {
                    continue;
                }
                data.truncate(read);
                return Ok(if is_stderr {
                    LibsshChannelPoll::ExtendedData(data)
                } else {
                    LibsshChannelPoll::Data(data)
                });
            }
            libssh_rs::PollStatus::AvailableBytes(_) | libssh_rs::PollStatus::EndOfFile => {}
        }
    }

    let closed = channel.is_closed();
    if closed || channel.is_eof() {
        return Ok(LibsshChannelPoll::Finished {
            exit_status: collect_exit_metadata
                .then(|| channel.get_exit_status())
                .flatten(),
            exit_signal: collect_exit_metadata
                .then(|| channel.get_exit_signal())
                .flatten(),
            closed,
        });
    }
    Ok(LibsshChannelPoll::Pending)
}

async fn close_libssh_channel(
    channel: Arc<tokio::sync::Mutex<libssh_rs::Channel>>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || channel.blocking_lock().close())
        .await
        .map_err(|error| format!("libssh close worker failed: {error}"))?
        .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SshBackendMessage {
    Data(Vec<u8>),
    ExtendedData {
        data: Vec<u8>,
        ext: u32,
    },
    ExitStatus(u32),
    ExitSignal {
        signal_name: String,
        error_message: String,
    },
    Failure,
    Error(String),
    Eof,
    Close,
    Other,
}

impl From<ChannelMsg> for SshBackendMessage {
    fn from(message: ChannelMsg) -> Self {
        match message {
            ChannelMsg::Data { data } => Self::Data(data.to_vec()),
            ChannelMsg::ExtendedData { data, ext } => Self::ExtendedData {
                data: data.to_vec(),
                ext,
            },
            ChannelMsg::ExitStatus { exit_status } => Self::ExitStatus(exit_status),
            ChannelMsg::ExitSignal {
                signal_name,
                error_message,
                ..
            } => Self::ExitSignal {
                signal_name: format!("{signal_name:?}"),
                error_message,
            },
            ChannelMsg::Failure => Self::Failure,
            ChannelMsg::Eof => Self::Eof,
            ChannelMsg::Close => Self::Close,
            _ => Self::Other,
        }
    }
}

pub(super) trait SshExecChannelOpener {
    async fn open_exec_channel(
        &self,
        command: &str,
        timeout: Duration,
        label: &str,
    ) -> Result<SshBackendChannel, String>;
}

impl<H> SshExecChannelOpener for Arc<tokio::sync::Mutex<SshBackendSession<H>>>
where
    H: client::Handler,
{
    async fn open_exec_channel(
        &self,
        command: &str,
        timeout: Duration,
        label: &str,
    ) -> Result<SshBackendChannel, String> {
        open_shared_ssh_exec_channel(self, command, timeout, label).await
    }
}

impl<H> SshExecChannelOpener for Arc<tokio::sync::Mutex<client::Handle<H>>>
where
    H: client::Handler,
{
    async fn open_exec_channel(
        &self,
        command: &str,
        timeout: Duration,
        label: &str,
    ) -> Result<SshBackendChannel, String> {
        open_shared_russh_exec_channel(self, command, timeout, label)
            .await
            .map(SshBackendChannel::from_russh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn russh_messages_are_normalized_without_transport_types() {
        assert_eq!(
            SshBackendMessage::from(ChannelMsg::Data {
                data: b"hello".as_slice().into(),
            }),
            SshBackendMessage::Data(b"hello".to_vec())
        );
        assert_eq!(
            SshBackendMessage::from(ChannelMsg::ExitStatus { exit_status: 23 }),
            SshBackendMessage::ExitStatus(23)
        );
        assert_eq!(
            SshBackendMessage::from(ChannelMsg::Eof),
            SshBackendMessage::Eof
        );
    }
}

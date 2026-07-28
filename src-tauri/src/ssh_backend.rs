use super::*;

pub(super) enum SshBackendSession<H = PortMateSshHandler>
where
    H: client::Handler,
{
    Russh(client::Handle<H>),
    Libssh(libssh_rs::Session),
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

    pub(super) async fn open_russh_exec_compat(
        &self,
        command: &str,
        label: &str,
    ) -> Result<Channel<client::Msg>, String> {
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
                Ok(channel)
            }
            Self::Libssh(_) => Err(format!("{label} 尚未迁移到 libssh backend")),
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
        Self::Libssh(LibsshChannelReader::new(channel))
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
}

pub(super) struct LibsshChannelReader {
    channel: Arc<tokio::sync::Mutex<libssh_rs::Channel>>,
    pending: VecDeque<SshBackendMessage>,
    completed: bool,
}

impl LibsshChannelReader {
    fn new(channel: libssh_rs::Channel) -> Self {
        Self {
            channel: Arc::new(tokio::sync::Mutex::new(channel)),
            pending: VecDeque::new(),
            completed: false,
        }
    }

    async fn wait(&mut self) -> Option<SshBackendMessage> {
        loop {
            if let Some(message) = self.pending.pop_front() {
                return Some(message);
            }
            if self.completed {
                return None;
            }

            let channel = Arc::clone(&self.channel);
            let polled = tokio::task::spawn_blocking(move || poll_libssh_channel(channel))
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
            exit_status: channel.get_exit_status(),
            exit_signal: channel.get_exit_signal(),
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

pub(super) trait RusshExecChannelOpener {
    async fn open_russh_exec_channel(
        &self,
        command: &str,
        timeout: Duration,
        label: &str,
    ) -> Result<Channel<client::Msg>, String>;
}

impl<H> RusshExecChannelOpener for Arc<tokio::sync::Mutex<SshBackendSession<H>>>
where
    H: client::Handler,
{
    async fn open_russh_exec_channel(
        &self,
        command: &str,
        timeout: Duration,
        label: &str,
    ) -> Result<Channel<client::Msg>, String> {
        open_shared_russh_compat_exec_channel(self, command, timeout, label).await
    }
}

impl<H> RusshExecChannelOpener for Arc<tokio::sync::Mutex<client::Handle<H>>>
where
    H: client::Handler,
{
    async fn open_russh_exec_channel(
        &self,
        command: &str,
        timeout: Duration,
        label: &str,
    ) -> Result<Channel<client::Msg>, String> {
        open_shared_russh_exec_channel(self, command, timeout, label).await
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

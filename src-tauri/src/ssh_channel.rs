use super::*;

pub(super) const SSH_TERMINAL_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

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
                let writer = SshBackendChannelWriter::Libssh(reader.shared_channel());
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
        self.data_with_timeout(data, SSH_RUNTIME_OPERATION_TIMEOUT)
            .await
    }

    pub(super) async fn data_with_timeout(
        &self,
        data: &[u8],
        timeout: Duration,
    ) -> Result<(), String> {
        match self {
            Self::Russh(channel) => tokio::time::timeout(timeout, channel.data(data))
                .await
                .map_err(|_| format!("SSH write timed out after {} ms", timeout.as_millis()))?
                .map_err(|error| error.to_string()),
            Self::Libssh(reader) => {
                let channel = reader.shared_channel();
                let data = data.to_vec();
                run_libssh_channel_operation_with_timeout(
                    channel,
                    timeout,
                    "libssh write",
                    move |channel| {
                        let mut stdin = channel.stdin();
                        stdin.write_all(&data).map_err(|error| error.to_string())?;
                        stdin.flush().map_err(|error| error.to_string())
                    },
                )
                .await
            }
        }
    }

    pub(super) async fn eof(&self) -> Result<(), String> {
        self.eof_with_timeout(SSH_RUNTIME_OPERATION_TIMEOUT).await
    }

    pub(super) async fn eof_with_timeout(&self, timeout: Duration) -> Result<(), String> {
        match self {
            Self::Russh(channel) => tokio::time::timeout(timeout, channel.eof())
                .await
                .map_err(|_| format!("SSH EOF timed out after {} ms", timeout.as_millis()))?
                .map_err(|error| error.to_string()),
            Self::Libssh(reader) => {
                run_libssh_channel_operation_with_timeout(
                    reader.shared_channel(),
                    timeout,
                    "libssh EOF",
                    |channel| channel.send_eof().map_err(|error| error.to_string()),
                )
                .await
            }
        }
    }

    pub(super) async fn close_with_timeout(&self, timeout: Duration) -> Result<(), String> {
        match self {
            Self::Russh(channel) => tokio::time::timeout(timeout, channel.close())
                .await
                .map_err(|_| format!("SSH close timed out after {} ms", timeout.as_millis()))?
                .map_err(|error| error.to_string()),
            Self::Libssh(reader) => reader.close_with_timeout(timeout).await,
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

pub(super) async fn write_ssh_channel_bytes_with_timeout(
    writer: &Arc<tokio::sync::Mutex<SshBackendChannelWriter>>,
    data: &[u8],
    timeout: Duration,
    label: &str,
) -> Result<(), String> {
    let started = Instant::now();
    let writer = tokio::time::timeout(timeout, Arc::clone(writer).lock_owned())
        .await
        .map_err(|_| format!("{label} writer lock 超时（{} ms）", timeout.as_millis()))?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{label}超时（{} ms）", timeout.as_millis()))?;
    writer
        .data_with_timeout(data, remaining)
        .await
        .map_err(|error| format!("{label}失败: {error}"))
}

pub(super) async fn resize_ssh_channel_with_timeout(
    writer: &Arc<tokio::sync::Mutex<SshBackendChannelWriter>>,
    cols: u32,
    rows: u32,
    timeout: Duration,
    label: &str,
) -> Result<(), String> {
    let started = Instant::now();
    let writer = tokio::time::timeout(timeout, Arc::clone(writer).lock_owned())
        .await
        .map_err(|_| format!("{label} writer lock 超时（{} ms）", timeout.as_millis()))?;
    let remaining = timeout
        .checked_sub(started.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{label}超时（{} ms）", timeout.as_millis()))?;
    writer
        .window_change_with_timeout(cols, rows, remaining)
        .await
        .map_err(|error| format!("{label}失败: {error}"))
}

impl SshBackendChannelWriter {
    pub(super) async fn data(&self, data: &[u8]) -> Result<(), String> {
        self.data_with_timeout(data, SSH_RUNTIME_OPERATION_TIMEOUT)
            .await
    }

    pub(super) async fn data_with_timeout(
        &self,
        data: &[u8],
        timeout: Duration,
    ) -> Result<(), String> {
        match self {
            Self::Russh(writer) => tokio::time::timeout(timeout, writer.data(data))
                .await
                .map_err(|_| format!("SSH write timed out after {} ms", timeout.as_millis()))?
                .map_err(|_| "SSH channel 已关闭".to_string()),
            Self::Libssh(channel) => {
                let channel = Arc::clone(channel);
                let data = data.to_vec();
                run_libssh_channel_operation_with_timeout(
                    channel,
                    timeout,
                    "libssh write",
                    move |channel| {
                        let mut stdin = channel.stdin();
                        stdin.write_all(&data).map_err(|error| error.to_string())?;
                        stdin.flush().map_err(|error| error.to_string())
                    },
                )
                .await
            }
        }
    }

    pub(super) async fn window_change_with_timeout(
        &self,
        cols: u32,
        rows: u32,
        timeout: Duration,
    ) -> Result<(), String> {
        match self {
            Self::Russh(writer) => {
                tokio::time::timeout(timeout, writer.window_change(cols, rows, 0, 0))
                .await
                    .map_err(|_| {
                        format!("SSH resize timed out after {} ms", timeout.as_millis())
                    })?
                    .map_err(|error| error.to_string())
            }
            Self::Libssh(channel) => {
                let channel = Arc::clone(channel);
                run_libssh_channel_operation_with_timeout(
                    channel,
                    timeout,
                    "libssh resize",
                    move |channel| {
                        channel
                            .change_pty_size(cols, rows)
                            .map_err(|error| error.to_string())
                    },
                )
                .await
            }
        }
    }

    pub(super) async fn eof(&self) -> Result<(), String> {
        self.eof_with_timeout(SSH_RUNTIME_OPERATION_TIMEOUT).await
    }

    pub(super) async fn eof_with_timeout(&self, timeout: Duration) -> Result<(), String> {
        match self {
            Self::Russh(writer) => tokio::time::timeout(timeout, writer.eof())
                .await
                .map_err(|_| format!("SSH EOF timed out after {} ms", timeout.as_millis()))?
                .map_err(|error| error.to_string()),
            Self::Libssh(channel) => {
                let channel = Arc::clone(channel);
                run_libssh_channel_operation_with_timeout(
                    channel,
                    timeout,
                    "libssh EOF",
                    |channel| channel.send_eof().map_err(|error| error.to_string()),
                )
                .await
            }
        }
    }
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

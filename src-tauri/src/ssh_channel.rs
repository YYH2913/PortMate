use super::*;

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
        match self {
            Self::Russh(channel) => channel.data(data).await.map_err(|error| error.to_string()),
            Self::Libssh(reader) => {
                let channel = reader.shared_channel();
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
                let channel = reader.shared_channel();
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
            Self::Libssh(reader) => reader.close().await,
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

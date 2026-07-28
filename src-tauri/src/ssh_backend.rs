use super::*;

pub(super) enum SshBackendSession<H = PortMateSshHandler>
where
    H: client::Handler,
{
    Russh(client::Handle<H>),
}

impl<H> SshBackendSession<H>
where
    H: client::Handler,
{
    pub(super) fn from_russh(handle: client::Handle<H>) -> Self {
        Self::Russh(handle)
    }

    pub(super) fn russh(&self) -> &client::Handle<H> {
        match self {
            Self::Russh(handle) => handle,
        }
    }

    pub(super) async fn disconnect(&self, description: &str) -> Result<(), String> {
        match self {
            Self::Russh(handle) => handle
                .disconnect(Disconnect::ByApplication, description, "en")
                .await
                .map_err(|error| error.to_string()),
        }
    }

    pub(super) async fn send_ping(&self) -> Result<(), String> {
        match self {
            Self::Russh(handle) => handle.send_ping().await.map_err(|error| error.to_string()),
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
        }
    }
}

#[derive(Debug)]
pub(super) enum SshBackendChannel {
    Russh(Channel<client::Msg>),
}

impl SshBackendChannel {
    pub(super) fn from_russh(channel: Channel<client::Msg>) -> Self {
        Self::Russh(channel)
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
        }
    }

    pub(super) async fn wait(&mut self) -> Option<SshBackendMessage> {
        match self {
            Self::Russh(channel) => channel.wait().await.map(SshBackendMessage::from),
        }
    }

    pub(super) async fn close(&self) -> Result<(), String> {
        match self {
            Self::Russh(channel) => channel.close().await.map_err(|error| error.to_string()),
        }
    }
}

pub(super) enum SshBackendChannelReader {
    Russh(ChannelReadHalf),
}

impl SshBackendChannelReader {
    pub(super) async fn wait(&mut self) -> Option<SshBackendMessage> {
        match self {
            Self::Russh(reader) => reader.wait().await.map(SshBackendMessage::from),
        }
    }
}

pub(super) enum SshBackendChannelWriter {
    Russh(ChannelWriteHalf<client::Msg>),
}

impl SshBackendChannelWriter {
    pub(super) async fn data(&self, data: &[u8]) -> Result<(), String> {
        match self {
            Self::Russh(writer) => writer
                .data(data)
                .await
                .map_err(|_| "SSH channel 已关闭".to_string()),
        }
    }

    pub(super) async fn window_change(&self, cols: u32, rows: u32) -> Result<(), String> {
        match self {
            Self::Russh(writer) => writer
                .window_change(cols, rows, 0, 0)
                .await
                .map_err(|error| error.to_string()),
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

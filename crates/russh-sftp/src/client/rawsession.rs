use bytes::Bytes;
use dashmap::DashMap as HashMap;
use std::{
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, watch},
    time,
};

use super::{error::Error, Handler};
use crate::{
    client::{run_with_error, Config},
    de,
    error::Error as StreamError,
    extensions::{
        self, FsyncExtension, HardlinkExtension, LimitsExtension, Statvfs, StatvfsExtension,
    },
    protocol::{
        Attrs, Close, Data, Extended, ExtendedReply, FSetStat, FileAttributes, Fstat, Handle, Init,
        Lstat, MkDir, Name, Open, OpenDir, OpenFlags, Packet, Read, ReadDir, ReadLink, RealPath,
        Remove, Rename, RmDir, SetStat, Stat, Status, StatusCode, Symlink, Version, Write,
    },
};

pub type SftpResult<T> = Result<T, Error>;
type SharedRequests = HashMap<Option<u32>, oneshot::Sender<SftpResult<Packet>>>;

pub(crate) struct SessionInner {
    version: Option<u32>,
    requests: Arc<SharedRequests>,
}

impl SessionInner {
    pub fn reply(&mut self, id: Option<u32>, packet: Packet) -> SftpResult<()> {
        if let Some((_, sender)) = self.requests.remove(&id) {
            let validate = if id.is_some() && self.version.is_none() {
                Err(Error::UnexpectedPacket)
            } else if id.is_none() && self.version.is_some() {
                Err(Error::UnexpectedBehavior("Duplicate version".to_owned()))
            } else {
                Ok(())
            };

            // Ignore send error: receiver was dropped (request timed out).
            let _ = sender.send(validate.clone().map(|_| packet));

            return validate;
        }

        Err(Error::UnexpectedBehavior(format!(
            "Packet {:?} for unknown recipient",
            id
        )))
    }
}

impl Handler for SessionInner {
    type Error = Error;

    async fn version(&mut self, packet: Version) -> Result<(), Self::Error> {
        let version = packet.version;
        self.reply(None, packet.into())?;
        self.version = Some(version);
        Ok(())
    }

    async fn name(&mut self, name: Name) -> Result<(), Self::Error> {
        self.reply(Some(name.id), name.into())
    }

    async fn status(&mut self, status: Status) -> Result<(), Self::Error> {
        self.reply(Some(status.id), status.into())
    }

    async fn handle(&mut self, handle: Handle) -> Result<(), Self::Error> {
        self.reply(Some(handle.id), handle.into())
    }

    async fn data(&mut self, data: Data) -> Result<(), Self::Error> {
        self.reply(Some(data.id), data.into())
    }

    async fn attrs(&mut self, attrs: Attrs) -> Result<(), Self::Error> {
        self.reply(Some(attrs.id), attrs.into())
    }

    async fn extended_reply(&mut self, reply: ExtendedReply) -> Result<(), Self::Error> {
        self.reply(Some(reply.id), reply.into())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Limits {
    pub packet_len: Option<u64>,
    pub read_len: Option<u64>,
    pub write_len: Option<u64>,
    pub open_handles: Option<u64>,
}

impl From<LimitsExtension> for Limits {
    fn from(limits: LimitsExtension) -> Self {
        Self {
            packet_len: (limits.max_packet_len > 0).then_some(limits.max_packet_len),
            read_len: (limits.max_read_len > 0).then_some(limits.max_read_len),
            write_len: (limits.max_write_len > 0).then_some(limits.max_write_len),
            open_handles: (limits.max_open_handles > 0).then_some(limits.max_open_handles),
        }
    }
}

/// Implements raw work with the protocol in request-response format.
/// If the server returns a `Status` packet and it has the code Ok
/// then the packet is returned as Ok in other error cases
/// the packet is stored as Err.
pub struct RawSftpSession {
    tx: mpsc::UnboundedSender<Bytes>,
    stream_error: watch::Receiver<Option<StreamError>>,
    requests: Arc<SharedRequests>,
    next_req_id: AtomicU32,
    handles: AtomicU64,
    timeout: AtomicU64,
    limits: Limits,
}

macro_rules! into_with_status {
    ($result:ident, $packet:ident) => {
        match $result {
            Packet::$packet(p) => Ok(p),
            Packet::Status(p) => Err(p.into()),
            _ => Err(Error::UnexpectedPacket),
        }
    };
}

macro_rules! into_status {
    ($result:ident) => {
        match $result {
            Packet::Status(status) if status.status_code == StatusCode::Ok => Ok(status),
            Packet::Status(status) => Err(status.into()),
            _ => Err(Error::UnexpectedPacket),
        }
    };
}

impl RawSftpSession {
    pub fn new<S>(stream: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::new_with_config(stream, Config::default())
    }

    pub fn new_with_config<S>(stream: S, cfg: Config) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let req_map = Arc::new(HashMap::new());
        let inner = SessionInner {
            version: None,
            requests: req_map.clone(),
        };

        let (tx, stream_error) = run_with_error(stream, inner, cfg.max_packet_len);
        Self {
            tx,
            stream_error,
            requests: req_map,
            next_req_id: AtomicU32::new(1),
            handles: AtomicU64::new(0),
            timeout: AtomicU64::new(cfg.request_timeout_secs),
            limits: Limits::default(),
        }
    }

    /// Set the maximum response time in seconds.
    /// Default: 10 seconds
    pub fn set_timeout(&self, secs: u64) {
        self.timeout.store(secs, Ordering::Relaxed);
    }

    /// Setting limits. For the `limits@openssh.com` extension
    pub fn set_limits(&mut self, limits: Limits) {
        self.limits = limits;
    }

    fn send(
        &self,
        id: Option<u32>,
        packet: Packet,
    ) -> SftpResult<oneshot::Receiver<SftpResult<Packet>>> {
        if self.tx.is_closed() {
            return Err(Error::UnexpectedBehavior("session closed".into()));
        }

        let bytes = Bytes::try_from(packet)?;

        if let Some(max_len) = self.limits.packet_len {
            if bytes.len() as u64 > max_len {
                return Err(Error::Limited("packet exceeds server limit".to_owned()));
            }
        }

        let (tx, rx) = oneshot::channel();
        self.requests.insert(id, tx);
        self.tx.send(bytes)?;

        Ok(rx)
    }

    async fn request(&self, id: Option<u32>, packet: Packet) -> SftpResult<Packet> {
        let rx = self.send(id, packet)?;
        let timeout = self.timeout.load(Ordering::Relaxed);
        let mut stream_error = self.stream_error.clone();
        if let Some(error) = stream_error.borrow().clone() {
            self.requests.remove(&id);
            return Err(error.into());
        }

        tokio::select! {
            biased;
            result = rx => match result {
                Ok(result) => result,
                Err(_) => Err(Error::UnexpectedBehavior("sender dropped".into())),
            },
            changed = stream_error.changed() => {
                self.requests.remove(&id);
                match changed {
                    Ok(()) => {
                        let error = stream_error.borrow().clone().ok_or_else(|| {
                            Error::UnexpectedBehavior("SFTP stream closed without an error".into())
                        })?;
                        Err(error.into())
                    }
                    Err(_) => Err(Error::UnexpectedBehavior("SFTP stream error sender dropped".into())),
                }
            },
            _ = time::sleep(Duration::from_secs(timeout)) => {
                self.requests.remove(&id);
                Err(Error::Timeout)
            }
        }
    }

    fn use_next_id(&self) -> u32 {
        self.next_req_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Closes the inner channel stream. Called by [`Drop`]
    pub fn close_session(&self) -> SftpResult<()> {
        if self.tx.is_closed() {
            return Ok(());
        }

        Ok(self.tx.send(Bytes::new())?)
    }

    pub async fn init(&self) -> SftpResult<Version> {
        let result = self.request(None, Init::default().into()).await?;
        if let Packet::Version(version) = result {
            Ok(version)
        } else {
            Err(Error::UnexpectedPacket)
        }
    }

    pub async fn open<T: Into<String>>(
        &self,
        filename: T,
        flags: OpenFlags,
        attrs: FileAttributes,
    ) -> SftpResult<Handle> {
        if self
            .limits
            .open_handles
            .is_some_and(|h| self.handles.load(Ordering::SeqCst) >= h)
        {
            return Err(Error::Limited("handle limit reached".to_owned()));
        }

        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Open {
                    id,
                    filename: filename.into(),
                    pflags: flags,
                    attrs,
                }
                .into(),
            )
            .await?;

        if let Packet::Handle(_) = result {
            self.handles.fetch_add(1, Ordering::SeqCst);
        }

        into_with_status!(result, Handle)
    }

    pub async fn close<H: Into<String>>(&self, handle: H) -> SftpResult<Status> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Close {
                    id,
                    handle: handle.into(),
                }
                .into(),
            )
            .await?;

        if let Packet::Status(status) = &result {
            if status.status_code == StatusCode::Ok
                && self
                    .handles
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |h| {
                        if h > 0 {
                            Some(h - 1)
                        } else {
                            None
                        }
                    })
                    .is_err()
            {
                warn!("attempt to close more handles than exist");
            }
        }

        into_status!(result)
    }

    pub async fn read<H: Into<String>>(
        &self,
        handle: H,
        offset: u64,
        len: u32,
    ) -> SftpResult<Data> {
        if self.limits.read_len.is_some_and(|r| len as u64 > r) {
            return Err(Error::Limited("read limit reached".to_owned()));
        }

        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Read {
                    id,
                    handle: handle.into(),
                    offset,
                    len,
                }
                .into(),
            )
            .await?;

        into_with_status!(result, Data)
    }

    pub async fn write<H: Into<String>>(
        &self,
        handle: H,
        offset: u64,
        data: Vec<u8>,
    ) -> SftpResult<Status> {
        if self.limits.write_len.is_some_and(|w| data.len() as u64 > w) {
            return Err(Error::Limited("write limit reached".to_owned()));
        }

        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Write {
                    id,
                    handle: handle.into(),
                    offset,
                    data,
                }
                .into(),
            )
            .await?;

        into_status!(result)
    }

    /// Sends a write packet without awaiting the server's acknowledgement.
    pub(crate) fn write_nowait(
        &self,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> SftpResult<oneshot::Receiver<SftpResult<Packet>>> {
        if self.limits.write_len.is_some_and(|w| data.len() as u64 > w) {
            return Err(Error::Limited("write limit reached".to_owned()));
        }

        let id = self.use_next_id();
        self.send(
            Some(id),
            Write {
                id,
                handle,
                offset,
                data,
            }
            .into(),
        )
    }

    pub async fn lstat<P: Into<String>>(&self, path: P) -> SftpResult<Attrs> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Lstat {
                    id,
                    path: path.into(),
                }
                .into(),
            )
            .await?;

        into_with_status!(result, Attrs)
    }

    pub async fn fstat<H: Into<String>>(&self, handle: H) -> SftpResult<Attrs> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Fstat {
                    id,
                    handle: handle.into(),
                }
                .into(),
            )
            .await?;

        into_with_status!(result, Attrs)
    }

    pub async fn setstat<P: Into<String>>(
        &self,
        path: P,
        attrs: FileAttributes,
    ) -> SftpResult<Status> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                SetStat {
                    id,
                    path: path.into(),
                    attrs,
                }
                .into(),
            )
            .await?;

        into_status!(result)
    }

    pub async fn fsetstat<H: Into<String>>(
        &self,
        handle: H,
        attrs: FileAttributes,
    ) -> SftpResult<Status> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                FSetStat {
                    id,
                    handle: handle.into(),
                    attrs,
                }
                .into(),
            )
            .await?;

        into_status!(result)
    }

    pub async fn opendir<P: Into<String>>(&self, path: P) -> SftpResult<Handle> {
        if self
            .limits
            .open_handles
            .is_some_and(|h| self.handles.load(Ordering::SeqCst) >= h)
        {
            return Err(Error::Limited("Handle limit reached".to_owned()));
        }

        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                OpenDir {
                    id,
                    path: path.into(),
                }
                .into(),
            )
            .await?;

        if let Packet::Handle(_) = result {
            self.handles.fetch_add(1, Ordering::SeqCst);
        }

        into_with_status!(result, Handle)
    }

    pub async fn readdir<H: Into<String>>(&self, handle: H) -> SftpResult<Name> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                ReadDir {
                    id,
                    handle: handle.into(),
                }
                .into(),
            )
            .await?;

        into_with_status!(result, Name)
    }

    pub async fn remove<T: Into<String>>(&self, filename: T) -> SftpResult<Status> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Remove {
                    id,
                    filename: filename.into(),
                }
                .into(),
            )
            .await?;

        into_status!(result)
    }

    pub async fn mkdir<P: Into<String>>(
        &self,
        path: P,
        attrs: FileAttributes,
    ) -> SftpResult<Status> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                MkDir {
                    id,
                    path: path.into(),
                    attrs,
                }
                .into(),
            )
            .await?;

        into_status!(result)
    }

    pub async fn rmdir<P: Into<String>>(&self, path: P) -> SftpResult<Status> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                RmDir {
                    id,
                    path: path.into(),
                }
                .into(),
            )
            .await?;

        into_status!(result)
    }

    pub async fn realpath<P: Into<String>>(&self, path: P) -> SftpResult<Name> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                RealPath {
                    id,
                    path: path.into(),
                }
                .into(),
            )
            .await?;

        into_with_status!(result, Name)
    }

    pub async fn stat<P: Into<String>>(&self, path: P) -> SftpResult<Attrs> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Stat {
                    id,
                    path: path.into(),
                }
                .into(),
            )
            .await?;

        into_with_status!(result, Attrs)
    }

    pub async fn rename<O, N>(&self, oldpath: O, newpath: N) -> SftpResult<Status>
    where
        O: Into<String>,
        N: Into<String>,
    {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Rename {
                    id,
                    oldpath: oldpath.into(),
                    newpath: newpath.into(),
                }
                .into(),
            )
            .await?;

        into_status!(result)
    }

    pub async fn readlink<P: Into<String>>(&self, path: P) -> SftpResult<Name> {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                ReadLink {
                    id,
                    path: path.into(),
                }
                .into(),
            )
            .await?;

        into_with_status!(result, Name)
    }

    pub async fn symlink<P, T>(&self, path: P, target: T) -> SftpResult<Status>
    where
        P: Into<String>,
        T: Into<String>,
    {
        let id = self.use_next_id();
        let result = self
            .request(
                Some(id),
                Symlink {
                    id,
                    linkpath: path.into(),
                    targetpath: target.into(),
                }
                .into(),
            )
            .await?;

        into_status!(result)
    }

    /// Equivalent to `SSH_FXP_EXTENDED`. Allows protocol expansion.
    /// The extension can return any packet, so it's not specific
    pub async fn extended<R: Into<String>>(&self, request: R, data: Vec<u8>) -> SftpResult<Packet> {
        let id = self.use_next_id();
        self.request(
            Some(id),
            Extended {
                id,
                request: request.into(),
                data,
            }
            .into(),
        )
        .await
    }

    pub async fn limits(&self) -> SftpResult<LimitsExtension> {
        match self.extended(extensions::LIMITS, vec![]).await? {
            Packet::ExtendedReply(reply) => {
                Ok(de::from_bytes::<LimitsExtension>(&mut reply.data.into())?)
            }
            Packet::Status(status) if status.status_code != StatusCode::Ok => {
                Err(Error::Status(status))
            }
            _ => Err(Error::UnexpectedPacket),
        }
    }

    pub async fn hardlink<O, N>(&self, oldpath: O, newpath: N) -> SftpResult<Status>
    where
        O: Into<String>,
        N: Into<String>,
    {
        let result = self
            .extended(
                extensions::HARDLINK,
                HardlinkExtension {
                    oldpath: oldpath.into(),
                    newpath: newpath.into(),
                }
                .try_into()?,
            )
            .await?;

        into_status!(result)
    }

    pub async fn fsync<H: Into<String>>(&self, handle: H) -> SftpResult<Status> {
        let result = self
            .extended(
                extensions::FSYNC,
                FsyncExtension {
                    handle: handle.into(),
                }
                .try_into()?,
            )
            .await?;

        into_status!(result)
    }

    pub async fn statvfs<P>(&self, path: P) -> SftpResult<Statvfs>
    where
        P: Into<String>,
    {
        let result = self
            .extended(
                extensions::STATVFS,
                StatvfsExtension { path: path.into() }.try_into()?,
            )
            .await?;

        match result {
            Packet::ExtendedReply(reply) => Ok(de::from_bytes::<Statvfs>(&mut reply.data.into())?),
            Packet::Status(status) if status.status_code != StatusCode::Ok => {
                Err(Error::Status(status))
            }
            _ => Err(Error::UnexpectedPacket),
        }
    }
}

impl Drop for RawSftpSession {
    fn drop(&mut self) {
        let _ = self.close_session();
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use tokio::{
        io::{duplex, AsyncWriteExt, DuplexStream},
        sync::oneshot,
    };

    use super::*;
    use crate::{protocol::Version, utils::read_packet};

    async fn read_client_packet(stream: &mut DuplexStream) -> Packet {
        let mut encoded = read_packet(stream, u32::MAX).await.unwrap();
        Packet::try_from(&mut encoded).unwrap()
    }

    async fn write_server_packet(stream: &mut DuplexStream, packet: Packet) {
        let encoded = Bytes::try_from(packet).unwrap();
        stream.write_all(&encoded).await.unwrap();
    }

    async fn initialize_server(stream: &mut DuplexStream) {
        assert!(matches!(read_client_packet(stream).await, Packet::Init(_)));
        write_server_packet(stream, Version::default().into()).await;
    }

    fn long_timeout_config() -> Config {
        Config {
            request_timeout_secs: 30,
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn malformed_response_fails_pending_request_without_waiting_for_timeout() {
        let (client, mut server) = duplex(4096);
        let session = RawSftpSession::new_with_config(client, long_timeout_config());
        let (release_tx, release_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            initialize_server(&mut server).await;
            assert!(matches!(
                read_client_packet(&mut server).await,
                Packet::Lstat(_)
            ));
            server.write_all(&[0, 0, 0, 1, 0xff]).await.unwrap();
            let _ = release_rx.await;
        });

        session.init().await.unwrap();
        let error = time::timeout(Duration::from_secs(1), session.lstat("/malformed"))
            .await
            .expect("malformed response waited for the request timeout")
            .unwrap_err();
        assert!(error.to_string().contains("unknown type"), "{error}");

        let _ = release_tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn zero_length_response_fails_pending_request_without_waiting_for_timeout() {
        let (client, mut server) = duplex(4096);
        let session = RawSftpSession::new_with_config(client, long_timeout_config());
        let (release_tx, release_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            initialize_server(&mut server).await;
            assert!(matches!(
                read_client_packet(&mut server).await,
                Packet::Lstat(_)
            ));
            server.write_all(&0_u32.to_be_bytes()).await.unwrap();
            let _ = release_rx.await;
        });

        session.init().await.unwrap();
        let error = time::timeout(Duration::from_secs(1), session.lstat("/zero-length"))
            .await
            .expect("zero-length response waited for the request timeout")
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only 0 bytes remaining, but 1 requested"),
            "{error}"
        );

        let _ = release_tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_status_payload_fails_pending_request_without_waiting_for_timeout() {
        let (client, mut server) = duplex(4096);
        let session = RawSftpSession::new_with_config(client, long_timeout_config());
        let (release_tx, release_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            initialize_server(&mut server).await;
            let request_id = read_client_packet(&mut server).await.get_request_id();
            server.write_all(&5_u32.to_be_bytes()).await.unwrap();
            server.write_all(&[101]).await.unwrap();
            server.write_all(&request_id.to_be_bytes()).await.unwrap();
            let _ = release_rx.await;
        });

        session.init().await.unwrap();
        let error = time::timeout(Duration::from_secs(1), session.lstat("/malformed-status"))
            .await
            .expect("malformed status response waited for the request timeout")
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only 0 bytes remaining, but 4 requested"),
            "{error}"
        );

        let _ = release_tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn truncated_response_fails_pending_request_without_waiting_for_timeout() {
        let (client, mut server) = duplex(4096);
        let session = RawSftpSession::new_with_config(client, long_timeout_config());
        let server_task = tokio::spawn(async move {
            initialize_server(&mut server).await;
            assert!(matches!(
                read_client_packet(&mut server).await,
                Packet::Lstat(_)
            ));
            server.write_all(&[0, 0, 0, 9, 101]).await.unwrap();
        });

        session.init().await.unwrap();
        let error = time::timeout(Duration::from_secs(1), session.lstat("/truncated"))
            .await
            .expect("truncated response waited for the request timeout")
            .unwrap_err();
        assert!(
            error.to_string().contains("Unexpected EOF on stream"),
            "{error}"
        );

        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn oversized_response_fails_before_reading_the_declared_payload() {
        let (client, mut server) = duplex(4096);
        let session = RawSftpSession::new_with_config(
            client,
            Config {
                max_packet_len: 64,
                ..long_timeout_config()
            },
        );
        let (release_tx, release_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            initialize_server(&mut server).await;
            assert!(matches!(
                read_client_packet(&mut server).await,
                Packet::Lstat(_)
            ));
            server.write_all(&65_u32.to_be_bytes()).await.unwrap();
            let _ = release_rx.await;
        });

        session.init().await.unwrap();
        let error = time::timeout(Duration::from_secs(1), session.lstat("/oversized"))
            .await
            .expect("oversized response waited for its declared payload")
            .unwrap_err();
        assert!(
            error.to_string().contains("packet length limit exceeded"),
            "{error}"
        );

        let _ = release_tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn unknown_response_id_fails_pending_request_without_waiting_for_timeout() {
        let (client, mut server) = duplex(4096);
        let session = RawSftpSession::new_with_config(client, long_timeout_config());
        let (release_tx, release_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            initialize_server(&mut server).await;
            let request_id = read_client_packet(&mut server).await.get_request_id();
            write_server_packet(
                &mut server,
                Packet::status(request_id + 1, StatusCode::Failure, "wrong request id", ""),
            )
            .await;
            let _ = release_rx.await;
        });

        session.init().await.unwrap();
        let error = time::timeout(Duration::from_secs(1), session.lstat("/wrong-id"))
            .await
            .expect("unknown response id waited for the request timeout")
            .unwrap_err();
        assert!(error.to_string().contains("unknown recipient"), "{error}");

        let _ = release_tx.send(());
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn reverse_order_responses_are_routed_to_their_request_ids() {
        let (client, mut server) = duplex(4096);
        let session = RawSftpSession::new_with_config(client, long_timeout_config());
        let (release_tx, release_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            initialize_server(&mut server).await;
            let first_id = read_client_packet(&mut server).await.get_request_id();
            let second_id = read_client_packet(&mut server).await.get_request_id();
            write_server_packet(&mut server, Packet::error(second_id, StatusCode::Ok)).await;
            write_server_packet(&mut server, Packet::error(first_id, StatusCode::Ok)).await;
            let _ = release_rx.await;
        });

        session.init().await.unwrap();
        let first = session
            .write_nowait("handle".to_owned(), 0, vec![1])
            .unwrap();
        let second = session
            .write_nowait("handle".to_owned(), 1, vec![2])
            .unwrap();
        let second_packet = time::timeout(Duration::from_secs(1), second)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let first_packet = time::timeout(Duration::from_secs(1), first)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(first_packet, Packet::Status(status) if status.id == 1));
        assert!(matches!(second_packet, Packet::Status(status) if status.id == 2));

        let _ = release_tx.send(());
        server_task.await.unwrap();
    }
}

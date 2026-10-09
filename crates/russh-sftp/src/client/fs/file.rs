use std::{
    collections::VecDeque,
    future::{self, Future},
    io::{self, SeekFrom},
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncSeek, AsyncWrite, ReadBuf},
    runtime::Handle,
    sync::oneshot,
};

use super::Metadata;
use crate::{
    client::{error::Error, rawsession::SftpResult, session::Features, RawSftpSession},
    protocol::{Packet, StatusCode},
};

type StateFn<T> = Option<Pin<Box<dyn Future<Output = io::Result<T>> + Send + Sync + 'static>>>;
type WriteAck = Pin<Box<dyn Future<Output = io::Result<()>> + Send + Sync + 'static>>;

mod read_pipeline;

// read packet overhead: type(1) + id(4) + data_len(4)
const READ_OVERHEAD_LENGTH: u32 = 9;
// Include the SFTP length prefix so a DATA reply fits the common 32 KiB
// SSH channel packet without an extra 13-byte tail packet.
const PIPELINED_READ_PAYLOAD: u32 = 32 * 1024 - READ_OVERHEAD_LENGTH - 4;
// write packet overhead excluding handle: type(1) + id(4) + handle_len(4) + offset(8) + data_len(4)
const WRITE_OVERHEAD_LENGTH: u32 = 21;

struct FileState {
    f_read: StateFn<Option<Vec<u8>>>,
    f_seek: StateFn<u64>,
    f_flush: StateFn<()>,
    f_shutdown: StateFn<()>,
    write_acks: VecDeque<WriteAck>,
}

/// Provides high-level methods for interaction with a remote file.
///
/// In order to properly close the handle, [`shutdown`] on a file should be called.
/// Also implement [`AsyncSeek`] and other async i/o implementations.
///
/// # Weakness
/// Using [`SeekFrom::End`] is costly and time-consuming because we need to
/// request the actual file size from the remote server.
pub struct File {
    session: Arc<RawSftpSession>,
    handle: String,
    state: FileState,
    pos: u64,
    closed: bool,
    features: Features,
}

impl File {
    pub(crate) fn new(session: Arc<RawSftpSession>, handle: String, features: Features) -> Self {
        Self {
            session,
            handle,
            state: FileState {
                f_read: None,
                f_seek: None,
                f_flush: None,
                f_shutdown: None,
                write_acks: VecDeque::with_capacity(features.max_concurrent_writes),
            },
            pos: 0,
            closed: false,
            features,
        }
    }

    /// Queries metadata about the remote file.
    pub async fn metadata(&self) -> SftpResult<Metadata> {
        Ok(self.session.fstat(self.handle.as_str()).await?.attrs)
    }

    /// Sets metadata for a remote file.
    pub async fn set_metadata(&self, metadata: Metadata) -> SftpResult<()> {
        self.session
            .fsetstat(self.handle.as_str(), metadata)
            .await
            .map(|_| ())
    }

    /// Attempts to sync all data.
    ///
    /// If the server does not support `fsync@openssh.com` sending the request will
    /// be omitted, but will still pseudo-successfully
    pub async fn sync_all(&self) -> SftpResult<()> {
        if !self.features.fsync {
            return Ok(());
        }

        self.session.fsync(self.handle.as_str()).await.map(|_| ())
    }

    /// Read a bounded batch of ranges concurrently on this file's existing handle.
    ///
    /// At most 16 requests of 32 KiB are outstanding, each also bounded by the
    /// negotiated packet/read limits. Short DATA replies are filled before the
    /// next range is returned, and responses are assembled in offset order.
    /// Unlike speculative read-ahead, no bytes beyond this buffer are requested.
    /// One request timeout covers the whole batch, including short-read retries.
    ///
    /// A cancelled call leaves its batch attached to the file. Resume it with
    /// an equally large buffer, or close/drop the file to drain it before CLOSE;
    /// seeking and writing while the batch is pending are rejected.
    pub async fn read_pipelined(
        &mut self,
        buffer: &mut [u8],
        max_requests: usize,
    ) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.closed
            || self.state.f_seek.is_some()
            || self.state.f_shutdown.is_some()
            || self.state.f_flush.is_some()
            || !self.state.write_acks.is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file is closed or another operation is pending",
            ));
        }
        if self.state.f_read.is_none() {
            let max_read_len = u64::from(
                self.features
                    .max_packet_len
                    .saturating_sub(READ_OVERHEAD_LENGTH),
            )
            .min(
                self.features
                    .limits
                    .and_then(|limits| limits.read_len)
                    .unwrap_or(u64::MAX),
            );
            let request_len = max_read_len.min(u64::from(PIPELINED_READ_PAYLOAD)) as usize;
            if request_len == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "SFTP packet limit leaves no room for DATA",
                ));
            }
            let length = buffer.len().min(request_len * max_requests.clamp(1, 16));
            self.pos.checked_add(length as u64).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "SFTP read offset overflow")
            })?;
            let session = Arc::clone(&self.session);
            let handle = self.handle.clone();
            let offset = self.pos;
            self.state.f_read = Some(Box::pin(async move {
                read_pipeline::read_batch(session, handle, offset, length, request_len)
                    .await
                    .map(Some)
            }));
        }
        let result = future::poll_fn(|cx| {
            self.state
                .f_read
                .as_mut()
                .expect("pending batch read")
                .as_mut()
                .poll(cx)
        })
        .await;
        self.state.f_read = None;
        let data = result?.unwrap_or_default();
        if data.len() > buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer is smaller than the pending SFTP read",
            ));
        }
        buffer[..data.len()].copy_from_slice(&data);
        self.pos += data.len() as u64;
        Ok(data.len())
    }
}

fn check_write_result(
    result: Result<SftpResult<Packet>, oneshot::error::RecvError>,
) -> io::Result<()> {
    match result {
        Err(_) => Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "write channel closed",
        )),
        Ok(Ok(Packet::Status(s))) if s.status_code == StatusCode::Ok => Ok(()),
        Ok(Ok(Packet::Status(s))) => Err(io::Error::other(s.error_message)),
        Ok(Ok(_)) => Err(io::Error::other("unexpected response packet")),
        Ok(Err(Error::Timeout)) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "SFTP write timed out",
        )),
        Ok(Err(e)) => Err(io::Error::other(e.to_string())),
    }
}

fn poll_oldest_write(
    pending: &mut VecDeque<WriteAck>,
    cx: &mut Context<'_>,
) -> Option<Poll<io::Result<()>>> {
    let rx = pending.front_mut()?;
    Some(match rx.as_mut().poll(cx) {
        Poll::Pending => Poll::Pending,
        Poll::Ready(r) => {
            pending.pop_front();
            Poll::Ready(r)
        }
    })
}

fn poll_drain_writes(
    pending: &mut VecDeque<WriteAck>,
    cx: &mut Context<'_>,
) -> Poll<io::Result<()>> {
    while let Some(poll) = poll_oldest_write(pending, cx) {
        ready!(poll)?;
    }
    Poll::Ready(Ok(()))
}

impl Drop for File {
    fn drop(&mut self) {
        if self.closed {
            return;
        }

        if let Ok(handle) = Handle::try_current() {
            let session = self.session.clone();
            let file_handle = self.handle.clone();
            let pending_read = self.state.f_read.take();

            handle.spawn(async move {
                // Cancellation drops the caller, not the queued protocol work.
                // Drain the bounded batch before CLOSE so no pending READ can
                // outlive its handle or poison the next operation's responses.
                if let Some(read) = pending_read {
                    let _ = read.await;
                }
                let _ = session.close(file_handle).await;
            });
        }
    }
}

impl AsyncRead for File {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let poll = Pin::new(match self.state.f_read.as_mut() {
            Some(f) => f,
            None => {
                let session = self.session.clone();
                let max_read_len = self
                    .features
                    .limits
                    .and_then(|l| l.read_len)
                    .unwrap_or_else(|| {
                        self.features
                            .max_packet_len
                            .saturating_sub(READ_OVERHEAD_LENGTH) as u64
                    }) as usize;

                let file_handle = self.handle.clone();

                let offset = self.pos;
                let len = usize::min(buf.remaining(), max_read_len);

                self.state.f_read.get_or_insert(Box::pin(async move {
                    let result = session.read(file_handle, offset, len as u32).await;
                    match result {
                        Ok(data) => Ok(Some(data.data)),
                        Err(Error::Status(status)) if status.status_code == StatusCode::Eof => {
                            Ok(None)
                        }
                        Err(e) => Err(io::Error::other(e.to_string())),
                    }
                }))
            }
        })
        .poll(cx);

        if poll.is_ready() {
            self.state.f_read = None;
        }

        match poll {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Ready(Ok(None)) => Poll::Ready(Ok(())),
            Poll::Ready(Ok(Some(data))) => {
                if data.len() > buf.remaining() {
                    return Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "SFTP server returned more data than requested",
                    )));
                }
                self.pos += data.len() as u64;
                buf.put_slice(&data[..]);
                Poll::Ready(Ok(()))
            }
        }
    }
}

impl AsyncSeek for File {
    fn start_seek(mut self: Pin<&mut Self>, position: io::SeekFrom) -> io::Result<()> {
        if self.state.f_seek.is_some() || self.state.f_read.is_some() {
            return Err(io::Error::other(
                "other file operation is pending, call poll_complete before start_seek",
            ));
        }

        self.state.f_seek = Some(match position {
            SeekFrom::Start(pos) => Box::pin(future::ready(Ok(pos))),
            SeekFrom::Current(pos) => {
                let new_pos = self.pos as i64 + pos;
                if new_pos < 0 {
                    return Err(io::Error::other(
                        "cannot move file pointer before the beginning",
                    ));
                }
                Box::pin(future::ready(Ok(new_pos as u64)))
            }
            SeekFrom::End(pos) => {
                let session = self.session.clone();
                let file_handle = self.handle.clone();

                Box::pin(async move {
                    let result = session
                        .fstat(file_handle)
                        .await
                        .map_err(|e| io::Error::other(e.to_string()))?;
                    match result.attrs.size {
                        Some(size) => {
                            let new_pos = size as i64 + pos;
                            if new_pos < 0 {
                                return Err(io::Error::other(
                                    "cannot move file pointer before the beginning",
                                ));
                            }
                            Ok(new_pos as u64)
                        }
                        None => Err(io::Error::other("file size unknown")),
                    }
                })
            }
        });

        Ok(())
    }

    fn poll_complete(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<u64>> {
        match self.state.f_seek.as_mut() {
            None => Poll::Ready(Ok(self.pos)),
            Some(f) => {
                self.pos = ready!(Pin::new(f).poll(cx))?;
                self.state.f_seek = None;
                Poll::Ready(Ok(self.pos))
            }
        }
    }
}

impl AsyncWrite for File {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        if self.state.f_read.is_some() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "read operation is pending",
            )));
        }
        if self.state.write_acks.len() >= self.features.max_concurrent_writes {
            if let Some(poll) = poll_oldest_write(&mut self.state.write_acks, cx) {
                ready!(poll)?;
            }
        }

        let max_write_len = self
            .features
            .limits
            .and_then(|l| l.write_len)
            .unwrap_or_else(|| {
                let overhead = WRITE_OVERHEAD_LENGTH + self.handle.len() as u32;
                self.features.max_packet_len.saturating_sub(overhead) as u64
            }) as usize;

        let len = usize::min(buf.len(), max_write_len);
        let data = buf[..len].to_vec();
        let handle = self.handle.clone();
        let offset = self.pos;

        match self.session.write_nowait(handle, offset, data) {
            Ok(rx) => {
                self.pos += len as u64;
                let ack = self.session.write_acknowledgement(rx);
                self.state
                    .write_acks
                    .push_back(Box::pin(async move { check_write_result(ack.await) }));
                Poll::Ready(Ok(len))
            }
            Err(e) => Poll::Ready(Err(io::Error::other(e.to_string()))),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        ready!(poll_drain_writes(&mut self.state.write_acks, cx))?;

        if !self.features.fsync {
            return Poll::Ready(Ok(()));
        }

        let poll = Pin::new(match self.state.f_flush.as_mut() {
            Some(f) => f,
            None => {
                let session = self.session.clone();
                let file_handle = self.handle.clone();

                self.state.f_flush.get_or_insert(Box::pin(async move {
                    session
                        .fsync(file_handle)
                        .await
                        .map(|_| ())
                        .map_err(|e| io::Error::other(e.to_string()))
                }))
            }
        })
        .poll(cx);

        if poll.is_ready() {
            self.state.f_flush = None;
        }

        poll
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        if let Some(read) = self.state.f_read.as_mut() {
            let result = ready!(read.as_mut().poll(cx));
            self.state.f_read = None;
            result?;
        }
        ready!(poll_drain_writes(&mut self.state.write_acks, cx))?;

        let poll = Pin::new(match self.state.f_shutdown.as_mut() {
            Some(f) => f,
            None => {
                let session = self.session.clone();
                let file_handle = self.handle.clone();

                self.state.f_shutdown.get_or_insert(Box::pin(async move {
                    session
                        .close(file_handle)
                        .await
                        .map_err(|e| io::Error::other(e.to_string()))?;
                    Ok(())
                }))
            }
        })
        .poll(cx);

        if poll.is_ready() {
            self.state.f_shutdown = None;
            self.closed = true;
        }

        poll
    }
}

#[cfg(test)]
#[path = "../../../../../test/rust/russh-sftp/unit/client/fs/file.rs"]
mod tests;

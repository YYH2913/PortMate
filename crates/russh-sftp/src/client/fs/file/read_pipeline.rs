use super::*;

type RangeRead = Pin<Box<dyn Future<Output = io::Result<Vec<u8>>> + Send + Sync>>;

pub(super) async fn read_batch(
    session: Arc<RawSftpSession>,
    handle: String,
    offset: u64,
    length: usize,
    request_len: usize,
) -> io::Result<Vec<u8>> {
    let timeout = session.request_timeout();
    let mut reads: Vec<Option<RangeRead>> = (0..length)
        .step_by(request_len)
        .map(|start| {
            let read: RangeRead = Box::pin(read_range(
                Arc::clone(&session),
                handle.clone(),
                offset + start as u64,
                (length - start).min(request_len),
            ));
            Some(read)
        })
        .collect();
    let mut results: Vec<Option<io::Result<Vec<u8>>>> = (0..reads.len()).map(|_| None).collect();
    let batch = future::poll_fn(|cx| {
        let mut pending = false;
        for (read, result) in reads.iter_mut().zip(&mut results) {
            if let Some(future) = read {
                match future.as_mut().poll(cx) {
                    Poll::Pending => pending = true,
                    Poll::Ready(value) => {
                        *result = Some(value);
                        *read = None;
                    }
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    });
    if tokio::time::timeout(timeout, batch).await.is_err() {
        // A stalled or malicious short-read peer must not retain the batch
        // forever. Closing the SFTP stream also prevents late replies from
        // being mistaken for responses to a subsequent transfer.
        let _ = session.close_session();
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "SFTP read batch timed out",
        ));
    }
    let mut output = Vec::with_capacity(length);
    let mut eof = false;
    for (index, result) in results.into_iter().enumerate() {
        let data = match result.expect("completed range") {
            Ok(data) => data,
            Err(error) => {
                if error.kind() == io::ErrorKind::TimedOut {
                    let _ = session.close_session();
                }
                return Err(error);
            }
        };
        if eof && !data.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SFTP data follows an earlier EOF",
            ));
        }
        eof |= data.len() < (length - index * request_len).min(request_len);
        output.extend_from_slice(&data);
    }
    Ok(output)
}

async fn read_range(
    session: Arc<RawSftpSession>,
    handle: String,
    offset: u64,
    length: usize,
) -> io::Result<Vec<u8>> {
    let mut data = Vec::with_capacity(length);
    while data.len() < length {
        let remaining = length - data.len();
        match session
            .read(
                handle.as_str(),
                offset + data.len() as u64,
                remaining as u32,
            )
            .await
        {
            Ok(reply) => {
                if reply.data.is_empty() || reply.data.len() > remaining {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "SFTP server returned an invalid DATA length",
                    ));
                }
                data.extend_from_slice(&reply.data);
            }
            Err(Error::Status(status)) if status.status_code == StatusCode::Eof => break,
            Err(Error::Timeout) => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "SFTP read timed out",
                ));
            }
            Err(error) => return Err(io::Error::other(error.to_string())),
        }
    }
    Ok(data)
}

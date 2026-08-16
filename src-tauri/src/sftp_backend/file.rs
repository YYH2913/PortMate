use super::*;

fn sftp_file_operation_error(kind: std::io::ErrorKind, message: String) -> std::io::Error {
    std::io::Error::new(kind, message)
}

fn run_libssh_sftp_file_operation<T>(
    session: &libssh_rs::Sftp,
    file: &mut Option<libssh_rs::SftpFile>,
    deadline: Instant,
    label: &str,
    operation: impl FnOnce(
        &libssh_rs::Sftp,
        &mut Option<libssh_rs::SftpFile>,
        Instant,
    ) -> std::io::Result<T>,
) -> std::io::Result<T> {
    let result = session.with_session_operation_until(deadline, || {
        session
            .set_session_timeout_until(deadline)
            .map_err(std::io::Error::other)?;
        let result = operation(session, file, deadline);
        let restored = session
            .set_session_timeout(SSH_RUNTIME_OPERATION_TIMEOUT)
            .map_err(std::io::Error::other);
        match (result, restored) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(restore_error)) => Err(sftp_file_operation_error(
                error.kind(),
                format!("{error}; {label} libssh runtime timeout restore failed: {restore_error}"),
            )),
        }
    });
    result.map_err(std::io::Error::other)?
}

async fn run_libssh_sftp_file_operation_with_timeout<T, F>(
    session: Arc<tokio::sync::Mutex<libssh_rs::Sftp>>,
    file: Arc<tokio::sync::Mutex<Option<libssh_rs::SftpFile>>>,
    timeout: Duration,
    label: &str,
    operation: F,
) -> std::io::Result<T>
where
    T: Send + 'static,
    F: FnOnce(
            &libssh_rs::Sftp,
            &mut Option<libssh_rs::SftpFile>,
            Instant,
        ) -> std::io::Result<T>
        + Send
        + 'static,
{
    let deadline = Instant::now().checked_add(timeout).ok_or_else(|| {
        sftp_file_operation_error(
            std::io::ErrorKind::InvalidInput,
            format!("{label} deadline is outside the supported range"),
        )
    })?;
    let session = tokio::time::timeout(timeout, session.lock_owned())
        .await
        .map_err(|_| {
            sftp_file_operation_error(
                std::io::ErrorKind::TimedOut,
                format!("{label} SFTP lock timed out after {} ms", timeout.as_millis()),
            )
        })?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            sftp_file_operation_error(
                std::io::ErrorKind::TimedOut,
                format!("{label} timed out after {} ms", timeout.as_millis()),
            )
        })?;
    let mut file = tokio::time::timeout(remaining, file.lock_owned())
        .await
        .map_err(|_| {
            sftp_file_operation_error(
                std::io::ErrorKind::TimedOut,
                format!("{label} file lock timed out after {} ms", timeout.as_millis()),
            )
        })?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| {
            sftp_file_operation_error(
                std::io::ErrorKind::TimedOut,
                format!("{label} timed out after {} ms", timeout.as_millis()),
            )
        })?;
    let worker_label = label.to_string();
    let worker = tokio::task::spawn_blocking(move || {
        run_libssh_sftp_file_operation(&session, &mut file, deadline, &worker_label, operation)
    });
    tokio::time::timeout(remaining, worker)
        .await
        .map_err(|_| {
            sftp_file_operation_error(
                std::io::ErrorKind::TimedOut,
                format!("{label} timed out after {} ms", timeout.as_millis()),
            )
        })?
        .map_err(|error| std::io::Error::other(format!("{label} worker failed: {error}")))?
}

pub(crate) enum SftpBackendFile {
    Russh(Box<russh_sftp::client::fs::File>),
    Libssh {
        session: Arc<tokio::sync::Mutex<libssh_rs::Sftp>>,
        file: Arc<tokio::sync::Mutex<Option<libssh_rs::SftpFile>>>,
    },
}

impl SftpBackendFile {
    pub(super) fn from_libssh(
        file: libssh_rs::SftpFile,
        session: Arc<tokio::sync::Mutex<libssh_rs::Sftp>>,
    ) -> Self {
        Self::Libssh {
            session,
            file: Arc::new(tokio::sync::Mutex::new(Some(file))),
        }
    }

    pub(crate) async fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Russh(file) => tokio::io::AsyncReadExt::read(file, buffer).await,
            Self::Libssh { session, file } => {
                let session = Arc::clone(session);
                let file = Arc::clone(file);
                let capacity = buffer.len();
                let (read, data) = run_libssh_sftp_file_operation_with_timeout(
                    session,
                    file,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP read",
                    move |_, file, _| {
                        let mut data = vec![0_u8; capacity];
                        let file = file.as_mut().ok_or_else(sftp_file_closed_error)?;
                        let read = file.read(&mut data)?;
                        Ok((read, data))
                    },
                )
                .await?;
                buffer[..read].copy_from_slice(&data[..read]);
                Ok(read)
            }
        }
    }

    pub(crate) async fn read_exact(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Russh(file) => tokio::io::AsyncReadExt::read_exact(file, buffer).await,
            Self::Libssh { session, file } => {
                let session = Arc::clone(session);
                let file = Arc::clone(file);
                let capacity = buffer.len();
                let data = run_libssh_sftp_file_operation_with_timeout(
                    session,
                    file,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP read_exact",
                    move |session, file, deadline| {
                        let mut data = vec![0_u8; capacity];
                        let file = file.as_mut().ok_or_else(sftp_file_closed_error)?;
                        read_exact_until(session, file, &mut data, deadline)?;
                        Ok(data)
                    },
                )
                .await?;
                buffer.copy_from_slice(&data);
                Ok(capacity)
            }
        }
    }

    pub(crate) async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Russh(file) => tokio::io::AsyncWriteExt::write_all(file, data).await,
            Self::Libssh { session, file } => {
                let session = Arc::clone(session);
                let file = Arc::clone(file);
                let data = data.to_vec();
                run_libssh_sftp_file_operation_with_timeout(
                    session,
                    file,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP write",
                    move |session, file, deadline| {
                        let file = file.as_mut().ok_or_else(sftp_file_closed_error)?;
                        write_all_until(session, file, &data, deadline)
                    },
                )
                .await
            }
        }
    }

    pub(crate) async fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Russh(file) => tokio::io::AsyncWriteExt::flush(file).await,
            Self::Libssh { session, file } => {
                let session = Arc::clone(session);
                let file = Arc::clone(file);
                run_libssh_sftp_file_operation_with_timeout(
                    session,
                    file,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP flush",
                    |_, file, _| {
                        file
                        .as_mut()
                        .ok_or_else(sftp_file_closed_error)?
                        .flush()
                    },
                )
                .await
            }
        }
    }

    pub(crate) async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            Self::Russh(file) => tokio::io::AsyncWriteExt::shutdown(file).await,
            Self::Libssh { session, file } => {
                let session = Arc::clone(session);
                let file = Arc::clone(file);
                run_libssh_sftp_file_operation_with_timeout(
                    session,
                    file,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP shutdown",
                    |session, file, deadline| {
                        let Some(mut file) = file.take() else {
                            return Ok(());
                        };
                        file.flush()?;
                        session
                            .set_session_timeout_until(deadline)
                            .map_err(std::io::Error::other)?;
                        drop(file);
                        Ok(())
                    },
                )
                .await
            }
        }
    }

    pub(crate) async fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Russh(file) => tokio::io::AsyncSeekExt::seek(file, position).await,
            Self::Libssh { session, file } => {
                let session = Arc::clone(session);
                let file = Arc::clone(file);
                run_libssh_sftp_file_operation_with_timeout(
                    session,
                    file,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP seek",
                    move |_, file, _| {
                        file
                        .as_mut()
                        .ok_or_else(sftp_file_closed_error)?
                        .seek(position)
                    },
                )
                .await
            }
        }
    }
}

fn sftp_file_closed_error() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "SFTP file is closed")
}

fn read_exact_until(
    session: &libssh_rs::Sftp,
    file: &mut libssh_rs::SftpFile,
    mut buffer: &mut [u8],
    deadline: Instant,
) -> std::io::Result<()> {
    while !buffer.is_empty() {
        session
            .set_session_timeout_until(deadline)
            .map_err(std::io::Error::other)?;
        match file.read(buffer) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "failed to fill whole buffer",
                ));
            }
            Ok(read) => buffer = &mut buffer[read..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn write_all_until(
    session: &libssh_rs::Sftp,
    file: &mut libssh_rs::SftpFile,
    mut data: &[u8],
    deadline: Instant,
) -> std::io::Result<()> {
    while !data.is_empty() {
        session
            .set_session_timeout_until(deadline)
            .map_err(std::io::Error::other)?;
        match file.write(data) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(written) => data = &data[written..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

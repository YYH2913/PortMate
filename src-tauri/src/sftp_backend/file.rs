use super::*;

pub(crate) enum SftpBackendFile {
    Russh(Box<russh_sftp::client::fs::File>),
    Libssh(Arc<tokio::sync::Mutex<Option<libssh_rs::SftpFile>>>),
}

impl SftpBackendFile {
    pub(super) fn from_libssh(file: libssh_rs::SftpFile) -> Self {
        Self::Libssh(Arc::new(tokio::sync::Mutex::new(Some(file))))
    }

    pub(crate) async fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Russh(file) => tokio::io::AsyncReadExt::read(file, buffer).await,
            Self::Libssh(file) => {
                let file = Arc::clone(file);
                let capacity = buffer.len();
                let (read, data) = tokio::task::spawn_blocking(move || {
                    let mut data = vec![0_u8; capacity];
                    let mut file = file.blocking_lock();
                    let file = file.as_mut().ok_or_else(sftp_file_closed_error)?;
                    let read = file.read(&mut data)?;
                    Ok::<_, std::io::Error>((read, data))
                })
                .await
                .map_err(std::io::Error::other)??;
                buffer[..read].copy_from_slice(&data[..read]);
                Ok(read)
            }
        }
    }

    pub(crate) async fn read_exact(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Russh(file) => tokio::io::AsyncReadExt::read_exact(file, buffer).await,
            Self::Libssh(file) => {
                let file = Arc::clone(file);
                let capacity = buffer.len();
                let data = tokio::task::spawn_blocking(move || {
                    let mut data = vec![0_u8; capacity];
                    let mut file = file.blocking_lock();
                    file.as_mut()
                        .ok_or_else(sftp_file_closed_error)?
                        .read_exact(&mut data)?;
                    Ok::<_, std::io::Error>(data)
                })
                .await
                .map_err(std::io::Error::other)??;
                buffer.copy_from_slice(&data);
                Ok(capacity)
            }
        }
    }

    pub(crate) async fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Russh(file) => tokio::io::AsyncWriteExt::write_all(file, data).await,
            Self::Libssh(file) => {
                let file = Arc::clone(file);
                let data = data.to_vec();
                tokio::task::spawn_blocking(move || {
                    file.blocking_lock()
                        .as_mut()
                        .ok_or_else(sftp_file_closed_error)?
                        .write_all(&data)
                })
                .await
                .map_err(std::io::Error::other)?
            }
        }
    }

    pub(crate) async fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Russh(file) => tokio::io::AsyncWriteExt::flush(file).await,
            Self::Libssh(file) => {
                let file = Arc::clone(file);
                tokio::task::spawn_blocking(move || {
                    file.blocking_lock()
                        .as_mut()
                        .ok_or_else(sftp_file_closed_error)?
                        .flush()
                })
                .await
                .map_err(std::io::Error::other)?
            }
        }
    }

    pub(crate) async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            Self::Russh(file) => tokio::io::AsyncWriteExt::shutdown(file).await,
            Self::Libssh(file) => {
                let file = Arc::clone(file);
                tokio::task::spawn_blocking(move || {
                    let mut file = file.blocking_lock();
                    let Some(mut file) = file.take() else {
                        return Ok(());
                    };
                    file.flush()
                })
                .await
                .map_err(std::io::Error::other)?
            }
        }
    }

    pub(crate) async fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::Russh(file) => tokio::io::AsyncSeekExt::seek(file, position).await,
            Self::Libssh(file) => {
                let file = Arc::clone(file);
                tokio::task::spawn_blocking(move || {
                    file.blocking_lock()
                        .as_mut()
                        .ok_or_else(sftp_file_closed_error)?
                        .seek(position)
                })
                .await
                .map_err(std::io::Error::other)?
            }
        }
    }
}

fn sftp_file_closed_error() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "SFTP file is closed")
}

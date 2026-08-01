use super::*;

mod file;
mod metadata;

pub(super) use file::SftpBackendFile;
use metadata::SftpBackendMetadataRaw;
pub(super) use metadata::{SftpBackendDirEntry, SftpBackendMetadata};

pub(super) enum SftpBackendSession {
    Russh(SftpSession),
    Libssh(Arc<tokio::sync::Mutex<libssh_rs::Sftp>>),
}

impl SftpBackendSession {
    pub(super) fn from_russh(session: SftpSession) -> Self {
        Self::Russh(session)
    }

    pub(super) fn from_libssh(session: libssh_rs::Sftp) -> Self {
        Self::Libssh(Arc::new(tokio::sync::Mutex::new(session)))
    }

    pub(super) fn set_timeout(&self, seconds: u64) {
        if let Self::Russh(session) = self {
            session.set_timeout(seconds);
        }
    }

    pub(super) async fn read_dir<P: Into<String>>(
        &self,
        path: P,
    ) -> Result<std::vec::IntoIter<SftpBackendDirEntry>, String> {
        let path = path.into();
        let entries = match self {
            Self::Russh(session) => session
                .read_dir(path)
                .await
                .map_err(|error| error.to_string())?
                .map(SftpBackendDirEntry::from_russh)
                .collect(),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .read_dir(&path)
                        .map(|entries| {
                            entries
                                .into_iter()
                                .filter_map(SftpBackendDirEntry::from_libssh)
                                .collect::<Vec<_>>()
                        })
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP read_dir worker failed: {error}"))??
            }
        };
        Ok(entries.into_iter())
    }

    pub(super) async fn canonicalize(&self, path: &str) -> Result<String, String> {
        match self {
            Self::Russh(session) => session
                .canonicalize(path.to_string())
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                let path = path.to_string();
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .canonicalize(&path)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP canonicalize worker failed: {error}"))?
            }
        }
    }

    #[cfg(test)]
    pub(super) async fn close(&self) -> Result<(), String> {
        match self {
            Self::Russh(session) => session.close().await.map_err(|error| error.to_string()),
            Self::Libssh(_) => Ok(()),
        }
    }

    pub(super) async fn metadata(&self, path: String) -> Result<SftpBackendMetadata, String> {
        match self {
            Self::Russh(session) => session
                .metadata(path)
                .await
                .map(SftpBackendMetadata::from_russh)
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .metadata(&path)
                        .map(SftpBackendMetadata::from_libssh)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP stat worker failed: {error}"))?
            }
        }
    }

    pub(super) async fn symlink_metadata(
        &self,
        path: String,
    ) -> Result<SftpBackendMetadata, String> {
        match self {
            Self::Russh(session) => session
                .symlink_metadata(path)
                .await
                .map(SftpBackendMetadata::from_russh)
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .symlink_metadata(&path)
                        .map(SftpBackendMetadata::from_libssh)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP lstat worker failed: {error}"))?
            }
        }
    }

    pub(super) async fn try_exists(&self, path: String) -> Result<bool, String> {
        match self {
            Self::Russh(session) => session
                .try_exists(path)
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(_) => match self.symlink_metadata(path).await {
                Ok(_) => Ok(true),
                Err(error) if libssh_sftp_missing_error(&error) => Ok(false),
                Err(error) => Err(error),
            },
        }
    }

    pub(super) async fn open(&self, path: String) -> Result<SftpBackendFile, String> {
        self.open_with_flags(path, OpenFlags::READ).await
    }

    pub(super) async fn open_with_flags(
        &self,
        path: String,
        flags: OpenFlags,
    ) -> Result<SftpBackendFile, String> {
        match self {
            Self::Russh(session) => session
                .open_with_flags(path, flags)
                .await
                .map(|file| SftpBackendFile::Russh(Box::new(file)))
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .open(&path, libssh_open_flags(flags), 0o600)
                        .map(SftpBackendFile::from_libssh)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP open worker failed: {error}"))?
            }
        }
    }

    pub(super) async fn create_dir(&self, path: String) -> Result<(), String> {
        match self {
            Self::Russh(session) => session
                .create_dir(path)
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .create_dir(&path, 0o755)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP mkdir worker failed: {error}"))?
            }
        }
    }

    pub(super) async fn remove_dir(&self, path: String) -> Result<(), String> {
        match self {
            Self::Russh(session) => session
                .remove_dir(path)
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .remove_dir(&path)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP rmdir worker failed: {error}"))?
            }
        }
    }

    pub(super) async fn remove_file(&self, path: String) -> Result<(), String> {
        match self {
            Self::Russh(session) => session
                .remove_file(path)
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .remove_file(&path)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP unlink worker failed: {error}"))?
            }
        }
    }

    pub(super) async fn rename(&self, old_path: String, new_path: String) -> Result<(), String> {
        match self {
            Self::Russh(session) => session
                .rename(old_path, new_path)
                .await
                .map_err(|error| error.to_string()),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .rename(&old_path, &new_path)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP rename worker failed: {error}"))?
            }
        }
    }

    pub(super) async fn set_metadata(
        &self,
        path: String,
        metadata: SftpBackendMetadata,
    ) -> Result<(), String> {
        match (self, metadata.raw) {
            (Self::Russh(session), SftpBackendMetadataRaw::Russh(mut raw)) => {
                raw.permissions = metadata.permissions;
                session
                    .set_metadata(path, raw)
                    .await
                    .map_err(|error| error.to_string())
            }
            (Self::Libssh(session), _) => {
                let session = Arc::clone(session);
                let attributes = libssh_rs::SetAttributes {
                    size: None,
                    uid_gid: None,
                    permissions: metadata.permissions,
                    atime_mtime: None,
                };
                tokio::task::spawn_blocking(move || {
                    session
                        .blocking_lock()
                        .set_metadata(&path, &attributes)
                        .map_err(|error| error.to_string())
                })
                .await
                .map_err(|error| format!("libssh SFTP setstat worker failed: {error}"))?
            }
            (Self::Russh(_), SftpBackendMetadataRaw::Libssh) => {
                Err("SFTP metadata backend mismatch".to_string())
            }
        }
    }
}

fn libssh_sftp_missing_error(error: &str) -> bool {
    error.ends_with("Sftp error code 2") || error.ends_with("Sftp error code 10")
}

fn libssh_open_flags(flags: OpenFlags) -> libssh_rs::OpenFlags {
    let mut mapped = if flags.contains(OpenFlags::READ) && flags.contains(OpenFlags::WRITE) {
        libssh_rs::OpenFlags::READ_WRITE
    } else if flags.contains(OpenFlags::WRITE) {
        libssh_rs::OpenFlags::WRITE_ONLY
    } else {
        libssh_rs::OpenFlags::READ_ONLY
    };
    if flags.contains(OpenFlags::CREATE) {
        mapped |= libssh_rs::OpenFlags::CREATE;
    }
    if flags.contains(OpenFlags::EXCLUDE) {
        mapped |= libssh_rs::OpenFlags::EXCLUSIVE;
    }
    if flags.contains(OpenFlags::TRUNCATE) {
        mapped |= libssh_rs::OpenFlags::TRUNCATE;
    }
    if flags.contains(OpenFlags::APPEND) {
        mapped |= libssh_rs::OpenFlags::APPEND;
    }
    mapped
}

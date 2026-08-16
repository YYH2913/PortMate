use super::*;

mod file;
mod metadata;

pub(super) use file::SftpBackendFile;
use metadata::SftpBackendMetadataRaw;
pub(super) use metadata::{SftpBackendDirEntry, SftpBackendMetadata};

const MAX_SFTP_DIRECTORY_ENTRIES: usize = super::file_metadata::MAX_FILE_DIRECTORY_ENTRIES;

fn run_libssh_sftp_operation<T>(
    session: &libssh_rs::Sftp,
    deadline: Instant,
    label: &str,
    operation: impl FnOnce(&libssh_rs::Sftp, Instant) -> Result<T, String>,
) -> Result<T, String> {
    let result = session.with_session_operation_until(deadline, || {
        session
            .set_session_timeout_until(deadline)
            .map_err(|error| format!("{label} libssh deadline setup failed: {error}"))?;
        let result = operation(session, deadline);
        let restored = session
            .set_session_timeout(SSH_RUNTIME_OPERATION_TIMEOUT)
            .map_err(|error| format!("{label} libssh runtime timeout restore failed: {error}"));
        match (result, restored) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Err(error), Err(restore_error)) => Err(format!("{error}; {restore_error}")),
        }
    });
    result.map_err(|error| format!("{label} libssh operation gate failed: {error}"))?
}

pub(super) async fn run_libssh_sftp_operation_with_timeout<T, F>(
    session: Arc<tokio::sync::Mutex<libssh_rs::Sftp>>,
    timeout: Duration,
    label: &str,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&libssh_rs::Sftp, Instant) -> Result<T, String> + Send + 'static,
{
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| format!("{label} deadline is outside the supported range"))?;
    let session = tokio::time::timeout(timeout, session.lock_owned())
        .await
        .map_err(|_| format!("{label} SFTP lock timed out after {} ms", timeout.as_millis()))?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{label} timed out after {} ms", timeout.as_millis()))?;
    let worker_label = label.to_string();
    let worker = tokio::task::spawn_blocking(move || {
        run_libssh_sftp_operation(&session, deadline, &worker_label, operation)
    });
    tokio::time::timeout(remaining, worker)
        .await
        .map_err(|_| format!("{label} timed out after {} ms", timeout.as_millis()))?
        .map_err(|error| format!("{label} worker failed: {error}"))?
}

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
                .read_dir_bounded(path, MAX_SFTP_DIRECTORY_ENTRIES)
                .await
                .map_err(|error| error.to_string())?
                .map(SftpBackendDirEntry::from_russh)
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect(),
            Self::Libssh(session) => {
                let session = Arc::clone(session);
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP read_dir",
                    move |session, deadline| {
                        let directory = session.open_dir(&path).map_err(|error| error.to_string())?;
                        let mut entries = Vec::new();
                        let mut raw_entry_count = 0_usize;
                        loop {
                            session
                                .set_session_timeout_until(deadline)
                                .map_err(|error| error.to_string())?;
                            let Some(metadata) = directory.read_dir() else {
                                break;
                            };
                            if raw_entry_count >= MAX_SFTP_DIRECTORY_ENTRIES {
                                return Err(format!(
                                    "SFTP directory entry count exceeds {MAX_SFTP_DIRECTORY_ENTRIES}"
                                ));
                            }
                            raw_entry_count += 1;
                            if let Some(entry) = SftpBackendDirEntry::from_libssh(
                                metadata.map_err(|error| error.to_string())?,
                            )? {
                                entries.push(entry);
                            }
                        }
                        Ok(entries)
                    },
                )
                .await?
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
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP canonicalize",
                    move |session, _| session.canonicalize(&path).map_err(|error| error.to_string()),
                )
                .await
            }
        }
    }

    #[cfg(all(test, unix))]
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
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP stat",
                    move |session, _| {
                        session
                            .metadata(&path)
                            .map(SftpBackendMetadata::from_libssh)
                            .map_err(|error| error.to_string())
                    },
                )
                .await
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
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP lstat",
                    move |session, _| {
                        session
                            .symlink_metadata(&path)
                            .map(SftpBackendMetadata::from_libssh)
                            .map_err(|error| error.to_string())
                    },
                )
                .await
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
                let file_session = Arc::clone(&session);
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP open",
                    move |session, _| {
                        session
                            .open(&path, libssh_open_flags(flags), 0o600)
                            .map(|file| SftpBackendFile::from_libssh(file, file_session))
                            .map_err(|error| error.to_string())
                    },
                )
                .await
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
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP mkdir",
                    move |session, _| {
                        session
                            .create_dir(&path, 0o755)
                            .map_err(|error| error.to_string())
                    },
                )
                .await
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
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP rmdir",
                    move |session, _| session.remove_dir(&path).map_err(|error| error.to_string()),
                )
                .await
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
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP unlink",
                    move |session, _| session.remove_file(&path).map_err(|error| error.to_string()),
                )
                .await
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
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP rename",
                    move |session, _| {
                        session
                            .rename(&old_path, &new_path)
                            .map_err(|error| error.to_string())
                    },
                )
                .await
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
                run_libssh_sftp_operation_with_timeout(
                    session,
                    SSH_RUNTIME_OPERATION_TIMEOUT,
                    "libssh SFTP setstat",
                    move |session, _| {
                        session
                            .set_metadata(&path, &attributes)
                            .map_err(|error| error.to_string())
                    },
                )
                .await
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

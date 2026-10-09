impl SftpBackendSession {
    #[cfg(all(test, unix))]
    pub(super) async fn close(&self) -> Result<(), String> {
        match self {
            Self::Russh(session) => session.close().await.map_err(|error| error.to_string()),
            Self::Libssh(_) => Ok(()),
        }
    }
}

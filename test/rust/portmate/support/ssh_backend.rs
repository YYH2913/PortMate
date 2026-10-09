impl<H> SshBackendSession<H>
where
    H: client::Handler,
{
    #[cfg(all(test, unix))]
    pub(super) fn russh_compat(&self) -> Result<&client::Handle<H>, String> {
        match self {
            Self::Russh(handle) => Ok(handle),
            Self::Libssh(_) => Err("该 SSH 操作尚未迁移到 libssh backend".to_string()),
        }
    }
}

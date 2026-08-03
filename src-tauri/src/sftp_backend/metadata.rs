use std::time::SystemTime;

const MAX_SFTP_DIRECTORY_ENTRY_NAME_BYTES: usize = 4096;

pub(crate) struct SftpBackendDirEntry {
    name: String,
    metadata: SftpBackendMetadata,
}

impl SftpBackendDirEntry {
    pub(super) fn from_russh(
        entry: russh_sftp::client::fs::DirEntry,
    ) -> Result<Option<Self>, String> {
        let name = entry.file_name();
        if matches!(name.as_str(), "." | "..") {
            return Ok(None);
        }
        validate_sftp_directory_entry_name(&name)?;
        Ok(Some(Self {
            name,
            metadata: SftpBackendMetadata::from_russh(entry.metadata()),
        }))
    }

    pub(super) fn from_libssh(metadata: libssh_rs::Metadata) -> Result<Option<Self>, String> {
        let name = metadata
            .name()
            .ok_or_else(|| "SFTP 服务端返回了缺失或非 UTF-8 的目录项名称".to_string())?
            .to_string();
        if matches!(name.as_str(), "." | "..") {
            return Ok(None);
        }
        validate_sftp_directory_entry_name(&name)?;
        Ok(Some(Self {
            name,
            metadata: SftpBackendMetadata::from_libssh(metadata),
        }))
    }

    pub(crate) fn file_name(&self) -> String {
        self.name.clone()
    }

    pub(crate) fn metadata(&self) -> SftpBackendMetadata {
        self.metadata.clone()
    }
}

fn validate_sftp_directory_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.len() > MAX_SFTP_DIRECTORY_ENTRY_NAME_BYTES
        || name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err("SFTP 服务端返回了不安全的目录项名称".to_string());
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct SftpBackendMetadata {
    pub(super) raw: SftpBackendMetadataRaw,
    kind: SftpBackendFileType,
    size: u64,
    pub(crate) permissions: Option<u32>,
    pub(crate) mtime: Option<u32>,
}

#[derive(Clone)]
pub(super) enum SftpBackendMetadataRaw {
    Russh(russh_sftp::client::fs::Metadata),
    Libssh,
}

impl SftpBackendMetadata {
    pub(super) fn from_russh(metadata: russh_sftp::client::fs::Metadata) -> Self {
        let kind = if metadata.is_symlink() {
            SftpBackendFileType::Symlink
        } else if metadata.is_dir() {
            SftpBackendFileType::Directory
        } else if metadata.is_regular() {
            SftpBackendFileType::Regular
        } else {
            SftpBackendFileType::Other
        };
        Self {
            size: metadata.len(),
            permissions: metadata.permissions,
            mtime: metadata.mtime,
            raw: SftpBackendMetadataRaw::Russh(metadata),
            kind,
        }
    }

    pub(super) fn from_libssh(metadata: libssh_rs::Metadata) -> Self {
        let kind = match metadata.file_type() {
            Some(libssh_rs::FileType::Directory) => SftpBackendFileType::Directory,
            Some(libssh_rs::FileType::Regular) => SftpBackendFileType::Regular,
            Some(libssh_rs::FileType::Symlink) => SftpBackendFileType::Symlink,
            _ => SftpBackendFileType::Other,
        };
        let mtime = metadata
            .modified()
            .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
            .and_then(|duration| u32::try_from(duration.as_secs()).ok());
        Self {
            raw: SftpBackendMetadataRaw::Libssh,
            kind,
            size: metadata.len().unwrap_or(0),
            permissions: metadata.permissions(),
            mtime,
        }
    }

    pub(crate) fn is_dir(&self) -> bool {
        self.kind == SftpBackendFileType::Directory
    }

    pub(crate) fn is_regular(&self) -> bool {
        self.kind == SftpBackendFileType::Regular
    }

    pub(crate) fn is_symlink(&self) -> bool {
        self.kind == SftpBackendFileType::Symlink
    }

    pub(crate) fn len(&self) -> u64 {
        self.size
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SftpBackendFileType {
    Directory,
    Regular,
    Symlink,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sftp_directory_entry_names_cannot_escape_the_listed_directory() {
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "child/file",
            "child\\file",
            "line\nfeed",
            "nul\0name",
        ] {
            assert!(
                validate_sftp_directory_entry_name(name).is_err(),
                "{name:?}"
            );
        }
        assert!(validate_sftp_directory_entry_name("normal file.txt").is_ok());
        assert!(validate_sftp_directory_entry_name(
            &"a".repeat(MAX_SFTP_DIRECTORY_ENTRY_NAME_BYTES + 1)
        )
        .is_err());
    }
}

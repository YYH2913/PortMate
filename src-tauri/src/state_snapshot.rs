#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::Read,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

pub(super) use iota_stronghold::SnapshotPath;
use iota_stronghold::{KeyProvider, Stronghold as IotaStronghold};
use rusqlite::{params, Connection as SqliteConnection};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{
    app_data_migration::{PORTABLE_VAULT_FILE_NAME, STORE_FILE_NAME},
    PROFILE_SECRET_MIGRATION_RESTART_REQUIRED, STORE_KEY,
};

pub(super) const STATE_FILE_HASH_BUFFER_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PortableVaultSnapshotVersion {
    Missing,
    Sha256([u8; 32]),
    UnknownAfterCommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StoreSnapshotVersion {
    Missing,
    Sha256([u8; 32]),
    UnknownAfterCommit,
}

pub(super) static STORE_SNAPSHOT_VERSIONS: OnceLock<Mutex<HashMap<PathBuf, StoreSnapshotVersion>>> =
    OnceLock::new();

pub(super) struct PortableStronghold {
    inner: IotaStronghold,
    pub(super) path: SnapshotPath,
    snapshot_path: PathBuf,
    pub(super) snapshot_version: PortableVaultSnapshotVersion,
    pub(super) opened_existing_snapshot: bool,
    key_provider: KeyProvider,
}

fn portable_vault_lock_path(snapshot_path: &Path) -> PathBuf {
    let file_name = snapshot_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(PORTABLE_VAULT_FILE_NAME);
    snapshot_path.with_file_name(format!("{file_name}.lock"))
}

pub(super) fn secure_portable_vault_parent(snapshot_path: &Path) -> Result<(), String> {
    let parent = snapshot_path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("无法创建 portable vault 目录 {}: {error}", parent.display()))?;
    let metadata = fs::symlink_metadata(parent)
        .map_err(|error| format!("无法检查 portable vault 目录 {}: {error}", parent.display()))?;
    if portable_vault_metadata_is_link(&metadata) || !metadata.is_dir() {
        return Err(format!(
            "portable vault 目录必须是真实目录: {}",
            parent.display()
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700)).map_err(|error| {
            format!("无法保护 portable vault 目录 {}: {error}", parent.display())
        })?;
    }
    Ok(())
}

fn validate_portable_vault_file_metadata(
    metadata: &fs::Metadata,
    label: &str,
) -> Result<(), String> {
    if portable_vault_metadata_is_link(metadata) || !metadata.is_file() {
        return Err(format!("portable vault {label} 必须是普通文件"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(format!("portable vault {label} 不得存在多个硬链接"));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn validate_opened_portable_vault_file_link_count(
    file: &fs::File,
    label: &str,
) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the borrowed std::fs::File keeps the HANDLE valid for the call and
    // the output points to a fully allocated BY_HANDLE_FILE_INFORMATION value.
    let result = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) };
    if result == 0 {
        return Err(format!(
            "无法检查 portable vault {label} 硬链接数量: {}",
            std::io::Error::last_os_error()
        ));
    }
    if information.nNumberOfLinks != 1 {
        return Err(format!("portable vault {label} 不得存在多个硬链接"));
    }
    Ok(())
}

fn portable_vault_metadata_is_link(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    false
}

pub(super) fn portable_vault_file_exists(path: &Path, label: &str) -> Result<bool, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "无法检查 portable vault {label} {}: {error}",
                path.display()
            ));
        }
    };
    validate_portable_vault_file_metadata(&metadata, label)?;
    #[cfg(windows)]
    {
        let file = fs::File::open(path).map_err(|error| {
            format!(
                "无法打开 portable vault {label} 检查文件身份 {}: {error}",
                path.display()
            )
        })?;
        let opened_metadata = file
            .metadata()
            .map_err(|error| format!("无法检查已打开的 portable vault {label}: {error}"))?;
        validate_portable_vault_file_metadata(&opened_metadata, label)?;
        validate_opened_portable_vault_file_link_count(&file, label)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
            format!(
                "无法保护 portable vault {label} {}: {error}",
                path.display()
            )
        })?;
    }
    Ok(true)
}

pub(super) fn secure_opened_portable_vault_file(
    file: &fs::File,
    label: &str,
) -> Result<fs::Metadata, String> {
    let metadata = file
        .metadata()
        .map_err(|error| format!("无法检查已打开的 portable vault {label}: {error}"))?;
    validate_portable_vault_file_metadata(&metadata, label)?;
    #[cfg(windows)]
    validate_opened_portable_vault_file_link_count(file, label)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("无法保护已打开的 portable vault {label}: {error}"))?;
    }
    Ok(metadata)
}

fn store_lock_path(store_path: &Path) -> PathBuf {
    let file_name = store_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(STORE_FILE_NAME);
    store_path.with_file_name(format!("{file_name}.lock"))
}

pub(super) fn lock_store_snapshot(store_path: &Path) -> Result<fs::File, String> {
    if let Some(parent) = store_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "无法创建 PortMate store 锁目录 {}: {error}",
                    parent.display()
                )
            })?;
        }
    }
    let lock_path = store_lock_path(store_path);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("无法打开 PortMate store 文件锁: {error}"))?;
    lock.lock()
        .map_err(|error| format!("无法获取 PortMate store 文件锁: {error}"))?;
    Ok(lock)
}

pub(super) fn store_snapshot_version(store_path: &Path) -> Result<StoreSnapshotVersion, String> {
    if store_path.extension().and_then(|value| value.to_str()) == Some("sqlite3") {
        if !store_path.exists() {
            return Ok(StoreSnapshotVersion::Missing);
        }
        let connection = SqliteConnection::open(store_path).map_err(|error| {
            format!(
                "无法打开 PortMate SQLite store 读取版本 {}: {error}",
                store_path.display()
            )
        })?;
        let store_json = match connection.query_row(
            "select value from kv where key = ?1",
            params![STORE_KEY],
            |row| row.get::<_, String>(0),
        ) {
            Ok(store_json) => store_json,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                return Ok(StoreSnapshotVersion::Missing);
            }
            Err(error) => {
                return Err(format!("无法读取 PortMate SQLite store 快照内容: {error}"));
            }
        };
        let has_metadata_table = connection
            .query_row(
                "select exists(select 1 from sqlite_master where type = 'table' and name = 'metadata')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| format!("无法检查 PortMate SQLite metadata 表: {error}"))?;
        let revision = if has_metadata_table {
            match connection.query_row(
                "select value from metadata where key = 'storeRevision'",
                [],
                |row| row.get::<_, String>(0),
            ) {
                Ok(revision) => Some(revision),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(error) => {
                    return Err(format!("无法读取 PortMate SQLite store 版本: {error}"));
                }
            }
        } else {
            None
        };
        let mut digest = Sha256::new();
        digest.update(b"portmate-store-snapshot-v1\0");
        if let Some(revision) = revision {
            digest.update((revision.len() as u64).to_le_bytes());
            digest.update(revision.as_bytes());
        } else {
            digest.update(0_u64.to_le_bytes());
        }
        digest.update((store_json.len() as u64).to_le_bytes());
        digest.update(store_json.as_bytes());
        return Ok(StoreSnapshotVersion::Sha256(digest.finalize().into()));
    }
    let digest = match sha256_file_digest(store_path) {
        Ok(digest) => digest,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(StoreSnapshotVersion::Missing);
        }
        Err(error) => return Err(format!("无法读取 PortMate store 指纹: {error}")),
    };
    Ok(StoreSnapshotVersion::Sha256(digest))
}

pub(super) fn sha256_file_digest(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut file = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; STATE_FILE_HASH_BUFFER_BYTES];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest.finalize().into())
}

pub(super) fn verify_store_snapshot_is_current(
    store_path: &Path,
) -> Result<StoreSnapshotVersion, String> {
    let snapshot_lock = lock_store_snapshot(store_path)?;
    let current = store_snapshot_version(store_path)?;
    let mut versions = STORE_SNAPSHOT_VERSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|error| error.to_string())?;
    let expected = versions.entry(store_path.to_path_buf()).or_insert(current);
    let result = if *expected == StoreSnapshotVersion::UnknownAfterCommit {
        Err(format!(
            "{PROFILE_SECRET_MIGRATION_RESTART_REQUIRED} PortMate store 上次提交后无法验证版本，请重启应用后再迁移凭据"
        ))
    } else if *expected != current {
        Err(format!(
            "{PROFILE_SECRET_MIGRATION_RESTART_REQUIRED} PortMate store 已被另一实例修改，请重启应用加载最新数据后再迁移凭据"
        ))
    } else {
        Ok(current)
    };
    drop(versions);
    drop(snapshot_lock);
    result
}

pub(super) fn lock_portable_vault_snapshot(snapshot_path: &Path) -> Result<fs::File, String> {
    secure_portable_vault_parent(snapshot_path)?;
    let lock_path = portable_vault_lock_path(snapshot_path);
    portable_vault_file_exists(&lock_path, "lock")?;
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let lock = options
        .open(&lock_path)
        .map_err(|error| format!("无法打开 portable vault 文件锁: {error}"))?;
    secure_opened_portable_vault_file(&lock, "lock")?;
    lock.lock()
        .map_err(|error| format!("无法获取 portable vault 文件锁: {error}"))?;
    Ok(lock)
}

fn portable_vault_snapshot_version(
    snapshot_path: &Path,
) -> Result<PortableVaultSnapshotVersion, String> {
    if !portable_vault_file_exists(snapshot_path, "snapshot")? {
        return Ok(PortableVaultSnapshotVersion::Missing);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(snapshot_path)
        .map_err(|error| format!("无法打开 portable vault snapshot 读取指纹: {error}"))?;
    secure_opened_portable_vault_file(&file, "snapshot")?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; STATE_FILE_HASH_BUFFER_BYTES];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("无法读取 portable vault snapshot 指纹: {error}"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let digest = digest.finalize().into();
    Ok(PortableVaultSnapshotVersion::Sha256(digest))
}

impl PortableStronghold {
    pub(super) fn new(path: &Path, key: Vec<u8>) -> Result<Self, String> {
        let key = Zeroizing::new(key);
        let snapshot_lock = lock_portable_vault_snapshot(path)?;
        let snapshot_path = path.to_path_buf();
        let path = SnapshotPath::from_path(path);
        let inner = IotaStronghold::default();
        let key_provider = KeyProvider::try_from(key)
            .map_err(|error| format!("portable vault key provider 初始化失败: {error}"))?;
        let opened_existing_snapshot = portable_vault_file_exists(&snapshot_path, "snapshot")?;
        if opened_existing_snapshot {
            inner
                .load_snapshot(&key_provider, &path)
                .map_err(|error| format!("portable vault snapshot 加载失败: {error}"))?;
        }
        let snapshot_version = portable_vault_snapshot_version(&snapshot_path)?;
        drop(snapshot_lock);
        Ok(Self {
            inner,
            path,
            snapshot_path,
            snapshot_version,
            opened_existing_snapshot,
            key_provider,
        })
    }

    pub(super) fn ensure_snapshot_current(&self) -> Result<(), String> {
        let snapshot_lock = lock_portable_vault_snapshot(&self.snapshot_path)?;
        let result = self.ensure_snapshot_current_locked();
        drop(snapshot_lock);
        result
    }

    fn ensure_snapshot_current_locked(&self) -> Result<(), String> {
        if self.snapshot_version == PortableVaultSnapshotVersion::UnknownAfterCommit {
            return Err("portable vault snapshot 提交后无法刷新版本，请锁定后重新解锁".to_string());
        }
        let current = portable_vault_snapshot_version(&self.snapshot_path)?;
        if current != self.snapshot_version {
            return Err(
                "portable vault snapshot 已被另一 PortMate 实例修改，请锁定后重新解锁".to_string(),
            );
        }
        Ok(())
    }

    fn refresh_snapshot_version_after_commit(&mut self) {
        match portable_vault_snapshot_version(&self.snapshot_path) {
            Ok(version) => self.snapshot_version = version,
            Err(error) => {
                self.snapshot_version = PortableVaultSnapshotVersion::UnknownAfterCommit;
                eprintln!("PortMate: {error}; portable vault must be reopened before reuse");
            }
        }
    }

    pub(super) fn save(&mut self) -> Result<(), String> {
        let snapshot_lock = lock_portable_vault_snapshot(&self.snapshot_path)?;
        self.ensure_snapshot_current_locked()?;
        self.inner
            .commit_with_keyprovider(&self.path, &self.key_provider)
            .map_err(|error| error.to_string())?;
        self.refresh_snapshot_version_after_commit();
        drop(snapshot_lock);
        Ok(())
    }

    pub(super) fn rekey(&mut self, key: Vec<u8>) -> Result<(), String> {
        let key = Zeroizing::new(key);
        let snapshot_lock = lock_portable_vault_snapshot(&self.snapshot_path)?;
        self.ensure_snapshot_current_locked()?;
        let key_provider = KeyProvider::try_from(key)
            .map_err(|error| format!("portable vault key provider 初始化失败: {error}"))?;
        self.inner
            .commit_with_keyprovider(&self.path, &key_provider)
            .map_err(|error| format!("portable vault 换密提交失败: {error}"))?;
        self.key_provider = key_provider;
        self.refresh_snapshot_version_after_commit();
        drop(snapshot_lock);
        Ok(())
    }
}

impl Deref for PortableStronghold {
    type Target = IotaStronghold;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

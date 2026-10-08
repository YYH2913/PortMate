use super::*;

pub(super) struct SafeSqliteConnection {
    connection: SqliteConnection,
    _file: fs::File,
    _parent: BoundDirectory,
}

impl Deref for SafeSqliteConnection {
    type Target = SqliteConnection;
    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

pub(super) fn validate_single_link_file(file: &fs::File, label: &str) -> Result<(), String> {
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err(format!("{label} must be a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if file.metadata().map_err(|error| error.to_string())?.nlink() != 1 {
            return Err(format!("{label} must not be a hard link"));
        }
    }
    #[cfg(windows)]
    {
        let information =
            windows_transfer_file_information(file).map_err(|error| error.to_string())?;
        if information.nNumberOfLinks != 1
            || information.dwFileAttributes
                & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
                != 0
        {
            return Err(format!("{label} must not be a hard link or reparse point"));
        }
    }
    Ok(())
}

pub(super) fn open_store_file(
    path: &Path,
    create: bool,
) -> Result<(fs::File, BoundDirectory), String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(path)
    };
    let parent = BoundDirectory::open(path.parent().ok_or("store path has no parent")?)
        .map_err(|error| format!("cannot bind store parent: {error}"))?;
    let file = parent
        .open_regular(path.file_name().ok_or("store path has no name")?, create)
        .map_err(|error| format!("cannot open store without following links: {error}"))?;
    validate_single_link_file(&file, "PortMate store file")?;
    Ok((file, parent))
}

pub(super) fn open_store_sqlite(path: &Path) -> Result<SafeSqliteConnection, String> {
    open_store_sqlite_with_hook(path, || {})
}

fn open_store_sqlite_with_hook(
    path: &Path,
    before_sqlite_open: impl FnOnce(),
) -> Result<SafeSqliteConnection, String> {
    let (file, parent) = open_store_file(path, true)?;
    before_sqlite_open();
    // SQLite's NOFOLLOW is enforced by its VFS/open, not a check followed by
    // an ordinary open. The pin denies delete sharing on Windows and keeps
    // the original inode available for the post-open identity check on Unix.
    let connection = SqliteConnection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("cannot open safe SQLite store: {error}"))?;
    parent.ensure_current().map_err(|error| error.to_string())?;
    validate_single_link_file(&file, "PortMate SQLite store")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let current = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        let original = file.metadata().map_err(|error| error.to_string())?;
        let mut moved: libc::c_int = 0;
        // HAS_MOVED compares the VFS's actual opened database inode with its
        // name. No schema statement or write is issued before this check.
        let code = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                connection.handle(),
                c"main".as_ptr(),
                rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                (&mut moved as *mut libc::c_int).cast(),
            )
        };
        if code != rusqlite::ffi::SQLITE_OK
            || moved != 0
            || current.file_type().is_symlink()
            || current.dev() != original.dev()
            || current.ino() != original.ino()
            || current.nlink() != 1
        {
            return Err("SQLite store changed while opening".into());
        }
    }
    Ok(SafeSqliteConnection {
        connection,
        _file: file,
        _parent: parent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn sqlite_rejects_a_symlink_substituted_after_the_guard_open() {
        let root = tempfile::tempdir().unwrap();
        let selected = root.path().join("store.sqlite3");
        let outside = root.path().join("outside.sqlite3");
        SqliteConnection::open(&outside)
            .unwrap()
            .execute_batch("CREATE TABLE victim(value); INSERT INTO victim VALUES ('original');")
            .unwrap();
        fs::write(&selected, []).unwrap();
        let result = open_store_sqlite_with_hook(&selected, || {
            fs::remove_file(&selected).unwrap();
            std::os::unix::fs::symlink(&outside, &selected).unwrap();
        });
        assert!(result.is_err());
        let db = SqliteConnection::open(outside).unwrap();
        let value: String = db
            .query_row("SELECT value FROM victim", [], |row| row.get(0))
            .unwrap();
        assert_eq!(value, "original");
    }

    #[test]
    fn store_lock_and_sqlite_reject_preexisting_hard_links() {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().join("outside.sqlite3");
        fs::write(&outside, b"unchanged").unwrap();
        let selected = root.path().join("store.sqlite3");
        fs::hard_link(&outside, &selected).unwrap();
        assert!(open_store_file(&selected, true).is_err());
        assert!(open_store_sqlite(&selected).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"unchanged");
    }
}

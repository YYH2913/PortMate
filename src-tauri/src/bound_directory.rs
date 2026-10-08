use super::*;

pub(super) struct BoundDirectory {
    file: fs::File,
    path: PathBuf,
    // On Windows these handles deny directory replacement. Unix child opens
    // use dirfd-relative openat, not the mutable display path.
    parents: Arc<Vec<fs::File>>,
}

impl BoundDirectory {
    pub(super) fn pin_entry(path: &Path) -> std::io::Result<fs::File> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::*;
            options
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
        }
        let file = options.open(path)?;
        Self::validate(&file)?;
        Ok(file)
    }

    pub(super) fn entry_matches(file: &fs::File, other: &fs::File) -> std::io::Result<bool> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let before = file.metadata()?;
            let after = other.metadata()?;
            Ok(before.dev() == after.dev() && before.ino() == after.ino())
        }
        #[cfg(windows)]
        {
            let before = windows_transfer_file_information(file)?;
            let after = windows_transfer_file_information(other)?;
            Ok(before.dwVolumeSerialNumber == after.dwVolumeSerialNumber
                && before.nFileIndexHigh == after.nFileIndexHigh
                && before.nFileIndexLow == after.nFileIndexLow)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (file, other);
            Err(std::io::Error::other("file identity unsupported"))
        }
    }
    pub(super) fn open_regular(
        &self,
        name: &std::ffi::OsStr,
        create: bool,
    ) -> std::io::Result<fs::File> {
        #[cfg(unix)]
        {
            use std::os::{
                fd::{AsRawFd, FromRawFd},
                unix::ffi::OsStrExt,
            };
            let name = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
            let flags = libc::O_RDWR
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK
                | if create { libc::O_CREAT } else { 0 };
            let fd = unsafe { libc::openat(self.file.as_raw_fd(), name.as_ptr(), flags, 0o600) };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(unsafe { fs::File::from_raw_fd(fd) })
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::*;
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(create)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(name))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (name, create);
            Err(std::io::Error::other("safe regular-file open unavailable"))
        }
    }
    pub(super) fn open(path: &Path) -> std::io::Result<Self> {
        let mut parents = Vec::new();
        let mut current = PathBuf::new();
        for component in path.components() {
            current.push(component);
            if matches!(component, std::path::Component::Prefix(_)) {
                continue;
            }
            #[cfg(unix)]
            let file = if let Some(parent) = parents.last() {
                Self::open_child_file(parent, component.as_os_str())?
            } else {
                Self::open_directory_file(&current)?
            };
            #[cfg(not(unix))]
            let file = Self::open_directory_file(&current)?;
            if !file.metadata()?.is_dir() {
                return Err(std::io::Error::other("tree parent is not a directory"));
            }
            parents.push(file);
        }
        let file = parents
            .pop()
            .ok_or_else(|| std::io::Error::other("empty directory path"))?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
            parents: Arc::new(parents),
        })
    }

    fn open_directory_file(path: &Path) -> std::io::Result<fs::File> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC);
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::*;
            options
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS);
        }
        let file = options.open(path)?;
        Self::validate(&file)?;
        Ok(file)
    }

    fn validate(file: &fs::File) -> std::io::Result<()> {
        #[cfg(windows)]
        if windows_transfer_file_information(file)?.dwFileAttributes
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(std::io::Error::other("tree path is a reparse point"));
        }
        let _ = file;
        Ok(())
    }

    #[cfg(not(windows))]
    fn open_child_file(parent: &fs::File, name: &std::ffi::OsStr) -> std::io::Result<fs::File> {
        #[cfg(unix)]
        {
            use std::os::{
                fd::{AsRawFd, FromRawFd},
                unix::ffi::OsStrExt,
            };
            let name = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
            // SAFETY: parent stays open; the CString is terminated. NOFOLLOW
            // applies at the only untrusted path component of this open.
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(unsafe { fs::File::from_raw_fd(fd) })
        }
        #[cfg(not(unix))]
        {
            // Only used for root traversal on Windows; its parent path has
            // already been locked against rename by retained directory handles.
            let _ = (parent, name);
            Err(std::io::Error::other(
                "relative native directory open unavailable",
            ))
        }
    }

    pub(super) fn names(&self) -> std::io::Result<Vec<std::ffi::OsString>> {
        #[cfg(unix)]
        {
            use std::os::{
                fd::{AsRawFd, IntoRawFd},
                unix::ffi::OsStringExt,
            };
            let fresh = Self::open_child_file(&self.file, std::ffi::OsStr::new("."))?;
            let fd = fresh.into_raw_fd();
            let directory = unsafe { libc::fdopendir(fd) };
            if directory.is_null() {
                unsafe {
                    libc::close(fd);
                }
                return Err(std::io::Error::last_os_error());
            }
            struct Directory(*mut libc::DIR);
            impl Drop for Directory {
                fn drop(&mut self) {
                    unsafe {
                        libc::closedir(self.0);
                    }
                }
            }
            let guard = Directory(directory);
            let mut names = Vec::new();
            loop {
                #[cfg(any(target_os = "linux", target_os = "android"))]
                unsafe {
                    *libc::__errno_location() = 0;
                }
                #[cfg(any(
                    target_os = "macos",
                    target_os = "ios",
                    target_os = "freebsd",
                    target_os = "dragonfly"
                ))]
                unsafe {
                    *libc::__error() = 0;
                }
                let entry = unsafe { libc::readdir(guard.0) };
                if entry.is_null() {
                    let error = std::io::Error::last_os_error();
                    if error.raw_os_error().is_some_and(|code| code != 0) {
                        return Err(error);
                    }
                    break;
                }
                let bytes =
                    unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
                if bytes != b"." && bytes != b".." {
                    names.push(std::ffi::OsString::from_vec(bytes.to_vec()));
                }
                if names.len() > MAX_EXTERNAL_DROP_ENTRIES {
                    return Err(std::io::Error::other("tree entry limit exceeded"));
                }
            }
            let _ = self.file.as_raw_fd();
            Ok(names)
        }
        #[cfg(windows)]
        {
            fs::read_dir(&self.path)?
                .map(|entry| entry.map(|entry| entry.file_name()))
                .collect()
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(std::io::Error::other(
                "safe directory enumeration is unsupported",
            ))
        }
    }

    pub(super) fn child(
        &self,
        name: &std::ffi::OsStr,
    ) -> std::io::Result<(fs::Metadata, Option<Self>)> {
        #[cfg(unix)]
        {
            use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};
            let name = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
            let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::fstatat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    &mut metadata,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if !matches!(
                metadata.st_mode & libc::S_IFMT,
                libc::S_IFDIR | libc::S_IFREG
            ) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "symbolic link or special file",
                ));
            }
        }
        #[cfg(windows)]
        if fs::symlink_metadata(self.path.join(name))?.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "symbolic link or reparse point",
            ));
        }
        #[cfg(unix)]
        let file = Self::open_child_file(&self.file, name)?;
        #[cfg(windows)]
        let file = Self::open_directory_file(&self.path.join(name))?;
        #[cfg(not(any(unix, windows)))]
        let file = Self::open_child_file(&self.file, name)?;
        Self::validate(&file)?;
        let metadata = file.metadata()?;
        let directory = metadata.is_dir().then(|| Self {
            file,
            path: self.path.join(name),
            parents: Arc::clone(&self.parents),
        });
        Ok((metadata, directory))
    }

    pub(super) fn ensure_current(&self) -> std::io::Result<()> {
        let reopened = Self::open(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let before = self.file.metadata()?;
            let after = reopened.file.metadata()?;
            if before.dev() != after.dev() || before.ino() != after.ino() {
                return Err(std::io::Error::other(
                    "directory path changed during enumeration",
                ));
            }
        }
        #[cfg(windows)]
        {
            let before = windows_transfer_file_information(&self.file)?;
            let after = windows_transfer_file_information(&reopened.file)?;
            if before.dwVolumeSerialNumber != after.dwVolumeSerialNumber
                || before.nFileIndexHigh != after.nFileIndexHigh
                || before.nFileIndexLow != after.nFileIndexLow
            {
                return Err(std::io::Error::other(
                    "directory path changed during enumeration",
                ));
            }
        }
        Ok(())
    }
}

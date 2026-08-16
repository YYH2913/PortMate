use crate::{Error, SessionHolder, SshResult};
use libssh_rs_sys as sys;
use std::convert::TryInto;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

const FSYNC_EXTENSION_NAME: &[u8] = b"fsync@openssh.com\0";
const FSYNC_EXTENSION_VERSION: &[u8] = b"1\0";
use thiserror::Error;

fn sftp_v3_timestamp(time: SystemTime) -> SshResult<u32> {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| Error::fatal("SFTP v3 timestamps cannot represent times before Unix epoch"))?
        .as_secs();
    seconds
        .try_into()
        .map_err(|_| Error::fatal("SFTP v3 timestamp exceeds the 32-bit seconds range"))
}

fn system_time_from_sftp_timestamp(seconds: u64, nanoseconds: u32) -> Option<SystemTime> {
    let duration =
        Duration::from_secs(seconds).checked_add(Duration::from_nanos(u64::from(nanoseconds)))?;
    SystemTime::UNIX_EPOCH.checked_add(duration)
}

#[derive(Error, Debug, PartialEq, Eq)]
#[error("Sftp error code {}", .0)]
pub struct SftpError(u32);

impl SftpError {
    pub(crate) fn from_session(sftp: sys::sftp_session) -> Self {
        let code = unsafe { sys::sftp_get_error(sftp) as u32 };
        Self(code)
    }

    pub(crate) fn result<T>(sftp: sys::sftp_session, status: i32, res: T) -> SshResult<T> {
        if status == sys::SSH_OK as i32 {
            Ok(res)
        } else {
            Err(Error::Sftp(SftpError::from_session(sftp)))
        }
    }
}

struct SftpInner {
    sess: Arc<Mutex<SessionHolder>>,
    sftp_inner: sys::sftp_session,
}

unsafe impl Send for SftpInner {}
unsafe impl Sync for SftpInner {}

impl Drop for SftpInner {
    fn drop(&mut self) {
        let _sess = self.sess.lock().unwrap();
        if !self.sftp_inner.is_null() {
            unsafe {
                sys::sftp_free(self.sftp_inner);
            }
        }
    }
}

pub struct Sftp {
    inner: Arc<SftpInner>,
}

impl Sftp {
    pub(crate) fn new(sess: Arc<Mutex<SessionHolder>>, sftp_inner: sys::sftp_session) -> Self {
        Self {
            inner: Arc::new(SftpInner { sess, sftp_inner }),
        }
    }

    fn lock_session(&self) -> (MutexGuard<'_, SessionHolder>, sys::sftp_session) {
        (self.inner.sess.lock().unwrap(), self.inner.sftp_inner)
    }

    /// Set the owning session's connection timeout.
    pub fn set_session_timeout(&self, timeout: Duration) -> SshResult<()> {
        let (session, _) = self.lock_session();
        session.set_timeout(timeout)
    }

    /// Set the owning session's timeout from an absolute deadline.
    ///
    /// The remaining duration is computed after the session mutex is acquired.
    pub fn set_session_timeout_until(&self, deadline: Instant) -> SshResult<Duration> {
        let (session, _) = self.lock_session();
        session.set_timeout_until(deadline)
    }

    pub(crate) fn init(&self) -> SshResult<()> {
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_init(sftp) };
        SftpError::result(sftp, res, ())
    }

    /// Create a directory.
    /// `mode` specifies the permission bits to use on the directory.
    /// They will be modified by the effective umask on the server.
    pub fn create_dir(&self, filename: &str, mode: sys::mode_t) -> SshResult<()> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_mkdir(sftp, filename.as_ptr(), mode) };
        SftpError::result(sftp, res, ())
    }

    /// Canonicalize `filename`, resolving relative directory references
    /// and symlinks.
    pub fn canonicalize(&self, filename: &str) -> SshResult<String> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_canonicalize_path(sftp, filename.as_ptr()) };
        if res.is_null() {
            Err(Error::Sftp(SftpError::from_session(sftp)))
        } else {
            let result = unsafe { CStr::from_ptr(res) }.to_string_lossy().to_string();
            unsafe { sys::ssh_string_free_char(res) };
            Ok(result)
        }
    }

    /// Change the permissions of a file
    pub fn chmod(&self, filename: &str, mode: sys::mode_t) -> SshResult<()> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_chmod(sftp, filename.as_ptr(), mode) };
        SftpError::result(sftp, res, ())
    }

    /// Change the ownership of a file.
    pub fn chown(&self, filename: &str, owner: sys::uid_t, group: sys::gid_t) -> SshResult<()> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_chown(sftp, filename.as_ptr(), owner, group) };
        SftpError::result(sftp, res, ())
    }

    /// Read the payload of a symlink
    pub fn read_link(&self, filename: &str) -> SshResult<String> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_readlink(sftp, filename.as_ptr()) };
        if res.is_null() {
            Err(Error::Sftp(SftpError::from_session(sftp)))
        } else {
            let result = unsafe { CStr::from_ptr(res) }.to_string_lossy().to_string();
            unsafe { sys::ssh_string_free_char(res) };
            Ok(result)
        }
    }

    /// Change certain metadata attributes of the named file.
    pub fn set_metadata(&self, filename: &str, metadata: &SetAttributes) -> SshResult<()> {
        let filename = CString::new(filename)?;
        let mut attributes: sys::sftp_attributes_struct = unsafe { std::mem::zeroed() };

        if let Some(size) = metadata.size {
            attributes.size = size;
            attributes.flags |= sys::SSH_FILEXFER_ATTR_SIZE;
        }

        if let Some((uid, gid)) = metadata.uid_gid {
            attributes.uid = uid;
            attributes.gid = gid;
            attributes.flags |= sys::SSH_FILEXFER_ATTR_UIDGID;
        }

        if let Some(perms) = metadata.permissions {
            attributes.permissions = perms;
            attributes.flags |= sys::SSH_FILEXFER_ATTR_PERMISSIONS;
        }

        if let Some((atime, mtime)) = metadata.atime_mtime {
            attributes.atime = sftp_v3_timestamp(atime)?;
            attributes.mtime = sftp_v3_timestamp(mtime)?;
            attributes.flags |= sys::SSH_FILEXFER_ATTR_ACMODTIME;
        }

        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_setstat(sftp, filename.as_ptr(), &mut attributes) };
        SftpError::result(sftp, res, ())
    }

    /// Retrieve metadata for a file, traversing symlinks
    pub fn metadata(&self, filename: &str) -> SshResult<Metadata> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let attr = unsafe { sys::sftp_stat(sftp, filename.as_ptr()) };
        if attr.is_null() {
            Err(Error::Sftp(SftpError::from_session(sftp)))
        } else {
            Ok(Metadata { attr })
        }
    }

    /// Retrieve metadata for a file, without traversing symlinks.
    pub fn symlink_metadata(&self, filename: &str) -> SshResult<Metadata> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let attr = unsafe { sys::sftp_lstat(sftp, filename.as_ptr()) };
        if attr.is_null() {
            Err(Error::Sftp(SftpError::from_session(sftp)))
        } else {
            Ok(Metadata { attr })
        }
    }

    /// Rename a file from `filename` to `new_name`
    pub fn rename(&self, filename: &str, new_name: &str) -> SshResult<()> {
        let filename = CString::new(filename)?;
        let new_name = CString::new(new_name)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_rename(sftp, filename.as_ptr(), new_name.as_ptr()) };
        SftpError::result(sftp, res, ())
    }

    /// Remove a file or an empty directory
    pub fn remove_file(&self, filename: &str) -> SshResult<()> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_unlink(sftp, filename.as_ptr()) };
        SftpError::result(sftp, res, ())
    }

    /// Remove an empty directory
    pub fn remove_dir(&self, filename: &str) -> SshResult<()> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_rmdir(sftp, filename.as_ptr()) };
        SftpError::result(sftp, res, ())
    }

    /// Create a symlink on the server.
    /// `target` is the filename of the symlink to be created,
    /// and `dest` is the payload of the symlink.
    pub fn symlink(&self, target: &str, dest: &str) -> SshResult<()> {
        let target = CString::new(target)?;
        let dest = CString::new(dest)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_symlink(sftp, target.as_ptr(), dest.as_ptr()) };
        SftpError::result(sftp, res, ())
    }

    /// Open a file on the server.
    /// `accesstype` corresponds to the `open(2)` `flags` parameter
    /// and controls whether the file is opened for read/write and so on.
    /// `mode` specified the permission bits to use when creating a new file;
    /// they will be modified by the effective umask on the server side.
    pub fn open(
        &self,
        filename: &str,
        accesstype: OpenFlags,
        mode: sys::mode_t,
    ) -> SshResult<SftpFile> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_open(sftp, filename.as_ptr(), accesstype.bits(), mode) };
        if res.is_null() {
            Err(Error::Sftp(SftpError::from_session(sftp)))
        } else {
            Ok(SftpFile {
                file_inner: res,
                sftp: Arc::clone(&self.inner),
            })
        }
    }

    /// Open a directory to obtain directory entries
    pub fn open_dir(&self, filename: &str) -> SshResult<SftpDir> {
        let filename = CString::new(filename)?;
        let (_sess, sftp) = self.lock_session();
        let res = unsafe { sys::sftp_opendir(sftp, filename.as_ptr()) };
        if res.is_null() {
            Err(Error::Sftp(SftpError::from_session(sftp)))
        } else {
            Ok(SftpDir {
                dir_inner: res,
                sftp: Arc::clone(&self.inner),
            })
        }
    }

    /// Convenience function that reads all of the directory entries
    /// into a Vec.  If you need to deal with very large directories,
    /// you may wish to directly use [open_dir](#method.open_dir)
    /// and manually iterate the directory contents.
    pub fn read_dir(&self, filename: &str) -> SshResult<Vec<Metadata>> {
        self.read_dir_bounded(filename, usize::MAX)
    }

    /// Reads a directory while enforcing an explicit entry limit.
    /// The result is never silently truncated.
    pub fn read_dir_bounded(&self, filename: &str, max_entries: usize) -> SshResult<Vec<Metadata>> {
        let dir = self.open_dir(filename)?;
        let mut res = Vec::with_capacity(max_entries.min(1024));
        while let Some(item) = dir.read_dir() {
            if res.len() >= max_entries {
                return Err(Error::fatal(format!(
                    "SFTP directory entry count exceeds {max_entries}"
                )));
            }
            res.push(item?);
        }
        Ok(res)
    }
}

pub struct SftpFile {
    pub(crate) file_inner: sys::sftp_file,
    sftp: Arc<SftpInner>,
}

unsafe impl Send for SftpFile {}

impl Drop for SftpFile {
    fn drop(&mut self) {
        if self.file_inner.is_null() {
            return;
        }
        let (_sess, file) = self.lock_session();
        unsafe {
            sys::sftp_close(file);
        }
    }
}

impl SftpFile {
    fn lock_session(&self) -> (MutexGuard<'_, SessionHolder>, sys::sftp_file) {
        (self.sftp.sess.lock().unwrap(), self.file_inner)
    }

    pub fn set_blocking(&self, blocking: bool) {
        let (_sess, file) = self.lock_session();
        if blocking {
            unsafe { sys::sftp_file_set_blocking(file) }
        } else {
            unsafe { sys::sftp_file_set_nonblocking(file) }
        }
    }

    /// Retrieve metadata for the file
    pub fn metadata(&self) -> SshResult<Metadata> {
        let (_sess, file) = self.lock_session();
        let attr = unsafe { sys::sftp_fstat(file) };
        if attr.is_null() {
            Err(Error::Sftp(SftpError::from_session(self.sftp.sftp_inner)))
        } else {
            Ok(Metadata { attr })
        }
    }

    /// Attempts to synchronize remote file data when the server advertises
    /// the OpenSSH fsync extension. Servers without the extension require no
    /// request because libssh does not buffer file writes locally.
    pub fn sync_all(&self) -> std::io::Result<()> {
        let (_sess, file) = self.lock_session();
        let supported = unsafe {
            sys::sftp_extension_supported(
                self.sftp.sftp_inner,
                FSYNC_EXTENSION_NAME.as_ptr().cast(),
                FSYNC_EXTENSION_VERSION.as_ptr().cast(),
            )
        };
        if supported != 1 {
            return Ok(());
        }

        let res = unsafe { sys::sftp_fsync(file) };
        if res == sys::SSH_OK as i32 {
            Ok(())
        } else {
            Err(io_err_from_sftp(self.sftp.sftp_inner, "fsync"))
        }
    }
}

fn io_err_from_sftp(sftp: sys::sftp_session, reason: &str) -> std::io::Error {
    use std::io::ErrorKind;
    let res = unsafe { sys::sftp_get_error(sftp) };
    let kind = match res as u32 {
        sys::SSH_FX_OK => ErrorKind::Other,
        sys::SSH_FX_EOF => ErrorKind::UnexpectedEof,
        sys::SSH_FX_NO_SUCH_FILE => ErrorKind::NotFound,
        sys::SSH_FX_PERMISSION_DENIED => ErrorKind::PermissionDenied,
        sys::SSH_FX_FAILURE => ErrorKind::Other,
        sys::SSH_FX_BAD_MESSAGE => ErrorKind::Other,
        sys::SSH_FX_NO_CONNECTION => ErrorKind::NotConnected,
        sys::SSH_FX_CONNECTION_LOST => ErrorKind::ConnectionReset,
        sys::SSH_FX_OP_UNSUPPORTED => ErrorKind::Unsupported,
        sys::SSH_FX_INVALID_HANDLE => ErrorKind::Other,
        sys::SSH_FX_NO_SUCH_PATH => ErrorKind::NotFound,
        sys::SSH_FX_FILE_ALREADY_EXISTS => ErrorKind::AlreadyExists,
        sys::SSH_FX_WRITE_PROTECT => ErrorKind::Other,
        sys::SSH_FX_NO_MEDIA => ErrorKind::Other,
        _ => ErrorKind::Other,
    };
    std::io::Error::new(kind, format!("{}: sftp error code {}", reason, res))
}

impl std::io::Read for SftpFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let (_sess, file) = self.lock_session();

        let res = unsafe { sys::sftp_read(file, buf.as_mut_ptr() as _, buf.len()) };

        if res >= 0 {
            Ok(res as usize)
        } else {
            let err = io_err_from_sftp(self.sftp.sftp_inner, "read");
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                Ok(0)
            } else {
                Err(err)
            }
        }
    }
}

impl std::io::Write for SftpFile {
    fn flush(&mut self) -> std::io::Result<()> {
        self.sync_all()
    }

    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let (_sess, file) = self.lock_session();

        let res = unsafe { sys::sftp_write(file, buf.as_ptr() as _, buf.len()) };

        if res >= 0 {
            Ok(res as usize)
        } else {
            let err = io_err_from_sftp(self.sftp.sftp_inner, "write");
            if err.kind() == std::io::ErrorKind::UnexpectedEof {
                Ok(0)
            } else {
                Err(err)
            }
        }
    }
}

fn checked_seek_target(base: u64, offset: i64) -> std::io::Result<u64> {
    let target = if offset < 0 {
        base.checked_sub(offset.unsigned_abs())
    } else {
        base.checked_add(offset as u64)
    };
    target.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "SFTP seek offset is outside the supported file range",
        )
    })
}

impl std::io::Seek for SftpFile {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        match pos {
            std::io::SeekFrom::Start(p) => {
                let (_sess, file) = self.lock_session();
                let res = unsafe { sys::sftp_seek64(file, p) };
                if res == 0 {
                    Ok(p)
                } else {
                    Err(io_err_from_sftp(self.sftp.sftp_inner, "seek"))
                }
            }
            std::io::SeekFrom::End(p) => {
                let end = self.metadata()?.len().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "metadata didn't return the length",
                    )
                })?;
                let target = checked_seek_target(end, p)?;
                let (_sess, file) = self.lock_session();
                let res = unsafe { sys::sftp_seek64(file, target) };
                if res == 0 {
                    Ok(target)
                } else {
                    Err(io_err_from_sftp(self.sftp.sftp_inner, "seek"))
                }
            }
            std::io::SeekFrom::Current(p) => {
                let (_sess, file) = self.lock_session();
                let current = unsafe { sys::sftp_tell64(file) };
                let target = checked_seek_target(current, p)?;
                let res = unsafe { sys::sftp_seek64(file, target) };
                if res == 0 {
                    Ok(target)
                } else {
                    Err(io_err_from_sftp(self.sftp.sftp_inner, "seek"))
                }
            }
        }
    }

    fn stream_position(&mut self) -> std::io::Result<u64> {
        let (_sess, file) = self.lock_session();
        let current = unsafe { sys::sftp_tell64(file) };
        Ok(current)
    }
}

/// Change multiple file attributes at once.
/// If a field is_some, then its value will be applied
/// to the file on the server side.  If it is_none, then
/// that particular field will be left unmodified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetAttributes {
    /// Change the file length
    pub size: Option<u64>,
    /// Change the ownership (chown)
    pub uid_gid: Option<(sys::uid_t, sys::gid_t)>,
    /// Change the permissions (chmod)
    pub permissions: Option<u32>,
    /// Note that the protocol/libssh implementation has
    /// 1-second granularity for access and mtime
    pub atime_mtime: Option<(SystemTime, SystemTime)>,
}

/// Represents metadata about a file.
/// libssh returns this in a couple of contexts, and not all
/// fields are used in all contexts.
pub struct Metadata {
    attr: sys::sftp_attributes,
}

impl Drop for Metadata {
    fn drop(&mut self) {
        unsafe { sys::sftp_attributes_free(self.attr) }
    }
}

impl Metadata {
    fn attr(&self) -> &sys::sftp_attributes_struct {
        unsafe { &*self.attr }
    }

    pub fn len(&self) -> Option<u64> {
        if self.attr().flags & sys::SSH_FILEXFER_ATTR_SIZE != 0 {
            Some(self.attr().size)
        } else {
            None
        }
    }

    fn name_helper(&self, name: *const c_char) -> Option<&str> {
        if name.is_null() {
            None
        } else {
            unsafe { CStr::from_ptr(name) }.to_str().ok()
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.name_helper(self.attr().name)
    }

    /// libssh docs say that this is the ls -l output on openssh
    /// servers, but is unreliable with other servers
    pub fn long_name(&self) -> Option<&str> {
        self.name_helper(self.attr().longname)
    }

    /// Set in openssh version 4 and up
    pub fn owner(&self) -> Option<&str> {
        self.name_helper(self.attr().owner)
    }

    /// Set in openssh version 4 and up
    pub fn group(&self) -> Option<&str> {
        self.name_helper(self.attr().group)
    }

    /// Flags the indicate which attributes are present.
    /// Is a bitmask of `SSH_FILEXFER_ATTR_XXX` constants
    pub fn flags(&self) -> u32 {
        self.attr().flags
    }

    /// The owner uid of the file
    pub fn uid(&self) -> Option<u32> {
        if self.attr().flags & sys::SSH_FILEXFER_ATTR_UIDGID != 0 {
            Some(self.attr().uid)
        } else {
            None
        }
    }

    /// The owner gid of the file
    pub fn gid(&self) -> Option<u32> {
        if self.attr().flags & sys::SSH_FILEXFER_ATTR_UIDGID != 0 {
            Some(self.attr().gid)
        } else {
            None
        }
    }

    /// The unix mode_t permission bits
    pub fn permissions(&self) -> Option<u32> {
        if self.attr().flags & sys::SSH_FILEXFER_ATTR_PERMISSIONS != 0 {
            Some(self.attr().permissions)
        } else {
            None
        }
    }

    /// The type of the file decoded from the permissions
    pub fn file_type(&self) -> Option<FileType> {
        if self.attr().flags & sys::SSH_FILEXFER_ATTR_PERMISSIONS != 0 {
            Some(match self.attr().type_ as u32 {
                sys::SSH_FILEXFER_TYPE_SPECIAL => FileType::Special,
                sys::SSH_FILEXFER_TYPE_SYMLINK => FileType::Symlink,
                sys::SSH_FILEXFER_TYPE_REGULAR => FileType::Regular,
                sys::SSH_FILEXFER_TYPE_DIRECTORY => FileType::Directory,
                sys::SSH_FILEXFER_TYPE_UNKNOWN | _ => FileType::Unknown,
            })
        } else {
            None
        }
    }

    /// The last-accessed time
    pub fn accessed(&self) -> Option<SystemTime> {
        let (seconds, nanoseconds) = if self.attr().flags & sys::SSH_FILEXFER_ATTR_ACCESSTIME != 0 {
            (
                self.attr().atime64,
                if self.attr().flags & sys::SSH_FILEXFER_ATTR_SUBSECOND_TIMES != 0 {
                    self.attr().atime_nseconds
                } else {
                    0
                },
            )
        } else if self.attr().flags & sys::SSH_FILEXFER_ATTR_ACMODTIME != 0 {
            (self.attr().atime.into(), 0)
        } else {
            return None;
        };
        system_time_from_sftp_timestamp(seconds, nanoseconds)
    }

    /// The file creation time
    pub fn created(&self) -> Option<SystemTime> {
        if self.attr().flags & sys::SSH_FILEXFER_ATTR_CREATETIME == 0 {
            return None;
        }
        let nanoseconds = if self.attr().flags & sys::SSH_FILEXFER_ATTR_SUBSECOND_TIMES != 0 {
            self.attr().createtime_nseconds
        } else {
            0
        };
        system_time_from_sftp_timestamp(self.attr().createtime, nanoseconds)
    }

    /// The file modification time
    pub fn modified(&self) -> Option<SystemTime> {
        let (seconds, nanoseconds) = if self.attr().flags & sys::SSH_FILEXFER_ATTR_MODIFYTIME != 0 {
            (
                self.attr().mtime64,
                if self.attr().flags & sys::SSH_FILEXFER_ATTR_SUBSECOND_TIMES != 0 {
                    self.attr().mtime_nseconds
                } else {
                    0
                },
            )
        } else if self.attr().flags & sys::SSH_FILEXFER_ATTR_ACMODTIME != 0 {
            (self.attr().mtime.into(), 0)
        } else {
            return None;
        };
        system_time_from_sftp_timestamp(seconds, nanoseconds)
    }
}

pub struct SftpDir {
    pub(crate) dir_inner: sys::sftp_dir,
    sftp: Arc<SftpInner>,
}

unsafe impl Send for SftpDir {}

impl Drop for SftpDir {
    fn drop(&mut self) {
        if self.dir_inner.is_null() {
            return;
        }
        let (_sess, dir) = self.lock_session();
        unsafe {
            sys::sftp_closedir(dir);
        }
    }
}

impl SftpDir {
    fn lock_session(&self) -> (MutexGuard<'_, SessionHolder>, sys::sftp_dir) {
        (self.sftp.sess.lock().unwrap(), self.dir_inner)
    }

    /// Read the next entry from the directory.
    /// Returns None if there are no more entries.
    pub fn read_dir(&self) -> Option<SshResult<Metadata>> {
        let (_sess, dir) = self.lock_session();
        let attr = unsafe { sys::sftp_readdir(self.sftp.sftp_inner, dir) };
        if attr.is_null() {
            if unsafe { sys::sftp_dir_eof(dir) } == 1 {
                None
            } else {
                Some(Err(Error::Sftp(SftpError::from_session(
                    self.sftp.sftp_inner,
                ))))
            }
        } else {
            Some(Ok(Metadata { attr }))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileType {
    Special,
    Symlink,
    Regular,
    Directory,
    Unknown,
}

bitflags::bitflags! {
     /// Bitflags that indicate options for opening a sftp file.
    pub struct OpenFlags: c_int {
        /// The file should be opened as read-only.
        const READ_ONLY = libc::O_RDONLY;
        /// The file should be opened as write-only.
        const WRITE_ONLY = libc::O_WRONLY;
        /// The file should be opened as read and write.
        ///
        /// Note that this is a different value than `READ_ONLY | WRITE_ONLY`, which is a logic error.
        const READ_WRITE = libc::O_RDWR;
        /// Create the file if it does not exist.
        const CREATE = libc::O_CREAT;
        /// When used with `CREATE`, this flag ensures that a new file is created.
        const EXCLUSIVE = libc::O_EXCL;
        /// If the file exists, truncate it.
        const TRUNCATE = libc::O_TRUNC;
        /// Before each write, the file offset is set to the end of the file.
        const APPEND = libc::O_APPEND;
        /// Create a new file, failing if it already exists.
        ///
        /// This is an alias for `CREATE | EXCLUSIVE`.
        const CREATE_NEW = libc::O_CREAT | libc::O_EXCL;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sftp_v3_timestamp_accepts_its_full_wire_range() {
        assert_eq!(sftp_v3_timestamp(SystemTime::UNIX_EPOCH).unwrap(), 0);
        assert_eq!(
            sftp_v3_timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(u32::MAX)))
                .unwrap(),
            u32::MAX
        );
    }

    #[test]
    fn sftp_v3_timestamp_rejects_unrepresentable_times_without_panicking() {
        let before_epoch = SystemTime::UNIX_EPOCH
            .checked_sub(Duration::from_secs(1))
            .unwrap();
        assert!(sftp_v3_timestamp(before_epoch)
            .unwrap_err()
            .to_string()
            .contains("before Unix epoch"));

        let after_wire_range =
            SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(u32::MAX) + 1);
        assert!(sftp_v3_timestamp(after_wire_range)
            .unwrap_err()
            .to_string()
            .contains("32-bit seconds range"));
    }

    #[test]
    fn remote_sftp_timestamp_overflow_is_treated_as_unavailable() {
        assert_eq!(
            system_time_from_sftp_timestamp(1, 500_000_000),
            Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1_500))
        );
        assert_eq!(system_time_from_sftp_timestamp(u64::MAX, u32::MAX), None);
    }

    #[test]
    fn sftp_seek_offsets_reject_underflow_and_overflow() {
        assert_eq!(checked_seek_target(10, -10).unwrap(), 0);
        assert_eq!(checked_seek_target(10, 5).unwrap(), 15);
        assert_eq!(
            checked_seek_target(0, -1).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(
            checked_seek_target(u64::MAX, 1).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(
            checked_seek_target(0, i64::MIN).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn file_handles_keep_the_sftp_session_alive() {
        let session = crate::Session::new().unwrap();
        let sftp = Sftp::new(Arc::clone(&session.sess), std::ptr::null_mut());
        let owner = Arc::downgrade(&sftp.inner);
        let file = SftpFile {
            file_inner: std::ptr::null_mut(),
            sftp: Arc::clone(&sftp.inner),
        };

        drop(sftp);
        assert!(owner.upgrade().is_some());
        drop(file);
        assert!(owner.upgrade().is_none());
    }
}

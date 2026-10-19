use crate::RawSocket;
use std::net::TcpStream;

/// libssh normally borrows SSH_OPTIONS_FD, but its transport-error paths can
/// close it. Keep that raw slot out of Rust's OwnedFd/OwnedSocket, and retain a
/// duplicate so its identity cannot be recycled before final cleanup.
pub(crate) struct OwnedTcpSocket {
    socket: RawSocket,
    keeper: TcpStream,
}

impl OwnedTcpSocket {
    pub(crate) fn new(stream: TcpStream) -> std::io::Result<Self> {
        let keeper = stream.try_clone()?;
        #[cfg(unix)]
        let socket = std::os::fd::IntoRawFd::into_raw_fd(stream);
        #[cfg(windows)]
        let socket = std::os::windows::io::IntoRawSocket::into_raw_socket(stream);
        Ok(Self { socket, keeper })
    }

    pub(crate) fn raw_socket(&self) -> RawSocket {
        self.socket
    }

    #[cfg(unix)]
    fn still_owns_socket(&self) -> bool {
        use std::os::fd::AsRawFd;
        let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
        let mut retained = std::mem::MaybeUninit::<libc::stat>::uninit();
        // Both fstat calls only query raw handles. The keeper pins the original
        // socket inode even if libssh closed/recycled its other descriptor.
        unsafe {
            libc::fstat(self.socket, current.as_mut_ptr()) == 0
                && libc::fstat(self.keeper.as_raw_fd(), retained.as_mut_ptr()) == 0
                && {
                    let current = current.assume_init();
                    let retained = retained.assume_init();
                    current.st_mode & libc::S_IFMT == libc::S_IFSOCK
                        && current.st_dev == retained.st_dev
                        && current.st_ino == retained.st_ino
                        && (current.st_ino != 0
                            || same_socket_addresses(self.socket, self.keeper.as_raw_fd()))
                }
        }
    }

    #[cfg(windows)]
    fn still_owns_socket(&self) -> bool {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Foundation::CompareObjectHandles;
        // Compare kernel-object identity, not the reusable numeric socket value.
        unsafe { CompareObjectHandles(self.socket as _, self.keeper.as_raw_socket() as _) != 0 }
    }
}

#[cfg(unix)]
fn same_socket_addresses(left: RawSocket, right: RawSocket) -> bool {
    fn address(socket: RawSocket, peer: bool) -> Option<Vec<u8>> {
        let mut kind: libc::c_int = 0;
        let mut kind_length = std::mem::size_of_val(&kind) as libc::socklen_t;
        let mut storage = std::mem::MaybeUninit::<libc::sockaddr_storage>::zeroed();
        let mut length = std::mem::size_of_val(&storage) as libc::socklen_t;
        unsafe {
            if libc::getsockopt(
                socket,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                (&mut kind as *mut libc::c_int).cast(),
                &mut kind_length,
            ) != 0
                || kind != libc::SOCK_STREAM
            {
                return None;
            }
            let result = if peer {
                libc::getpeername(socket, storage.as_mut_ptr().cast(), &mut length)
            } else {
                libc::getsockname(socket, storage.as_mut_ptr().cast(), &mut length)
            };
            if result != 0 || length == 0 || length as usize > std::mem::size_of_val(&storage) {
                return None;
            }
            Some(std::slice::from_raw_parts(storage.as_ptr().cast(), length as usize).to_vec())
        }
    }
    // Some BSD socket stat implementations omit an inode. The retained TCP
    // connection then pins its local/peer pair instead of accepting inode 0.
    [false, true].iter().copied().all(|peer| {
        matches!((address(left, peer), address(right, peer)), (Some(left), Some(right)) if left == right)
    })
}

impl Drop for OwnedTcpSocket {
    fn drop(&mut self) {
        if !self.still_owns_socket() {
            return;
        }
        // Native users have already gone away (or setup rejected the option).
        // Reclaim only our still-open socket, never an unrelated recycled slot.
        #[cfg(unix)]
        unsafe {
            use std::os::fd::FromRawFd;
            drop(TcpStream::from_raw_fd(self.socket));
        }
        #[cfg(windows)]
        unsafe {
            use std::os::windows::io::FromRawSocket;
            drop(TcpStream::from_raw_socket(self.socket));
        }
    }
}

#[cfg(test)]
#[path = "../../../test/rust/libssh-rs/unit/owned_socket.rs"]
mod tests;

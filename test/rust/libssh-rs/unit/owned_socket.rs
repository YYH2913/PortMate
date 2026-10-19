use super::*;
use std::io::Read;
#[cfg(unix)]
use std::io::Write;
use std::net::TcpListener;

fn sockets() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let stream = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let peer = listener.accept().unwrap().0;
    peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    (stream, peer)
}

fn simulate_native_close(socket: RawSocket) {
    #[cfg(unix)]
    unsafe {
        use std::os::fd::FromRawFd;
        drop(TcpStream::from_raw_fd(socket));
    }
    #[cfg(windows)]
    unsafe {
        use std::os::windows::io::FromRawSocket;
        drop(TcpStream::from_raw_socket(socket));
    }
}

#[test]
fn supplied_socket_is_closed_when_native_code_only_borrowed_it() {
    let (stream, mut peer) = sockets();
    peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let socket = OwnedTcpSocket::new(stream).unwrap();
    assert!(socket.still_owns_socket());
    drop(socket);
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}

#[test]
fn native_transport_error_may_close_the_supplied_socket_before_guard_drop() {
    let (stream, mut peer) = sockets();
    peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let socket = OwnedTcpSocket::new(stream).unwrap();
    simulate_native_close(socket.raw_socket());
    assert!(!socket.still_owns_socket());
    drop(socket);
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}

#[cfg(unix)]
#[test]
fn cleanup_never_closes_an_unrelated_recycled_descriptor() {
    use std::os::fd::AsRawFd;
    let (original, _) = sockets();
    let socket = OwnedTcpSocket::new(original).unwrap();
    let (mut unrelated, mut peer) = sockets();
    // Atomically simulate close+reuse. A separate close would let another test
    // allocate this slot before dup2 and make the fixture close its descriptor.
    assert_eq!(
        unsafe { libc::dup2(unrelated.as_raw_fd(), socket.raw_socket()) },
        socket.raw_socket()
    );
    assert!(!socket.still_owns_socket());
    let recycled = socket.raw_socket();
    drop(socket);
    assert!(unsafe { libc::fcntl(recycled, libc::F_GETFD) } >= 0);
    unrelated.write_all(b"alive").unwrap();
    let mut bytes = [0; 5];
    peer.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"alive");
    simulate_native_close(recycled);
}

#[cfg(unix)]
#[test]
fn zero_inode_fallback_compares_both_live_tcp_endpoints() {
    use std::os::fd::AsRawFd;
    let (stream, _) = sockets();
    let duplicate = stream.try_clone().unwrap();
    let (unrelated, _) = sockets();
    assert!(same_socket_addresses(
        stream.as_raw_fd(),
        duplicate.as_raw_fd()
    ));
    assert!(!same_socket_addresses(
        stream.as_raw_fd(),
        unrelated.as_raw_fd()
    ));
}

#[test]
fn unconnected_sessions_reclaim_supplied_sockets_without_leaking() {
    let (stream, mut peer) = sockets();
    peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    let session = crate::Session::new().unwrap();
    session.set_owned_tcp_stream(stream).unwrap();
    drop(session);
    assert_eq!(peer.read(&mut [0]).unwrap(), 0);
}

#[test]
fn replacing_an_owned_socket_is_rejected_and_reclaims_only_the_new_stream() {
    let session = crate::Session::new().unwrap();
    let (first, mut first_peer) = sockets();
    session.set_owned_tcp_stream(first).unwrap();
    let (replacement, mut rejected_peer) = sockets();
    rejected_peer
        .set_read_timeout(Some(std::time::Duration::from_secs(1)))
        .unwrap();
    assert!(session.set_owned_tcp_stream(replacement).is_err());
    assert_eq!(rejected_peer.read(&mut [0]).unwrap(), 0);
    drop(session);
    assert_eq!(first_peer.read(&mut [0]).unwrap(), 0);
}

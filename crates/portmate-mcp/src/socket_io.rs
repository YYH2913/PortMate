use std::io::{self, Read};
use std::net::TcpStream;
use std::time::Instant;

pub(super) fn read_stream_chunk_before(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
    timeout_message: &'static str,
) -> io::Result<usize> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
    }
    if let Err(error) =
        stream.set_read_timeout(Some(remaining.max(std::time::Duration::from_millis(1))))
    {
        if error.kind() == io::ErrorKind::InvalidInput {
            return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
        }
        return Err(error);
    }
    stream.read(buffer).map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(io::ErrorKind::TimedOut, timeout_message)
        } else {
            error
        }
    })
}

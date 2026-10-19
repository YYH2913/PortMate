use std::io::{self, Read};
use std::net::TcpStream;
use std::time::{Duration, Instant};

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
            // macOS may reject the socket timeout value. Preserve the actual
            // absolute deadline instead of treating setup failure as elapsed
            // time, and never fall back to an unbounded blocking read.
            return read_nonblocking_before(stream, buffer, deadline, timeout_message);
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

fn read_nonblocking_before(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
    timeout_message: &'static str,
) -> io::Result<usize> {
    stream.set_nonblocking(true)?;
    let result = loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
        }
        match stream.read(buffer) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(remaining.min(Duration::from_millis(2)));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => break result,
        }
    };
    let restored = stream.set_nonblocking(false);
    match (result, restored) {
        (Ok(read), Ok(())) => Ok(read),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}

#[cfg(test)]
#[path = "../../../test/rust/portmate-mcp/unit/socket_io.rs"]
mod tests;

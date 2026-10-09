#[cfg(test)]
use tokio::net::tcp::OwnedWriteHalf;

#[cfg(test)]
pub(super) fn box_tcp_write_half(writer: OwnedWriteHalf) -> TcpWriteHalf {
    Box::new(writer)
}

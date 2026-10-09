#[cfg(test)]
pub(super) fn read_http_request_with_timeout(
    stream: &mut TcpStream,
    timeout: Duration,
) -> Result<HttpRequest> {
    read_http_request_with_timeout_and_body_limit(stream, timeout, |_, _, _| {
        Ok(MAX_HTTP_BODY_BYTES)
    })
}

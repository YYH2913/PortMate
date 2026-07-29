use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use crate::socket_io::read_stream_chunk_before;

const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_HTTP_CHUNK_FRAMING_BYTES: usize = 64 * 1024;
const MAX_HTTP_HEADERS: usize = 128;
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub(super) struct HttpRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) headers: HashMap<String, String>,
    pub(super) body: Vec<u8>,
}

pub(super) fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    read_http_request_with_timeout(stream, HTTP_REQUEST_TIMEOUT)
}

pub(super) fn read_http_request_with_timeout(
    stream: &mut TcpStream,
    timeout: Duration,
) -> Result<HttpRequest> {
    let deadline = Instant::now() + timeout;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let read = read_stream_chunk_before(
            stream,
            &mut buffer,
            deadline,
            "HTTP request deadline exceeded",
        )?;
        if read == 0 {
            return Err(anyhow!("connection closed before HTTP headers"));
        }
        raw.extend_from_slice(&buffer[..read]);
        if let Some(index) = find_header_end(&raw) {
            if index + 4 > MAX_HTTP_HEADER_BYTES {
                return Err(anyhow!("HTTP headers exceed the 64 KiB limit"));
            }
            break index;
        }
        if raw.len() > MAX_HTTP_HEADER_BYTES {
            return Err(anyhow!("HTTP headers exceed the 64 KiB limit"));
        }
    };

    let body_start = header_end + 4;
    let mut parsed_headers = [httparse::EMPTY_HEADER; MAX_HTTP_HEADERS];
    let mut parsed_request = httparse::Request::new(&mut parsed_headers);
    let parsed_bytes = match parsed_request
        .parse(&raw[..body_start])
        .map_err(|error| anyhow!("invalid HTTP headers: {error}"))?
    {
        httparse::Status::Complete(parsed_bytes) => parsed_bytes,
        httparse::Status::Partial => return Err(anyhow!("incomplete HTTP headers")),
    };
    if parsed_bytes != body_start {
        return Err(anyhow!("invalid bytes after HTTP headers"));
    }
    let method = parsed_request
        .method
        .ok_or_else(|| anyhow!("missing HTTP method"))?
        .to_string();
    let path = parsed_request
        .path
        .ok_or_else(|| anyhow!("missing HTTP path"))?
        .to_string();
    if parsed_request.version.is_none() {
        return Err(anyhow!("missing HTTP version"));
    }
    let mut headers = HashMap::new();
    for header in parsed_request.headers.iter() {
        let value = std::str::from_utf8(header.value)
            .map_err(|_| anyhow!("HTTP header `{}` is not valid UTF-8", header.name))?;
        insert_http_header(&mut headers, header.name, value)?;
    }
    let initial_body = raw.get(body_start..).unwrap_or_default();
    let body = match headers.get("transfer-encoding") {
        Some(transfer_encoding) => {
            if headers.contains_key("content-length") {
                return Err(anyhow!(
                    "HTTP request cannot combine Transfer-Encoding and Content-Length"
                ));
            }
            if !transfer_encoding.eq_ignore_ascii_case("chunked") {
                return Err(anyhow!(
                    "unsupported Transfer-Encoding; only chunked is accepted"
                ));
            }
            read_http_chunked_body(stream, initial_body, &mut buffer, deadline)?
        }
        None => {
            let content_length = headers
                .get("content-length")
                .map(|value| value.parse::<usize>())
                .transpose()
                .map_err(|error| anyhow!("invalid Content-Length: {error}"))?
                .unwrap_or(0);
            read_http_content_length_body(
                stream,
                initial_body,
                content_length,
                &mut buffer,
                deadline,
            )?
        }
    };
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn read_http_content_length_body(
    stream: &mut TcpStream,
    initial_body: &[u8],
    content_length: usize,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<Vec<u8>> {
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err(anyhow!("HTTP body is too large"));
    }
    let mut body = initial_body.to_vec();
    if body.len() > content_length {
        return Err(anyhow!(
            "HTTP request contains bytes after its declared body"
        ));
    }
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let read_buffer_length = remaining.min(buffer.len());
        let read = read_stream_chunk_before(
            stream,
            &mut buffer[..read_buffer_length],
            deadline,
            "HTTP request deadline exceeded",
        )?;
        if read == 0 {
            return Err(anyhow!("connection closed before HTTP body"));
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Ok(body)
}

fn read_http_chunked_body(
    stream: &mut TcpStream,
    initial_body: &[u8],
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<Vec<u8>> {
    let mut encoded = initial_body.to_vec();
    let mut position = 0;
    let mut framing_bytes = 0;
    let mut body = Vec::new();

    loop {
        let line = read_http_chunk_line(
            stream,
            &mut encoded,
            &mut position,
            buffer,
            deadline,
            &mut framing_bytes,
        )?;
        let extension_start = line.iter().position(|byte| *byte == b';');
        let (size_bytes, extension) = extension_start
            .map_or((line.as_slice(), None), |separator| {
                (&line[..separator], Some(&line[separator + 1..]))
            });
        if size_bytes.is_empty() || !size_bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err(anyhow!("invalid HTTP chunk size"));
        }
        if let Some(extension) = extension {
            if extension.is_empty()
                || extension
                    .iter()
                    .any(|byte| !byte.is_ascii() || byte.is_ascii_control())
            {
                return Err(anyhow!("invalid HTTP chunk extension"));
            }
        }
        let size = std::str::from_utf8(size_bytes)
            .ok()
            .and_then(|value| usize::from_str_radix(value, 16).ok())
            .ok_or_else(|| anyhow!("invalid HTTP chunk size"))?;
        if size > MAX_HTTP_BODY_BYTES.saturating_sub(body.len()) {
            return Err(anyhow!("HTTP body is too large"));
        }
        if size == 0 {
            read_http_chunk_trailers(
                stream,
                &mut encoded,
                &mut position,
                buffer,
                deadline,
                &mut framing_bytes,
            )?;
            if position != encoded.len() {
                return Err(anyhow!(
                    "HTTP request contains bytes after its chunked body"
                ));
            }
            return Ok(body);
        }

        ensure_http_chunk_bytes(stream, &mut encoded, position, size + 2, buffer, deadline)?;
        let chunk_end = position + size;
        if encoded.get(chunk_end..chunk_end + 2) != Some(b"\r\n") {
            return Err(anyhow!("HTTP chunk data is not terminated by CRLF"));
        }
        body.extend_from_slice(&encoded[position..chunk_end]);
        position = chunk_end + 2;
        framing_bytes += 2;
        if framing_bytes > MAX_HTTP_CHUNK_FRAMING_BYTES {
            return Err(anyhow!("HTTP chunk framing exceeds the 64 KiB limit"));
        }
    }
}

fn read_http_chunk_trailers(
    stream: &mut TcpStream,
    encoded: &mut Vec<u8>,
    position: &mut usize,
    buffer: &mut [u8],
    deadline: Instant,
    framing_bytes: &mut usize,
) -> Result<()> {
    let mut trailer_count = 0;
    loop {
        let line =
            read_http_chunk_line(stream, encoded, position, buffer, deadline, framing_bytes)?;
        if line.is_empty() {
            return Ok(());
        }
        trailer_count += 1;
        if trailer_count > MAX_HTTP_HEADERS {
            return Err(anyhow!("HTTP chunk trailers exceed the 128-field limit"));
        }
        let Some(separator) = line.iter().position(|byte| *byte == b':') else {
            return Err(anyhow!("invalid HTTP chunk trailer"));
        };
        if separator == 0 || !line[..separator].iter().copied().all(is_http_token_byte) {
            return Err(anyhow!("invalid HTTP chunk trailer"));
        }
        let trailer_name = std::str::from_utf8(&line[..separator])?.to_ascii_lowercase();
        if is_single_value_http_header(&trailer_name)
            || line[separator + 1..]
                .iter()
                .any(|byte| *byte != b'\t' && (byte.is_ascii_control() || *byte == 0x7f))
        {
            return Err(anyhow!("invalid HTTP chunk trailer"));
        }
    }
}

fn read_http_chunk_line(
    stream: &mut TcpStream,
    encoded: &mut Vec<u8>,
    position: &mut usize,
    buffer: &mut [u8],
    deadline: Instant,
    framing_bytes: &mut usize,
) -> Result<Vec<u8>> {
    loop {
        if let Some(line_end) = encoded[*position..]
            .windows(2)
            .position(|window| window == b"\r\n")
        {
            let line = encoded[*position..*position + line_end].to_vec();
            *position += line_end + 2;
            *framing_bytes += line_end + 2;
            if *framing_bytes > MAX_HTTP_CHUNK_FRAMING_BYTES {
                return Err(anyhow!("HTTP chunk framing exceeds the 64 KiB limit"));
            }
            return Ok(line);
        }
        if encoded.len().saturating_sub(*position) >= MAX_HTTP_CHUNK_FRAMING_BYTES {
            return Err(anyhow!("HTTP chunk framing exceeds the 64 KiB limit"));
        }
        let remaining = MAX_HTTP_CHUNK_FRAMING_BYTES + 1 - encoded.len() + *position;
        let read_length = remaining.min(buffer.len());
        let read = read_stream_chunk_before(
            stream,
            &mut buffer[..read_length],
            deadline,
            "HTTP request deadline exceeded",
        )?;
        if read == 0 {
            return Err(anyhow!("connection closed before HTTP chunk framing"));
        }
        encoded.extend_from_slice(&buffer[..read]);
    }
}

fn ensure_http_chunk_bytes(
    stream: &mut TcpStream,
    encoded: &mut Vec<u8>,
    position: usize,
    required: usize,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<()> {
    while encoded.len().saturating_sub(position) < required {
        let remaining = required - encoded.len().saturating_sub(position);
        let read_length = remaining.min(buffer.len());
        let read = read_stream_chunk_before(
            stream,
            &mut buffer[..read_length],
            deadline,
            "HTTP request deadline exceeded",
        )?;
        if read == 0 {
            return Err(anyhow!("connection closed before HTTP chunk data"));
        }
        encoded.extend_from_slice(&buffer[..read]);
    }
    Ok(())
}

fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn insert_http_header(
    headers: &mut HashMap<String, String>,
    name: &str,
    value: &str,
) -> Result<()> {
    let name = name.to_ascii_lowercase();
    let value = value.trim();
    if let Some(existing) = headers.get_mut(&name) {
        if is_single_value_http_header(&name) {
            return Err(anyhow!("duplicate HTTP header `{name}`"));
        }
        existing.push_str(", ");
        existing.push_str(value);
    } else {
        headers.insert(name, value.to_string());
    }
    Ok(())
}

fn is_single_value_http_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "content-length"
            | "content-type"
            | "host"
            | "mcp-protocol-version"
            | "origin"
            | "transfer-encoding"
            | "x-portmate-mcp-token"
    )
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

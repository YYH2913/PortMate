use crate::http_protocol::MCP_PROTOCOL_VERSION;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{self, Write};

pub(crate) const MAX_JSON_RPC_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

struct BoundedJsonWriter {
    bytes: Vec<u8>,
    max_bytes: usize,
    exceeded: bool,
}

impl BoundedJsonWriter {
    fn new(max_bytes: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(max_bytes.min(8192)),
            max_bytes,
            exceeded: false,
        }
    }
}

impl Write for BoundedJsonWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.len() > self.max_bytes.saturating_sub(self.bytes.len()) {
            self.exceeded = true;
            return Err(io::Error::other("JSON response exceeds its byte limit"));
        }
        self.bytes.extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn try_encode_json_with_limit(
    value: &Value,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>> {
    let mut writer = BoundedJsonWriter::new(max_bytes);
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(Some(writer.bytes)),
        Err(_) if writer.exceeded => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn encode_json_rpc_response(response: &Value, max_bytes: usize) -> Result<Vec<u8>> {
    if let Some(encoded) = try_encode_json_with_limit(response, max_bytes)? {
        return Ok(encoded);
    }
    let response_id = response
        .as_object()
        .and_then(|object| object.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
    let fallback = json!({
        "jsonrpc": "2.0",
        "id": response_id,
        "error": {
            "code": -32603,
            "message": format!("JSON-RPC response exceeds the {max_bytes}-byte limit")
        }
    });
    try_encode_json_with_limit(&fallback, max_bytes)?
        .ok_or_else(|| anyhow!("JSON-RPC response limit is too small to encode its overflow error"))
}

pub(crate) fn json_rpc_http_body(response: &Value) -> String {
    match encode_json_rpc_response(response, MAX_JSON_RPC_RESPONSE_BYTES) {
        Ok(encoded) => String::from_utf8(encoded).unwrap_or_else(|error| {
            internal_json_rpc_error_body(format!("failed to encode JSON-RPC response: {error}"))
        }),
        Err(error) => internal_json_rpc_error_body(error.to_string()),
    }
}

fn internal_json_rpc_error_body(message: impl Into<String>) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": null,
        "error": { "code": -32603, "message": message.into() }
    })
    .to_string()
}

pub(crate) fn http_response(status: u16, reason: &str, body: &str, origin: Option<&str>) -> String {
    http_response_with_protocol(status, reason, body, origin, MCP_PROTOCOL_VERSION)
}

pub(crate) fn http_response_with_protocol(
    status: u16,
    reason: &str,
    body: &str,
    origin: Option<&str>,
    protocol_version: &str,
) -> String {
    let content_type = if body.is_empty() {
        "text/plain"
    } else {
        "application/json"
    };
    let mut response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(origin) = origin {
        response.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\n"
        ));
    }
    response.push_str(&format!("MCP-Protocol-Version: {protocol_version}\r\n"));
    response.push_str(
        "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Accept, Authorization, Content-Type, MCP-Protocol-Version, X-PortMate-MCP-Token\r\n\r\n",
    );
    response.push_str(body);
    response
}

pub(crate) fn http_sse_message_response(
    body: &str,
    origin: Option<&str>,
    protocol_version: &str,
) -> String {
    let event = sse_event(
        "message",
        &serde_json::from_str::<Value>(body).unwrap_or_else(|_| json!({ "text": body })),
    );
    let mut response = http_sse_headers(origin, Some(event.len()), protocol_version);
    response.push_str(&event);
    response
}

pub(crate) fn http_sse_headers(
    origin: Option<&str>,
    content_length: Option<usize>,
    protocol_version: &str,
) -> String {
    let mut response =
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\n"
            .to_string();
    if let Some(content_length) = content_length {
        response.push_str(&format!("Content-Length: {content_length}\r\n"));
    } else {
        response.push_str("Connection: keep-alive\r\n");
    }
    if let Some(origin) = origin {
        response.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\n"
        ));
    }
    response.push_str(&format!("MCP-Protocol-Version: {protocol_version}\r\n"));
    response.push_str(
        "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Accept, Authorization, Content-Type, MCP-Protocol-Version, X-PortMate-MCP-Token\r\n\r\n",
    );
    response
}

pub(crate) fn sse_event(event: &str, data: &Value) -> String {
    sse_event_with_limit(event, data, MAX_JSON_RPC_RESPONSE_BYTES)
}

pub(crate) fn sse_event_with_limit(event: &str, data: &Value, max_data_bytes: usize) -> String {
    let data = match try_encode_json_with_limit(data, max_data_bytes) {
        Ok(Some(encoded)) => String::from_utf8(encoded).unwrap_or_else(|error| {
            json!({ "error": format!("failed to encode SSE data: {error}") }).to_string()
        }),
        Ok(None) => json!({
            "error": format!("SSE data exceeds the {max_data_bytes}-byte limit")
        })
        .to_string(),
        Err(error) => json!({ "error": format!("failed to encode SSE data: {error}") }).to_string(),
    };
    let mut output = format!("event: {event}\n");
    for line in data.lines() {
        output.push_str("data: ");
        output.push_str(line);
        output.push('\n');
    }
    output.push('\n');
    output
}

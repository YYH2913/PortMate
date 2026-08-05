use super::http_protocol::{
    accepts_json_http_response, accepts_sse_http_response, has_json_http_content_type,
    http_protocol_version, is_sse_stream_request, validate_mcp_protocol_version,
};
use super::http_request::{read_http_request, HttpRequest};
use super::http_security::{
    authorized_http_request, validate_origin, HttpSecurityConfig, HTTP_TOKEN_REF,
};
use super::response_encoding::{
    http_response, http_response_with_protocol, http_sse_headers, http_sse_message_response,
    json_rpc_http_body, sse_event,
};
use super::{handle_json_rpc_value, PortMateMcp};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::io::{self, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const MAX_HTTP_CONNECTIONS: usize = 64;
const HTTP_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub(super) struct HttpConfig {
    pub(super) addr: SocketAddr,
    pub(super) security: HttpSecurityConfig,
}

pub(super) fn run_http_server() -> Result<()> {
    let config = http_config()?;
    PortMateMcp::new()?;
    let listener = TcpListener::bind(config.addr)?;
    let active_connections = Arc::new(AtomicUsize::new(0));
    eprintln!("PortMate MCP HTTP listening on http://{}/mcp", config.addr);
    eprintln!("PortMate MCP HTTP token source: {HTTP_TOKEN_REF} or PORTMATE_MCP_HTTP_TOKEN");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let config = config.clone();
                spawn_http_connection(
                    stream,
                    config,
                    Arc::clone(&active_connections),
                    MAX_HTTP_CONNECTIONS,
                );
            }
            Err(error) => eprintln!("PortMate MCP HTTP accept failed: {error}"),
        }
    }
    Ok(())
}

pub(super) struct HttpConnectionPermit {
    active: Arc<AtomicUsize>,
}

impl Drop for HttpConnectionPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) fn try_acquire_http_connection(
    active: &Arc<AtomicUsize>,
    max_connections: usize,
) -> Option<HttpConnectionPermit> {
    active
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            (current < max_connections).then_some(current + 1)
        })
        .ok()?;
    Some(HttpConnectionPermit {
        active: Arc::clone(active),
    })
}

pub(super) fn spawn_http_connection(
    mut stream: TcpStream,
    config: HttpConfig,
    active: Arc<AtomicUsize>,
    max_connections: usize,
) -> bool {
    let Some(permit) = try_acquire_http_connection(&active, max_connections) else {
        let _ = stream.set_write_timeout(Some(HTTP_WRITE_TIMEOUT));
        let response = http_response(
            503,
            "Service Unavailable",
            &json!({ "error": "MCP HTTP connection limit reached" }).to_string(),
            None,
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.shutdown(Shutdown::Both);
        return false;
    };
    thread::spawn(move || {
        let _permit = permit;
        handle_http_connection(stream, config);
    });
    true
}

fn handle_http_connection(mut stream: TcpStream, config: HttpConfig) {
    if let Err(error) = stream.set_write_timeout(Some(HTTP_WRITE_TIMEOUT)) {
        eprintln!("PortMate MCP HTTP failed to set write timeout: {error}");
        return;
    }
    let response = match read_http_request(&mut stream) {
        Ok(request) if is_sse_stream_request(&request) => {
            if let Err(error) = write_http_sse_stream(&mut stream, request, &config) {
                eprintln!("PortMate MCP HTTP SSE stream failed: {error}");
            }
            return;
        }
        Ok(request) => handle_http_request(request, &config),
        Err(error) => {
            let timed_out = error.downcast_ref::<io::Error>().is_some_and(|error| {
                matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                )
            });
            http_response(
                if timed_out { 408 } else { 400 },
                if timed_out {
                    "Request Timeout"
                } else {
                    "Bad Request"
                },
                &json!({ "error": error.to_string() }).to_string(),
                None,
            )
        }
    };
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

fn http_config() -> Result<HttpConfig> {
    let addr = std::env::var("PORTMATE_MCP_HTTP_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8787".to_string())
        .parse::<SocketAddr>()
        .map_err(|error| anyhow!("PORTMATE_MCP_HTTP_ADDR must be host:port: {error}"))?;
    let allow_remote = std::env::var("PORTMATE_MCP_HTTP_ALLOW_REMOTE")
        .ok()
        .as_deref()
        == Some("1");
    validate_http_bind_addr(addr, allow_remote)?;
    let security = HttpSecurityConfig::from_environment(addr)?;
    Ok(HttpConfig { addr, security })
}

pub(super) fn validate_http_bind_addr(addr: SocketAddr, allow_remote: bool) -> Result<()> {
    let loopback = matches!(addr.ip(), IpAddr::V4(ip) if ip.is_loopback())
        || matches!(addr.ip(), IpAddr::V6(ip) if ip.is_loopback());
    if !loopback && !allow_remote {
        return Err(anyhow!(
            "MCP HTTP non-loopback bind {addr} requires PORTMATE_MCP_HTTP_ALLOW_REMOTE=1"
        ));
    }
    Ok(())
}

pub(super) fn handle_http_request(request: HttpRequest, config: &HttpConfig) -> String {
    let origin = request.headers.get("origin").cloned();
    if let Err(error) = validate_origin(origin.as_deref(), &config.security) {
        return http_response(
            403,
            "Forbidden",
            &json!({ "error": error.to_string() }).to_string(),
            origin.as_deref(),
        );
    }

    if request.path != "/mcp" {
        return http_response(
            404,
            "Not Found",
            &json!({ "error": "unknown endpoint" }).to_string(),
            origin.as_deref(),
        );
    }
    if request.method == "OPTIONS" {
        return http_response(204, "No Content", "", origin.as_deref());
    }
    if request.method == "GET" && accepts_sse_http_response(&request) {
        return http_sse_stream_start_response(&request, config);
    }
    if request.method != "POST" {
        return http_response(
            405,
            "Method Not Allowed",
            &json!({ "error": "use POST /mcp or GET /mcp with Accept: text/event-stream" })
                .to_string(),
            origin.as_deref(),
        );
    }
    if let Err(error) = validate_mcp_protocol_version(&request) {
        return http_response(
            400,
            "Bad Request",
            &json!({ "error": error.to_string() }).to_string(),
            origin.as_deref(),
        );
    }
    let protocol_version = http_protocol_version(&request).to_string();
    if !has_json_http_content_type(&request) {
        return http_response(
            415,
            "Unsupported Media Type",
            &json!({
                "error": "MCP HTTP POST requests require Content-Type: application/json"
            })
            .to_string(),
            origin.as_deref(),
        );
    }
    let accepts_json = accepts_json_http_response(&request);
    let accepts_sse = accepts_sse_http_response(&request);
    if !accepts_json && !accepts_sse {
        return http_response(
            406,
            "Not Acceptable",
            &json!({
                "error": "PortMate MCP HTTP returns JSON-RPC or SSE responses; send Accept: application/json, text/event-stream for streamable-http JSON compatibility"
            })
            .to_string(),
            origin.as_deref(),
        );
    }
    if !authorized_http_request(&request, config.security.token()) {
        return http_response(
            401,
            "Unauthorized",
            &json!({ "error": "missing or invalid MCP HTTP token" }).to_string(),
            origin.as_deref(),
        );
    }

    let value = match serde_json::from_slice::<Value>(&request.body) {
        Ok(value) => value,
        Err(parse_error) => {
            let response = json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32700,
                    "message": format!("parse error: {parse_error}")
                }
            });
            return http_response_with_protocol(
                200,
                "OK",
                &json_rpc_http_body(&response),
                origin.as_deref(),
                &protocol_version,
            );
        }
    };
    let body = match handle_http_json_rpc(value) {
        Ok(Some(value)) => json_rpc_http_body(&value),
        Ok(None) => {
            return http_response_with_protocol(
                202,
                "Accepted",
                "",
                origin.as_deref(),
                &protocol_version,
            );
        }
        Err(error) => json_rpc_http_body(&json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32603, "message": error.to_string() }
        })),
    };
    if accepts_sse && !accepts_json {
        http_sse_message_response(&body, origin.as_deref(), &protocol_version)
    } else {
        http_response_with_protocol(200, "OK", &body, origin.as_deref(), &protocol_version)
    }
}

pub(super) fn handle_http_json_rpc(value: Value) -> Result<Option<Value>> {
    let mut server = PortMateMcp::new()?;
    handle_json_rpc_value(&mut server, value)
}

fn http_sse_stream_start_response(request: &HttpRequest, config: &HttpConfig) -> String {
    let origin = request.headers.get("origin").cloned();
    if let Err(error) = validate_origin(origin.as_deref(), &config.security) {
        return http_response(
            403,
            "Forbidden",
            &json!({ "error": error.to_string() }).to_string(),
            origin.as_deref(),
        );
    }
    if let Err(error) = validate_mcp_protocol_version(request) {
        return http_response(
            400,
            "Bad Request",
            &json!({ "error": error.to_string() }).to_string(),
            origin.as_deref(),
        );
    }
    if !authorized_http_request(request, config.security.token()) {
        return http_response(
            401,
            "Unauthorized",
            &json!({ "error": "missing or invalid MCP HTTP token" }).to_string(),
            origin.as_deref(),
        );
    }
    let protocol_version = http_protocol_version(request);
    let mut response = http_sse_headers(origin.as_deref(), None, protocol_version);
    response.push_str(&sse_event(
        "endpoint",
        &json!({
            "uri": "/mcp",
            "method": "POST",
            "protocolVersion": protocol_version
        }),
    ));
    let state = match mcp_sse_state_payload(protocol_version) {
        Ok(state) => state,
        Err(error) => {
            return http_response(
                500,
                "Internal Server Error",
                &json!({ "error": error.to_string() }).to_string(),
                origin.as_deref(),
            );
        }
    };
    response.push_str(&sse_event("portmate.state", &state));
    response
}

fn write_http_sse_stream(
    stream: &mut TcpStream,
    request: HttpRequest,
    config: &HttpConfig,
) -> Result<()> {
    let protocol_version = http_protocol_version(&request).to_string();
    let response = http_sse_stream_start_response(&request, config);
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    if !response.starts_with("HTTP/1.1 200 OK") {
        let _ = stream.shutdown(Shutdown::Both);
        return Ok(());
    }

    loop {
        thread::sleep(Duration::from_secs(5));
        let event = format!(
            ": keep-alive\n\n{}",
            sse_event("portmate.state", &mcp_sse_state_payload(&protocol_version)?)
        );
        stream.write_all(event.as_bytes())?;
        stream.flush()?;
    }
}

fn mcp_sse_state_payload(protocol_version: &str) -> Result<Value> {
    Ok(PortMateMcp::new()?.sse_state_payload(protocol_version))
}

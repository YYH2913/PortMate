#[cfg(test)]
use std::io::{self, Read, Write};

#[cfg(test)]
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(test)]
use std::{thread, time::Duration};

#[cfg(test)]
use http_protocol::{
    accepts_json_http_response, accepts_sse_http_response, MCP_PROTOCOL_VERSION,
    MCP_PROTOCOL_VERSIONS,
};

#[cfg(test)]
use http_request::{read_http_request_with_body_limit, HttpRequest};

#[cfg(test)]
use http_security::{authorized_http_request, validate_origin, HttpSecurityConfig};

#[cfg(test)]
use http_server::{
    handle_http_json_rpc, handle_http_request, spawn_http_connection, try_acquire_http_connection,
    validate_http_bind_addr, HttpConfig,
};

#[cfg(test)]
use json_rpc::MAX_JSON_RPC_BATCH_ITEMS;

#[cfg(test)]
use mcp_resources::{parse_session_uri, parse_transfer_uri};

#[cfg(test)]
use mcp_tools::{bounded_log_query_limit, bounded_transfer_query_limit};

#[cfg(test)]
use response_encoding::{
    encode_json_rpc_response, sse_event_with_limit, try_encode_json_with_limit,
};

#[cfg(test)]
use stdio_server::{read_stdio_message, StdioMessage};

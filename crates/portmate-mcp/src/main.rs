use anyhow::{anyhow, Context, Result};
use portmate_core::{
    prompt_templates, redact_session_summary, resource_templates, tool_definitions, McpScope,
    SessionStore, SessionSummary,
};
use serde_json::{json, Value};
use std::ffi::OsStr;
#[cfg(test)]
use std::io::{self, Read, Write};
#[cfg(test)]
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::{thread, time::Duration};

mod content_upload;
mod desktop_ipc;
mod http_protocol;
mod http_request;
mod http_security;
mod http_server;
mod json_rpc;
mod keyring_store;
mod mcp_resources;
mod mcp_tools;
mod response_encoding;
mod socket_io;
mod stdio_server;
mod store_loader;

use desktop_ipc::{call_ipc_value as call_desktop_ipc_value, load_ipc_endpoint, IpcEndpointFile};
use http_protocol::negotiated_mcp_protocol_version;
#[cfg(test)]
use http_protocol::{
    accepts_json_http_response, accepts_sse_http_response, MCP_PROTOCOL_VERSION,
    MCP_PROTOCOL_VERSIONS,
};
#[cfg(test)]
use http_request::HttpRequest;
#[cfg(test)]
use http_security::{authorized_http_request, validate_origin, HttpSecurityConfig};
use http_server::run_http_server;
#[cfg(test)]
use http_server::{
    handle_http_json_rpc, handle_http_request, spawn_http_connection, try_acquire_http_connection,
    validate_http_bind_addr, HttpConfig,
};
#[cfg(test)]
use json_rpc::MAX_JSON_RPC_BATCH_ITEMS;
use json_rpc::{dispatch_json_rpc_value, error, JsonRpcRequest, JsonRpcResponse};
#[cfg(test)]
use mcp_resources::{parse_session_uri, parse_transfer_uri};
#[cfg(test)]
use mcp_tools::{bounded_log_query_limit, bounded_transfer_query_limit};
#[cfg(test)]
use response_encoding::{
    encode_json_rpc_response, sse_event_with_limit, try_encode_json_with_limit,
};
use stdio_server::run_stdio_server;
#[cfg(test)]
use stdio_server::{read_stdio_message, StdioMessage};
use store_loader::load_store_from_path;

struct PortMateMcp {
    store: SessionStore,
    store_path: Option<PathBuf>,
    ipc: Option<IpcEndpointFile>,
    client_id: String,
    allow_write: bool,
}

impl PortMateMcp {
    fn new() -> Result<Self> {
        let store_path = std::env::var("PORTMATE_STORE_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let store = load_initial_store(store_path.as_deref())?;
        let ipc = store_path.as_deref().and_then(load_ipc_endpoint);
        Ok(Self {
            store,
            store_path,
            ipc,
            client_id: std::env::var("PORTMATE_MCP_CLIENT_ID")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "portmate-local".to_string()),
            allow_write: std::env::var("PORTMATE_MCP_TRUSTED").ok().as_deref() == Some("1"),
        })
    }

    fn refresh_runtime_sources(&mut self) {
        let Some(store_path) = self.store_path.clone() else {
            return;
        };
        self.store = load_store_from_path(&store_path).unwrap_or_default();
        self.ipc = load_ipc_endpoint(&store_path);
    }

    fn handle(&mut self, request: JsonRpcRequest) -> Result<Option<JsonRpcResponse>> {
        let response_id = request.id.clone();
        if request.jsonrpc != "2.0" {
            return Ok(response_id.map(|id| error(id, -32600, "invalid JSON-RPC version")));
        }

        let result = match request.method.as_str() {
            "initialize" => self.initialize_result(&request.params),
            "ping" | "notifications/initialized" | "notifications/cancelled" => json!({}),
            "tools/list" => json!({
                "tools": tool_definitions().into_iter().map(|tool| json!({
                    "name": tool.name,
                    "title": tool.title,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                    "annotations": { "readOnlyHint": tool.read_only }
                })).collect::<Vec<_>>()
            }),
            "resources/list" => self.resources_list_result(),
            "resources/templates/list" => json!({
                "resourceTemplates": resource_templates().into_iter()
                    .filter(|resource| resource.uri_template.contains('{'))
                    .map(|resource| json!({
                        "uriTemplate": resource.uri_template,
                        "name": resource.name,
                        "title": resource.title,
                        "description": resource.description,
                        "mimeType": resource.mime_type
                    })).collect::<Vec<_>>()
            }),
            "prompts/list" => json!({
                "prompts": prompt_templates().into_iter().map(|prompt| json!({
                    "name": prompt.name,
                    "title": prompt.title,
                    "description": prompt.description,
                    "arguments": prompt.arguments
                })).collect::<Vec<_>>()
            }),
            "prompts/get" => self.prompt_get(&request.params)?,
            "resources/read" => self.resource_read(&request.params)?,
            "tools/call" => self.tool_call(&request.params)?,
            _ => {
                return Ok(response_id
                    .map(|id| error(id, -32601, format!("unknown method: {}", request.method))));
            }
        };

        Ok(response_id.map(|id| JsonRpcResponse {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }))
    }

    fn initialize_result(&self, params: &Value) -> Value {
        let protocol_version = negotiated_mcp_protocol_version(params);
        json!({
            "protocolVersion": protocol_version,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
                "prompts": { "listChanged": false }
            },
            "serverInfo": {
                "name": "portmate-mcp",
                "title": "PortMate MCP Bridge",
                "version": env!("CARGO_PKG_VERSION")
            }
        })
    }

    fn read_scope_enabled(&self, scope: McpScope) -> bool {
        self.store.mcp_can_read(&self.client_id, scope, None)
    }

    fn read_session_allowed(&self, scope: McpScope, session_id: &str) -> bool {
        self.store
            .mcp_can_read(&self.client_id, scope, Some(session_id))
    }

    fn has_session(&self, session_id: &str) -> bool {
        self.store
            .profiles
            .iter()
            .any(|profile| profile.id == session_id)
    }

    fn require_known_session(&self, session_id: &str) -> Result<()> {
        self.has_session(session_id)
            .then_some(())
            .ok_or_else(|| anyhow!("unknown or unavailable session"))
    }

    fn guard_read_scope(&self, scope: McpScope, session_id: Option<&str>) -> Result<()> {
        if self.store.mcp_can_read(&self.client_id, scope, session_id) {
            Ok(())
        } else {
            Err(anyhow!(
                "MCP read grant does not permit {scope:?} for the requested session"
            ))
        }
    }

    fn visible_summaries(
        &self,
        summaries: Vec<SessionSummary>,
        scope: McpScope,
    ) -> Vec<SessionSummary> {
        summaries
            .into_iter()
            .filter(|summary| self.read_session_allowed(scope, &summary.profile.id))
            .map(redact_session_summary)
            .collect()
    }

    fn sse_state_payload(&self, protocol_version: &str) -> Value {
        json!({
            "protocolVersion": protocol_version,
            "serverInfo": {
                "name": "portmate-mcp",
                "title": "PortMate MCP Bridge",
                "version": env!("CARGO_PKG_VERSION")
            },
            "sessions": self.visible_summaries(
                self.store.summaries(),
                McpScope::ReadSessions,
            )
        })
    }

    fn call_ipc_value(&self, command: &str, args: Value) -> Result<Option<Value>> {
        let Some(endpoint) = &self.ipc else {
            return Ok(None);
        };
        let store_path = self
            .store_path
            .as_deref()
            .ok_or_else(|| anyhow!("desktop IPC endpoint has no configured store path"))?;
        call_desktop_ipc_value(
            endpoint,
            store_path,
            &self.client_id,
            self.allow_write,
            command,
            args,
        )
    }
}

fn load_initial_store(store_path: Option<&std::path::Path>) -> Result<SessionStore> {
    let Some(store_path) = store_path else {
        return Ok(SessionStore::default());
    };
    load_store_from_path(store_path).with_context(|| {
        format!(
            "PORTMATE_STORE_PATH `{}` is not a readable PortMate Store",
            store_path.display()
        )
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum McpTransportMode {
    Stdio,
    Http,
}

fn select_transport_mode<I, S>(
    args: I,
    http_environment: Option<&OsStr>,
) -> Result<McpTransportMode>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut selected = None;
    for argument in args {
        let argument = argument.as_ref();
        let mode = if argument == OsStr::new("--stdio") {
            McpTransportMode::Stdio
        } else if argument == OsStr::new("--http") {
            McpTransportMode::Http
        } else {
            return Err(anyhow!(
                "unknown MCP argument `{}`; expected `--stdio` or `--http`",
                argument.to_string_lossy()
            ));
        };
        if selected.replace(mode).is_some() {
            return Err(anyhow!("MCP transport mode may be selected only once"));
        }
    }
    Ok(selected.unwrap_or_else(|| {
        if http_environment == Some(OsStr::new("1")) {
            McpTransportMode::Http
        } else {
            McpTransportMode::Stdio
        }
    }))
}

fn main() -> Result<()> {
    let mode = select_transport_mode(
        std::env::args_os().skip(1),
        std::env::var_os("PORTMATE_MCP_HTTP").as_deref(),
    )?;
    portmate_process_watchdog::install_parent_watchdog_from_environment("PORTMATE_MCP_PARENT_PID")
        .map_err(anyhow::Error::msg)?;
    match mode {
        McpTransportMode::Stdio => run_stdio_server(),
        McpTransportMode::Http => run_http_server(),
    }
}

fn handle_json_rpc_value(server: &mut PortMateMcp, value: Value) -> Result<Option<Value>> {
    server.refresh_runtime_sources();
    dispatch_json_rpc_value(value, |request| server.handle(request))
}

#[cfg(test)]
mod tests;

use anyhow::{anyhow, Result};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use portmate_core::{
    prompt_templates, redact_secrets, redact_session_event, redact_session_events,
    redact_session_summary, redact_sysmon_snapshot, redact_timeline_marks, redact_transfer_task,
    resource_templates, tool_definitions, McpScope, SessionEvent, SessionStore, SessionSummary,
    TransferTask,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

mod desktop_ipc;
mod http_request;
mod http_security;
mod keyring_store;
mod socket_io;
mod store_loader;

use desktop_ipc::{call_ipc_value as call_desktop_ipc_value, load_ipc_endpoint, IpcEndpointFile};
use http_request::{read_http_request, HttpRequest};
use http_security::{authorized_http_request, validate_origin, HttpSecurityConfig, HTTP_TOKEN_REF};
use store_loader::load_store_from_path;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MCP_PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", MCP_PROTOCOL_VERSION];
const MAX_STDIO_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_JSON_RPC_BATCH_ITEMS: usize = 128;
const MAX_JSON_RPC_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_HTTP_CONNECTIONS: usize = 64;
const HTTP_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_LOG_QUERY_LIMIT: u64 = 100;
const MAX_LOG_QUERY_LIMIT: u64 = 1000;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

struct PortMateMcp {
    store: SessionStore,
    store_path: Option<PathBuf>,
    ipc: Option<IpcEndpointFile>,
    client_id: String,
    allow_write: bool,
}

#[derive(Debug, Clone)]
struct HttpConfig {
    addr: SocketAddr,
    security: HttpSecurityConfig,
}

impl PortMateMcp {
    fn new() -> Self {
        let store_path = std::env::var("PORTMATE_STORE_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let store = store_path
            .as_deref()
            .and_then(load_store_from_path)
            .unwrap_or_default();
        let ipc = store_path.as_deref().and_then(load_ipc_endpoint);
        Self {
            store,
            store_path,
            ipc,
            client_id: std::env::var("PORTMATE_MCP_CLIENT_ID")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "portmate-local".to_string()),
            allow_write: std::env::var("PORTMATE_MCP_TRUSTED").ok().as_deref() == Some("1"),
        }
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

    fn resources_list_result(&self) -> Value {
        let mut resources = Vec::new();
        if self.read_scope_enabled(McpScope::ReadSessions) {
            resources.push(json!({
                "uri": "portmate://sessions",
                "name": "sessions",
                "title": "Sessions",
                "description": "All visible session summaries",
                "mimeType": "application/json"
            }));
        }
        let log_resources = [
            ("screen", "Screen", "text/plain"),
            ("log", "Log", "application/jsonl"),
            ("timeline", "Timeline", "application/json"),
            ("sysmon", "Sysmon", "application/json"),
            ("tmux", "Tmux", "application/json"),
        ];
        for summary in self.store.summaries() {
            let encoded_session_id = encode_mcp_uri_segment(&summary.profile.id);
            if self.read_session_allowed(McpScope::ReadSessions, &summary.profile.id) {
                resources.push(json!({
                    "uri": format!("portmate://sessions/{encoded_session_id}/state"),
                    "name": format!("session_{}_state", summary.profile.id),
                    "title": format!("{} State", summary.profile.name),
                    "mimeType": "application/json"
                }));
            }
            if self.read_session_allowed(McpScope::ReadLogs, &summary.profile.id) {
                for (suffix, label, mime_type) in log_resources {
                    resources.push(json!({
                        "uri": format!("portmate://sessions/{encoded_session_id}/{suffix}"),
                        "name": format!("session_{}_{}", summary.profile.id, suffix),
                        "title": format!("{} {label}", summary.profile.name),
                        "mimeType": mime_type
                    }));
                }
            }
        }
        for transfer in &self.store.transfers {
            if !self.has_session(&transfer.session_id)
                || !self.read_session_allowed(McpScope::ReadLogs, &transfer.session_id)
            {
                continue;
            }
            let encoded_transfer_id = encode_mcp_uri_segment(&transfer.id);
            resources.push(json!({
                "uri": format!("portmate://transfers/{encoded_transfer_id}"),
                "name": format!("transfer_{}", transfer.id),
                "title": format!("Transfer {}", transfer.id),
                "mimeType": "application/json"
            }));
        }
        json!({ "resources": resources })
    }

    fn prompt_get(&self, params: &Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing prompt name"))?;
        let session_id = params
            .get("arguments")
            .and_then(|args| args.get("sessionId"))
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("missing prompt sessionId"))?;
        self.guard_read_scope(McpScope::ReadLogs, Some(session_id))?;
        self.require_known_session(session_id)?;
        let screen = redact_secrets(&self.store.screen(session_id).unwrap_or_default());
        let text = match name {
            "diagnose_session" => format!("Diagnose PortMate session `{session_id}` using this terminal snapshot:\n\n{screen}"),
            "compare_serial_and_ssh" => format!("Compare serial and SSH behavior for `{session_id}`. Correlate boot output, SSH state, and timeline marks."),
            "prepare_repro_report" => format!("Prepare a reproducible report for `{session_id}` using logs, timeline marks, transfers, and MCP audit records."),
            _ => return Err(anyhow!("unknown prompt: {name}")),
        };
        Ok(json!({
            "description": name,
            "messages": [{ "role": "user", "content": { "type": "text", "text": text } }]
        }))
    }

    fn resource_read(&self, params: &Value) -> Result<Value> {
        let uri = params
            .get("uri")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing resource uri"))?;
        let content = if uri == "portmate://sessions" {
            self.guard_read_scope(McpScope::ReadSessions, None)?;
            serde_json::to_string_pretty(
                &self.visible_summaries(self.store.summaries(), McpScope::ReadSessions),
            )?
        } else if let Some((session_id, suffix)) = parse_session_uri(uri) {
            let scope = if suffix == "state" {
                McpScope::ReadSessions
            } else {
                McpScope::ReadLogs
            };
            self.guard_read_scope(scope, Some(&session_id))?;
            self.require_known_session(&session_id)?;
            match suffix {
                "state" => serde_json::to_string_pretty(
                    &self
                        .store
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == session_id)
                        .map(redact_session_summary),
                )?,
                "screen" => redact_secrets(&self.store.screen(&session_id).unwrap_or_default()),
                "log" => redact_session_events(self.store.tail_log(&session_id, 200))
                    .into_iter()
                    .map(|event| serde_json::to_string(&event).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\n"),
                "timeline" => serde_json::to_string_pretty(&redact_timeline_marks(
                    self.store.timeline_for(&session_id),
                ))?,
                "sysmon" => serde_json::to_string_pretty(
                    &self
                        .store
                        .sysmon_for(&session_id)
                        .map(redact_sysmon_snapshot),
                )?,
                "tmux" => {
                    if let Some(value) =
                        self.call_ipc_value("list_tmux_state", json!({ "sessionId": session_id }))?
                    {
                        redact_secrets(&ipc_value_to_text(value)?)
                    } else {
                        serde_json::to_string_pretty(&json!({
                            "sessions": [],
                            "panes": [],
                            "message": "desktop IPC is not available"
                        }))?
                    }
                }
                _ => return Err(anyhow!("unknown session resource suffix: {suffix}")),
            }
        } else if let Some(id) = parse_transfer_uri(uri) {
            let transfer = self
                .store
                .transfer_by_id(&id)
                .ok_or_else(|| anyhow!("unknown or unauthorized transfer resource"))?;
            self.guard_read_scope(McpScope::ReadLogs, Some(&transfer.session_id))?;
            self.require_known_session(&transfer.session_id)?;
            serde_json::to_string_pretty(&redact_transfer_task(transfer))?
        } else {
            return Err(anyhow!("unknown resource uri: {uri}"));
        };

        Ok(json!({
            "contents": [{
                "uri": uri,
                "mimeType": if uri.ends_with("/screen") { "text/plain" } else { "application/json" },
                "text": content
            }]
        }))
    }

    fn tool_call(&mut self, params: &Value) -> Result<Value> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("missing tool name"))?;
        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let mut is_error = false;

        let output = match name {
            "list_sessions" => {
                self.guard_read_scope(McpScope::ReadSessions, None)?;
                let summaries =
                    if let Some(value) = self.call_ipc_value("list_sessions", json!({}))? {
                        serde_json::from_value::<Vec<SessionSummary>>(value)
                            .map_err(|error| anyhow!("invalid desktop session response: {error}"))?
                    } else {
                        self.store.summaries()
                    };
                serde_json::to_string_pretty(
                    &self.visible_summaries(summaries, McpScope::ReadSessions),
                )?
            }
            "read_screen" => {
                let session_id = required_string(&arguments, "sessionId")?;
                self.guard_read_scope(McpScope::ReadLogs, Some(session_id))?;
                self.require_known_session(session_id)?;
                if let Some(value) = self.call_ipc_value("read_screen", arguments.clone())? {
                    redact_secrets(&ipc_value_to_text(value)?)
                } else {
                    redact_secrets(&self.store.screen(session_id).unwrap_or_default())
                }
            }
            "tail_log" => {
                let session_id = required_string(&arguments, "sessionId")?;
                self.guard_read_scope(McpScope::ReadLogs, Some(session_id))?;
                self.require_known_session(session_id)?;
                let limit = arguments.get("limit").and_then(Value::as_u64);
                let limit = bounded_log_query_limit(limit);
                let mut events =
                    if let Some(value) = self.call_ipc_value("tail_log", arguments.clone())? {
                        serde_json::from_value::<Vec<SessionEvent>>(value)
                            .map_err(|error| anyhow!("invalid desktop log response: {error}"))?
                    } else {
                        self.store.tail_log(session_id, limit)
                    };
                events.retain(|event| {
                    event.session_id == session_id
                        && self.has_session(&event.session_id)
                        && self.read_session_allowed(McpScope::ReadLogs, &event.session_id)
                });
                serde_json::to_string_pretty(&redact_session_events(events))?
            }
            "search_logs" => {
                let query = required_string(&arguments, "query")?;
                let session_id = arguments.get("sessionId").and_then(Value::as_str);
                self.guard_read_scope(McpScope::ReadLogs, session_id)?;
                if let Some(session_id) = session_id {
                    self.require_known_session(session_id)?;
                }
                let limit = arguments.get("limit").and_then(Value::as_u64);
                let limit = bounded_log_query_limit(limit);
                let mut events =
                    if let Some(value) = self.call_ipc_value("search_logs", arguments.clone())? {
                        serde_json::from_value::<Vec<SessionEvent>>(value)
                            .map_err(|error| anyhow!("invalid desktop log response: {error}"))?
                    } else {
                        self.store.search_logs(query, session_id, limit)
                    };
                events.retain(|event| {
                    session_id.is_none_or(|session_id| event.session_id == session_id)
                        && self.has_session(&event.session_id)
                        && self.read_session_allowed(McpScope::ReadLogs, &event.session_id)
                });
                serde_json::to_string_pretty(&redact_session_events(events))?
            }
            "send_text" | "send_key" | "run_command" => {
                if let Some(output) = self.write_tool(name, &arguments)? {
                    output
                } else {
                    is_error = true;
                    format!(
                        "{name} was NOT executed: desktop IPC is not available, so no session input was sent."
                    )
                }
            }
            "export_session_bundle" => {
                let session_id = required_string(&arguments, "sessionId")?;
                self.guard_read_scope(McpScope::ReadLogs, Some(session_id))?;
                self.require_known_session(session_id)?;
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    redact_secrets(&ipc_value_to_text(value)?)
                } else {
                    serde_json::to_string_pretty(
                        &self.store.export_session_bundle_redacted(session_id),
                    )?
                }
            }
            "open_session" | "close_session" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    let summary = serde_json::from_value::<SessionSummary>(value)
                        .map_err(|error| anyhow!("invalid desktop session response: {error}"))?;
                    serde_json::to_string_pretty(&redact_session_summary(summary))?
                } else {
                    is_error = true;
                    format!("{name} was NOT executed: desktop IPC is not available, so no session state changed.")
                }
            }
            "start_transfer" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    let transfer = serde_json::from_value::<TransferTask>(value)
                        .map_err(|error| anyhow!("invalid desktop transfer response: {error}"))?;
                    serde_json::to_string_pretty(&redact_transfer_task(transfer))?
                } else {
                    is_error = true;
                    "start_transfer was NOT executed: desktop IPC is not available, so no transfer was started."
                        .to_string()
                }
            }
            "create_tunnel" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    is_error = true;
                    "create_tunnel was NOT executed: desktop IPC is not available, so no tunnel was created."
                        .to_string()
                }
            }
            "list_tmux_state" => {
                let session_id = required_string(&arguments, "sessionId")?;
                self.guard_read_scope(McpScope::ReadLogs, Some(session_id))?;
                self.require_known_session(session_id)?;
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    serde_json::to_string_pretty(&json!({
                        "sessions": [],
                        "panes": [],
                        "message": "desktop IPC is not available"
                    }))?
                }
            }
            "attach_tmux" => {
                if let Some(output) = self.write_tool(name, &arguments)? {
                    output
                } else {
                    is_error = true;
                    "attach_tmux was NOT executed: desktop IPC is not available, so tmux was not attached."
                        .to_string()
                }
            }
            _ => return Err(anyhow!("unknown tool: {name}")),
        };

        Ok(json!({
            "content": [{ "type": "text", "text": output }],
            "isError": is_error
        }))
    }

    fn write_tool(&self, name: &str, arguments: &Value) -> Result<Option<String>> {
        if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
            let event = serde_json::from_value::<SessionEvent>(value)
                .map_err(|error| anyhow!("invalid desktop event response: {error}"))?;
            return serde_json::to_string_pretty(&redact_session_event(event))
                .map(Some)
                .map_err(Into::into);
        }
        Ok(None)
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

fn ipc_value_to_text(value: Value) -> Result<String> {
    if let Some(text) = value.as_str() {
        Ok(text.to_string())
    } else {
        Ok(serde_json::to_string_pretty(&value)?)
    }
}

fn main() -> Result<()> {
    if std::env::args().any(|arg| arg == "--http")
        || std::env::var("PORTMATE_MCP_HTTP").ok().as_deref() == Some("1")
    {
        run_http_server()
    } else {
        run_stdio_server()
    }
}

fn run_stdio_server() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut server = PortMateMcp::new();
    let mut stdin = stdin.lock();

    loop {
        let line = match read_stdio_message(&mut stdin, MAX_STDIO_MESSAGE_BYTES)? {
            StdioMessage::Eof => break,
            StdioMessage::TooLarge => {
                let response = serde_json::to_value(error(
                    Value::Null,
                    -32700,
                    format!(
                        "parse error: stdio message exceeds the {MAX_STDIO_MESSAGE_BYTES}-byte limit"
                    ),
                ))?;
                write_stdio_json_response(&mut stdout, &response)?;
                continue;
            }
            StdioMessage::Message(line) => line,
        };
        if line.iter().all(|byte| byte.is_ascii_whitespace()) {
            continue;
        }

        let value = match serde_json::from_slice::<Value>(&line) {
            Ok(value) => value,
            Err(error_message) => {
                let response = serde_json::to_value(error(
                    Value::Null,
                    -32700,
                    format!("parse error: {error_message}"),
                ))?;
                write_stdio_json_response(&mut stdout, &response)?;
                continue;
            }
        };

        if let Some(response) = match handle_json_rpc_value(&mut server, value) {
            Ok(response) => response,
            Err(error_message) => Some(serde_json::to_value(error(
                Value::Null,
                -32603,
                error_message.to_string(),
            ))?),
        } {
            write_stdio_json_response(&mut stdout, &response)?;
        }
    }

    Ok(())
}

fn write_stdio_json_response<W: Write>(writer: &mut W, response: &Value) -> Result<()> {
    let encoded = encode_json_rpc_response(response, MAX_JSON_RPC_RESPONSE_BYTES)?;
    writer.write_all(&encoded)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

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

fn try_encode_json_with_limit(value: &Value, max_bytes: usize) -> Result<Option<Vec<u8>>> {
    let mut writer = BoundedJsonWriter::new(max_bytes);
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok(Some(writer.bytes)),
        Err(_) if writer.exceeded => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn encode_json_rpc_response(response: &Value, max_bytes: usize) -> Result<Vec<u8>> {
    if let Some(encoded) = try_encode_json_with_limit(response, max_bytes)? {
        return Ok(encoded);
    }
    let response_id = response
        .as_object()
        .and_then(|object| object.get("id"))
        .cloned()
        .unwrap_or(Value::Null);
    let fallback = serde_json::to_value(error(
        response_id,
        -32603,
        format!("JSON-RPC response exceeds the {max_bytes}-byte limit"),
    ))?;
    try_encode_json_with_limit(&fallback, max_bytes)?
        .ok_or_else(|| anyhow!("JSON-RPC response limit is too small to encode its overflow error"))
}

#[derive(Debug, PartialEq, Eq)]
enum StdioMessage {
    Eof,
    Message(Vec<u8>),
    TooLarge,
}

fn read_stdio_message<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<StdioMessage> {
    let buffered_limit = max_bytes.saturating_add(2);
    let mut line = Vec::with_capacity(buffered_limit.min(8192));
    let read = {
        let mut limited = Read::take(&mut *reader, buffered_limit as u64);
        limited.read_until(b'\n', &mut line)?
    };
    if read == 0 {
        return Ok(StdioMessage::Eof);
    }
    let terminated = line.last() == Some(&b'\n');
    if !terminated && line.len() == buffered_limit {
        discard_stdio_line(reader)?;
    }
    if terminated {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    if line.len() > max_bytes {
        Ok(StdioMessage::TooLarge)
    } else {
        Ok(StdioMessage::Message(line))
    }
}

fn discard_stdio_line<R: BufRead>(reader: &mut R) -> io::Result<()> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(());
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            reader.consume(newline + 1);
            return Ok(());
        }
        let length = available.len();
        reader.consume(length);
    }
}

fn run_http_server() -> Result<()> {
    let config = http_config()?;
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

struct HttpConnectionPermit {
    active: Arc<AtomicUsize>,
}

impl Drop for HttpConnectionPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

fn try_acquire_http_connection(
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

fn spawn_http_connection(
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
    if !matches!(addr.ip(), IpAddr::V4(ip) if ip.is_loopback())
        && !matches!(addr.ip(), IpAddr::V6(ip) if ip.is_loopback())
    {
        return Err(anyhow!("MCP HTTP must bind a loopback address; got {addr}"));
    }
    let security = HttpSecurityConfig::from_environment(addr)?;
    Ok(HttpConfig { addr, security })
}

fn handle_http_request(request: HttpRequest, config: &HttpConfig) -> String {
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

fn json_rpc_http_body(response: &Value) -> String {
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

fn handle_http_json_rpc(value: Value) -> Result<Option<Value>> {
    let mut server = PortMateMcp::new();
    handle_json_rpc_value(&mut server, value)
}

fn handle_json_rpc_value(server: &mut PortMateMcp, value: Value) -> Result<Option<Value>> {
    server.refresh_runtime_sources();
    if let Value::Array(items) = value {
        if items.is_empty() {
            return Ok(Some(serde_json::to_value(error(
                Value::Null,
                -32600,
                "an empty JSON-RPC batch is invalid",
            ))?));
        }
        if items.len() > MAX_JSON_RPC_BATCH_ITEMS {
            return Ok(Some(serde_json::to_value(error(
                Value::Null,
                -32600,
                format!("JSON-RPC batch exceeds the {MAX_JSON_RPC_BATCH_ITEMS}-item limit"),
            ))?));
        }
        let mut responses = Vec::with_capacity(items.len());
        for item in items {
            if let Some(response) = handle_one_json_rpc_value(server, item)? {
                responses.push(serde_json::to_value(response)?);
            }
        }
        return if responses.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Value::Array(responses)))
        };
    }
    handle_one_json_rpc_value(server, value)?
        .map(serde_json::to_value)
        .transpose()
        .map_err(Into::into)
}

fn handle_one_json_rpc_value(
    server: &mut PortMateMcp,
    value: Value,
) -> Result<Option<JsonRpcResponse>> {
    let has_id = value.get("id").is_some();
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    if has_id && !matches!(id, Value::Null | Value::Number(_) | Value::String(_)) {
        return Ok(Some(error(
            Value::Null,
            -32600,
            "JSON-RPC id must be a string, number, or null",
        )));
    }
    if value
        .get("params")
        .is_some_and(|params| !params.is_array() && !params.is_object())
    {
        return Ok(has_id.then(|| error(id, -32602, "JSON-RPC params must be an object or array")));
    }
    let mut request = match serde_json::from_value::<JsonRpcRequest>(value) {
        Ok(request) => request,
        Err(error_message) => return Ok(Some(error(id, -32600, error_message.to_string()))),
    };
    if has_id {
        request.id = Some(id.clone());
    }
    match server.handle(request) {
        Ok(response) => Ok(response),
        Err(error_message) if has_id => Ok(Some(error(id, -32603, error_message.to_string()))),
        Err(_) => Ok(None),
    }
}

fn negotiated_mcp_protocol_version(params: &Value) -> &str {
    params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|version| MCP_PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(MCP_PROTOCOL_VERSION)
}

fn http_protocol_version(request: &HttpRequest) -> &str {
    request
        .headers
        .get("mcp-protocol-version")
        .map(String::as_str)
        .filter(|version| MCP_PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(MCP_PROTOCOL_VERSION)
}

fn validate_mcp_protocol_version(request: &HttpRequest) -> Result<()> {
    let Some(version) = request.headers.get("mcp-protocol-version") else {
        return Ok(());
    };
    if MCP_PROTOCOL_VERSIONS.contains(&version.as_str()) {
        Ok(())
    } else {
        Err(anyhow!(
            "unsupported MCP-Protocol-Version `{version}`; supported versions: {}",
            MCP_PROTOCOL_VERSIONS.join(", ")
        ))
    }
}

fn has_json_http_content_type(request: &HttpRequest) -> bool {
    request.headers.get("content-type").is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("application/json")
    })
}

fn accepts_json_http_response(request: &HttpRequest) -> bool {
    accepts_http_media_type(request, true, |media_type| {
        matches!(media_type, "*/*" | "application/*" | "application/json")
    })
}

fn accepts_sse_http_response(request: &HttpRequest) -> bool {
    accepts_http_media_type(request, false, |media_type| {
        media_type == "text/event-stream"
    })
}

fn accepts_http_media_type(
    request: &HttpRequest,
    default_when_missing: bool,
    matches_media_type: impl Fn(&str) -> bool,
) -> bool {
    let Some(accept) = request.headers.get("accept") else {
        return default_when_missing;
    };
    accept.split(',').any(|item| {
        let mut parts = item.split(';');
        let media_type = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        let mut quality = 1.0_f32;
        for parameter in parts {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("q") {
                quality = value.trim().parse::<f32>().unwrap_or(0.0);
            }
        }
        (0.0..=1.0).contains(&quality) && quality > 0.0 && matches_media_type(&media_type)
    })
}

fn is_sse_stream_request(request: &HttpRequest) -> bool {
    request.method == "GET" && request.path == "/mcp" && accepts_sse_http_response(request)
}

fn http_response(status: u16, reason: &str, body: &str, origin: Option<&str>) -> String {
    http_response_with_protocol(status, reason, body, origin, MCP_PROTOCOL_VERSION)
}

fn http_response_with_protocol(
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
    response.push_str(&sse_event(
        "portmate.state",
        &mcp_sse_state_payload(protocol_version),
    ));
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
            sse_event("portmate.state", &mcp_sse_state_payload(&protocol_version))
        );
        stream.write_all(event.as_bytes())?;
        stream.flush()?;
    }
}

fn http_sse_message_response(body: &str, origin: Option<&str>, protocol_version: &str) -> String {
    let event = sse_event(
        "message",
        &serde_json::from_str::<Value>(body).unwrap_or_else(|_| json!({ "text": body })),
    );
    let mut response = http_sse_headers(origin, Some(event.len()), protocol_version);
    response.push_str(&event);
    response
}

fn http_sse_headers(
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

fn sse_event(event: &str, data: &Value) -> String {
    sse_event_with_limit(event, data, MAX_JSON_RPC_RESPONSE_BYTES)
}

fn sse_event_with_limit(event: &str, data: &Value, max_data_bytes: usize) -> String {
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

fn mcp_sse_state_payload(protocol_version: &str) -> Value {
    PortMateMcp::new().sse_state_payload(protocol_version)
}

fn error(id: Value, code: i64, message: impl Into<String>) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: message.into(),
            data: None,
        }),
    }
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string argument `{key}`"))
}

fn bounded_log_query_limit(limit: Option<u64>) -> usize {
    limit
        .unwrap_or(DEFAULT_LOG_QUERY_LIMIT)
        .clamp(1, MAX_LOG_QUERY_LIMIT) as usize
}

const MCP_URI_SEGMENT_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

fn encode_mcp_uri_segment(value: &str) -> String {
    utf8_percent_encode(value, MCP_URI_SEGMENT_ENCODE_SET).to_string()
}

fn decode_mcp_uri_segment(value: &str) -> Option<String> {
    if value.is_empty() || !has_valid_percent_encoding(value) {
        return None;
    }
    percent_decode_str(value)
        .decode_utf8()
        .ok()
        .map(|value| value.into_owned())
}

fn has_valid_percent_encoding(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

fn parse_session_uri(uri: &str) -> Option<(String, &str)> {
    let path = uri.strip_prefix("portmate://sessions/")?;
    if path.contains(['?', '#']) {
        return None;
    }
    let mut parts = path.split('/');
    let id = decode_mcp_uri_segment(parts.next()?)?;
    let suffix = parts.next()?;
    if suffix.is_empty() || parts.next().is_some() {
        return None;
    }
    Some((id, suffix))
}

fn parse_transfer_uri(uri: &str) -> Option<String> {
    let id = uri.strip_prefix("portmate://transfers/")?;
    if id.contains(['/', '?', '#']) {
        return None;
    }
    decode_mcp_uri_segment(id)
}

#[cfg(test)]
mod tests;

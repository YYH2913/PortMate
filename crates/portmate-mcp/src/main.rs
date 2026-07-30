use anyhow::{anyhow, Result};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use portmate_core::{
    prompt_templates, redact_secrets, redact_session_event, redact_session_events,
    redact_session_summary, redact_sysmon_snapshot, redact_timeline_marks, redact_transfer_task,
    resource_templates, tool_definitions, McpScope, SessionEvent, SessionStore, SessionSummary,
    TransferTask,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};
#[cfg(test)]
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::{thread, time::Duration};

mod desktop_ipc;
mod http_protocol;
mod http_request;
mod http_security;
mod http_server;
mod json_rpc;
mod keyring_store;
mod response_encoding;
mod socket_io;
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
    HttpConfig,
};
#[cfg(test)]
use json_rpc::MAX_JSON_RPC_BATCH_ITEMS;
use json_rpc::{dispatch_json_rpc_value, error, JsonRpcRequest, JsonRpcResponse};
use response_encoding::{encode_json_rpc_response, MAX_JSON_RPC_RESPONSE_BYTES};
#[cfg(test)]
use response_encoding::{sse_event_with_limit, try_encode_json_with_limit};
use store_loader::load_store_from_path;

const MAX_STDIO_MESSAGE_BYTES: usize = 1024 * 1024;
const DEFAULT_LOG_QUERY_LIMIT: u64 = 100;
const MAX_LOG_QUERY_LIMIT: u64 = 1000;

struct PortMateMcp {
    store: SessionStore,
    store_path: Option<PathBuf>,
    ipc: Option<IpcEndpointFile>,
    client_id: String,
    allow_write: bool,
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

fn handle_json_rpc_value(server: &mut PortMateMcp, value: Value) -> Result<Option<Value>> {
    server.refresh_runtime_sources();
    dispatch_json_rpc_value(value, |request| server.handle(request))
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

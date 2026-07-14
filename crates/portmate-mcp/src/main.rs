use anyhow::{anyhow, Result};
use keyring_core::Entry;
use portmate_core::{
    prompt_templates, redact_secrets, resource_templates, tool_definitions, SessionEvent,
    SessionStore,
};
use rusqlite::{params, Connection as SqliteConnection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::net::{IpAddr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const STORE_KEY: &str = "session-store";
const HTTP_TOKEN_REF: &str = "keychain:mcp-http-token";
const MAX_STDIO_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_JSON_RPC_BATCH_ITEMS: usize = 128;
const MAX_JSON_RPC_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 1024 * 1024;
const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
const MAX_HTTP_HEADERS: usize = 128;
const MAX_HTTP_CONNECTIONS: usize = 64;
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_IPC_REQUEST_BYTES: usize = 1024 * 1024;
const MAX_IPC_RESPONSE_BYTES: usize = MAX_JSON_RPC_RESPONSE_BYTES;
const MAX_IPC_ENDPOINT_BYTES: usize = 64 * 1024;
const MAX_IPC_TOKEN_BYTES: usize = 4096;
const IPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const IPC_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const IPC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(120);
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IpcEndpointFile {
    addr: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    token_ref: Option<String>,
    store_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IpcRequest {
    token: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    trusted_write: bool,
    command: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IpcResponse {
    ok: bool,
    value: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct HttpConfig {
    addr: SocketAddr,
    token: String,
    allowed_origins: Vec<String>,
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
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
                .unwrap_or_else(|_| "portmate-local".to_string()),
            allow_write: std::env::var("PORTMATE_MCP_TRUSTED").ok().as_deref() == Some("1"),
        }
    }

    fn handle(&mut self, request: JsonRpcRequest) -> Result<Option<JsonRpcResponse>> {
        let response_id = request.id.clone();
        if request.jsonrpc != "2.0" {
            return Ok(response_id.map(|id| error(id, -32600, "invalid JSON-RPC version")));
        }

        let result = match request.method.as_str() {
            "initialize" => self.initialize_result(),
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

    fn initialize_result(&self) -> Value {
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
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
        let mut resources = vec![json!({
            "uri": "portmate://sessions",
            "name": "sessions",
            "title": "Sessions",
            "description": "All visible session summaries",
            "mimeType": "application/json"
        })];
        let session_resources = [
            ("state", "State", "application/json"),
            ("screen", "Screen", "text/plain"),
            ("log", "Log", "application/jsonl"),
            ("timeline", "Timeline", "application/json"),
            ("sysmon", "Sysmon", "application/json"),
            ("tmux", "Tmux", "application/json"),
        ];
        for summary in self.store.summaries() {
            for (suffix, label, mime_type) in session_resources {
                resources.push(json!({
                    "uri": format!("portmate://sessions/{}/{suffix}", summary.profile.id),
                    "name": format!("session_{}_{}", summary.profile.id, suffix),
                    "title": format!("{} {label}", summary.profile.name),
                    "mimeType": mime_type
                }));
            }
        }
        for transfer in &self.store.transfers {
            resources.push(json!({
                "uri": format!("portmate://transfers/{}", transfer.id),
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
            .unwrap_or_default();
        let screen = self.store.screen(session_id).unwrap_or_default();
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
            serde_json::to_string_pretty(&self.store.summaries())?
        } else if let Some((session_id, suffix)) = parse_session_uri(uri) {
            match suffix {
                "state" => serde_json::to_string_pretty(
                    &self
                        .store
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == session_id),
                )?,
                "screen" => redact_secrets(&self.store.screen(session_id).unwrap_or_default()),
                "log" => redact_events(self.store.tail_log(session_id, 200))
                    .into_iter()
                    .map(|event| serde_json::to_string(&event).unwrap_or_default())
                    .collect::<Vec<_>>()
                    .join("\n"),
                "timeline" => serde_json::to_string_pretty(&self.store.timeline_for(session_id))?,
                "sysmon" => serde_json::to_string_pretty(&self.store.sysmon_for(session_id))?,
                "tmux" => {
                    if let Some(value) =
                        self.call_ipc_value("list_tmux_state", json!({ "sessionId": session_id }))?
                    {
                        ipc_value_to_text(value)?
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
        } else if let Some(id) = uri.strip_prefix("portmate://transfers/") {
            serde_json::to_string_pretty(&self.store.transfer_by_id(id))?
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
                if let Some(value) = self.call_ipc_value("list_sessions", json!({}))? {
                    ipc_value_to_text(value)?
                } else {
                    serde_json::to_string_pretty(&self.store.summaries())?
                }
            }
            "read_screen" => {
                let session_id = required_string(&arguments, "sessionId")?;
                if let Some(value) = self.call_ipc_value("read_screen", arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    redact_secrets(&self.store.screen(session_id).unwrap_or_default())
                }
            }
            "tail_log" => {
                let session_id = required_string(&arguments, "sessionId")?;
                let limit = arguments.get("limit").and_then(Value::as_u64);
                let limit = bounded_log_query_limit(limit);
                if let Some(value) = self.call_ipc_value("tail_log", arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    serde_json::to_string_pretty(&redact_events(
                        self.store.tail_log(session_id, limit),
                    ))?
                }
            }
            "search_logs" => {
                let query = required_string(&arguments, "query")?;
                let session_id = arguments.get("sessionId").and_then(Value::as_str);
                let limit = arguments.get("limit").and_then(Value::as_u64);
                let limit = bounded_log_query_limit(limit);
                if let Some(value) = self.call_ipc_value("search_logs", arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    serde_json::to_string_pretty(&redact_events(
                        self.store.search_logs(query, session_id, limit),
                    ))?
                }
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
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    serde_json::to_string_pretty(
                        &self.store.export_session_bundle_redacted(session_id),
                    )?
                }
            }
            "open_session" | "close_session" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    is_error = true;
                    format!("{name} was NOT executed: desktop IPC is not available, so no session state changed.")
                }
            }
            "start_transfer" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
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
                let _ = required_string(&arguments, "sessionId")?;
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
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
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
            return ipc_value_to_text(value).map(Some);
        }
        Ok(None)
    }

    fn call_ipc_value(&self, command: &str, args: Value) -> Result<Option<Value>> {
        let Some(endpoint) = &self.ipc else {
            return Ok(None);
        };
        let store_path = self
            .store_path
            .as_deref()
            .ok_or_else(|| anyhow!("desktop IPC endpoint has no configured store path"))?;
        let addr = validate_ipc_endpoint(endpoint, store_path)?;
        let mut stream = match TcpStream::connect_timeout(&addr, IPC_CONNECT_TIMEOUT) {
            Ok(stream) => stream,
            Err(_) => return Ok(None),
        };
        stream.set_write_timeout(Some(IPC_WRITE_TIMEOUT))?;
        let token = endpoint_ipc_token(endpoint)?;
        let request = IpcRequest {
            token,
            client_id: self.client_id.clone(),
            trusted_write: self.allow_write,
            command: command.to_string(),
            args,
        };
        let request = encode_ipc_request(&request, MAX_IPC_REQUEST_BYTES)?;
        stream.write_all(&request)?;
        stream.shutdown(Shutdown::Write)?;
        let raw = read_ipc_response_with_limits(
            &mut stream,
            MAX_IPC_RESPONSE_BYTES,
            IPC_RESPONSE_TIMEOUT,
        )?;
        let response = serde_json::from_slice::<IpcResponse>(&raw)?;
        if response.ok {
            Ok(Some(response.value.unwrap_or(Value::Null)))
        } else {
            Err(anyhow!(
                "desktop IPC error: {}",
                response
                    .error
                    .unwrap_or_else(|| "unknown error".to_string())
            ))
        }
    }
}

fn load_store_from_path(path: &std::path::Path) -> Option<SessionStore> {
    let mut store = if path.extension().and_then(|value| value.to_str()) == Some("sqlite3") {
        let connection = SqliteConnection::open(path).ok()?;
        ensure_store_schema(&connection).ok()?;
        let raw = connection
            .query_row(
                "select value from kv where key = ?1",
                params![STORE_KEY],
                |row| row.get::<_, String>(0),
            )
            .ok()?;
        serde_json::from_str::<SessionStore>(&raw).ok()?
    } else {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<SessionStore>(&raw).ok())?
    };
    store.normalize_bounded_histories();
    Some(store)
}

fn load_ipc_endpoint(store_path: &std::path::Path) -> Option<IpcEndpointFile> {
    let endpoint_path = store_path.with_file_name("portmate-ipc.json");
    let raw = match read_ipc_endpoint_file(&endpoint_path) {
        Ok(raw) => raw,
        Err(error) => {
            eprintln!("PortMate MCP ignored unreadable desktop IPC endpoint: {error}");
            return None;
        }
    };
    let endpoint = serde_json::from_slice::<IpcEndpointFile>(&raw).ok()?;
    if let Err(error) = validate_ipc_endpoint(&endpoint, store_path) {
        eprintln!("PortMate MCP ignored invalid desktop IPC endpoint: {error}");
        return None;
    }
    Some(endpoint)
}

fn read_ipc_endpoint_file(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(anyhow!("desktop IPC endpoint must be a regular file"));
    }
    if metadata.len() > MAX_IPC_ENDPOINT_BYTES as u64 {
        return Err(anyhow!(
            "desktop IPC endpoint exceeds the {MAX_IPC_ENDPOINT_BYTES}-byte limit"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(anyhow!(
                "desktop IPC endpoint permissions must not allow group or world access"
            ));
        }
    }
    let mut file = fs::File::open(path)?;
    let mut raw = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_IPC_ENDPOINT_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut raw)?;
    if raw.len() > MAX_IPC_ENDPOINT_BYTES {
        return Err(anyhow!(
            "desktop IPC endpoint exceeds the {MAX_IPC_ENDPOINT_BYTES}-byte limit"
        ));
    }
    Ok(raw)
}

fn validate_ipc_endpoint(endpoint: &IpcEndpointFile, store_path: &Path) -> Result<SocketAddr> {
    let addr = endpoint
        .addr
        .parse::<SocketAddr>()
        .map_err(|error| anyhow!("desktop IPC address must be an IP socket address: {error}"))?;
    if !addr.ip().is_loopback() {
        return Err(anyhow!("desktop IPC address must be loopback; got {addr}"));
    }
    if !paths_refer_to_same_store(Path::new(&endpoint.store_path), store_path) {
        return Err(anyhow!(
            "desktop IPC endpoint storePath does not match PORTMATE_STORE_PATH"
        ));
    }
    match (&endpoint.token, &endpoint.token_ref) {
        (Some(token), None) if valid_inline_ipc_token(token) => {}
        (None, Some(token_ref)) if valid_ipc_token_ref(token_ref) => {}
        (Some(_), Some(_)) => {
            return Err(anyhow!(
                "desktop IPC endpoint must not contain both token and tokenRef"
            ))
        }
        (Some(_), None) => return Err(anyhow!("desktop IPC endpoint token is invalid")),
        (None, Some(_)) => return Err(anyhow!("desktop IPC endpoint tokenRef is invalid")),
        (None, None) => return Err(anyhow!("desktop IPC endpoint is missing token/tokenRef")),
    }
    Ok(addr)
}

fn paths_refer_to_same_store(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => absolute_path(left)
            .is_some_and(|left| absolute_path(right).is_some_and(|right| left == right)),
    }
}

fn absolute_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        std::env::current_dir().ok().map(|cwd| cwd.join(path))
    }
}

fn valid_inline_ipc_token(token: &str) -> bool {
    !token.trim().is_empty() && token.len() <= MAX_IPC_TOKEN_BYTES
}

fn valid_ipc_token_ref(token_ref: &str) -> bool {
    let Some(account) = token_ref.trim().strip_prefix("keychain:ipc-") else {
        return false;
    };
    !account.is_empty()
        && account.len() <= MAX_IPC_TOKEN_BYTES
        && account
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn endpoint_ipc_token(endpoint: &IpcEndpointFile) -> Result<String> {
    if let Some(token_ref) = endpoint
        .token_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !valid_ipc_token_ref(token_ref) {
            return Err(anyhow!("desktop IPC endpoint tokenRef is invalid"));
        }
        return read_secret_from_keyring(token_ref);
    }
    endpoint
        .token
        .clone()
        .filter(|value| valid_inline_ipc_token(value))
        .ok_or_else(|| anyhow!("desktop IPC endpoint is missing token/tokenRef"))
}

fn encode_ipc_request(request: &IpcRequest, max_bytes: usize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(request)?;
    if bytes.len() > max_bytes {
        return Err(anyhow!(
            "desktop IPC request exceeds the {max_bytes}-byte limit"
        ));
    }
    Ok(bytes)
}

fn read_ipc_response_with_limits(
    stream: &mut TcpStream,
    max_bytes: usize,
    timeout: Duration,
) -> Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = read_stream_chunk_before(
            stream,
            &mut buffer,
            deadline,
            "desktop IPC response deadline exceeded",
        )?;
        if read == 0 {
            break;
        }
        if raw.len().saturating_add(read) > max_bytes {
            return Err(anyhow!(
                "desktop IPC response exceeds the {max_bytes}-byte limit"
            ));
        }
        raw.extend_from_slice(&buffer[..read]);
    }
    Ok(raw)
}

fn ensure_keyring_store() -> Result<()> {
    static KEYRING_INIT: OnceLock<Result<(), String>> = OnceLock::new();
    KEYRING_INIT
        .get_or_init(|| {
            keyring::use_native_store(true)
                .or_else(|_| keyring::use_native_store(false))
                .map_err(|error| format!("system keyring initialization failed: {error}"))
        })
        .clone()
        .map_err(|error| anyhow!(error))
}

fn keyring_entry(secret_ref: &str) -> Result<Entry> {
    ensure_keyring_store()?;
    let account = secret_ref
        .trim()
        .strip_prefix("keychain:")
        .unwrap_or_else(|| secret_ref.trim());
    if account.is_empty() || account.contains('\0') {
        return Err(anyhow!("invalid secretRef"));
    }
    Entry::new("PortMate", account)
        .map_err(|error| anyhow!("failed to create keyring entry: {error}"))
}

fn read_secret_from_keyring(secret_ref: &str) -> Result<String> {
    keyring_entry(secret_ref)?
        .get_password()
        .map_err(|error| anyhow!("failed to read keyring secret {secret_ref}: {error:?}"))
}

fn write_secret_to_keyring(secret_ref: &str, secret: &str) -> Result<()> {
    keyring_entry(secret_ref)?
        .set_password(secret)
        .map_err(|error| anyhow!("failed to write keyring secret {secret_ref}: {error:?}"))
}

/// Redacts secrets out of events read from the local store snapshot fallback
/// (used when live desktop IPC is unavailable) before they reach an MCP client.
/// The live-IPC path is redacted on the desktop side (src-tauri) since that is
/// the same trust boundary crossing — an external MCP client, not the local operator.
fn redact_events(events: Vec<SessionEvent>) -> Vec<SessionEvent> {
    events
        .into_iter()
        .map(|mut event| {
            event.text = event.text.map(|text| redact_secrets(&text));
            event
        })
        .collect()
}

fn ipc_value_to_text(value: Value) -> Result<String> {
    if let Some(text) = value.as_str() {
        Ok(text.to_string())
    } else {
        Ok(serde_json::to_string_pretty(&value)?)
    }
}

fn ensure_store_schema(connection: &SqliteConnection) -> Result<()> {
    connection.execute_batch(
        "create table if not exists kv (
            key text primary key not null,
            value text not null,
            updated_at text not null
        );
        create table if not exists metadata (
            key text primary key not null,
            value text not null
        );
        create table if not exists profiles (
            id text primary key not null,
            name text not null,
            kind text not null,
            group_name text not null,
            tags_json text not null,
            connection_json text not null,
            terminal_json text not null,
            logging_json text not null,
            triggers_json text not null,
            transfer_json text not null,
            updated_at text not null
        );
        create table if not exists runtimes (
            session_id text primary key not null,
            pane_id text not null,
            status text not null,
            title text not null,
            cwd text,
            connected_since text,
            last_activity text not null,
            active_transport text not null,
            raw_json text not null
        );
        create table if not exists events (
            id text primary key not null,
            session_id text not null,
            pane_id text not null,
            ts text not null,
            direction text not null,
            stream text not null,
            bytes_ref text,
            text text,
            annotations_json text not null,
            raw_json text not null
        );
        create table if not exists transfers (
            id text primary key not null,
            session_id text not null,
            protocol text not null,
            source text not null,
            destination text not null,
            bytes_total integer not null,
            bytes_done integer not null,
            status text not null,
            message text,
            raw_json text not null
        );
        create table if not exists trusted_host_keys (
            id text primary key not null,
            profile_id text,
            alias text not null,
            host text not null,
            port integer not null,
            algorithm text not null,
            fingerprint_sha256 text not null,
            public_key_base64 text not null,
            scope text not null,
            label text,
            first_seen text not null,
            last_seen text not null,
            raw_json text not null
        );
        create table if not exists mcp_grants (
            client_id text primary key not null,
            name text not null,
            scopes_json text not null,
            allowed_sessions_json text not null,
            expires_at text,
            revoked_at text,
            raw_json text not null
        );
        create table if not exists mcp_audit (
            id text primary key not null,
            ts text not null,
            actor text not null,
            action text not null,
            session_id text,
            decision text not null,
            details_json text not null,
            raw_json text not null
        );
        create table if not exists timeline_marks (
            id text primary key not null,
            session_id text not null,
            ts text not null,
            label text not null,
            details text,
            raw_json text not null
        );
        create table if not exists sysmon_snapshots (
            session_id text not null,
            ts text not null,
            uptime_seconds integer not null,
            cpu_percent real not null,
            memory_percent real not null,
            rx_kbps real not null,
            tx_kbps real not null,
            raw_json text not null,
            primary key (session_id, ts)
        );
        create index if not exists idx_events_session_ts on events(session_id, ts);
        create index if not exists idx_events_text on events(text);
        create index if not exists idx_transfers_session on transfers(session_id);
        create index if not exists idx_host_keys_alias on trusted_host_keys(alias, port, algorithm);
        create index if not exists idx_audit_session_ts on mcp_audit(session_id, ts);
        create index if not exists idx_timeline_session_ts on timeline_marks(session_id, ts);
        create index if not exists idx_sysmon_session_ts on sysmon_snapshots(session_id, ts);
        insert into metadata (key, value) values ('schemaVersion', '2')
            on conflict(key) do update set value = excluded.value;",
    )?;
    Ok(())
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
    let token = std::env::var("PORTMATE_MCP_HTTP_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
        .or_else(|| read_or_create_http_token().ok())
        .ok_or_else(|| anyhow!("failed to load or create MCP HTTP token"))?;
    let mut allowed_origins = std::env::var("PORTMATE_MCP_HTTP_ORIGINS")
        .or_else(|_| std::env::var("PORTMATE_MCP_HTTP_ORIGIN"))
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if allowed_origins.is_empty() {
        allowed_origins.push(format!("http://127.0.0.1:{}", addr.port()));
        allowed_origins.push(format!("http://localhost:{}", addr.port()));
    }
    Ok(HttpConfig {
        addr,
        token,
        allowed_origins,
    })
}

fn read_or_create_http_token() -> Result<String> {
    match read_secret_from_keyring(HTTP_TOKEN_REF) {
        Ok(token) if !token.trim().is_empty() => Ok(token),
        _ => {
            let token = Uuid::new_v4().to_string();
            write_secret_to_keyring(HTTP_TOKEN_REF, &token)?;
            Ok(token)
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    read_http_request_with_timeout(stream, HTTP_REQUEST_TIMEOUT)
}

fn read_http_request_with_timeout(
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
    if headers.contains_key("transfer-encoding") {
        return Err(anyhow!(
            "Transfer-Encoding is not supported; send a Content-Length body"
        ));
    }

    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>())
        .transpose()
        .map_err(|error| anyhow!("invalid Content-Length: {error}"))?
        .unwrap_or(0);
    if content_length > MAX_HTTP_BODY_BYTES {
        return Err(anyhow!("HTTP body is too large"));
    }
    let mut body = raw.get(body_start..).unwrap_or_default().to_vec();
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
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
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

fn read_stream_chunk_before(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
    timeout_message: &'static str,
) -> io::Result<usize> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(io::Error::new(io::ErrorKind::TimedOut, timeout_message));
    }
    stream.set_read_timeout(Some(remaining))?;
    stream.read(buffer).map_err(|error| {
        if error.kind() == io::ErrorKind::WouldBlock {
            io::Error::new(io::ErrorKind::TimedOut, timeout_message)
        } else {
            error
        }
    })
}

fn find_header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|window| window == b"\r\n\r\n")
}

fn handle_http_request(request: HttpRequest, config: &HttpConfig) -> String {
    let origin = request.headers.get("origin").cloned();
    if let Err(error) = validate_origin(origin.as_deref(), config) {
        return http_response(
            403,
            "Forbidden",
            &json!({ "error": error.to_string() }).to_string(),
            origin.as_deref(),
        );
    }

    if request.method == "OPTIONS" {
        return http_response(204, "No Content", "", origin.as_deref());
    }

    if request.path != "/mcp" {
        return http_response(
            404,
            "Not Found",
            &json!({ "error": "unknown endpoint" }).to_string(),
            origin.as_deref(),
        );
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
    if !authorized_http_request(&request, &config.token) {
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
            return http_response(200, "OK", &json_rpc_http_body(&response), origin.as_deref());
        }
    };
    let body = match handle_http_json_rpc(value) {
        Ok(Some(value)) => json_rpc_http_body(&value),
        Ok(None) => {
            return http_response(202, "Accepted", "", origin.as_deref());
        }
        Err(error) => json_rpc_http_body(&json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32603, "message": error.to_string() }
        })),
    };
    if accepts_sse && !accepts_json {
        http_sse_message_response(&body, origin.as_deref())
    } else {
        http_response(200, "OK", &body, origin.as_deref())
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

fn validate_origin(origin: Option<&str>, config: &HttpConfig) -> Result<()> {
    let Some(origin) = origin.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if config
        .allowed_origins
        .iter()
        .any(|allowed| allowed == "*" || allowed == origin)
    {
        Ok(())
    } else {
        Err(anyhow!("Origin `{origin}` is not allowed"))
    }
}

fn authorized_http_request(request: &HttpRequest, token: &str) -> bool {
    if let Some(value) = request.headers.get("authorization") {
        let mut parts = value.split_whitespace();
        if parts
            .next()
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))
        {
            if let (Some(candidate), None) = (parts.next(), parts.next()) {
                return constant_time_str_eq(candidate, token);
            }
        }
    }
    request
        .headers
        .get("x-portmate-mcp-token")
        .is_some_and(|candidate| constant_time_str_eq(candidate.trim(), token))
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

fn constant_time_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn http_response(status: u16, reason: &str, body: &str, origin: Option<&str>) -> String {
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
    response.push_str(&format!("MCP-Protocol-Version: {MCP_PROTOCOL_VERSION}\r\n"));
    response.push_str(
        "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Accept, Authorization, Content-Type, X-PortMate-MCP-Token\r\n\r\n",
    );
    response.push_str(body);
    response
}

fn http_sse_stream_start_response(request: &HttpRequest, config: &HttpConfig) -> String {
    let origin = request.headers.get("origin").cloned();
    if let Err(error) = validate_origin(origin.as_deref(), config) {
        return http_response(
            403,
            "Forbidden",
            &json!({ "error": error.to_string() }).to_string(),
            origin.as_deref(),
        );
    }
    if !authorized_http_request(request, &config.token) {
        return http_response(
            401,
            "Unauthorized",
            &json!({ "error": "missing or invalid MCP HTTP token" }).to_string(),
            origin.as_deref(),
        );
    }
    let mut response = http_sse_headers(origin.as_deref(), None);
    response.push_str(&sse_event(
        "endpoint",
        &json!({
            "uri": "/mcp",
            "method": "POST",
            "protocolVersion": MCP_PROTOCOL_VERSION
        }),
    ));
    response.push_str(&sse_event("portmate.state", &mcp_sse_state_payload()));
    response
}

fn write_http_sse_stream(
    stream: &mut TcpStream,
    request: HttpRequest,
    config: &HttpConfig,
) -> Result<()> {
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
            sse_event("portmate.state", &mcp_sse_state_payload())
        );
        stream.write_all(event.as_bytes())?;
        stream.flush()?;
    }
}

fn http_sse_message_response(body: &str, origin: Option<&str>) -> String {
    let event = sse_event(
        "message",
        &serde_json::from_str::<Value>(body).unwrap_or_else(|_| json!({ "text": body })),
    );
    let mut response = http_sse_headers(origin, Some(event.len()));
    response.push_str(&event);
    response
}

fn http_sse_headers(origin: Option<&str>, content_length: Option<usize>) -> String {
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
    response.push_str(&format!("MCP-Protocol-Version: {MCP_PROTOCOL_VERSION}\r\n"));
    response.push_str(
        "Access-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Accept, Authorization, Content-Type, X-PortMate-MCP-Token\r\n\r\n",
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

fn mcp_sse_state_payload() -> Value {
    let server = PortMateMcp::new();
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "serverInfo": {
            "name": "portmate-mcp",
            "title": "PortMate MCP Bridge",
            "version": env!("CARGO_PKG_VERSION")
        },
        "sessions": server.store.summaries()
    })
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

fn parse_session_uri(uri: &str) -> Option<(&str, &str)> {
    let path = uri.strip_prefix("portmate://sessions/")?;
    let mut parts = path.split('/');
    let id = parts.next()?;
    let suffix = parts.next()?.split('?').next().unwrap_or_default();
    Some((id, suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_http_config() -> HttpConfig {
        HttpConfig {
            addr: "127.0.0.1:8787".parse().unwrap(),
            token: "secret-token".to_string(),
            allowed_origins: vec!["http://127.0.0.1:8787".to_string()],
        }
    }

    fn test_http_request(headers: HashMap<String, String>) -> HttpRequest {
        HttpRequest {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            headers,
            body: serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {}
            }))
            .unwrap(),
        }
    }

    fn test_http_get_request(headers: HashMap<String, String>) -> HttpRequest {
        HttpRequest {
            method: "GET".to_string(),
            path: "/mcp".to_string(),
            headers,
            body: Vec::new(),
        }
    }

    fn test_tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    fn parse_http_request_bytes(bytes: &[u8]) -> Result<HttpRequest> {
        let (mut client, mut server) = test_tcp_pair();
        let bytes = bytes.to_vec();
        let writer = thread::spawn(move || {
            client.write_all(&bytes).unwrap();
            client.shutdown(Shutdown::Write).unwrap();
        });
        let result = read_http_request_with_timeout(&mut server, Duration::from_secs(1));
        writer.join().unwrap();
        result
    }

    #[test]
    fn stdio_reader_bounds_messages_and_recovers_at_the_next_line() {
        let input = b"abcdefghijkl\n12345678\r\n{\"x\":1}\n";
        let mut reader = io::Cursor::new(input);

        assert_eq!(
            read_stdio_message(&mut reader, 8).unwrap(),
            StdioMessage::TooLarge
        );
        assert_eq!(
            read_stdio_message(&mut reader, 8).unwrap(),
            StdioMessage::Message(b"12345678".to_vec())
        );
        assert_eq!(
            read_stdio_message(&mut reader, 8).unwrap(),
            StdioMessage::Message(b"{\"x\":1}".to_vec())
        );
        assert_eq!(
            read_stdio_message(&mut reader, 8).unwrap(),
            StdioMessage::Eof
        );
    }

    #[test]
    fn json_rpc_response_serialization_is_bounded_and_preserves_id_on_overflow() {
        let compact = json!({ "ok": true });
        let compact_bytes = serde_json::to_vec(&compact).unwrap();
        assert_eq!(
            try_encode_json_with_limit(&compact, compact_bytes.len()).unwrap(),
            Some(compact_bytes.clone())
        );
        assert!(
            try_encode_json_with_limit(&compact, compact_bytes.len() - 1)
                .unwrap()
                .is_none()
        );

        let response = json!({
            "jsonrpc": "2.0",
            "id": "request-7",
            "result": { "content": "x".repeat(1024) }
        });
        let encoded = encode_json_rpc_response(&response, 256).unwrap();
        assert!(encoded.len() <= 256);
        let overflow: Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(overflow["id"], "request-7");
        assert_eq!(overflow["error"]["code"], -32603);
        assert!(overflow["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("256-byte limit")));
        assert!(overflow.get("result").is_none());
    }

    #[test]
    fn sse_event_replaces_oversized_state_data() {
        let event = sse_event_with_limit(
            "portmate.state",
            &json!({ "content": "sensitive-marker".repeat(128) }),
            128,
        );

        assert!(event.starts_with("event: portmate.state\n"));
        assert!(event.contains("SSE data exceeds the 128-byte limit"));
        assert!(!event.contains("sensitive-marker"));
        assert!(event.len() < 256);
    }

    #[test]
    fn desktop_ipc_endpoint_rejects_non_loopback_wrong_store_and_unsafe_token_refs() {
        let root = std::env::temp_dir().join(format!("portmate-mcp-endpoint-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let store_path = root.join("portmate-store.sqlite3");
        let other_store_path = root.join("other-store.sqlite3");
        fs::write(&store_path, b"store").unwrap();
        fs::write(&other_store_path, b"other").unwrap();
        let mut endpoint = IpcEndpointFile {
            addr: "127.0.0.1:43123".to_string(),
            token: None,
            token_ref: Some(format!("keychain:ipc-{}", Uuid::new_v4())),
            store_path: store_path.display().to_string(),
        };

        assert_eq!(
            validate_ipc_endpoint(&endpoint, &store_path).unwrap(),
            "127.0.0.1:43123".parse::<SocketAddr>().unwrap()
        );
        assert!(validate_ipc_endpoint(&endpoint, &other_store_path).is_err());

        endpoint.addr = "192.0.2.1:43123".to_string();
        assert!(validate_ipc_endpoint(&endpoint, &store_path)
            .unwrap_err()
            .to_string()
            .contains("must be loopback"));
        endpoint.addr = "127.0.0.1:43123".to_string();
        endpoint.token_ref = Some("keychain:mcp-http-token".to_string());
        assert!(validate_ipc_endpoint(&endpoint, &store_path)
            .unwrap_err()
            .to_string()
            .contains("tokenRef is invalid"));
        assert!(endpoint_ipc_token(&endpoint)
            .unwrap_err()
            .to_string()
            .contains("tokenRef is invalid"));

        endpoint.token = Some("inline-token".to_string());
        assert!(validate_ipc_endpoint(&endpoint, &store_path)
            .unwrap_err()
            .to_string()
            .contains("must not contain both"));
        endpoint.token_ref = None;
        assert!(validate_ipc_endpoint(&endpoint, &store_path).is_ok());
        assert_eq!(endpoint_ipc_token(&endpoint).unwrap(), "inline-token");

        let endpoint_path = root.join("portmate-ipc.json");
        fs::write(&endpoint_path, serde_json::to_vec(&endpoint).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(load_ipc_endpoint(&store_path).is_some());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(load_ipc_endpoint(&store_path).is_none());
            fs::remove_file(&endpoint_path).unwrap();
            std::os::unix::fs::symlink(&store_path, &endpoint_path).unwrap();
            assert!(read_ipc_endpoint_file(&endpoint_path)
                .unwrap_err()
                .to_string()
                .contains("regular file"));
            fs::remove_file(&endpoint_path).unwrap();
        }
        fs::write(&endpoint_path, vec![b'x'; MAX_IPC_ENDPOINT_BYTES + 1]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(read_ipc_endpoint_file(&endpoint_path)
            .unwrap_err()
            .to_string()
            .contains("byte limit"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn desktop_ipc_request_and_response_are_bounded() {
        let oversized = IpcRequest {
            token: "token".to_string(),
            client_id: "client".to_string(),
            trusted_write: false,
            command: "send_text".to_string(),
            args: json!({ "sessionId": "session", "text": "x".repeat(128) }),
        };
        let error = encode_ipc_request(&oversized, 64).unwrap_err();
        assert!(error.to_string().contains("64-byte limit"));

        let (mut client, mut server) = test_tcp_pair();
        let writer = thread::spawn(move || {
            server.write_all(&[b'x'; 33]).unwrap();
            server.shutdown(Shutdown::Write).unwrap();
        });
        let error =
            read_ipc_response_with_limits(&mut client, 32, Duration::from_secs(1)).unwrap_err();
        assert!(error.to_string().contains("32-byte limit"));
        writer.join().unwrap();
    }

    #[test]
    fn http_request_deadline_cannot_be_extended_by_trickle_bytes() {
        let (mut client, mut server) = test_tcp_pair();
        let writer = thread::spawn(move || {
            for byte in b"GET /mcp HTTP/1.1\r\nHost: localhost\r\n" {
                if client.write_all(&[*byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(15));
            }
        });
        let started = Instant::now();
        let error =
            read_http_request_with_timeout(&mut server, Duration::from_millis(60)).unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(io::ErrorKind::TimedOut)
        );
        assert!(started.elapsed() < Duration::from_millis(500));
        drop(server);
        writer.join().unwrap();
    }

    #[test]
    fn http_parser_rejects_ambiguous_or_unsupported_framing() {
        for request in [
            b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
            b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer one\r\nAuthorization: Bearer two\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
        ] {
            assert!(parse_http_request_bytes(request)
                .unwrap_err()
                .to_string()
                .contains("duplicate HTTP header"));
        }

        let chunked = b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n";
        assert!(parse_http_request_bytes(chunked)
            .unwrap_err()
            .to_string()
            .contains("Transfer-Encoding is not supported"));

        let extra = b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\n{}extra";
        assert!(parse_http_request_bytes(extra)
            .unwrap_err()
            .to_string()
            .contains("bytes after its declared body"));

        let malformed = b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nnot-a-header\r\n\r\n";
        assert!(parse_http_request_bytes(malformed)
            .unwrap_err()
            .to_string()
            .contains("invalid HTTP headers"));
    }

    #[test]
    fn http_parser_combines_repeatable_headers_and_reads_exact_body() {
        let request = parse_http_request_bytes(
            b"POST /mcp HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nAccept: text/event-stream\r\nContent-Length: 2\r\n\r\n{}",
        )
        .unwrap();

        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/mcp");
        assert_eq!(
            request.headers.get("accept").map(String::as_str),
            Some("application/json, text/event-stream")
        );
        assert_eq!(request.body, b"{}");
    }

    #[test]
    fn http_connection_limit_rejects_excess_and_releases_completed_slots() {
        let config = test_http_config();
        let active = Arc::new(AtomicUsize::new(0));
        let permit = try_acquire_http_connection(&active, 1).unwrap();
        assert_eq!(active.load(Ordering::Acquire), 1);

        let (mut rejected_client, rejected_server) = test_tcp_pair();
        assert!(!spawn_http_connection(
            rejected_server,
            config.clone(),
            Arc::clone(&active),
            1,
        ));
        let mut rejected_response = String::new();
        rejected_client
            .read_to_string(&mut rejected_response)
            .unwrap();
        assert!(rejected_response.starts_with("HTTP/1.1 503 Service Unavailable"));
        assert!(rejected_response.contains("Connection: close"));
        assert_eq!(active.load(Ordering::Acquire), 1);

        drop(permit);
        assert_eq!(active.load(Ordering::Acquire), 0);
        let (mut accepted_client, accepted_server) = test_tcp_pair();
        accepted_client
            .write_all(b"OPTIONS /mcp HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        accepted_client.shutdown(Shutdown::Write).unwrap();
        assert!(spawn_http_connection(
            accepted_server,
            config,
            Arc::clone(&active),
            1,
        ));
        let mut accepted_response = String::new();
        accepted_client
            .read_to_string(&mut accepted_response)
            .unwrap();
        assert!(accepted_response.starts_with("HTTP/1.1 204 No Content"));
        for _ in 0..100 {
            if active.load(Ordering::Acquire) == 0 {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(active.load(Ordering::Acquire), 0);
        assert!(try_acquire_http_connection(&active, 1).is_some());
    }

    #[test]
    fn http_origin_requires_allow_list_match_when_present() {
        let config = test_http_config();
        assert!(validate_origin(None, &config).is_ok());
        assert!(validate_origin(Some("http://127.0.0.1:8787"), &config).is_ok());
        assert!(validate_origin(Some("http://evil.example"), &config).is_err());
    }

    #[test]
    fn http_token_accepts_bearer_or_portmate_header() {
        let mut headers = HashMap::new();
        headers.insert(
            "authorization".to_string(),
            "bearer secret-token".to_string(),
        );
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            headers,
            body: Vec::new(),
        };
        assert!(authorized_http_request(&request, "secret-token"));

        let mut invalid_headers = HashMap::new();
        invalid_headers.insert(
            "authorization".to_string(),
            "Bearer secret-token trailing".to_string(),
        );
        assert!(!authorized_http_request(
            &test_http_request(invalid_headers),
            "secret-token"
        ));

        let mut headers = HashMap::new();
        headers.insert(
            "x-portmate-mcp-token".to_string(),
            "secret-token".to_string(),
        );
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            headers,
            body: Vec::new(),
        };
        assert!(authorized_http_request(&request, "secret-token"));
        assert!(!authorized_http_request(&request, "different-token"));
    }

    #[test]
    fn http_accept_respects_zero_quality_values() {
        let mut headers = HashMap::new();
        headers.insert(
            "accept".to_string(),
            "application/json; q=0.0, text/event-stream; q=1".to_string(),
        );
        let request = test_http_request(headers);

        assert!(!accepts_json_http_response(&request));
        assert!(accepts_sse_http_response(&request));
    }

    #[test]
    fn http_json_rpc_initialize_returns_server_info() {
        let response = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        }))
        .unwrap()
        .unwrap();
        assert_eq!(response["id"], json!(1));
        assert_eq!(response["result"]["serverInfo"]["name"], "portmate-mcp");
    }

    #[test]
    fn mcp_lists_concrete_resources_separately_from_templates() {
        let resources = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "resources/list",
            "params": {}
        }))
        .unwrap()
        .unwrap();
        let listed = resources["result"]["resources"].as_array().unwrap();
        assert_eq!(listed[0]["uri"], "portmate://sessions");
        assert!(listed
            .iter()
            .all(|resource| !resource["uri"].as_str().unwrap().contains('{')));

        let templates = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "resources/templates/list",
            "params": {}
        }))
        .unwrap()
        .unwrap();
        let listed = templates["result"]["resourceTemplates"].as_array().unwrap();
        assert!(!listed.is_empty());
        assert!(listed
            .iter()
            .all(|resource| resource["uriTemplate"].as_str().unwrap().contains('{')));
    }

    #[test]
    fn mcp_ping_returns_empty_result() {
        let response = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "id": "ping-1",
            "method": "ping"
        }))
        .unwrap()
        .unwrap();

        assert_eq!(response["id"], "ping-1");
        assert_eq!(response["result"], json!({}));
    }

    #[test]
    fn mcp_log_query_limit_matches_declared_schema_bounds() {
        assert_eq!(bounded_log_query_limit(None), 100);
        assert_eq!(bounded_log_query_limit(Some(0)), 1);
        assert_eq!(bounded_log_query_limit(Some(600)), 600);
        assert_eq!(bounded_log_query_limit(Some(u64::MAX)), 1000);
    }

    #[test]
    fn json_rpc_empty_batch_is_invalid_and_notifications_have_no_payload() {
        let empty = handle_http_json_rpc(json!([])).unwrap().unwrap();
        assert_eq!(empty["error"]["code"], -32600);

        let notification = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .unwrap();
        assert!(notification.is_none());

        let notification_batch = handle_http_json_rpc(json!([
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {}}
        ]))
        .unwrap();
        assert!(notification_batch.is_none());
    }

    #[test]
    fn json_rpc_envelopes_preserve_null_ids_and_reject_invalid_shapes() {
        let null_id = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "id": null,
            "method": "ping"
        }))
        .unwrap()
        .unwrap();
        assert_eq!(null_id["id"], Value::Null);
        assert_eq!(null_id["result"], json!({}));

        let invalid_id = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "id": { "nested": true },
            "method": "ping"
        }))
        .unwrap()
        .unwrap();
        assert_eq!(invalid_id["id"], Value::Null);
        assert_eq!(invalid_id["error"]["code"], -32600);

        let invalid_params = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "ping",
            "params": null
        }))
        .unwrap()
        .unwrap();
        assert_eq!(invalid_params["id"], 1);
        assert_eq!(invalid_params["error"]["code"], -32602);

        let invalid_notification = handle_http_json_rpc(json!({
            "jsonrpc": "2.0",
            "method": "ping",
            "params": "invalid"
        }))
        .unwrap();
        assert!(invalid_notification.is_none());
    }

    #[test]
    fn json_rpc_batch_is_bounded_before_dispatch() {
        let accepted = (0..MAX_JSON_RPC_BATCH_ITEMS)
            .map(|id| json!({ "jsonrpc": "2.0", "id": id, "method": "ping" }))
            .collect::<Vec<_>>();
        let accepted = handle_http_json_rpc(Value::Array(accepted))
            .unwrap()
            .unwrap();
        assert_eq!(
            accepted.as_array().map(Vec::len),
            Some(MAX_JSON_RPC_BATCH_ITEMS)
        );

        let oversized = (0..=MAX_JSON_RPC_BATCH_ITEMS)
            .map(|id| json!({ "jsonrpc": "2.0", "id": id, "method": "ping" }))
            .collect::<Vec<_>>();
        let rejected = handle_http_json_rpc(Value::Array(oversized))
            .unwrap()
            .unwrap();
        assert!(!rejected.is_array());
        assert_eq!(rejected["id"], Value::Null);
        assert_eq!(rejected["error"]["code"], -32600);
        assert!(rejected["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("128-item limit")));
    }

    #[test]
    fn http_notification_returns_accepted_without_json_null() {
        let config = test_http_config();
        let mut headers = HashMap::new();
        headers.insert(
            "authorization".to_string(),
            "Bearer secret-token".to_string(),
        );
        headers.insert("accept".to_string(), "application/json".to_string());
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/mcp".to_string(),
            headers,
            body: serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .unwrap(),
        };

        let response = handle_http_request(request, &config);

        assert!(response.starts_with("HTTP/1.1 202 Accepted"));
        assert!(response.ends_with("\r\n\r\n"));
        assert!(!response.ends_with("null"));
    }

    #[test]
    fn http_streamable_accept_header_allows_json_response() {
        let config = test_http_config();
        let mut headers = HashMap::new();
        headers.insert(
            "authorization".to_string(),
            "Bearer secret-token".to_string(),
        );
        headers.insert(
            "accept".to_string(),
            "application/json, text/event-stream".to_string(),
        );

        let response = handle_http_request(test_http_request(headers), &config);

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("MCP-Protocol-Version: 2025-06-18"));
        assert!(response.contains("\"serverInfo\""));
    }

    #[test]
    fn http_get_sse_accept_header_returns_event_stream() {
        let config = test_http_config();
        let mut headers = HashMap::new();
        headers.insert(
            "authorization".to_string(),
            "Bearer secret-token".to_string(),
        );
        headers.insert("accept".to_string(), "text/event-stream".to_string());

        let response = handle_http_request(test_http_get_request(headers), &config);

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("Content-Type: text/event-stream"));
        assert!(response.contains("Connection: keep-alive"));
        assert!(response.contains("event: endpoint"));
        assert!(response.contains("event: portmate.state"));
        assert!(response.contains("\"protocolVersion\":\"2025-06-18\""));
    }

    #[test]
    fn http_post_sse_only_accept_header_returns_message_event() {
        let config = test_http_config();
        let mut headers = HashMap::new();
        headers.insert(
            "authorization".to_string(),
            "Bearer secret-token".to_string(),
        );
        headers.insert("accept".to_string(), "text/event-stream".to_string());

        let response = handle_http_request(test_http_request(headers), &config);

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("Content-Type: text/event-stream"));
        assert!(response.contains("Content-Length:"));
        assert!(response.contains("event: message"));
        assert!(response.contains("\"serverInfo\""));
    }
}

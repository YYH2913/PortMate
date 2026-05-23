use anyhow::{anyhow, Result};
use keyring_core::Entry;
use portmate_core::{
    prompt_templates, resource_templates, tool_definitions, McpScope, SessionStore,
};
use rusqlite::{params, Connection as SqliteConnection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::PathBuf;
use std::sync::OnceLock;

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const STORE_KEY: &str = "session-store";
const SQLITE_SCHEMA_VERSION: &str = "2";

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
        if request.jsonrpc != "2.0" {
            return Ok(Some(error(
                request.id.unwrap_or(Value::Null),
                -32600,
                "invalid JSON-RPC version",
            )));
        }

        let Some(id) = request.id.clone() else {
            return Ok(None);
        };

        let result = match request.method.as_str() {
            "initialize" => self.initialize_result(),
            "tools/list" => json!({
                "tools": tool_definitions().into_iter().map(|tool| json!({
                    "name": tool.name,
                    "title": tool.title,
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                    "annotations": { "readOnlyHint": tool.read_only }
                })).collect::<Vec<_>>()
            }),
            "resources/list" => json!({
                "resources": resource_templates().into_iter().map(|resource| json!({
                    "uri": resource.uri_template,
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
                return Ok(Some(error(
                    id,
                    -32601,
                    format!("unknown method: {}", request.method),
                )));
            }
        };

        Ok(Some(JsonRpcResponse {
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
                "screen" => self.store.screen(session_id).unwrap_or_default(),
                "log" => self
                    .store
                    .tail_log(session_id, 200)
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
                    self.store.screen(session_id).unwrap_or_default()
                }
            }
            "tail_log" => {
                let session_id = required_string(&arguments, "sessionId")?;
                let limit = arguments
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(100) as usize;
                if let Some(value) = self.call_ipc_value("tail_log", arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    serde_json::to_string_pretty(&self.store.tail_log(session_id, limit))?
                }
            }
            "search_logs" => {
                let query = required_string(&arguments, "query")?;
                let session_id = arguments.get("sessionId").and_then(Value::as_str);
                let limit = arguments
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(100) as usize;
                if let Some(value) = self.call_ipc_value("search_logs", arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    serde_json::to_string_pretty(&self.store.search_logs(query, session_id, limit))?
                }
            }
            "send_text" | "send_key" | "run_command" => self.write_tool(name, &arguments)?,
            "export_session_bundle" => {
                let session_id = required_string(&arguments, "sessionId")?;
                serde_json::to_string_pretty(&self.store.export_session_bundle(session_id))?
            }
            "open_session" | "close_session" => {
                let session_id = required_string(&arguments, "sessionId")?;
                self.guard_scope(McpScope::ManageSessions, session_id)?;
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    format!("{name} accepted by PortMate bridge; desktop IPC is not available.")
                }
            }
            "start_transfer" => {
                let session_id = required_string(&arguments, "sessionId")?;
                self.guard_scope(McpScope::Transfer, session_id)?;
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    "start_transfer accepted by PortMate bridge; desktop IPC is not available."
                        .to_string()
                }
            }
            "create_tunnel" => {
                let session_id = required_string(&arguments, "sessionId")?;
                self.guard_scope(McpScope::Tunnel, session_id)?;
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    "create_tunnel accepted by PortMate bridge; desktop IPC is not available."
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
                let session_id = required_string(&arguments, "sessionId")?;
                let _ = required_string(&arguments, "target")?;
                self.guard_scope(McpScope::WriteInput, session_id)?;
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    ipc_value_to_text(value)?
                } else {
                    "attach_tmux accepted by PortMate bridge; desktop IPC is not available."
                        .to_string()
                }
            }
            _ => return Err(anyhow!("unknown tool: {name}")),
        };

        Ok(json!({
            "content": [{ "type": "text", "text": output }],
            "isError": false
        }))
    }

    fn write_tool(&mut self, name: &str, arguments: &Value) -> Result<String> {
        let session_id = required_string(arguments, "sessionId")?;
        self.guard_scope(McpScope::WriteInput, session_id)?;
        if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
            return ipc_value_to_text(value);
        }
        let text = match name {
            "send_text" => required_string(arguments, "text")?.to_string(),
            "send_key" => format!("<{}>", required_string(arguments, "key")?),
            "run_command" => format!("{}\n", required_string(arguments, "command")?),
            _ => unreachable!(),
        };
        let event = self
            .store
            .send_text(&self.client_id, session_id, &text)
            .map_err(|message| anyhow!(message))?;
        self.persist_store()?;
        Ok(serde_json::to_string_pretty(&event)?)
    }

    fn call_ipc_value(&self, command: &str, args: Value) -> Result<Option<Value>> {
        let Some(endpoint) = &self.ipc else {
            return Ok(None);
        };
        let mut stream = match TcpStream::connect(&endpoint.addr) {
            Ok(stream) => stream,
            Err(_) => return Ok(None),
        };
        let token = endpoint_ipc_token(endpoint)?;
        let request = IpcRequest {
            token,
            command: command.to_string(),
            args,
        };
        stream.write_all(&serde_json::to_vec(&request)?)?;
        stream.shutdown(Shutdown::Write)?;
        let mut raw = String::new();
        stream.read_to_string(&mut raw)?;
        let response = serde_json::from_str::<IpcResponse>(&raw)?;
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

    fn persist_store(&self) -> Result<()> {
        let Some(path) = &self.store_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.extension().and_then(|value| value.to_str()) == Some("sqlite3") {
            save_store_to_sqlite(path, &self.store)?;
        } else {
            fs::write(path, serde_json::to_vec_pretty(&self.store)?)?;
        }
        Ok(())
    }

    fn guard_scope(&self, scope: McpScope, session_id: &str) -> Result<()> {
        if self.allow_write
            && (self.store.mcp_can(&self.client_id, scope, Some(session_id))
                || self.store.grants.is_empty())
        {
            Ok(())
        } else {
            Err(anyhow!(
                "write denied: set PORTMATE_MCP_TRUSTED=1 and grant {:?} for client `{}`",
                scope,
                self.client_id
            ))
        }
    }
}

fn load_store_from_path(path: &std::path::Path) -> Option<SessionStore> {
    if path.extension().and_then(|value| value.to_str()) == Some("sqlite3") {
        let connection = SqliteConnection::open(path).ok()?;
        ensure_store_schema(&connection).ok()?;
        let raw = connection
            .query_row(
                "select value from kv where key = ?1",
                params![STORE_KEY],
                |row| row.get::<_, String>(0),
            )
            .ok()?;
        serde_json::from_str::<SessionStore>(&raw).ok()
    } else {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str::<SessionStore>(&raw).ok())
    }
}

fn load_ipc_endpoint(store_path: &std::path::Path) -> Option<IpcEndpointFile> {
    let endpoint_path = store_path.with_file_name("portmate-ipc.json");
    let raw = fs::read_to_string(endpoint_path).ok()?;
    serde_json::from_str::<IpcEndpointFile>(&raw).ok()
}

fn endpoint_ipc_token(endpoint: &IpcEndpointFile) -> Result<String> {
    if let Some(token_ref) = endpoint
        .token_ref
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return read_secret_from_keyring(token_ref);
    }
    endpoint
        .token
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("desktop IPC endpoint is missing token/tokenRef"))
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

fn ipc_value_to_text(value: Value) -> Result<String> {
    if let Some(text) = value.as_str() {
        Ok(text.to_string())
    } else {
        Ok(serde_json::to_string_pretty(&value)?)
    }
}

fn save_store_to_sqlite(path: &std::path::Path, store: &SessionStore) -> Result<()> {
    let connection = SqliteConnection::open(path)?;
    ensure_store_schema(&connection)?;
    connection.execute(
        "insert into kv (key, value, updated_at) values (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
         on conflict(key) do update set value = excluded.value, updated_at = excluded.updated_at",
        params![STORE_KEY, serde_json::to_string_pretty(store)?],
    )?;
    save_store_sqlite_tables(&connection, store)?;
    Ok(())
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

fn save_store_sqlite_tables(connection: &SqliteConnection, store: &SessionStore) -> Result<()> {
    connection.execute_batch(
        "delete from profiles;
         delete from runtimes;
         delete from events;
         delete from transfers;
         delete from trusted_host_keys;
         delete from mcp_grants;
         delete from mcp_audit;
         delete from timeline_marks;
         delete from sysmon_snapshots;",
    )?;

    for profile in &store.profiles {
        connection.execute(
            "insert into profiles (
                id, name, kind, group_name, tags_json, connection_json, terminal_json,
                logging_json, triggers_json, transfer_json, updated_at
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            params![
                profile.id,
                profile.name,
                enum_text(&profile.kind)?,
                profile.group,
                json_text(&profile.tags)?,
                json_text(&profile.connection)?,
                json_text(&profile.terminal)?,
                json_text(&profile.logging)?,
                json_text(&profile.triggers)?,
                json_text(&profile.transfer)?,
            ],
        )?;
    }

    for runtime in &store.runtimes {
        connection.execute(
            "insert into runtimes (
                session_id, pane_id, status, title, cwd, connected_since, last_activity,
                active_transport, raw_json
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                runtime.session_id,
                runtime.pane_id,
                enum_text(&runtime.status)?,
                runtime.title,
                runtime.cwd,
                runtime.connected_since.map(|value| value.to_rfc3339()),
                runtime.last_activity.to_rfc3339(),
                enum_text(&runtime.active_transport)?,
                json_text(runtime)?,
            ],
        )?;
    }

    for event in &store.events {
        connection.execute(
            "insert into events (
                id, session_id, pane_id, ts, direction, stream, bytes_ref, text,
                annotations_json, raw_json
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                event.id,
                event.session_id,
                event.pane_id,
                event.ts.to_rfc3339(),
                enum_text(&event.direction)?,
                enum_text(&event.stream)?,
                event.bytes_ref,
                event.text,
                json_text(&event.annotations)?,
                json_text(event)?,
            ],
        )?;
    }

    for transfer in &store.transfers {
        connection.execute(
            "insert into transfers (
                id, session_id, protocol, source, destination, bytes_total, bytes_done,
                status, message, raw_json
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                transfer.id,
                transfer.session_id,
                enum_text(&transfer.protocol)?,
                transfer.source,
                transfer.destination,
                transfer.bytes_total as i64,
                transfer.bytes_done as i64,
                enum_text(&transfer.status)?,
                transfer.message,
                json_text(transfer)?,
            ],
        )?;
    }

    for key in &store.host_keys.keys {
        connection.execute(
            "insert into trusted_host_keys (
                id, profile_id, alias, host, port, algorithm, fingerprint_sha256,
                public_key_base64, scope, label, first_seen, last_seen, raw_json
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                key.id,
                key.profile_id,
                key.alias,
                key.host,
                i64::from(key.port),
                key.algorithm,
                key.fingerprint_sha256,
                key.public_key_base64,
                enum_text(&key.scope)?,
                key.label,
                key.first_seen.to_rfc3339(),
                key.last_seen.to_rfc3339(),
                json_text(key)?,
            ],
        )?;
    }

    for grant in &store.grants {
        connection.execute(
            "insert into mcp_grants (
                client_id, name, scopes_json, allowed_sessions_json, expires_at, revoked_at, raw_json
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                grant.client_id,
                grant.name,
                json_text(&grant.scopes)?,
                json_text(&grant.allowed_sessions)?,
                grant.expires_at.map(|value| value.to_rfc3339()),
                grant.revoked_at.map(|value| value.to_rfc3339()),
                json_text(grant)?,
            ],
        )?;
    }

    for record in &store.audit {
        connection.execute(
            "insert into mcp_audit (
                id, ts, actor, action, session_id, decision, details_json, raw_json
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                record.id,
                record.ts.to_rfc3339(),
                record.actor,
                record.action,
                record.session_id,
                record.decision,
                json_text(&record.details)?,
                json_text(record)?,
            ],
        )?;
    }

    for mark in &store.timeline {
        connection.execute(
            "insert into timeline_marks (
                id, session_id, ts, label, details, raw_json
            ) values (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                mark.id,
                mark.session_id,
                mark.ts.to_rfc3339(),
                mark.label,
                mark.details,
                json_text(mark)?,
            ],
        )?;
    }

    for snapshot in &store.sysmon {
        connection.execute(
            "insert into sysmon_snapshots (
                session_id, ts, uptime_seconds, cpu_percent, memory_percent, rx_kbps, tx_kbps, raw_json
            ) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                snapshot.session_id,
                snapshot.ts.to_rfc3339(),
                snapshot.uptime_seconds as i64,
                snapshot.cpu_percent,
                snapshot.memory_percent,
                snapshot.rx_kbps,
                snapshot.tx_kbps,
                json_text(snapshot)?,
            ],
        )?;
    }

    connection.execute(
        "insert into metadata (key, value) values ('schemaVersion', ?1)
            on conflict(key) do update set value = excluded.value",
        params![SQLITE_SCHEMA_VERSION],
    )?;
    Ok(())
}

fn json_text<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string(value)?)
}

fn enum_text<T: Serialize>(value: &T) -> Result<String> {
    let value = serde_json::to_value(value)?;
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow!("expected enum to serialize as a string"))
}

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut server = PortMateMcp::new();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request = match serde_json::from_str::<JsonRpcRequest>(&line) {
            Ok(request) => request,
            Err(error_message) => {
                let response = error(Value::Null, -32700, format!("parse error: {error_message}"));
                writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
                stdout.flush()?;
                continue;
            }
        };

        if let Some(response) = match server.handle(request) {
            Ok(response) => response,
            Err(error_message) => Some(error(Value::Null, -32603, error_message.to_string())),
        } {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }
    }

    Ok(())
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

fn parse_session_uri(uri: &str) -> Option<(&str, &str)> {
    let path = uri.strip_prefix("portmate://sessions/")?;
    let mut parts = path.split('/');
    let id = parts.next()?;
    let suffix = parts.next()?.split('?').next().unwrap_or_default();
    Some((id, suffix))
}

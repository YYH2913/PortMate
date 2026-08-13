use super::{desktop_ipc::ipc_value_to_text, PortMateMcp};
use anyhow::{anyhow, Result};
use portmate_core::{
    redact_secrets, redact_session_event, redact_session_events, redact_session_summary,
    redact_transfer_task, McpScope, SessionEvent, SessionSummary, TransferTask,
};
use serde_json::{json, Value};

const DEFAULT_LOG_QUERY_LIMIT: u64 = 100;
const MAX_LOG_QUERY_LIMIT: u64 = 1000;
const DEFAULT_TRANSFER_QUERY_LIMIT: u64 = 100;
const MAX_TRANSFER_QUERY_LIMIT: u64 = 1000;

impl PortMateMcp {
    pub(super) fn tool_call(&mut self, params: &Value) -> Result<Value> {
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
            "list_transfers" => {
                let session_id = arguments.get("sessionId").and_then(Value::as_str);
                self.guard_read_scope(McpScope::ReadTransfers, session_id)?;
                if let Some(session_id) = session_id {
                    self.require_known_session(session_id)?;
                }
                let limit =
                    bounded_transfer_query_limit(arguments.get("limit").and_then(Value::as_u64));
                let transfers = if let Some(value) =
                    self.call_ipc_value("list_transfers", arguments.clone())?
                {
                    serde_json::from_value::<Vec<TransferTask>>(value).map_err(|error| {
                        anyhow!("invalid desktop transfer-list response: {error}")
                    })?
                } else {
                    self.store.transfers.clone()
                };
                let visible = recent_visible_transfers(self, transfers, session_id, limit);
                serde_json::to_string_pretty(&visible)?
            }
            "get_transfer" => {
                let transfer_id = required_string(&arguments, "transferId")?;
                let transfer = self
                    .store
                    .transfer_by_id(transfer_id)
                    .filter(|transfer| self.has_session(&transfer.session_id))
                    .ok_or_else(|| anyhow!("unknown or unauthorized transfer"))?;
                self.guard_read_scope(McpScope::ReadTransfers, Some(&transfer.session_id))?;
                let transfer = if let Some(value) =
                    self.call_ipc_value("get_transfer", arguments.clone())?
                {
                    let current = serde_json::from_value::<TransferTask>(value)
                        .map_err(|error| anyhow!("invalid desktop transfer response: {error}"))?;
                    if current.id != transfer_id || current.session_id != transfer.session_id {
                        return Err(anyhow!(
                            "desktop transfer response did not match the authorized task"
                        ));
                    }
                    self.guard_read_scope(McpScope::ReadTransfers, Some(&current.session_id))?;
                    self.require_known_session(&current.session_id)?;
                    current
                } else {
                    transfer
                };
                serde_json::to_string_pretty(&redact_transfer_task(transfer))?
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
            "start_transfer" | "start_content_transfer" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    let transfer = serde_json::from_value::<TransferTask>(value)
                        .map_err(|error| anyhow!("invalid desktop transfer response: {error}"))?;
                    serde_json::to_string_pretty(&redact_transfer_task(transfer))?
                } else {
                    is_error = true;
                    format!(
                        "{name} was NOT executed: desktop IPC is not available, so no transfer was started."
                    )
                }
            }
            "cancel_transfer" | "retry_transfer" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    let transfer = serde_json::from_value::<TransferTask>(value)
                        .map_err(|error| anyhow!("invalid desktop transfer response: {error}"))?;
                    serde_json::to_string_pretty(&redact_transfer_task(transfer))?
                } else {
                    is_error = true;
                    format!(
                        "{name} was NOT executed: desktop IPC is not available, so no transfer state changed."
                    )
                }
            }
            "create_tunnel" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    redact_secrets(&ipc_value_to_text(value)?)
                } else {
                    is_error = true;
                    "create_tunnel was NOT executed: desktop IPC is not available, so no tunnel was created."
                        .to_string()
                }
            }
            "list_tunnels" => {
                let session_id = required_string(&arguments, "sessionId")?;
                self.guard_read_scope(McpScope::ReadTunnels, Some(session_id))?;
                self.require_known_session(session_id)?;
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    redact_secrets(&ipc_value_to_text(value)?)
                } else {
                    serde_json::to_string_pretty(&Vec::<Value>::new())?
                }
            }
            "stop_tunnel" => {
                if let Some(value) = self.call_ipc_value(name, arguments.clone())? {
                    redact_secrets(&ipc_value_to_text(value)?)
                } else {
                    is_error = true;
                    "stop_tunnel was NOT executed: desktop IPC is not available, so no forward or proxy was stopped."
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
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string argument `{key}`"))
}

pub(super) fn bounded_log_query_limit(limit: Option<u64>) -> usize {
    limit
        .unwrap_or(DEFAULT_LOG_QUERY_LIMIT)
        .clamp(1, MAX_LOG_QUERY_LIMIT) as usize
}

pub(super) fn bounded_transfer_query_limit(limit: Option<u64>) -> usize {
    limit
        .unwrap_or(DEFAULT_TRANSFER_QUERY_LIMIT)
        .clamp(1, MAX_TRANSFER_QUERY_LIMIT) as usize
}

fn recent_visible_transfers(
    server: &PortMateMcp,
    transfers: Vec<TransferTask>,
    session_id: Option<&str>,
    limit: usize,
) -> Vec<TransferTask> {
    let mut visible = transfers
        .into_iter()
        .filter(|transfer| {
            session_id.is_none_or(|session_id| transfer.session_id == session_id)
                && server.has_session(&transfer.session_id)
                && server.read_session_allowed(McpScope::ReadTransfers, &transfer.session_id)
        })
        .map(redact_transfer_task)
        .collect::<Vec<_>>();
    if visible.len() > limit {
        visible.drain(..visible.len() - limit);
    }
    visible
}

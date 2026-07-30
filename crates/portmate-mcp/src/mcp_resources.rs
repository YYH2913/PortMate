use super::{desktop_ipc::ipc_value_to_text, PortMateMcp};
use anyhow::{anyhow, Result};
use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use portmate_core::{
    redact_secrets, redact_sysmon_snapshot, redact_timeline_marks, redact_transfer_task, McpScope,
};
use serde_json::{json, Value};

impl PortMateMcp {
    pub(super) fn resources_list_result(&self) -> Value {
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

    pub(super) fn prompt_get(&self, params: &Value) -> Result<Value> {
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

    pub(super) fn resource_read(&self, params: &Value) -> Result<Value> {
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
                        .map(portmate_core::redact_session_summary),
                )?,
                "screen" => redact_secrets(&self.store.screen(&session_id).unwrap_or_default()),
                "log" => {
                    portmate_core::redact_session_events(self.store.tail_log(&session_id, 200))
                        .into_iter()
                        .map(|event| serde_json::to_string(&event).unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\n")
                }
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

pub(super) fn parse_session_uri(uri: &str) -> Option<(String, &str)> {
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

pub(super) fn parse_transfer_uri(uri: &str) -> Option<String> {
    let id = uri.strip_prefix("portmate://transfers/")?;
    if id.contains(['/', '?', '#']) {
        return None;
    }
    decode_mcp_uri_segment(id)
}

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolDefinition {
    pub name: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResourceTemplate {
    pub uri_template: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub mime_type: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptTemplate {
    pub name: String,
    pub title: String,
    pub description: String,
    pub arguments: Vec<McpPromptArgument>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpPromptArgument {
    pub name: String,
    pub description: String,
    pub required: bool,
}

pub fn tool_definitions() -> Vec<McpToolDefinition> {
    vec![
        tool(
            "list_sessions",
            "List Sessions",
            "List visible PortMate sessions.",
            json!({"type":"object","properties":{}}),
            true,
        ),
        tool(
            "read_screen",
            "Read Screen",
            "Read the current terminal screen snapshot.",
            session_schema(),
            true,
        ),
        tool(
            "tail_log",
            "Tail Log",
            "Read recent structured log lines from a session.",
            json!({
                "type":"object",
                "required":["sessionId"],
                "properties":{
                    "sessionId":{"type":"string"},
                    "limit":{"type":"integer","minimum":1,"maximum":1000,"default":100}
                }
            }),
            true,
        ),
        tool(
            "search_logs",
            "Search Logs",
            "Search text logs across sessions.",
            json!({
                "type":"object",
                "required":["query"],
                "properties":{
                    "query":{"type":"string"},
                    "sessionId":{"type":"string"},
                    "limit":{"type":"integer","minimum":1,"maximum":1000,"default":100}
                }
            }),
            true,
        ),
        tool(
            "send_text",
            "Send Text",
            "Send text to a trusted session.",
            json!({
                "type":"object",
                "required":["sessionId","text"],
                "properties":{"sessionId":{"type":"string"},"text":{"type":"string"}}
            }),
            false,
        ),
        tool(
            "send_key",
            "Send Key",
            "Send a terminal key sequence to a trusted session.",
            json!({
                "type":"object",
                "required":["sessionId","key"],
                "properties":{"sessionId":{"type":"string"},"key":{"type":"string"}}
            }),
            false,
        ),
        tool(
            "run_command",
            "Run Command",
            "Send a command followed by newline.",
            json!({
                "type":"object",
                "required":["sessionId","command"],
                "properties":{"sessionId":{"type":"string"},"command":{"type":"string"}}
            }),
            false,
        ),
        tool(
            "open_session",
            "Open Session",
            "Open a saved session profile.",
            session_schema(),
            false,
        ),
        tool(
            "close_session",
            "Close Session",
            "Close a running session.",
            session_schema(),
            false,
        ),
        tool(
            "start_transfer",
            "Start Transfer",
            "Start SFTP/SCP or modem transfer.",
            json!({
                "type":"object",
                "required":["sessionId","protocol","source","destination"],
                "properties":{
                    "sessionId":{"type":"string"},
                    "protocol":{"type":"string","enum":["sftp","scp","xmodem","ymodem","zmodem"]},
                    "source":{"type":"string"},
                    "destination":{"type":"string"}
                }
            }),
            false,
        ),
        tool(
            "create_tunnel",
            "Create Tunnel",
            "Create an SSH tunnel on a trusted session.",
            json!({
                "type":"object",
                "required":["sessionId","mode","bindHost","bindPort","targetHost","targetPort"],
                "properties":{
                    "sessionId":{"type":"string"},
                    "mode":{"type":"string","enum":["local","remote","dynamic"]},
                    "bindHost":{"type":"string"},
                    "bindPort":{"type":"integer"},
                    "targetHost":{"type":"string"},
                    "targetPort":{"type":"integer"}
                }
            }),
            false,
        ),
        tool(
            "list_tmux_state",
            "List Tmux State",
            "Read tmux sessions and panes for a connected SSH-backed session.",
            session_schema(),
            true,
        ),
        tool(
            "attach_tmux",
            "Attach Tmux",
            "Switch or attach a tmux session in a trusted SSH-backed session.",
            json!({
                "type":"object",
                "required":["sessionId","target"],
                "properties":{
                    "sessionId":{"type":"string"},
                    "target":{"type":"string"}
                }
            }),
            false,
        ),
        tool(
            "export_session_bundle",
            "Export Session Bundle",
            "Export logs and metadata for incident handoff.",
            session_schema(),
            true,
        ),
    ]
}

pub fn resource_templates() -> Vec<McpResourceTemplate> {
    vec![
        resource(
            "portmate://sessions",
            "sessions",
            "Sessions",
            "All visible session summaries",
            "application/json",
        ),
        resource(
            "portmate://sessions/{id}/state",
            "session_state",
            "Session State",
            "Current runtime state for one session",
            "application/json",
        ),
        resource(
            "portmate://sessions/{id}/screen",
            "session_screen",
            "Session Screen",
            "Current terminal screen snapshot",
            "text/plain",
        ),
        resource(
            "portmate://sessions/{id}/log?since={since}&limit={limit}",
            "session_log",
            "Session Log",
            "Recent session log lines",
            "application/jsonl",
        ),
        resource(
            "portmate://sessions/{id}/timeline",
            "session_timeline",
            "Session Timeline",
            "Session marks and correlated events",
            "application/json",
        ),
        resource(
            "portmate://sessions/{id}/sysmon",
            "session_sysmon",
            "Session Sysmon",
            "Latest system monitor snapshot",
            "application/json",
        ),
        resource(
            "portmate://sessions/{id}/tmux",
            "session_tmux",
            "Session Tmux",
            "Tmux sessions and panes for the current SSH-backed session",
            "application/json",
        ),
        resource(
            "portmate://transfers/{id}",
            "transfer",
            "Transfer Task",
            "File transfer task status",
            "application/json",
        ),
    ]
}

pub fn prompt_templates() -> Vec<McpPromptTemplate> {
    vec![
        prompt(
            "diagnose_session",
            "Diagnose Session",
            "Use terminal state and logs to diagnose one session.",
        ),
        prompt(
            "compare_serial_and_ssh",
            "Compare Serial And SSH",
            "Correlate serial console and SSH output for the same device.",
        ),
        prompt(
            "prepare_repro_report",
            "Prepare Repro Report",
            "Create a reproducible issue report from logs and timeline marks.",
        ),
    ]
}

fn tool(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
) -> McpToolDefinition {
    McpToolDefinition {
        name: name.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        input_schema,
        read_only,
    }
}

fn resource(
    uri_template: &str,
    name: &str,
    title: &str,
    description: &str,
    mime_type: &str,
) -> McpResourceTemplate {
    McpResourceTemplate {
        uri_template: uri_template.to_string(),
        name: name.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        mime_type: mime_type.to_string(),
    }
}

fn prompt(name: &str, title: &str, description: &str) -> McpPromptTemplate {
    McpPromptTemplate {
        name: name.to_string(),
        title: title.to_string(),
        description: description.to_string(),
        arguments: vec![McpPromptArgument {
            name: "sessionId".to_string(),
            description: "PortMate session identifier".to_string(),
            required: true,
        }],
    }
}

fn session_schema() -> Value {
    json!({
        "type":"object",
        "required":["sessionId"],
        "properties":{"sessionId":{"type":"string"}}
    })
}

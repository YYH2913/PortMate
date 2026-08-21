use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Maximum serialized MCP request accepted by the stdio, HTTP, and desktop IPC bridge.
pub const MAX_MCP_BRIDGE_REQUEST_BYTES: usize = 6 * 1024 * 1024;
/// Maximum decoded payload accepted by the MCP inline-content transfer tool.
/// Its standard Base64 form leaves room for the surrounding JSON-RPC and IPC envelopes.
pub const MAX_MCP_CONTENT_TRANSFER_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH: usize =
    MAX_MCP_CONTENT_TRANSFER_BYTES.div_ceil(3) * 4;
/// Maximum request and response payload for one MCP host-tunnel exchange.
pub const MAX_MCP_TUNNEL_EXCHANGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH: usize =
    MAX_MCP_TUNNEL_EXCHANGE_BYTES.div_ceil(3) * 4;
pub const MAX_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS: u64 = 30_000;
const _: () = assert!(MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH < MAX_MCP_BRIDGE_REQUEST_BYTES);
const _: () = assert!(MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH < MAX_MCP_BRIDGE_REQUEST_BYTES);
/// Maximum file size accepted by the resumable MCP content-upload workflow.
pub const MAX_MCP_CONTENT_UPLOAD_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_MCP_CONTENT_UPLOAD_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_MCP_CONTENT_UPLOADS: usize = 16;
pub const MCP_CONTENT_UPLOAD_EXPIRY_SECONDS: u64 = 24 * 60 * 60;
pub const MCP_CONTENT_UPLOAD_STAGING_DIRECTORY: &str = ".mcp-transfer-staging";
pub const MCP_CONTENT_UPLOADS_DIRECTORY: &str = "uploads";
pub const MCP_CONTENT_UPLOAD_METADATA_VERSION: u32 = 1;
pub const MCP_CONTENT_UPLOAD_METADATA_FILE: &str = "upload.json";
pub const MCP_CONTENT_UPLOAD_PAYLOAD_FILE: &str = "payload.bin";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpContentUploadMetadata {
    pub version: u32,
    pub upload_id: String,
    pub client_id: String,
    pub session_id: String,
    pub protocol: crate::TransferProtocol,
    pub file_name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub destination: String,
    pub created_at_unix_seconds: u64,
}

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
            "mcp_bridge_status",
            "MCP Bridge Status",
            "Read the PortMate MCP Bridge transport, desktop IPC, store, and managed HTTP sidecar status without exposing tokens or secret values.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            true,
        ),
        tool(
            "reload_mcp",
            "Reload MCP Bridge",
            "Reload the MCP Bridge store and desktop IPC endpoint sources, then return the current Bridge status. This does not restart the process.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            true,
        ),
        tool(
            "restart_mcp",
            "Restart MCP Bridge",
            "Restart the PortMate-managed MCP HTTP sidecar and return its runtime status. A managed HTTP sidecar cannot restart itself from an in-flight request; use a stdio Bridge or the PortMate desktop UI for this operation.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            false,
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
            "send_bytes",
            "Send Raw Bytes",
            "Pass bytes directly to a connected session without adding a newline or converting the payload to terminal text. Encode data as standard Base64 or hexadecimal; PortMate preserves the decoded bytes, applies only transport-required Telnet escaping, and records a redacted byte summary instead of the payload.",
            json!({
                "type":"object",
                "required":["sessionId","encoding","data"],
                "additionalProperties":false,
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "encoding":{"type":"string","enum":["base64","hex"]},
                    "data":{"type":"string","minLength":1,"maxLength":MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH}
                }
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
            "serial_send_break",
            "Send Serial Break",
            "Pulse the hardware Break condition on a connected serial session.",
            session_schema(),
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
            "list_custom_scripts",
            "List Custom Scripts",
            "List MCP-enabled custom scripts available to one session. Script bodies are never returned.",
            session_schema(),
            true,
        ),
        tool(
            "run_custom_script",
            "Run Custom Script",
            "Run a saved MCP-enabled custom script in an authorized session. The request selects an existing script and cannot provide or replace its body.",
            json!({
                "type":"object",
                "required":["sessionId","scriptId"],
                "additionalProperties":false,
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "scriptId":{"type":"string","format":"uuid"}
                }
            }),
            false,
        ),
        tool(
            "list_transfers",
            "List Transfers",
            "List recent file-transfer tasks visible to this client. Paths are redacted.",
            json!({
                "type":"object",
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "limit":{"type":"integer","minimum":1,"maximum":1000,"default":100}
                }
            }),
            true,
        ),
        tool(
            "get_transfer",
            "Get Transfer",
            "Read one file-transfer task by ID. Paths are redacted.",
            transfer_id_schema(),
            true,
        ),
        tool(
            "start_transfer",
            "Start Transfer",
            "Start an SFTP, SCP, TFTP, XModem, YModem, or ZModem transfer from exactly one source: a path string, a virtual MCP file object, legacy inline fields, or a completed resumable uploadId. Use `source: {kind: \"mcp\", fileName, contentBase64}` to pass client-held bytes without resolving a client path or selecting a local folder on the PortMate desktop host. Path transfers require sessionId, protocol, source, and destination. At least one endpoint must use `remote:`, `ssh:`, or a constrained `load:` receiver. Virtual and legacy inline transfers are limited to 4 MiB; larger client-held files use begin_content_upload and append_content_upload. For TFTP, use structured `destination: {kind: \"tftpboot\", deviceIp, ...}`; deviceIp is required, timeoutSeconds defaults to 60, must be at least 5, and has no application-defined upper limit. The legacy `load:tftpboot?deviceIp=...` string remains supported. Resumable uploads bind this destination in begin_content_upload and later start with uploadId only. Modem receivers use `load:loadx`, `load:loady`, or `load:loadz`. Poll get_transfer for completion.",
            json!({
                "type":"object",
                "additionalProperties":false,
                "oneOf":[
                    {
                        "required":["sessionId","protocol","source","destination"],
                        "not":{"anyOf":[{"required":["fileName"]},{"required":["contentBase64"]},{"required":["uploadId"]}]}
                    },
                    {
                        "required":["sessionId","protocol","fileName","contentBase64","destination"],
                        "not":{"anyOf":[{"required":["source"]},{"required":["uploadId"]}]}
                    },
                    {
                        "required":["uploadId"],
                        "not":{"anyOf":[{"required":["sessionId"]},{"required":["protocol"]},{"required":["source"]},{"required":["fileName"]},{"required":["contentBase64"]},{"required":["destination"]}]}
                    }
                ],
                "allOf":[
                    {
                        "if":{"properties":{"destination":{"type":"object"}},"required":["destination"]},
                        "then":{"properties":{"protocol":{"const":"tftp"}},"required":["protocol"]}
                    }
                ],
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "protocol":{"type":"string","enum":["sftp","scp","tftp","xmodem","ymodem","zmodem"]},
                    "source":{"oneOf":[{"type":"string","minLength":1,"maxLength":32768},{"type":"object","required":["kind","fileName","contentBase64"],"additionalProperties":false,"properties":{"kind":{"type":"string","const":"mcp"},"fileName":{"type":"string","minLength":1,"maxLength":255},"contentBase64":{"type":"string","minLength":1,"maxLength":MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH}}}]},
                    "fileName":{"type":"string","minLength":1,"maxLength":255},
                    "contentBase64":{"type":"string","minLength":1,"maxLength":MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH},
                    "destination":transfer_destination_schema(),
                    "uploadId":{"type":"string","format":"uuid"}
                }
            }),
            false,
        ),
        tool(
            "begin_content_upload",
            "Begin Content Upload",
            "Create a resumable private staging upload for content held by the MCP client. Files may be up to 512 MiB and active uploads share a 1 GiB declared-size quota. TFTP callers should provide structured destination `{kind: \"tftpboot\", deviceIp, ...}` so all route parameters are validated before content is uploaded. Append the file in ordered Base64 chunks, then call start_transfer with only the returned uploadId.",
            json!({
                "type":"object",
                "required":["sessionId","protocol","fileName","sizeBytes","sha256","destination"],
                "additionalProperties":false,
                "allOf":[
                    {
                        "if":{"properties":{"destination":{"type":"object"}},"required":["destination"]},
                        "then":{"properties":{"protocol":{"const":"tftp"}}}
                    }
                ],
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "protocol":{"type":"string","enum":["sftp","scp","tftp","xmodem","ymodem","zmodem"]},
                    "fileName":{"type":"string","minLength":1,"maxLength":255},
                    "sizeBytes":{"type":"integer","minimum":1,"maximum":MAX_MCP_CONTENT_UPLOAD_BYTES},
                    "sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},
                    "destination":transfer_destination_schema()
                }
            }),
            false,
        ),
        tool(
            "append_content_upload",
            "Append Content Upload",
            "Append one ordered standard-Base64 chunk to a resumable content upload. offset must equal the nextOffset returned by the previous call.",
            json!({
                "type":"object",
                "required":["uploadId","offset","contentBase64"],
                "additionalProperties":false,
                "properties":{
                    "uploadId":{"type":"string","format":"uuid"},
                    "offset":{"type":"integer","minimum":0,"maximum":MAX_MCP_CONTENT_UPLOAD_BYTES},
                    "contentBase64":{"type":"string","minLength":1,"maxLength":MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH}
                }
            }),
            false,
        ),
        tool(
            "cancel_content_upload",
            "Cancel Content Upload",
            "Delete an incomplete resumable content upload owned by this MCP client.",
            json!({
                "type":"object",
                "required":["uploadId"],
                "additionalProperties":false,
                "properties":{"uploadId":{"type":"string","format":"uuid"}}
            }),
            false,
        ),
        tool(
            "cancel_transfer",
            "Cancel Transfer",
            "Cancel a queued or running file-transfer task.",
            transfer_id_schema(),
            false,
        ),
        tool(
            "retry_transfer",
            "Retry Transfer",
            "Retry a previous file-transfer task with its original protocol and paths.",
            transfer_id_schema(),
            false,
        ),
        tool(
            "create_tunnel",
            "Create Forward Or Proxy",
            "Create a fixed TCP forward or dynamic SOCKS5 proxy. Use egress `ssh` with sessionId for a connected SSH/Tmux route, or egress `portmate-host` without sessionId to connect through a target reachable directly from the machine running PortMate. The latter is independent of terminal sessions and requires routeRules for dynamic SOCKS5.",
            json!({
                "type":"object",
                "required":["mode","bindHost","bindPort"],
                "additionalProperties":false,
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "egress":{"type":"string","enum":["ssh","portmate-host"]},
                    "mode":{"type":"string","enum":["local","remote","dynamic"]},
                    "bindHost":{"type":"string","maxLength":255},
                    "bindPort":{"type":"integer","minimum":0,"maximum":65535},
                    "allowRemoteBind":{"type":"boolean","default":false},
                    "targetHost":{"type":"string","maxLength":255,"default":""},
                    "targetPort":{"type":"integer","minimum":0,"maximum":65535,"default":0},
                    "routeRules":{
                        "type":"array",
                        "maxItems":64,
                        "default":[],
                        "items":{
                            "type":"object",
                            "required":["host"],
                            "additionalProperties":false,
                            "properties":{
                                "host":{"type":"string","minLength":1,"maxLength":255},
                                "port":{"type":["integer","null"],"minimum":1,"maximum":65535,"default":null}
                            }
                        }
                    },
                    "label":{"type":"string","minLength":1,"maxLength":128}
                },
                "allOf":[
                    {
                        "if":{"properties":{"mode":{"const":"remote"}},"required":["mode"]},
                        "else":{"properties":{"bindHost":{"type":"string","minLength":1,"maxLength":255}}}
                    },
                    {
                        "if":{"properties":{"mode":{"const":"dynamic"}},"required":["mode"]},
                        "then":{
                            "properties":{
                                "targetHost":{"type":"string","maxLength":0},
                                "targetPort":{"const":0},
                                "routeRules":{"type":"array","maxItems":64}
                            }
                        },
                        "else":{
                            "required":["targetHost","targetPort"],
                            "properties":{
                                "targetHost":{"type":"string","minLength":1,"maxLength":255},
                                "targetPort":{"type":"integer","minimum":1,"maximum":65535},
                                "routeRules":{"type":"array","maxItems":0}
                            }
                        }
                    },
                    {
                        "oneOf":[
                            {
                                "required":["sessionId"],
                                "properties":{
                                    "egress":{"enum":["ssh"]},
                                    "allowRemoteBind":{"const":false}
                                }
                            },
                            {
                                "required":["egress"],
                                "not":{"required":["sessionId"]},
                                "properties":{
                                    "egress":{"const":"portmate-host"},
                                    "mode":{"enum":["local","dynamic"]}
                                }
                            }
                        ]
                    },
                    {
                        "if":{
                            "properties":{"egress":{"const":"portmate-host"}},
                            "required":["egress"]
                        },
                        "then":{
                            "if":{"properties":{"mode":{"const":"dynamic"}},"required":["mode"]},
                            "then":{
                                "required":["routeRules"],
                                "properties":{"routeRules":{"minItems":1,"maxItems":64}}
                            }
                        }
                    }
                ]
            }),
            false,
        ),
        tool(
            "list_tunnels",
            "List Forwards And Proxies",
            "List active forwards and SOCKS5 proxies. Provide sessionId to list SSH/Tmux routes, or omit it (or set egress to `portmate-host`) to list PortMate-host routes owned by this MCP client. No terminal session is required for host routes.",
            json!({
                "type":"object",
                "additionalProperties":false,
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "egress":{"type":"string","enum":["ssh","portmate-host"]}
                }
            }),
            true,
        ),
        tool(
            "stop_tunnel",
            "Stop Forward Or Proxy",
            "Stop an active SSH forward or dynamic SOCKS5 proxy by tunnel ID.",
            tunnel_id_schema(),
            false,
        ),
        tool(
            "tunnel_request",
            "Request Through Tunnel",
            "Send one bounded raw TCP request through an existing PortMate-host tunnel from the desktop host and return the response as standard Base64. This is the MCP data plane for agents that cannot reach a Windows or other desktop listener directly. Fixed local tunnels use their configured target; dynamic SOCKS5 tunnels require targetHost and targetPort and enforce the tunnel routeRules. The tunnel must be owned by this MCP client.",
            json!({
                "type":"object",
                "required":["tunnelId","encoding","data"],
                "additionalProperties":false,
                "properties":{
                    "tunnelId":{"type":"string","minLength":1,"maxLength":128},
                    "encoding":{"type":"string","enum":["base64","hex"]},
                    "data":{"type":"string","minLength":1,"maxLength":MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH},
                    "targetHost":{"type":"string","minLength":1,"maxLength":255},
                    "targetPort":{"type":"integer","minimum":1,"maximum":65535},
                    "timeoutMs":{"type":"integer","minimum":100,"maximum":MAX_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS,"default":10000},
                    "maxResponseBytes":{"type":"integer","minimum":1,"maximum":MAX_MCP_TUNNEL_EXCHANGE_BYTES,"default":MAX_MCP_TUNNEL_EXCHANGE_BYTES},
                    "closeWrite":{"type":"boolean","default":true}
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
        "properties":{"sessionId":{"type":"string","minLength":1,"maxLength":128}}
    })
}

fn transfer_id_schema() -> Value {
    json!({
        "type":"object",
        "required":["transferId"],
        "properties":{"transferId":{"type":"string","minLength":1,"maxLength":128}}
    })
}

fn transfer_destination_schema() -> Value {
    json!({
        "oneOf":[
            {
                "type":"string",
                "minLength":1,
                "maxLength":32768,
                "description":"A desktop, remote:, ssh:, load: Modem, or legacy load:tftpboot?deviceIp=... endpoint. TFTP requires deviceIp in the query string."
            },
            {
                "type":"object",
                "required":["kind","deviceIp"],
                "additionalProperties":false,
                "properties":{
                    "kind":{"type":"string","const":"tftpboot"},
                    "deviceIp":{"type":"string","format":"ipv4","minLength":7,"maxLength":15},
                    "address":{"type":"string","pattern":"^(?:0[xX])?[0-9A-Fa-f]{1,16}$"},
                    "fileName":{"type":"string","minLength":1,"maxLength":255},
                    "serverIp":{"type":"string","format":"ipv4","minLength":7,"maxLength":15},
                    "bindHost":{"type":"string","format":"ipv4","minLength":7,"maxLength":15},
                    "bindPort":{"type":"integer","minimum":0,"maximum":65535,"default":69},
                    "timeoutSeconds":{"type":"integer","minimum":5,"default":60}
                }
            }
        ]
    })
}

fn tunnel_id_schema() -> Value {
    json!({
        "type":"object",
        "required":["tunnelId"],
        "properties":{"tunnelId":{"type":"string","minLength":1,"maxLength":128}}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(name: &str) -> McpToolDefinition {
        tool_definitions()
            .into_iter()
            .find(|definition| definition.name == name)
            .unwrap_or_else(|| panic!("missing MCP tool definition: {name}"))
    }

    #[test]
    fn bridge_management_tools_are_advertised_with_safe_schemas() {
        assert_eq!(tool_definitions().len(), 29);
        for name in ["mcp_bridge_status", "reload_mcp", "restart_mcp"] {
            let definition = definition(name);
            assert_eq!(definition.input_schema["type"], "object", "{name}");
            assert_eq!(
                definition.input_schema["additionalProperties"], false,
                "{name}"
            );
        }
        assert!(definition("mcp_bridge_status").read_only);
        assert!(definition("reload_mcp").read_only);
        assert!(!definition("restart_mcp").read_only);
    }

    #[test]
    fn serial_break_is_a_bounded_write_tool() {
        let serial_break = definition("serial_send_break");
        assert!(!serial_break.read_only);
        assert_eq!(serial_break.input_schema["type"], "object");
        assert_eq!(serial_break.input_schema["required"], json!(["sessionId"]));
        assert_eq!(
            serial_break.input_schema["properties"]["sessionId"]["maxLength"],
            128
        );
        assert!(serial_break
            .description
            .contains("connected serial session"));
    }

    #[test]
    fn raw_bytes_tool_exposes_binary_encodings_without_payload_echo() {
        let bytes = definition("send_bytes");
        assert!(!bytes.read_only);
        assert_eq!(
            bytes.input_schema["required"],
            json!(["sessionId", "encoding", "data"])
        );
        assert_eq!(
            bytes.input_schema["properties"]["encoding"]["enum"],
            json!(["base64", "hex"])
        );
        assert!(bytes.description.contains("without adding a newline"));
        assert!(bytes.description.contains("redacted byte summary"));
    }

    #[test]
    fn transfer_and_route_lifecycle_tools_expose_bounded_schemas() {
        for name in [
            "list_transfers",
            "get_transfer",
            "cancel_transfer",
            "retry_transfer",
            "list_tunnels",
            "stop_tunnel",
        ] {
            let tool = definition(name);
            assert_eq!(tool.input_schema["type"], "object", "{name}");
        }
        for name in ["list_transfers", "get_transfer", "list_tunnels"] {
            assert!(definition(name).read_only, "{name}");
        }
        for name in [
            "start_transfer",
            "begin_content_upload",
            "append_content_upload",
            "cancel_content_upload",
            "cancel_transfer",
            "retry_transfer",
            "create_tunnel",
            "stop_tunnel",
        ] {
            assert!(!definition(name).read_only, "{name}");
        }

        let transfer = definition("start_transfer");
        assert_eq!(
            transfer.input_schema["properties"]["source"]["oneOf"][0]["maxLength"],
            32_768
        );
        assert_eq!(
            transfer.input_schema["properties"]["source"]["oneOf"][1]["properties"]["kind"]
                ["const"],
            "mcp"
        );
        assert_eq!(
            transfer.input_schema["properties"]["source"]["oneOf"][1]["properties"]
                ["contentBase64"]["maxLength"],
            MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
        );
        assert!(transfer.input_schema["properties"]["protocol"]["enum"]
            .as_array()
            .is_some_and(|protocols| protocols.contains(&json!("tftp"))));
        assert_eq!(
            transfer.input_schema["oneOf"].as_array().map(Vec::len),
            Some(3)
        );
        assert_eq!(
            transfer.input_schema["properties"]["contentBase64"]["maxLength"],
            MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
        );
        assert_eq!(
            transfer.input_schema["properties"]["uploadId"]["format"],
            "uuid"
        );
        let tftp_destination = &transfer.input_schema["properties"]["destination"]["oneOf"][1];
        assert_eq!(tftp_destination["properties"]["kind"]["const"], "tftpboot");
        assert_eq!(tftp_destination["required"], json!(["kind", "deviceIp"]));
        assert_eq!(tftp_destination["additionalProperties"], false);
        assert_eq!(
            definition("begin_content_upload").input_schema["properties"]["destination"]["oneOf"]
                [1],
            *tftp_destination
        );
        let tunnel_request = definition("tunnel_request");
        assert!(!tunnel_request.read_only);
        assert_eq!(
            tunnel_request.input_schema["properties"]["data"]["maxLength"],
            MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH
        );
        assert_eq!(
            tunnel_request.input_schema["properties"]["timeoutMs"]["maximum"],
            MAX_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS
        );
        assert_eq!(
            tunnel_request.input_schema["properties"]["maxResponseBytes"]["maximum"],
            MAX_MCP_TUNNEL_EXCHANGE_BYTES
        );
        assert!(transfer.description.contains("exactly one source"));
        assert!(transfer.description.contains("limited to 4 MiB"));
        assert!(transfer.description.contains("deviceIp is required"));
        assert!(definition("begin_content_upload")
            .description
            .contains("validated before content is uploaded"));
        assert!(
            definition("begin_content_upload").input_schema["properties"]["protocol"]["enum"]
                .as_array()
                .is_some_and(|protocols| protocols.contains(&json!("tftp")))
        );
        assert_eq!(
            definition("begin_content_upload").input_schema["properties"]["sizeBytes"]["maximum"],
            MAX_MCP_CONTENT_UPLOAD_BYTES
        );
        assert_eq!(
            definition("append_content_upload").input_schema["properties"]["contentBase64"]
                ["maxLength"],
            MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
        );
        let list = definition("list_transfers");
        assert_eq!(list.input_schema["properties"]["limit"]["maximum"], 1_000);
        assert_eq!(
            list.input_schema["properties"]["sessionId"]["maxLength"],
            128
        );
    }

    #[test]
    fn route_schema_distinguishes_fixed_forwards_from_dynamic_socks() {
        let schema = definition("create_tunnel").input_schema;
        assert_eq!(
            schema["properties"]["mode"]["enum"],
            json!(["local", "remote", "dynamic"])
        );
        assert_eq!(schema["properties"]["bindPort"]["minimum"], 0);
        assert_eq!(schema["properties"]["bindPort"]["maximum"], 65_535);
        assert_eq!(
            schema["properties"]["egress"]["enum"],
            json!(["ssh", "portmate-host"])
        );
        assert!(schema["required"].as_array().is_some_and(|required| {
            !required
                .iter()
                .any(|value| value.as_str() == Some("sessionId"))
        }));
        assert!(schema["properties"]["allowRemoteBind"]
            .get("const")
            .is_none());

        let clauses = schema["allOf"].as_array().expect("route schema clauses");
        let target_clause = &clauses[1];
        assert_eq!(
            target_clause["if"]["properties"]["mode"]["const"],
            "dynamic"
        );
        assert_eq!(
            target_clause["then"]["properties"]["targetPort"]["const"],
            0
        );
        assert_eq!(schema["properties"]["routeRules"]["maxItems"], 64);
        assert_eq!(
            schema["properties"]["routeRules"]["items"]["properties"]["port"]["type"],
            json!(["integer", "null"])
        );
        assert_eq!(
            target_clause["else"]["properties"]["routeRules"]["maxItems"],
            0
        );
        assert_eq!(
            target_clause["else"]["required"],
            json!(["targetHost", "targetPort"])
        );
        assert_eq!(
            target_clause["else"]["properties"]["targetPort"]["minimum"],
            1
        );
        assert_eq!(clauses.len(), 4);
        assert_eq!(
            clauses[2]["oneOf"][1]["properties"]["egress"]["const"],
            "portmate-host"
        );
    }

    #[test]
    fn custom_script_tools_select_saved_scripts_without_accepting_script_bodies() {
        let list = definition("list_custom_scripts");
        assert!(list.read_only);
        assert!(list.description.contains("never returned"));
        assert_eq!(list.input_schema["required"], json!(["sessionId"]));
        assert!(list.input_schema["properties"].get("content").is_none());

        let run = definition("run_custom_script");
        assert!(!run.read_only);
        assert_eq!(run.input_schema["additionalProperties"], false);
        assert_eq!(
            run.input_schema["required"],
            json!(["sessionId", "scriptId"])
        );
        assert_eq!(run.input_schema["properties"]["scriptId"]["format"], "uuid");
        assert!(run.input_schema["properties"].get("content").is_none());
    }
}

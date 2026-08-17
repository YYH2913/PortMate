use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Maximum serialized MCP request accepted by the stdio, HTTP, and desktop IPC bridge.
pub const MAX_MCP_BRIDGE_REQUEST_BYTES: usize = 6 * 1024 * 1024;
/// Maximum decoded payload accepted by the MCP inline-content transfer tool.
/// Its standard Base64 form leaves room for the surrounding JSON-RPC and IPC envelopes.
pub const MAX_MCP_CONTENT_TRANSFER_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH: usize =
    MAX_MCP_CONTENT_TRANSFER_BYTES.div_ceil(3) * 4;
const _: () = assert!(MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH < MAX_MCP_BRIDGE_REQUEST_BYTES);
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
            "Start an SFTP, SCP, XModem, YModem, or ZModem transfer. At least one side must use a `remote:`, `ssh:`, or constrained `load:` endpoint; unprefixed paths are local to the PortMate desktop host. For a device-side Modem receiver, use destination `load:loadx`, `load:loady`, or `load:loadz`, optionally with validated `address` and `baud` query parameters. SFTP/SCP also support same-session remote-to-remote copy.",
            json!({
                "type":"object",
                "required":["sessionId","protocol","source","destination"],
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "protocol":{"type":"string","enum":["sftp","scp","xmodem","ymodem","zmodem"]},
                    "source":{"type":"string","minLength":1,"maxLength":32768},
                    "destination":{"type":"string","minLength":1,"maxLength":32768}
                }
            }),
            false,
        ),
        tool(
            "start_content_transfer",
            "Start Content Transfer",
            "Start a small transfer from inline Base64 content supplied by the MCP client. The decoded payload is limited to 4 MiB. For larger files, use begin_content_upload, append_content_upload, and start_content_upload_transfer. The destination must be a remote:/ssh: endpoint or a constrained load: Modem receiver.",
            json!({
                "type":"object",
                "required":["sessionId","protocol","fileName","contentBase64","destination"],
                "additionalProperties":false,
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "protocol":{"type":"string","enum":["sftp","scp","xmodem","ymodem","zmodem"]},
                    "fileName":{"type":"string","minLength":1,"maxLength":255},
                    "contentBase64":{"type":"string","minLength":1,"maxLength":MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH},
                    "destination":{"type":"string","minLength":1,"maxLength":32768}
                }
            }),
            false,
        ),
        tool(
            "begin_content_upload",
            "Begin Content Upload",
            "Create a resumable private staging upload for content held by the MCP client. Files may be up to 512 MiB and active uploads share a 1 GiB declared-size quota. Append the file in ordered Base64 chunks, then call start_content_upload_transfer.",
            json!({
                "type":"object",
                "required":["sessionId","protocol","fileName","sizeBytes","sha256","destination"],
                "additionalProperties":false,
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "protocol":{"type":"string","enum":["sftp","scp","xmodem","ymodem","zmodem"]},
                    "fileName":{"type":"string","minLength":1,"maxLength":255},
                    "sizeBytes":{"type":"integer","minimum":1,"maximum":MAX_MCP_CONTENT_UPLOAD_BYTES},
                    "sha256":{"type":"string","pattern":"^[0-9a-f]{64}$"},
                    "destination":{"type":"string","minLength":1,"maxLength":32768}
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
            "start_content_upload_transfer",
            "Start Uploaded Content Transfer",
            "Verify the completed upload's declared byte length and SHA-256 digest, then transfer it through SFTP, SCP, XModem, YModem, or ZModem. The desktop prompts for write approval only at this final step.",
            json!({
                "type":"object",
                "required":["uploadId"],
                "additionalProperties":false,
                "properties":{"uploadId":{"type":"string","format":"uuid"}}
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
            "Create SSH Forward Or Proxy",
            "Create a forward or dynamic SOCKS5 proxy through an authorized SSH/Tmux session. To expose routes reachable directly from the PortMate machine, use create_host_route instead.",
            json!({
                "type":"object",
                "required":["sessionId","mode","bindHost","bindPort"],
                "additionalProperties":false,
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "egress":{"type":"string","enum":["ssh"],"default":"ssh"},
                    "mode":{"type":"string","enum":["local","remote","dynamic"]},
                    "bindHost":{"type":"string","maxLength":255},
                    "bindPort":{"type":"integer","minimum":0,"maximum":65535},
                    "allowRemoteBind":{"type":"boolean","const":false,"default":false},
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
                    }
                ]
            }),
            false,
        ),
        tool(
            "list_tunnels",
            "List Forwards And Proxies",
            "List active SSH and PortMate-host forwards and SOCKS5 proxies for an authorized session.",
            session_schema(),
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
            "create_host_route",
            "Create PortMate Host Route",
            "Expose a TCP route reachable directly from the machine running PortMate. `local` creates a fixed TCP forward; `dynamic` creates a route-restricted SOCKS5 proxy. This tool is independent of terminal sessions. Non-loopback listeners require `allowRemoteBind: true`.",
            json!({
                "type":"object",
                "required":["mode","bindHost","bindPort"],
                "additionalProperties":false,
                "properties":{
                    "mode":{"type":"string","enum":["local","dynamic"]},
                    "bindHost":{"type":"string","minLength":1,"maxLength":255},
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
                        "if":{"properties":{"mode":{"const":"dynamic"}},"required":["mode"]},
                        "then":{
                            "required":["routeRules"],
                            "properties":{
                                "targetHost":{"type":"string","maxLength":0},
                                "targetPort":{"const":0},
                                "routeRules":{"type":"array","minItems":1,"maxItems":64}
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
                    }
                ]
            }),
            false,
        ),
        tool(
            "list_host_routes",
            "List PortMate Host Routes",
            "List active PortMate-host routes owned by this MCP client. No terminal session is required.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            true,
        ),
        tool(
            "stop_host_route",
            "Stop PortMate Host Route",
            "Stop a PortMate-host route owned by this MCP client.",
            tunnel_id_schema(),
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
    fn transfer_and_route_lifecycle_tools_expose_bounded_schemas() {
        for name in [
            "list_transfers",
            "get_transfer",
            "cancel_transfer",
            "retry_transfer",
            "list_tunnels",
            "stop_tunnel",
            "list_host_routes",
            "stop_host_route",
        ] {
            let tool = definition(name);
            assert_eq!(tool.input_schema["type"], "object", "{name}");
        }
        for name in [
            "list_transfers",
            "get_transfer",
            "list_tunnels",
            "list_host_routes",
        ] {
            assert!(definition(name).read_only, "{name}");
        }
        for name in [
            "start_transfer",
            "start_content_transfer",
            "begin_content_upload",
            "append_content_upload",
            "start_content_upload_transfer",
            "cancel_content_upload",
            "cancel_transfer",
            "retry_transfer",
            "create_tunnel",
            "stop_tunnel",
            "create_host_route",
            "stop_host_route",
        ] {
            assert!(!definition(name).read_only, "{name}");
        }

        let transfer = definition("start_transfer");
        assert_eq!(
            transfer.input_schema["properties"]["source"]["maxLength"],
            32_768
        );
        assert!(transfer.description.contains(
            "At least one side must use a `remote:`, `ssh:`, or constrained `load:` endpoint"
        ));
        let content_transfer = definition("start_content_transfer");
        assert_eq!(
            content_transfer.input_schema["properties"]["contentBase64"]["maxLength"],
            MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH
        );
        assert!(content_transfer.description.contains("limited to 4 MiB"));
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
        assert_eq!(schema["properties"]["egress"]["enum"], json!(["ssh"]));
        assert_eq!(schema["properties"]["allowRemoteBind"]["const"], false);

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
        assert_eq!(clauses.len(), 2);

        let host_schema = definition("create_host_route").input_schema;
        assert_eq!(
            host_schema["properties"]["mode"]["enum"],
            json!(["local", "dynamic"])
        );
        assert!(host_schema["properties"].get("sessionId").is_none());
        assert!(host_schema["properties"].get("egress").is_none());
        let host_dynamic_clause = &host_schema["allOf"][0];
        assert_eq!(
            host_dynamic_clause["then"]["properties"]["routeRules"]["minItems"],
            1
        );
        assert_eq!(
            host_dynamic_clause["then"]["required"],
            json!(["routeRules"])
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

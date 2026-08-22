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
/// Maximum payload for one MCP UDP datagram exchange (IPv4/IPv6 UDP payload bound).
pub const MAX_MCP_UDP_DATAGRAM_BYTES: usize = 65_507;
pub const MAX_MCP_UDP_DATAGRAM_BASE64_LENGTH: usize = MAX_MCP_UDP_DATAGRAM_BYTES.div_ceil(3) * 4;
const _: () = assert!(MAX_MCP_CONTENT_TRANSFER_BASE64_LENGTH < MAX_MCP_BRIDGE_REQUEST_BYTES);
const _: () = assert!(MAX_MCP_TUNNEL_EXCHANGE_BASE64_LENGTH < MAX_MCP_BRIDGE_REQUEST_BYTES);
const _: () = assert!(MAX_MCP_UDP_DATAGRAM_BASE64_LENGTH < MAX_MCP_BRIDGE_REQUEST_BYTES);
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
            "List visible PortMate session summaries. Read-only and requires the read-sessions scope. Results are limited to sessions allowed for this MCP client and redact sensitive profile fields; this tool never opens, reconnects, or closes a session.",
            json!({"type":"object","properties":{}}),
            true,
        ),
        tool(
            "mcp_bridge_status",
            "MCP Bridge Status",
            "Read the PortMate MCP Bridge transport, desktop IPC, store, and managed HTTP sidecar status. Read-only and requires read-mcp; tokens, passwords, private keys, and other secret values are never returned. This tool does not change sessions or grants.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            true,
        ),
        tool(
            "reload_mcp",
            "Reload MCP Bridge",
            "Reload the MCP Bridge store and desktop IPC endpoint sources, then return the current Bridge status. Read-only and requires read-mcp; it refreshes this bridge process only, does not restart PortMate or a managed HTTP sidecar, and does not change grants.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            true,
        ),
        tool(
            "restart_mcp",
            "Restart MCP Bridge",
            "Restart the PortMate-managed MCP HTTP sidecar and return its runtime status. This is a manage-mcp write and may interrupt active HTTP clients. A managed HTTP sidecar cannot restart itself from an in-flight request; use a stdio Bridge or the PortMate desktop UI. It never restarts the PortMate desktop process.",
            json!({"type":"object","additionalProperties":false,"properties":{}}),
            false,
        ),
        tool(
            "read_screen",
            "Read Screen",
            "Read the current terminal screen snapshot for one authorized session. Read-only and requires read-logs; output is secret-redacted and may be empty when no screen has been captured. It does not return a raw byte stream or send input.",
            session_schema(),
            true,
        ),
        tool(
            "tail_log",
            "Tail Log",
            "Read recent structured log lines from one authorized session. Read-only and requires read-logs; limit is clamped to 1-1000 and event text/metadata are redacted. It does not return private key material or change the session.",
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
            "Search redacted text logs for authorized sessions. Read-only and requires read-logs; an optional sessionId narrows the search and limit is clamped to 1-1000. Queries do not execute commands and secret values are filtered from results.",
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
            "Write the exact supplied text to a currently connected authorized terminal session. Requires the write-input scope and per-write confirmation when the grant enables it; no newline is added by this tool, although Telnet protocol framing may transform wire bytes. The returned event is redacted and the tool does not create a new process.",
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
            "Write decoded bytes to a currently connected authorized session without adding a newline or converting the payload to terminal text. Requires write-input; encoding is standard Base64 or hexadecimal, decoded payload is limited to 4 MiB, Telnet escaping is applied only when required by negotiation, and the audit/event surface contains a redacted byte summary rather than the payload. This is a TCP/serial/terminal byte path, not a UDP datagram API.",
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
            "Send one supported terminal key sequence to a currently connected authorized session. Requires write-input; the key name is converted to PortMate's bounded terminal sequence table and is not an arbitrary shell command or arbitrary escape payload. The resulting event is redacted.",
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
            "Pulse the hardware Break condition on a connected serial session (approximately 250 ms). Requires write-input and a serial runtime/driver that supports Break; it is not text input, Ctrl+C, or a network UDP operation.",
            session_schema(),
            false,
        ),
        tool(
            "run_command",
            "Run Command",
            "Write a command followed by its protocol terminator to one currently connected authorized session. Requires write-input; SSH/Tmux commands run remotely, Shell commands run inside the saved local Shell PTY, and Telnet receives its protocol-specific line ending. It does not accept a program path, working directory, password, or private key from MCP.",
            json!({
                "type":"object",
                "required":["sessionId","command"],
                "properties":{"sessionId":{"type":"string"},"command":{"type":"string"}}
            }),
            false,
        ),
        tool(
            "run_local_command",
            "Run Local Command",
            "Run a command followed by newline in a connected PortMate local Shell session. Requires write-input, an existing saved Shell profile, and its live PTY; the MCP caller cannot choose the shell program, arguments, or working directory. The same grant, confirmation, revalidation, redaction, and audit rules as other terminal input apply.",
            json!({
                "type":"object",
                "required":["sessionId","command"],
                "additionalProperties":false,
                "properties":{
                    "sessionId":{"type":"string","minLength":1,"maxLength":128},
                    "command":{"type":"string","minLength":1}
                }
            }),
            false,
        ),
        tool(
            "list_custom_scripts",
            "List Custom Scripts",
            "List saved MCP-enabled custom script summaries for one authorized session. Read-only and requires read-scripts; only IDs, names, descriptions, and version metadata are returned. Script bodies and secret material are never returned, and this tool does not execute a script.",
            session_schema(),
            true,
        ),
        tool(
            "run_custom_script",
            "Run Custom Script",
            "Run one saved MCP-enabled custom script in an authorized currently connected session. Requires run-scripts plus the script's own MCP/session boundary; the request selects an existing script by ID and cannot provide, read, or replace its body. Write confirmation, version revalidation, output redaction, and audit still apply.",
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
            "List recent file-transfer tasks visible to this MCP client. Read-only and requires read-transfers (or the transfer scope implication); paths are always redacted. An optional sessionId narrows results and limit is clamped to 1-1000. This tool does not start, cancel, or retry a transfer.",
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
            "Read one authorized file-transfer task by ID. Read-only and requires read-transfers (or the transfer scope implication); source and destination paths are redacted. It reports asynchronous state only and does not alter the task.",
            transfer_id_schema(),
            true,
        ),
        tool(
            "start_transfer",
            "Start Transfer",
            "Start one asynchronous SFTP, SCP, TFTP, XModem, YModem, or ZModem transfer. Requires the transfer scope and a sessionId-bound route unless using a completed uploadId. Select exactly one source: a desktop path, a virtual MCP file `{kind: \"mcp\", fileName, contentBase64}`, legacy inline fields, or uploadId. Desktop paths are resolved on the PortMate host; virtual/legacy inline content is limited to 4 MiB decoded, while resumable uploads support files up to 512 MiB with 4 MiB chunks. At least one endpoint must be `remote:`, `ssh:`, or a constrained `load:` receiver; pure local-to-local copy and arbitrary remote paths are rejected. For structured TFTP, deviceIp is required; destination options are bound before upload. Modem receivers are `load:loadx`, `load:loady`, or `load:loadz`. The call only queues the task; poll get_transfer for completion, cancellation, or failure.",
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
            "Begin a private resumable staging upload for client-held bytes. Requires the transfer scope; declares a 1-512 MiB file, full SHA-256, session, protocol, and destination before any bytes are accepted. TFTP destination options are validated before content is uploaded and fixed here. Active uploads share a 1 GiB declared-size quota and are owned by this MCP Client ID. Append ordered Base64 chunks (up to 4 MiB decoded each), then call start_transfer with only the returned uploadId; no transfer starts at this step.",
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
            "Append one ordered standard-Base64 chunk to this client's private resumable upload. Requires transfer scope and the upload owner; decoded chunk size is 1-4 MiB, offset must equal the previous nextOffset, and this call only stages bytes. It does not trigger the final transfer or a new approval prompt for every chunk.",
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
            "Delete an incomplete private resumable content upload owned by this MCP Client ID. Requires transfer scope; it cannot cancel a transfer that has already been finalized, and staged bytes are removed.",
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
            "Cancel a queued or running authorized file-transfer task. Requires transfer scope and the task's recorded session boundary; completed tasks cannot be cancelled. The result reports applied cancellation state, not a promise that a device has already stopped mid-protocol.",
            transfer_id_schema(),
            false,
        ),
        tool(
            "retry_transfer",
            "Retry Transfer",
            "Retry an eligible previous file-transfer task with its recorded protocol, endpoints, and session. Requires transfer scope; paths are taken from the stored task and cannot be replaced by MCP. Inline virtual-content tasks and tasks whose staged bytes were deleted are not retryable; poll get_transfer for the new asynchronous task state.",
            transfer_id_schema(),
            false,
        ),
        tool(
            "create_tunnel",
            "Create Forward Or Proxy",
            "Create a TCP-only fixed forward or dynamic SOCKS5 proxy; UDP datagrams, UDP ASSOCIATE, multicast, broadcast, DTLS, and QUIC are not supported. Requires the tunnel scope. Use egress `ssh` with a connected authorized SSH/Tmux session, or egress `portmate-host` without sessionId for a target reachable from the PortMate host. SSH egress supports local/remote/dynamic modes; PortMate-host egress is independent of terminal sessions and supports local/dynamic only. Dynamic host routes require routeRules. bindPort=0 selects an available listener port; non-loopback host listeners require allowRemoteBind and are protected only by network reachability, not an MCP token.",
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
            "List active TCP forwards and SOCKS5 proxies. Read-only and requires read-tunnels (or the tunnel scope implication). Provide sessionId for that session's SSH/Tmux routes; omit it for this MCP Client ID's PortMate-host routes. It reports runtime listeners, byte counters, and errors but never creates a route or exposes UDP state.",
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
            "Stop an active authorized TCP forward or SOCKS5 proxy by tunnel ID. Requires tunnel scope and ownership/session authorization; it closes listeners and active connections and does not modify the operating system routing table. UDP tunnels cannot be stopped because this API does not create them.",
            tunnel_id_schema(),
            false,
        ),
        tool(
            "tunnel_request",
            "Request Through Tunnel",
            "Send one bounded raw TCP request/response exchange through an existing owned PortMate-host tunnel from the desktop host. Requires tunnel scope; this is not a persistent stream, not a file-transfer protocol, and not UDP/UDP ASSOCIATE. Request and response are each limited to 4 MiB, timeout is 100 ms-30 s, fixed routes reject target overrides, and dynamic routes require targetHost/targetPort allowed by routeRules. Use a directly reachable host listener or resumable transfer for large/stateful streams.",
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
            "udp_request",
            "Request Through UDP Route",
            "Send one bounded UDP datagram through an existing PortMate-host route and wait for one response datagram. Requires tunnel scope and an owned host route; this is a datagram exchange, not a persistent UDP association. It can carry individual TFTP, QUIC, or DTLS packets, but it does not implement their connection/session state or SOCKS5 UDP ASSOCIATE control channel. Request and response are each limited to 65507 bytes and timeout is 100 ms-30 s.",
            json!({
                "type":"object",
                "required":["tunnelId","encoding","data"],
                "additionalProperties":false,
                "properties":{
                    "tunnelId":{"type":"string","minLength":1,"maxLength":128},
                    "encoding":{"type":"string","enum":["base64","hex"]},
                    "data":{"type":"string","minLength":1,"maxLength":MAX_MCP_UDP_DATAGRAM_BASE64_LENGTH},
                    "targetHost":{"type":"string","minLength":1,"maxLength":255},
                    "targetPort":{"type":"integer","minimum":1,"maximum":65535},
                    "timeoutMs":{"type":"integer","minimum":100,"maximum":MAX_MCP_TUNNEL_EXCHANGE_TIMEOUT_MS,"default":10000}
                }
            }),
            false,
        ),
        tool(
            "list_tmux_state",
            "List Tmux State",
            "Read tmux sessions and panes for one connected authorized SSH/Tmux session. Read-only and requires read-logs; output is bounded and redacted. It never creates, kills, or attaches a tmux session.",
            session_schema(),
            true,
        ),
        tool(
            "attach_tmux",
            "Attach Tmux",
            "Send a bounded attach/switch command to a connected authorized SSH/Tmux session. Requires write-input and a validated tmux target; it does not execute arbitrary shell text or create a new local process, and normal confirmation/revalidation/audit rules apply.",
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
            "Export a redacted incident bundle of logs and session metadata for one authorized session. Read-only and requires read-logs; credentials, private keys, raw secret values, and unapproved filesystem paths are excluded. It produces an export payload and does not mutate the session.",
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
        assert_eq!(tool_definitions().len(), 31);
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

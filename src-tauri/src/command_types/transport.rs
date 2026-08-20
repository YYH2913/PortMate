use portmate_core::{
    TransferProtocol, TransferTask, TunnelEgress, TunnelMode, TunnelRouteRule, TunnelSpec,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTransferRequest {
    pub session_id: String,
    pub protocol: TransferProtocol,
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartMcpContentTransferRequest {
    pub session_id: String,
    pub protocol: TransferProtocol,
    pub file_name: String,
    pub content_base64: String,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum McpVirtualFileSource {
    Mcp {
        file_name: String,
        content_base64: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartMcpContentUploadTransferRequest {
    pub upload_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartExternalDropRequest {
    pub session_id: String,
    pub paths: Vec<String>,
    pub destination: String,
    pub remote: bool,
    #[serde(default)]
    pub conflict_policy: TransferConflictPolicy,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferConflictPolicy {
    #[default]
    Fail,
    Overwrite,
    Skip,
    Rename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartFileBatchRequest {
    pub session_id: String,
    pub paths: Vec<String>,
    pub source_remote: bool,
    pub destination: String,
    pub destination_remote: bool,
    #[serde(default)]
    pub conflict_policy: TransferConflictPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDropResult {
    pub tasks: Vec<TransferTask>,
    pub directories_prepared: usize,
    pub skipped: Vec<String>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTunnelRequest {
    pub session_id: String,
    #[serde(default)]
    pub egress: TunnelEgress,
    pub mode: TunnelMode,
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: String,
    #[serde(default)]
    pub target_port: u16,
    #[serde(default)]
    pub route_rules: Vec<TunnelRouteRule>,
    #[serde(default)]
    pub allow_remote_bind: bool,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateMcpTunnelRequest {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub egress: Option<TunnelEgress>,
    pub mode: TunnelMode,
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: String,
    #[serde(default)]
    pub target_port: u16,
    #[serde(default)]
    pub route_rules: Vec<TunnelRouteRule>,
    #[serde(default)]
    pub allow_remote_bind: bool,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateHostRouteRequest {
    pub mode: TunnelMode,
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub target_host: String,
    #[serde(default)]
    pub target_port: u16,
    #[serde(default)]
    pub route_rules: Vec<TunnelRouteRule>,
    #[serde(default)]
    pub allow_remote_bind: bool,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub spec: TunnelSpec,
    pub active_connections: u64,
    pub total_connections: u64,
    pub tcp_to_ssh_bytes: u64,
    pub ssh_to_tcp_bytes: u64,
    pub last_activity: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpTunnelExchangeRequest {
    pub tunnel_id: String,
    pub encoding: String,
    #[serde(default)]
    pub data: String,
    #[serde(default)]
    pub target_host: Option<String>,
    #[serde(default)]
    pub target_port: Option<u16>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub max_response_bytes: Option<usize>,
    #[serde(default = "default_true")]
    pub close_write: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTunnelExchangeResult {
    pub tunnel_id: String,
    pub target_host: String,
    pub target_port: u16,
    pub sent_bytes: usize,
    pub received_bytes: usize,
    pub response_base64: String,
    pub truncated: bool,
    pub timed_out: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialLineRequest {
    pub session_id: String,
    pub dtr: Option<bool>,
    pub rts: Option<bool>,
}

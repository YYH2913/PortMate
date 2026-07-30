use portmate_core::{TransferProtocol, TransferTask, TunnelMode, TunnelSpec};
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
    pub mode: TunnelMode,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
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
#[serde(rename_all = "camelCase")]
pub struct SerialLineRequest {
    pub session_id: String,
    pub dtr: Option<bool>,
    pub rts: Option<bool>,
}

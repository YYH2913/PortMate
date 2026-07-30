use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogShardInfo {
    pub path: String,
    pub format: String,
    pub size: u64,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogShardPreview {
    pub path: String,
    pub content: String,
    pub encoding: String,
    pub bytes_read: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteLogShardsResult {
    pub deleted: usize,
    pub bytes_deleted: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchLogShardsRequest {
    pub query: String,
    #[serde(default)]
    pub paths: Vec<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogShardSearchMatch {
    pub path: String,
    pub format: String,
    pub line: u64,
    pub byte_offset: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchLogShardsResult {
    pub matches: Vec<LogShardSearchMatch>,
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveLogShardsRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveLogShardsResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub shards: usize,
    pub source_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSerialCaptureRequest {
    pub session_id: String,
    pub frame_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSerialCaptureResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub frames: usize,
    pub captured_bytes: usize,
    pub truncated_frames: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMcpAuditRequest {
    pub record_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMcpAuditResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub records: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalTextExportSource {
    Buffer,
    Selection,
}

impl TerminalTextExportSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Buffer => "buffer",
            Self::Selection => "selection",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTerminalTextRequest {
    pub session_id: String,
    pub view_id: String,
    pub source: TerminalTextExportSource,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTerminalTextResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub session_id: String,
    pub view_id: String,
    pub source: TerminalTextExportSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionBundleArchiveRequest {
    pub session_id: String,
    #[serde(default = "default_true")]
    pub redact_secrets: bool,
    #[serde(default)]
    pub include_raw_logs: bool,
    #[serde(default)]
    pub attachment_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionBundleArchiveResult {
    pub path: String,
    pub checksum_path: String,
    pub signature_path: String,
    pub sha256: String,
    pub signature_algorithm: String,
    pub signing_public_key: String,
    pub size: u64,
    pub files: usize,
    pub raw_log_segments: usize,
    pub attachments: usize,
    pub redacted: bool,
    pub warnings: Vec<String>,
}

fn default_true() -> bool {
    true
}

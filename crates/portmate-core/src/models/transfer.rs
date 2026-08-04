use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferSettings {
    pub sftp: bool,
    pub scp: bool,
    pub xmodem: bool,
    pub ymodem: bool,
    pub zmodem: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_bytes_per_second: Option<u64>,
    pub default_local_dir: Option<String>,
}

impl Default for TransferSettings {
    fn default() -> Self {
        Self {
            sftp: true,
            scp: true,
            xmodem: true,
            ymodem: true,
            zmodem: true,
            rate_limit_bytes_per_second: None,
            default_local_dir: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferProtocol {
    Sftp,
    Scp,
    Xmodem,
    Ymodem,
    Zmodem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferTask {
    pub id: String,
    pub session_id: String,
    pub protocol: TransferProtocol,
    pub source: String,
    pub destination: String,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub status: TransferStatus,
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average_bytes_per_second: Option<f64>,
}

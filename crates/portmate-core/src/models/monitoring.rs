use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonProcess {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub rss_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonDisk {
    pub filesystem: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_percent: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonNetworkInterface {
    pub name: String,
    #[serde(default)]
    pub addresses: Vec<String>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_kbps: f32,
    pub tx_kbps: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysmonSnapshot {
    pub session_id: String,
    pub ts: DateTime<Utc>,
    pub uptime_seconds: u64,
    pub cpu_percent: f32,
    pub memory_percent: f32,
    pub rx_kbps: f32,
    pub tx_kbps: f32,
    #[serde(default)]
    pub load_average: [f32; 3],
    #[serde(default)]
    pub memory_total_bytes: u64,
    #[serde(default)]
    pub memory_available_bytes: u64,
    #[serde(default)]
    pub processes: Vec<SysmonProcess>,
    #[serde(default)]
    pub disks: Vec<SysmonDisk>,
    #[serde(default)]
    pub network_interfaces: Vec<SysmonNetworkInterface>,
}

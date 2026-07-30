use super::OneKeySummary;
use portmate_core::{HostKeyStore, McpGrant, SessionSummary};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionProfileResponse {
    pub deleted_profile_id: String,
    pub sessions: Vec<SessionSummary>,
    pub one_keys: Vec<OneKeySummary>,
    pub host_keys: HostKeyStore,
    pub grants: Vec<McpGrant>,
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveCustomScriptRequest {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub content: String,
    #[serde(default)]
    pub allow_all_sessions: bool,
    #[serde(default)]
    pub allowed_session_ids: Vec<String>,
    #[serde(default)]
    pub mcp_enabled: bool,
    pub expected_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCustomScriptRequest {
    pub id: String,
    pub expected_updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCustomScriptRequest {
    pub script_id: String,
    pub session_id: String,
}

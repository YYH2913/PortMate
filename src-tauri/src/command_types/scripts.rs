use chrono::{DateTime, Utc};
use portmate_core::CustomScript;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveCustomScriptRequest {
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub content: String,
    pub host: portmate_core::HostScriptConfig,
    #[serde(default)]
    pub mcp_enabled: bool,
    pub expected_updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveCustomScriptResponse {
    pub scripts: Vec<CustomScript>,
    pub saved_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteCustomScriptRequest {
    pub id: String,
    pub expected_updated_at: DateTime<Utc>,
}

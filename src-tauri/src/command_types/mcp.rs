use portmate_core::McpHttpSettings;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpHttpConfig {
    #[serde(flatten)]
    pub settings: McpHttpSettings,
    pub remote_access: bool,
    pub endpoint: String,
    pub token_ref: String,
    pub token_available: bool,
    pub default_origin: String,
    pub executable: String,
    pub store_path: String,
    pub start_command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpHttpTokenResponse {
    pub config: McpHttpConfig,
    pub token: String,
}

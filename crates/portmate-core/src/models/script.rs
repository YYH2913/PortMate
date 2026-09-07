use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CustomScript {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
    pub host: HostScriptConfig,
    #[serde(default)]
    pub mcp_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CustomScript {
    pub fn allows_host_client(&self, client_id: &str) -> bool {
        self.mcp_enabled
            && self
                .host
                .allowed_client_ids
                .iter()
                .any(|id| id == client_id)
    }

    pub fn host_tool_name(&self) -> String {
        format!("host_script_{}", self.id.replace('-', ""))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostScriptLanguage {
    Python,
    Shell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostScriptConfig {
    pub language: HostScriptLanguage,
    /// Empty selects the platform default; nonempty must be an absolute executable path.
    #[serde(default)]
    pub interpreter: String,
    /// Empty selects the user's home, never a terminal session's directory.
    #[serde(default)]
    pub working_directory: String,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub allowed_client_ids: Vec<String>,
    #[serde(default)]
    pub parameters: Vec<HostScriptParameter>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostScriptParameter {
    pub name: String,
    pub description: String,
    pub kind: HostScriptParameterKind,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostScriptParameterKind {
    String,
    Number,
    Integer,
    Boolean,
}

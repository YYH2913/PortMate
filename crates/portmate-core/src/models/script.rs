use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomScript {
    pub id: String,
    pub name: String,
    pub description: String,
    pub content: String,
    #[serde(default)]
    pub allow_all_sessions: bool,
    #[serde(default)]
    pub allowed_session_ids: Vec<String>,
    #[serde(default)]
    pub mcp_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl CustomScript {
    pub fn allows_session(&self, session_id: &str) -> bool {
        self.allow_all_sessions
            || self
                .allowed_session_ids
                .iter()
                .any(|allowed| allowed == session_id)
    }

    pub fn summary(&self) -> CustomScriptSummary {
        CustomScriptSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            updated_at: self.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomScriptSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub updated_at: DateTime<Utc>,
}

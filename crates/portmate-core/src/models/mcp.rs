use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpScope {
    ReadSessions,
    ReadLogs,
    WriteInput,
    Transfer,
    Tunnel,
    ManageSessions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpGrant {
    pub client_id: String,
    pub name: String,
    pub scopes: Vec<McpScope>,
    pub allowed_sessions: Vec<String>,
    #[serde(default)]
    pub confirm_writes: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl McpGrant {
    pub fn allows(&self, scope: McpScope, session_id: Option<&str>, now: DateTime<Utc>) -> bool {
        if self.revoked_at.is_some() || self.expires_at.is_some_and(|expires| expires <= now) {
            return false;
        }
        if !self.scopes.contains(&scope) {
            return false;
        }
        match session_id {
            Some(id) => {
                self.allowed_sessions.is_empty() || self.allowed_sessions.iter().any(|s| s == id)
            }
            None => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditRecord {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub session_id: Option<String>,
    pub decision: String,
    pub details: BTreeMap<String, String>,
}

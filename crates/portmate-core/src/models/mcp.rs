use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const DEFAULT_MCP_HTTP_LISTEN_HOST: &str = "127.0.0.1";
pub const DEFAULT_MCP_HTTP_CLIENT_HOST: &str = "127.0.0.1";
pub const DEFAULT_MCP_HTTP_PORT: u16 = 8787;
pub const DEFAULT_MCP_HTTP_CLIENT_ID: &str = "portmate-local";
/// Reserved in-memory/store value for a grant that deliberately authorizes no
/// session. An empty `allowed_sessions` list remains the legacy "all sessions"
/// form, so existing grants keep their meaning.
pub const MCP_NO_SESSIONS_SENTINEL: &str = "__portmate_no_sessions__";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpScope {
    ReadSessions,
    ReadLogs,
    ReadTransfers,
    ReadTunnels,
    ReadScripts,
    ReadMcp,
    WriteInput,
    Transfer,
    Tunnel,
    ManageSessions,
    RunScripts,
    ManageMcp,
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
    pub fn denies_all_sessions(&self) -> bool {
        self.allowed_sessions.len() == 1 && self.allowed_sessions[0] == MCP_NO_SESSIONS_SENTINEL
    }

    pub fn allows(&self, scope: McpScope, session_id: Option<&str>, now: DateTime<Utc>) -> bool {
        if self.revoked_at.is_some() || self.expires_at.is_some_and(|expires| expires <= now) {
            return false;
        }
        let implied = match scope {
            McpScope::ReadTransfers => self.scopes.contains(&McpScope::Transfer),
            McpScope::ReadTunnels => self.scopes.contains(&McpScope::Tunnel),
            McpScope::ReadScripts => self.scopes.contains(&McpScope::RunScripts),
            _ => false,
        };
        if !self.scopes.contains(&scope) && !implied {
            return false;
        }
        match session_id {
            Some(id) => {
                !self.denies_all_sessions()
                    && (self.allowed_sessions.is_empty()
                        || self.allowed_sessions.iter().any(|s| s == id))
            }
            // Collection reads (list/search) are allowed to run without a
            // session id and are filtered by the caller. Session-scoped
            // writes still require a global grant when no target is supplied.
            None if is_collection_read_scope(scope) || is_host_level_scope(scope) => true,
            None => self.allowed_sessions.is_empty(),
        }
    }
}

fn is_collection_read_scope(scope: McpScope) -> bool {
    matches!(
        scope,
        McpScope::ReadSessions
            | McpScope::ReadLogs
            | McpScope::ReadTransfers
            | McpScope::ReadTunnels
            | McpScope::ReadMcp
    )
}

fn is_host_level_scope(scope: McpScope) -> bool {
    matches!(scope, McpScope::Tunnel | McpScope::ManageMcp)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct McpHttpSettings {
    pub listen_host: String,
    pub client_host: String,
    pub port: u16,
    pub allowed_origins: Vec<String>,
    pub client_id: String,
    pub trusted: bool,
    pub allow_remote: bool,
}

impl Default for McpHttpSettings {
    fn default() -> Self {
        Self {
            listen_host: DEFAULT_MCP_HTTP_LISTEN_HOST.to_string(),
            client_host: DEFAULT_MCP_HTTP_CLIENT_HOST.to_string(),
            port: DEFAULT_MCP_HTTP_PORT,
            allowed_origins: vec![
                format!("http://{DEFAULT_MCP_HTTP_LISTEN_HOST}:{DEFAULT_MCP_HTTP_PORT}"),
                format!("http://localhost:{DEFAULT_MCP_HTTP_PORT}"),
            ],
            client_id: DEFAULT_MCP_HTTP_CLIENT_ID.to_string(),
            trusted: false,
            allow_remote: false,
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

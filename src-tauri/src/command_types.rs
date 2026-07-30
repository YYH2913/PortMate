use super::*;

mod exports;
mod files;
mod one_key;
mod security;
mod tmux;
mod transport;
mod vault;

pub use exports::*;
pub use files::*;
pub use one_key::*;
pub use security::*;
pub use tmux::*;
pub use transport::*;
pub use vault::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretWriteRequest {
    pub secret_ref: Option<String>,
    pub secret: String,
    pub storage: Option<SecretStorage>,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum ProxyPasswordUpdate {
    Set {
        password: String,
        #[serde(default)]
        storage: Option<SecretStorage>,
    },
    Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretStorage {
    Native,
    Portable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretWriteResponse {
    pub secret_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionProfileResponse {
    pub deleted_profile_id: String,
    pub sessions: Vec<SessionSummary>,
    pub one_keys: Vec<OneKeySummary>,
    pub host_keys: HostKeyStore,
    pub grants: Vec<McpGrant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpHttpConfig {
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

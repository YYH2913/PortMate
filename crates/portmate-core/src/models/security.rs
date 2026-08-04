use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKeyMode {
    Strict,
    TrustOnFirstUse,
    AskEveryTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKeyScope {
    Profile,
    Project,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPolicy {
    pub mode: HostKeyMode,
    pub alias: Option<String>,
    pub trust_scope: HostKeyScope,
    pub allow_rotation: bool,
    pub check_ip: bool,
}

impl HostKeyPolicy {
    pub fn profile_alias(profile_id: &str) -> Self {
        Self {
            mode: HostKeyMode::Strict,
            alias: Some(profile_id.to_string()),
            trust_scope: HostKeyScope::Profile,
            allow_rotation: false,
            check_ip: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HostKeyDecision {
    TrustOnce,
    AppendToProfile,
    AppendToProject,
    ReplaceForProfile,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedHostKey {
    pub id: String,
    pub profile_id: Option<String>,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub algorithm: String,
    pub fingerprint_sha256: String,
    pub public_key_base64: String,
    pub scope: HostKeyScope,
    pub label: Option<String>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
}

impl TrustedHostKey {
    pub fn target_key(&self) -> String {
        format!("{}:{}", self.alias, self.port)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    PublicKey,
    KeyboardInteractive,
    Password,
    GssapiWithMic,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentitySource {
    ProfileVault,
    SystemFile,
    Agent,
    PublicKeyOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityRef {
    pub id: String,
    pub label: String,
    pub source: IdentitySource,
    pub fingerprint_sha256: Option<String>,
    pub path: Option<String>,
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OneKeyKind {
    Account,
    Ssh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneKeyIdentity {
    pub source_profile_id: String,
    pub identity: IdentityRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneKeyCredential {
    pub id: String,
    pub label: String,
    pub kind: OneKeyKind,
    pub username: String,
    pub password_secret_ref: Option<String>,
    pub passphrase_secret_ref: Option<String>,
    #[serde(default)]
    pub identity: Option<OneKeyIdentity>,
    #[serde(default)]
    pub session_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityPolicy {
    pub identities_only: bool,
    pub auth_order: Vec<AuthMethod>,
    pub record_success: bool,
    pub last_successful: Option<AuthMethod>,
}

impl Default for IdentityPolicy {
    fn default() -> Self {
        Self {
            identities_only: true,
            auth_order: vec![
                AuthMethod::PublicKey,
                AuthMethod::KeyboardInteractive,
                AuthMethod::Password,
            ],
            record_success: true,
            last_successful: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentOfferMode {
    Disabled,
    AfterProfileKeys,
    BeforeProfileKeys,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPolicy {
    pub enabled: bool,
    pub forwarding: bool,
    pub offer_mode: AgentOfferMode,
}

impl Default for AgentPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            forwarding: false,
            offer_mode: AgentOfferMode::AfterProfileKeys,
        }
    }
}

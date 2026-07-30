use super::SecretStorage;
use chrono::{DateTime, Utc};
use portmate_core::{IdentitySource, OneKeyKind};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneKeyIdentitySummary {
    pub source_profile_id: String,
    pub id: String,
    pub label: String,
    pub source: IdentitySource,
    pub fingerprint_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneKeySummary {
    pub id: String,
    pub label: String,
    pub kind: OneKeyKind,
    pub username: String,
    pub has_password: bool,
    pub has_passphrase: bool,
    pub identity: Option<OneKeyIdentitySummary>,
    pub session_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OneKeyMutationResponse {
    pub items: Vec<OneKeySummary>,
    pub saved_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum OneKeySecretUpdate {
    Preserve,
    Set {
        secret: String,
        #[serde(default)]
        storage: Option<SecretStorage>,
    },
    Clear,
}

#[derive(Debug, Default, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum OneKeyIdentityUpdate {
    #[default]
    Preserve,
    Set {
        source_profile_id: String,
        identity_id: String,
    },
    Clear,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveOneKeyRequest {
    pub id: Option<String>,
    pub label: String,
    pub kind: OneKeyKind,
    pub username: String,
    pub password_update: OneKeySecretUpdate,
    pub passphrase_update: OneKeySecretUpdate,
    #[serde(default)]
    pub identity_update: OneKeyIdentityUpdate,
    pub session_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteOneKeyRequest {
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OneKeyField {
    Username,
    Password,
    Passphrase,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OneKeySendSource {
    #[default]
    Manual,
    PromptCompletion,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOneKeyRequest {
    pub id: String,
    pub session_id: String,
    pub field: OneKeyField,
    #[serde(default)]
    pub source: OneKeySendSource,
    #[serde(default)]
    pub prompt_event_id: Option<String>,
}

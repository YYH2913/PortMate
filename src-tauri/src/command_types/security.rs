use super::SecretStorage;
use portmate_core::{
    HostKeyDecision, HostKeyEvaluation, HostKeyObservation, HostKeyScope, IdentityRef,
    IdentitySource, SessionProfile, SessionSummary, TrustedHostKey,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyDecisionRequest {
    pub profile_id: String,
    pub observation: HostKeyObservation,
    pub decision: HostKeyDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientIdentityUpdateRequest {
    pub profile_id: String,
    pub identity_id: String,
    pub expected_identity: IdentityRef,
    pub label: String,
    pub source: IdentitySource,
    pub fingerprint_sha256: Option<String>,
    pub path: Option<String>,
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientIdentityRotateRequest {
    pub profile_id: String,
    pub identity_id: String,
    pub private_key: String,
    pub passphrase: Option<String>,
    pub storage: Option<SecretStorage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientIdentityDeleteRequest {
    pub profile_id: String,
    pub identity_id: String,
    #[serde(default)]
    pub delete_secret: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientIdentityMutationResponse {
    pub summary: SessionSummary,
    pub old_secret_deleted: bool,
    pub old_secret_shared: bool,
    pub cleanup_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyScanResult {
    pub label: Option<String>,
    pub observation: HostKeyObservation,
    pub evaluation: HostKeyEvaluation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostKeyScanRequest {
    pub profile: SessionProfile,
    pub credential_handle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostsImportRequest {
    pub profile_id: String,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyUpdateRequest {
    pub key_id: String,
    pub expected_key: TrustedHostKey,
    pub profile_id: Option<String>,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub scope: HostKeyScope,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustScannedHostKeyRequest {
    pub profile: SessionProfile,
    pub observation: HostKeyObservation,
    pub decision: HostKeyDecision,
}

use super::SecretStorage;
use chrono::{DateTime, Utc};
use portmate_core::SessionSummary;
use serde::{Deserialize, Serialize};

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableVaultStatus {
    pub exists: bool,
    pub unlocked: bool,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableVaultUnlockRequest {
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableVaultCreateRequest {
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableVaultRotatePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSecretMigrationRequest {
    pub target_storage: SecretStorage,
    pub profile_ids: Vec<String>,
    #[serde(default = "default_true")]
    pub cleanup_source: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSecretMigrationPreview {
    pub plan_token: String,
    pub target_storage: SecretStorage,
    pub selected_profile_count: usize,
    pub affected_profile_count: usize,
    pub eligible_reference_count: usize,
    pub eligible_secret_count: usize,
    pub retained_shared_secret_count: usize,
    pub retained_in_flight_secret_count: usize,
    pub already_target_reference_count: usize,
    pub excluded_reserved_reference_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSecretCleanupStatus {
    Deleted,
    RetainedByRequest,
    RetainedShared,
    RetainedInUse,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSecretMigrationItem {
    pub source_ref: String,
    pub target_ref: String,
    pub reference_count: usize,
    pub remaining_source_references: usize,
    pub cleanup_status: ProfileSecretCleanupStatus,
    pub cleanup_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSecretMigrationResponse {
    pub migration_id: Option<String>,
    pub recovery_pending: bool,
    pub target_storage: SecretStorage,
    pub selected_profile_count: usize,
    pub migrated_profile_count: usize,
    pub migrated_reference_count: usize,
    pub migrated_secret_count: usize,
    pub summaries: Vec<SessionSummary>,
    pub items: Vec<ProfileSecretMigrationItem>,
    pub warnings: Vec<String>,
    pub portable_vault_requires_reunlock: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSecretMigrationJournalState {
    TargetWritePending,
    TargetsVerified,
    ProfilesCommitted,
    SourceCleanupPending,
    TargetCleanupPending,
    NeedsResolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSecretMigrationRecoveryDisposition {
    NotCommitted,
    Committed,
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSecretMigrationRecoverySummary {
    pub migration_id: String,
    pub state: ProfileSecretMigrationJournalState,
    pub disposition: ProfileSecretMigrationRecoveryDisposition,
    pub target_storage: SecretStorage,
    pub cleanup_source: bool,
    pub profile_count: usize,
    pub secret_count: usize,
    pub requires_portable_vault_unlock: bool,
    pub can_recover: bool,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSecretMigrationRecoveryRequest {
    pub migration_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSecretMigrationRecoveryResponse {
    pub migration_id: String,
    pub resolved: bool,
    pub action: String,
    pub warnings: Vec<String>,
    pub pending: Option<ProfileSecretMigrationRecoverySummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSecretMigrationDiagnosticExportResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub migration_id: Option<String>,
    pub journal_valid: bool,
    pub warnings: Vec<String>,
}

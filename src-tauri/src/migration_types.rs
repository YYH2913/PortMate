use super::*;

#[derive(Debug, Clone)]
pub(super) struct ProfileSecretMigrationPlan {
    pub(super) preview: ProfileSecretMigrationPreview,
    pub(super) selected_profile_ids: Vec<String>,
    pub(super) affected_profile_ids: Vec<String>,
    pub(super) source_ref_counts: BTreeMap<String, usize>,
    pub(super) in_flight_source_refs: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSecretMigrationJournalProfile {
    pub(super) profile_id: String,
    pub(super) before: ProfileSecretMigrationJournalProjection,
    pub(super) after: ProfileSecretMigrationJournalProjection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSecretMigrationJournalProjection {
    #[serde(default)]
    pub(super) proxy_password_secret_ref: Option<String>,
    pub(super) password_secret_ref: Option<String>,
    pub(super) passphrase_secret_ref: Option<String>,
    pub(super) identity_secret_refs: BTreeMap<String, String>,
    pub(super) jumps: Vec<ProfileSecretMigrationJournalJumpProjection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSecretMigrationJournalJumpProjection {
    pub(super) password_secret_ref: Option<String>,
    pub(super) passphrase_secret_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSecretMigrationJournalItem {
    pub(super) source_ref: String,
    pub(super) target_ref: String,
    pub(super) reference_count: usize,
    pub(super) in_flight_at_start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSecretMigrationJournalPayload {
    pub(super) version: u32,
    pub(super) migration_id: String,
    pub(super) target_storage: SecretStorage,
    pub(super) cleanup_source: bool,
    pub(super) plan_token: String,
    pub(super) selected_profile_ids: Vec<String>,
    pub(super) profiles: Vec<ProfileSecretMigrationJournalProfile>,
    pub(super) items: Vec<ProfileSecretMigrationJournalItem>,
}

#[derive(Debug, Clone)]
pub(super) struct LoadedProfileSecretMigrationJournal {
    pub(super) state: ProfileSecretMigrationJournalState,
    pub(super) payload: ProfileSecretMigrationJournalPayload,
    pub(super) created_at: chrono::DateTime<Utc>,
    pub(super) updated_at: chrono::DateTime<Utc>,
}

pub(super) enum ProfileSecretMigrationJournalEvent {
    Prepared(ProfileSecretMigrationJournalPayload),
    Transition {
        migration_id: String,
        state: ProfileSecretMigrationJournalState,
    },
    Clear {
        migration_id: String,
    },
}

pub(super) enum ProfileSecretMigrationJournalVerification {
    Active {
        migration_id: String,
        state: ProfileSecretMigrationJournalState,
        payload_json: Option<String>,
    },
    Cleared {
        migration_id: String,
    },
}

pub(super) struct PreparedProfileSecretMigration {
    pub(super) source_ref: String,
    pub(super) target_ref: String,
    pub(super) secret: Zeroizing<String>,
}

#[derive(Debug)]
pub(super) enum ProfileSecretStoreCommit {
    Committed { warning: Option<String> },
    NotCommitted(String),
    Unknown(String),
}

pub(super) struct SecretBatchDeleteOutcome {
    pub(super) results: BTreeMap<String, Result<(), String>>,
    pub(super) portable_vault_requires_reunlock: bool,
}

pub(super) enum SecretProbeResult {
    Present(Zeroizing<String>),
    Missing,
    Unavailable(String),
}

pub(super) struct ProfileSecretMigrationRecoveryOutcome {
    pub(super) resolved: bool,
    pub(super) action: String,
    pub(super) warnings: Vec<String>,
}

pub(super) struct ActiveProfileSecretMigrationJournalMetadata {
    pub(super) row_id: String,
    pub(super) state: String,
    pub(super) payload_bytes: u64,
    pub(super) created_at: String,
    pub(super) updated_at: String,
}

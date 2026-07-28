use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyDecisionRequest {
    pub profile_id: String,
    pub observation: HostKeyObservation,
    pub decision: portmate_core::HostKeyDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTransferRequest {
    pub session_id: String,
    pub protocol: TransferProtocol,
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartExternalDropRequest {
    pub session_id: String,
    pub paths: Vec<String>,
    pub destination: String,
    pub remote: bool,
    #[serde(default)]
    pub conflict_policy: TransferConflictPolicy,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransferConflictPolicy {
    #[default]
    Fail,
    Overwrite,
    Skip,
    Rename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartFileBatchRequest {
    pub session_id: String,
    pub paths: Vec<String>,
    pub source_remote: bool,
    pub destination: String,
    pub destination_remote: bool,
    #[serde(default)]
    pub conflict_policy: TransferConflictPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalDropResult {
    pub tasks: Vec<TransferTask>,
    pub directories_prepared: usize,
    pub skipped: Vec<String>,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogShardInfo {
    pub path: String,
    pub format: String,
    pub size: u64,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogShardPreview {
    pub path: String,
    pub content: String,
    pub encoding: String,
    pub bytes_read: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteLogShardsResult {
    pub deleted: usize,
    pub bytes_deleted: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchLogShardsRequest {
    pub query: String,
    #[serde(default)]
    pub paths: Vec<String>,
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogShardSearchMatch {
    pub path: String,
    pub format: String,
    pub line: u64,
    pub byte_offset: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchLogShardsResult {
    pub matches: Vec<LogShardSearchMatch>,
    pub files_scanned: usize,
    pub bytes_scanned: u64,
    pub truncated: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveLogShardsRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveLogShardsResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub shards: usize,
    pub source_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSerialCaptureRequest {
    pub session_id: String,
    pub frame_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSerialCaptureResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub frames: usize,
    pub captured_bytes: usize,
    pub truncated_frames: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMcpAuditRequest {
    pub record_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportMcpAuditResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub records: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalTextExportSource {
    Buffer,
    Selection,
}

impl TerminalTextExportSource {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Buffer => "buffer",
            Self::Selection => "selection",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTerminalTextRequest {
    pub session_id: String,
    pub view_id: String,
    pub source: TerminalTextExportSource,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportTerminalTextResult {
    pub path: String,
    pub checksum_path: String,
    pub sha256: String,
    pub size: u64,
    pub session_id: String,
    pub view_id: String,
    pub source: TerminalTextExportSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionBundleArchiveRequest {
    pub session_id: String,
    #[serde(default = "default_true")]
    pub redact_secrets: bool,
    #[serde(default)]
    pub include_raw_logs: bool,
    #[serde(default)]
    pub attachment_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSessionBundleArchiveResult {
    pub path: String,
    pub checksum_path: String,
    pub signature_path: String,
    pub sha256: String,
    pub signature_algorithm: String,
    pub signing_public_key: String,
    pub size: u64,
    pub files: usize,
    pub raw_log_segments: usize,
    pub attachments: usize,
    pub redacted: bool,
    pub warnings: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTunnelRequest {
    pub session_id: String,
    pub mode: TunnelMode,
    pub bind_host: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub spec: TunnelSpec,
    pub active_connections: u64,
    pub total_connections: u64,
    pub tcp_to_ssh_bytes: u64,
    pub ssh_to_tcp_bytes: u64,
    pub last_activity: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileProperties {
    pub name: String,
    pub path: String,
    pub remote: bool,
    pub kind: String,
    pub is_dir: bool,
    pub is_file: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub permissions: Option<u32>,
    pub modified: Option<String>,
    pub accessed: Option<String>,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListFilesRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePropertiesRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileOperationRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletePathsRequest {
    pub session_id: Option<String>,
    pub paths: Vec<String>,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenamePathRequest {
    pub session_id: Option<String>,
    pub old_path: String,
    pub new_path: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MovePathsRequest {
    pub session_id: Option<String>,
    pub paths: Vec<String>,
    pub destination: String,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChmodPathRequest {
    pub session_id: Option<String>,
    pub path: String,
    pub mode: u32,
    pub remote: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialLineRequest {
    pub session_id: String,
    pub dtr: Option<bool>,
    pub rts: Option<bool>,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionProfileResponse {
    pub deleted_profile_id: String,
    pub sessions: Vec<SessionSummary>,
    pub one_keys: Vec<OneKeySummary>,
    pub host_keys: HostKeyStore,
    pub grants: Vec<McpGrant>,
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
    pub created_at: chrono::DateTime<Utc>,
    pub updated_at: chrono::DateTime<Utc>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxSessionInfo {
    pub name: String,
    pub windows: u32,
    pub attached: u32,
    pub created: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxPaneInfo {
    pub session: String,
    pub window_index: u32,
    pub pane_index: u32,
    pub pane_id: String,
    pub active: bool,
    pub synchronized: bool,
    pub command: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxWindowInfo {
    pub session: String,
    pub window_index: u32,
    pub window_id: String,
    pub name: String,
    pub panes: u32,
    pub active: bool,
    pub synchronized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxState {
    pub sessions: Vec<TmuxSessionInfo>,
    pub windows: Vec<TmuxWindowInfo>,
    pub panes: Vec<TmuxPaneInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TmuxControlStatus {
    pub session_id: String,
    pub target: String,
    pub active: bool,
    #[serde(default)]
    pub runtime_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TmuxControlEvent {
    pub session_id: String,
    pub target: String,
    pub kind: String,
    pub active: bool,
    pub runtime_id: String,
    #[serde(default)]
    pub protocol_event: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TmuxMutationAction {
    RenameSession,
    KillSession,
    NewWindow,
    RenameWindow,
    KillWindow,
    KillPane,
    SelectPane,
    BreakPane,
    MovePaneHorizontal,
    MovePaneVertical,
    SplitPaneHorizontal,
    SplitPaneVertical,
    SwapPanePrevious,
    SwapPaneNext,
    ResizePaneLeft,
    ResizePaneRight,
    ResizePaneUp,
    ResizePaneDown,
    SelectLayout,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TmuxWindowLayout {
    EvenHorizontal,
    EvenVertical,
    MainHorizontal,
    MainVertical,
    Tiled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TmuxMutationRequest {
    pub session_id: String,
    pub action: TmuxMutationAction,
    pub target: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub destination: Option<String>,
    #[serde(default)]
    pub layout: Option<TmuxWindowLayout>,
    #[serde(default)]
    pub amount: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyScanResult {
    pub label: Option<String>,
    pub observation: HostKeyObservation,
    pub evaluation: HostKeyEvaluation,
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

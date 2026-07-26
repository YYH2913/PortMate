use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ProfileSecretMigrationDiagnosticProjectionStatus {
    Before,
    After,
    Conflict,
    Missing,
    Invalid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileSecretMigrationDiagnosticProfile {
    profile_id: String,
    profile_name: Option<String>,
    status: ProfileSecretMigrationDiagnosticProjectionStatus,
    before: ProfileSecretMigrationJournalProjection,
    current: Option<ProfileSecretMigrationJournalProjection>,
    after: ProfileSecretMigrationJournalProjection,
    projection_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ProfileSecretMigrationDiagnosticProbeStatus {
    Present,
    Missing,
    Unavailable,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileSecretMigrationDiagnosticProbe {
    status: ProfileSecretMigrationDiagnosticProbeStatus,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileSecretMigrationDiagnosticSecret {
    source_ref: String,
    target_ref: String,
    source_storage: SecretStorage,
    target_storage: SecretStorage,
    expected_reference_count: usize,
    current_source_references: usize,
    current_target_references: usize,
    in_flight_at_start: bool,
    currently_in_flight: bool,
    source: ProfileSecretMigrationDiagnosticProbe,
    target: ProfileSecretMigrationDiagnosticProbe,
    contents_match: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSecretMigrationDiagnosticPortableVault {
    pub(super) exists: Option<bool>,
    pub(super) unlocked: Option<bool>,
    pub(super) recovery_ready: Option<bool>,
    pub(super) error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileSecretMigrationDiagnosticJournal {
    valid: bool,
    migration_id: Option<String>,
    row_id_valid: bool,
    state: Option<ProfileSecretMigrationJournalState>,
    state_valid: bool,
    disposition: Option<ProfileSecretMigrationRecoveryDisposition>,
    target_storage: Option<SecretStorage>,
    cleanup_source: Option<bool>,
    selected_profile_ids: Vec<String>,
    payload_bytes: u64,
    created_at: Option<String>,
    updated_at: Option<String>,
    load_error: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfileSecretMigrationDiagnosticPlatform {
    os: &'static str,
    arch: &'static str,
    family: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProfileSecretMigrationDiagnosticReport {
    format: &'static str,
    version: u32,
    created_at: String,
    portmate_version: &'static str,
    store_schema_version: &'static str,
    platform: ProfileSecretMigrationDiagnosticPlatform,
    contains_secret_material: bool,
    journal: ProfileSecretMigrationDiagnosticJournal,
    portable_vault: ProfileSecretMigrationDiagnosticPortableVault,
    profiles: Vec<ProfileSecretMigrationDiagnosticProfile>,
    secrets: Vec<ProfileSecretMigrationDiagnosticSecret>,
    warnings: Vec<String>,
}

fn bounded_profile_secret_migration_diagnostic_error(error: &str) -> String {
    const MAX_ERROR_CHARS: usize = 2_048;
    let redacted = redact_secrets(error);
    let mut chars = redacted.chars();
    let mut bounded = chars.by_ref().take(MAX_ERROR_CHARS).collect::<String>();
    if chars.next().is_some() {
        bounded.push_str("...");
    }
    bounded
}

pub(super) fn profile_secret_migration_diagnostic_vault_status(
) -> ProfileSecretMigrationDiagnosticPortableVault {
    let (exists, unlocked, status_error) = match portable_vault_status_inner() {
        Ok(status) => (Some(status.exists), Some(status.unlocked), None),
        Err(error) => (None, None, Some(error)),
    };
    let (recovery_ready, recovery_error) = match portable_vault_recovery_ready() {
        Ok(recovery_ready) => (Some(recovery_ready), None),
        Err(error) => (None, Some(error)),
    };
    let errors = [status_error, recovery_error]
        .into_iter()
        .flatten()
        .map(|error| bounded_profile_secret_migration_diagnostic_error(&error))
        .collect::<Vec<_>>();
    ProfileSecretMigrationDiagnosticPortableVault {
        exists,
        unlocked,
        recovery_ready,
        error: (!errors.is_empty()).then(|| errors.join("; ")),
    }
}

fn profile_secret_migration_diagnostic_probe(
    probe: &SecretProbeResult,
) -> ProfileSecretMigrationDiagnosticProbe {
    match probe {
        SecretProbeResult::Present(_) => ProfileSecretMigrationDiagnosticProbe {
            status: ProfileSecretMigrationDiagnosticProbeStatus::Present,
            error: None,
        },
        SecretProbeResult::Missing => ProfileSecretMigrationDiagnosticProbe {
            status: ProfileSecretMigrationDiagnosticProbeStatus::Missing,
            error: None,
        },
        SecretProbeResult::Unavailable(error) => ProfileSecretMigrationDiagnosticProbe {
            status: ProfileSecretMigrationDiagnosticProbeStatus::Unavailable,
            error: Some(bounded_profile_secret_migration_diagnostic_error(error)),
        },
    }
}

fn profile_secret_migration_diagnostic_projection_status(
    current: &ProfileSecretMigrationJournalProjection,
    expected: &ProfileSecretMigrationJournalProfile,
) -> ProfileSecretMigrationDiagnosticProjectionStatus {
    if current == &expected.before {
        ProfileSecretMigrationDiagnosticProjectionStatus::Before
    } else if current == &expected.after {
        ProfileSecretMigrationDiagnosticProjectionStatus::After
    } else {
        ProfileSecretMigrationDiagnosticProjectionStatus::Conflict
    }
}

pub(super) fn build_profile_secret_migration_diagnostic_report<ProbeSecret>(
    store: &SessionStore,
    journal: &LoadedProfileSecretMigrationJournal,
    metadata: &ActiveProfileSecretMigrationJournalMetadata,
    mut probe_secret: ProbeSecret,
    portable_vault: ProfileSecretMigrationDiagnosticPortableVault,
) -> ProfileSecretMigrationDiagnosticReport
where
    ProbeSecret: FnMut(&str) -> SecretProbeResult,
{
    let profiles = journal
        .payload
        .profiles
        .iter()
        .map(|expected| {
            let current_profile = store
                .profiles
                .iter()
                .find(|profile| profile.id == expected.profile_id);
            let profile_name = current_profile.map(|profile| profile.name.clone());
            match current_profile {
                None => ProfileSecretMigrationDiagnosticProfile {
                    profile_id: expected.profile_id.clone(),
                    profile_name,
                    status: ProfileSecretMigrationDiagnosticProjectionStatus::Missing,
                    before: expected.before.clone(),
                    current: None,
                    after: expected.after.clone(),
                    projection_error: None,
                },
                Some(profile) => match profile_secret_migration_projection(profile) {
                    Ok(current) => ProfileSecretMigrationDiagnosticProfile {
                        profile_id: expected.profile_id.clone(),
                        profile_name,
                        status: profile_secret_migration_diagnostic_projection_status(
                            &current, expected,
                        ),
                        before: expected.before.clone(),
                        current: Some(current),
                        after: expected.after.clone(),
                        projection_error: None,
                    },
                    Err(error) => ProfileSecretMigrationDiagnosticProfile {
                        profile_id: expected.profile_id.clone(),
                        profile_name,
                        status: ProfileSecretMigrationDiagnosticProjectionStatus::Invalid,
                        before: expected.before.clone(),
                        current: None,
                        after: expected.after.clone(),
                        projection_error: Some(bounded_profile_secret_migration_diagnostic_error(
                            &error,
                        )),
                    },
                },
            }
        })
        .collect::<Vec<_>>();

    let secrets = journal
        .payload
        .items
        .iter()
        .map(|item| {
            let source = probe_secret(&item.source_ref);
            let target = probe_secret(&item.target_ref);
            let contents_match = match (&source, &target) {
                (SecretProbeResult::Present(source), SecretProbeResult::Present(target)) => {
                    Some(source.as_str() == target.as_str())
                }
                _ => None,
            };
            ProfileSecretMigrationDiagnosticSecret {
                source_ref: item.source_ref.clone(),
                target_ref: item.target_ref.clone(),
                source_storage: secret_ref_storage(&item.source_ref),
                target_storage: secret_ref_storage(&item.target_ref),
                expected_reference_count: item.reference_count,
                current_source_references: secret_ref_usage_count(store, &item.source_ref),
                current_target_references: secret_ref_usage_count(store, &item.target_ref),
                in_flight_at_start: item.in_flight_at_start,
                currently_in_flight: migration_source_ref_is_in_flight(
                    store,
                    &journal.payload,
                    &item.source_ref,
                ),
                source: profile_secret_migration_diagnostic_probe(&source),
                target: profile_secret_migration_diagnostic_probe(&target),
                contents_match,
            }
        })
        .collect::<Vec<_>>();

    ProfileSecretMigrationDiagnosticReport {
        format: "portmate-profile-secret-migration-diagnostic",
        version: 1,
        created_at: Utc::now().to_rfc3339(),
        portmate_version: env!("CARGO_PKG_VERSION"),
        store_schema_version: SQLITE_SCHEMA_VERSION,
        platform: ProfileSecretMigrationDiagnosticPlatform {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            family: std::env::consts::FAMILY,
        },
        contains_secret_material: false,
        journal: ProfileSecretMigrationDiagnosticJournal {
            valid: true,
            migration_id: Some(journal.payload.migration_id.clone()),
            row_id_valid: true,
            state: Some(journal.state),
            state_valid: true,
            disposition: Some(profile_secret_migration_disposition(store, journal)),
            target_storage: Some(journal.payload.target_storage),
            cleanup_source: Some(journal.payload.cleanup_source),
            selected_profile_ids: journal.payload.selected_profile_ids.clone(),
            payload_bytes: metadata.payload_bytes,
            created_at: Some(journal.created_at.to_rfc3339()),
            updated_at: Some(journal.updated_at.to_rfc3339()),
            load_error: None,
        },
        portable_vault,
        profiles,
        secrets,
        warnings: Vec::new(),
    }
}

fn build_corrupt_profile_secret_migration_diagnostic_report(
    metadata: &ActiveProfileSecretMigrationJournalMetadata,
    load_error: &str,
    portable_vault: ProfileSecretMigrationDiagnosticPortableVault,
) -> ProfileSecretMigrationDiagnosticReport {
    let migration_id = Uuid::parse_str(&metadata.row_id)
        .ok()
        .map(|value| value.to_string());
    let state = ProfileSecretMigrationJournalState::parse(&metadata.state).ok();
    ProfileSecretMigrationDiagnosticReport {
        format: "portmate-profile-secret-migration-diagnostic",
        version: 1,
        created_at: Utc::now().to_rfc3339(),
        portmate_version: env!("CARGO_PKG_VERSION"),
        store_schema_version: SQLITE_SCHEMA_VERSION,
        platform: ProfileSecretMigrationDiagnosticPlatform {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            family: std::env::consts::FAMILY,
        },
        contains_secret_material: false,
        journal: ProfileSecretMigrationDiagnosticJournal {
            valid: false,
            migration_id,
            row_id_valid: Uuid::parse_str(&metadata.row_id).is_ok(),
            state,
            state_valid: state.is_some(),
            disposition: None,
            target_storage: None,
            cleanup_source: None,
            selected_profile_ids: Vec::new(),
            payload_bytes: metadata.payload_bytes,
            created_at: parse_journal_timestamp(&metadata.created_at, "createdAt")
                .ok()
                .map(|value| value.to_rfc3339()),
            updated_at: parse_journal_timestamp(&metadata.updated_at, "updatedAt")
                .ok()
                .map(|value| value.to_rfc3339()),
            load_error: Some(bounded_profile_secret_migration_diagnostic_error(
                load_error,
            )),
        },
        portable_vault,
        profiles: Vec::new(),
        secrets: Vec::new(),
        warnings: vec![
            "journal payload 无法验证；导出仅包含受限行元数据，未包含原始 payload".to_string(),
        ],
    }
}

fn write_new_synced_profile_secret_migration_diagnostic_file(
    path: &Path,
    bytes: &[u8],
    label: &str,
) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(path)
        .map_err(|error| format!("无法创建{label} {}: {error}", path.display()))?;
    if let Err(error) = file
        .write_all(bytes)
        .and_then(|_| file.flush())
        .and_then(|_| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(format!("无法持久化{label} {}: {error}", path.display()));
    }
    Ok(())
}

fn write_profile_secret_migration_diagnostic_report(
    store_path: &Path,
    report: &ProfileSecretMigrationDiagnosticReport,
) -> Result<ProfileSecretMigrationDiagnosticExportResult, String> {
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| format!("无法编码凭据迁移诊断: {error}"))?;
    if bytes.len() > MAX_PROFILE_SECRET_MIGRATION_DIAGNOSTIC_BYTES {
        return Err(format!(
            "凭据迁移诊断超过 {} 字节限制",
            MAX_PROFILE_SECRET_MIGRATION_DIAGNOSTIC_BYTES
        ));
    }
    let export_dir = prepare_export_directory(store_path, "credential migration diagnostic")?;
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let migration_label = report
        .journal
        .migration_id
        .as_deref()
        .map(|migration_id| migration_id.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "invalid-journal".to_string());
    let nonce = &Uuid::new_v4().simple().to_string()[..8];
    let file_name = format!("credential-migration-{migration_label}-{timestamp}-{nonce}.json");
    let final_path = export_dir.join(&file_name);
    let temp_path = export_dir.join(format!(".{file_name}.part"));
    write_new_synced_profile_secret_migration_diagnostic_file(
        &temp_path,
        &bytes,
        "凭据迁移诊断临时文件",
    )?;
    if let Err(error) = fs::rename(&temp_path, &final_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "无法提交凭据迁移诊断 {}: {error}",
            final_path.display()
        ));
    }

    let sha256 = sha256_hex(&bytes);
    let checksum_path = final_path.with_extension("json.sha256");
    let checksum_temp_path = export_dir.join(format!(".{file_name}.sha256.part"));
    let checksum = format!("{sha256}  {file_name}\n");
    if let Err(error) = write_new_synced_profile_secret_migration_diagnostic_file(
        &checksum_temp_path,
        checksum.as_bytes(),
        "凭据迁移诊断校验文件",
    ) {
        let _ = fs::remove_file(&final_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&checksum_temp_path, &checksum_path) {
        let _ = fs::remove_file(&final_path);
        let _ = fs::remove_file(&checksum_temp_path);
        return Err(format!(
            "无法提交凭据迁移诊断校验文件 {}: {error}",
            checksum_path.display()
        ));
    }

    Ok(ProfileSecretMigrationDiagnosticExportResult {
        path: final_path.display().to_string(),
        checksum_path: checksum_path.display().to_string(),
        sha256,
        size: bytes.len() as u64,
        migration_id: report.journal.migration_id.clone(),
        journal_valid: report.journal.valid,
        warnings: report.warnings.clone(),
    })
}

pub(super) fn export_profile_secret_migration_diagnostics_with_io<ProbeSecret>(
    store_path: &Path,
    store: &SessionStore,
    probe_secret: ProbeSecret,
    portable_vault: ProfileSecretMigrationDiagnosticPortableVault,
) -> Result<ProfileSecretMigrationDiagnosticExportResult, String>
where
    ProbeSecret: FnMut(&str) -> SecretProbeResult,
{
    let connection = SqliteConnection::open(store_path)
        .map_err(|error| format!("无法打开 SQLite 导出凭据迁移诊断: {error}"))?;
    ensure_store_schema(&connection)?;
    let metadata = load_active_profile_secret_migration_journal_metadata(&connection)?
        .ok_or_else(|| "没有待诊断的凭据迁移恢复记录".to_string())?;
    let report = if metadata.payload_bytes > MAX_PROFILE_SECRET_MIGRATION_JOURNAL_BYTES {
        build_corrupt_profile_secret_migration_diagnostic_report(
            &metadata,
            "凭据迁移恢复记录超过大小限制",
            portable_vault,
        )
    } else {
        match load_profile_secret_migration_journal_from_connection(&connection) {
            Ok(Some(journal)) => build_profile_secret_migration_diagnostic_report(
                store,
                &journal,
                &metadata,
                probe_secret,
                portable_vault,
            ),
            Ok(None) => return Err("没有待诊断的凭据迁移恢复记录".to_string()),
            Err(error) => build_corrupt_profile_secret_migration_diagnostic_report(
                &metadata,
                &error,
                portable_vault,
            ),
        }
    };
    write_profile_secret_migration_diagnostic_report(store_path, &report)
}

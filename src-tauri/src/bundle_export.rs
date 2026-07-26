use super::*;

const MAX_BUNDLE_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_BUNDLE_ATTACHMENTS: usize = 32;
pub(super) const MAX_BUNDLE_ATTACHMENT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BUNDLE_ATTACHMENT_TOTAL_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleFileManifest {
    path: String,
    size: usize,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleAttachmentManifest {
    display_name: String,
    source_path: String,
    archive_path: String,
    size: usize,
    sha256: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleManifest {
    format: &'static str,
    version: u32,
    session_id: String,
    created_at: String,
    redacted: bool,
    raw_log_segments: usize,
    attachments: Vec<BundleAttachmentManifest>,
    files: Vec<BundleFileManifest>,
    warnings: Vec<String>,
}

pub(super) struct BundleArchiveEntry {
    path: String,
    bytes: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct PreparedBundleAttachment {
    display_name: String,
    source_path: String,
    archive_path: String,
    bytes: Vec<u8>,
}

pub(super) fn export_session_bundle_archive_inner(
    store_path: &Path,
    store: &SessionStore,
    request: ExportSessionBundleArchiveRequest,
    signing_key: &SigningKey,
) -> Result<ExportSessionBundleArchiveResult, String> {
    let profile = store
        .profile(&request.session_id)
        .ok_or_else(|| format!("unknown session: {}", request.session_id))?;
    let redacted = request.redact_secrets;
    let include_raw_logs = request.include_raw_logs && !redacted;
    if redacted && !request.attachment_paths.is_empty() {
        return Err(
            "bundle attachments are not redacted; disable redaction before attaching log shards"
                .to_string(),
        );
    }
    let prepared_attachments = prepare_bundle_attachments(store_path, &request.attachment_paths)?;
    let attachment_count = prepared_attachments.len();
    let created_at = Utc::now();
    let bundle = if redacted {
        store.export_session_bundle_redacted(&request.session_id)
    } else {
        store.export_session_bundle(&request.session_id)
    };
    let mut entries = Vec::new();
    let mut warnings = Vec::new();
    if request.include_raw_logs && redacted {
        warnings.push("raw log segments were omitted because redaction is enabled".to_string());
    }

    let bundle_bytes = serde_json::to_vec_pretty(&bundle)
        .map_err(|error| format!("failed to serialize session bundle: {error}"))?;
    push_bundle_entry(&mut entries, "bundle.json", bundle_bytes)?;

    let events = bundle
        .get("events")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut event_bytes = Vec::new();
    for event in &events {
        serde_json::to_writer(&mut event_bytes, event)
            .map_err(|error| format!("failed to serialize bundle event: {error}"))?;
        event_bytes.push(b'\n');
    }
    push_bundle_entry(&mut entries, "events.jsonl", event_bytes)?;

    let shard_inventory = if redacted {
        Vec::new()
    } else {
        match list_log_shards_inner(store_path) {
            Ok(shards) => shards,
            Err(error) => {
                warnings.push(format!("log shard inventory unavailable: {error}"));
                Vec::new()
            }
        }
    };
    let diagnostics = serde_json::json!({
        "createdAt": created_at.to_rfc3339(),
        "portmateVersion": env!("CARGO_PKG_VERSION"),
        "storeSchemaVersion": SQLITE_SCHEMA_VERSION,
        "platform": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "family": std::env::consts::FAMILY,
        },
        "session": {
            "id": profile.id,
            "name": profile.name,
            "kind": profile.kind,
        },
        "eventCount": events.len(),
        "availableLogShards": shard_inventory,
        "redacted": redacted,
        "rawLogsRequested": request.include_raw_logs,
        "rawLogsIncluded": include_raw_logs,
        "attachmentCount": attachment_count,
    });
    push_bundle_entry(
        &mut entries,
        "diagnostics.json",
        serde_json::to_vec_pretty(&diagnostics)
            .map_err(|error| format!("failed to serialize bundle diagnostics: {error}"))?,
    )?;

    let mut attachment_manifests = Vec::with_capacity(attachment_count);
    for attachment in prepared_attachments {
        let sha256 = sha256_hex(&attachment.bytes);
        attachment_manifests.push(BundleAttachmentManifest {
            display_name: attachment.display_name,
            source_path: attachment.source_path,
            archive_path: attachment.archive_path.clone(),
            size: attachment.bytes.len(),
            sha256,
        });
        push_bundle_entry(&mut entries, &attachment.archive_path, attachment.bytes)?;
    }

    let mut raw_log_segments = 0_usize;
    if include_raw_logs {
        let mut seen = HashSet::new();
        for reference in events
            .iter()
            .filter_map(|event| event.get("bytesRef").and_then(serde_json::Value::as_str))
        {
            if !seen.insert(reference.to_string()) {
                continue;
            }
            match read_log_bytes_ref(store_path, reference) {
                Ok((relative, offset, bytes)) => {
                    let file_name = Path::new(&relative)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("segment.raw");
                    let path = format!(
                        "log-segments/{raw_log_segments:04}-{}-{offset}-{}.bin",
                        sanitize_log_path_segment(file_name),
                        bytes.len()
                    );
                    push_bundle_entry(&mut entries, &path, bytes)?;
                    raw_log_segments += 1;
                }
                Err(error) => warnings.push(format!("{reference}: {error}")),
            }
        }
    }

    let manifest_files = entries
        .iter()
        .map(|entry| BundleFileManifest {
            path: entry.path.clone(),
            size: entry.bytes.len(),
            sha256: sha256_hex(&entry.bytes),
        })
        .collect::<Vec<_>>();
    let manifest = BundleManifest {
        format: "portmate-session-bundle",
        version: 2,
        session_id: request.session_id.clone(),
        created_at: created_at.to_rfc3339(),
        redacted,
        raw_log_segments,
        attachments: attachment_manifests,
        files: manifest_files,
        warnings: warnings.clone(),
    };
    push_bundle_entry(
        &mut entries,
        "manifest.json",
        serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("failed to serialize bundle manifest: {error}"))?,
    )?;

    let export_dir = prepare_export_directory(store_path, "session bundle")?;
    let timestamp = created_at.format("%Y%m%dT%H%M%SZ");
    let name = format!(
        "{}-{timestamp}-{}.tar.gz",
        sanitize_log_path_segment(&request.session_id),
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let final_path = export_dir.join(name);
    let temp_path = path_with_appended_suffix(&final_path, ".part")?;
    if let Err(error) =
        write_bundle_archive(&temp_path, &entries, created_at.timestamp().max(0) as u64)
    {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    let finalized = match finalize_signed_bundle_archive(
        &temp_path,
        &final_path,
        "session bundle",
        signing_key,
        &created_at.to_rfc3339(),
    ) {
        Ok(finalized) => finalized,
        Err(error) => {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
    };

    Ok(ExportSessionBundleArchiveResult {
        path: final_path.display().to_string(),
        checksum_path: finalized.checksum_path.display().to_string(),
        signature_path: finalized.signature_path.display().to_string(),
        sha256: finalized.sha256,
        signature_algorithm: "Ed25519".to_string(),
        signing_public_key: finalized.signing_public_key,
        size: finalized.size,
        files: entries.len(),
        raw_log_segments,
        attachments: attachment_count,
        redacted,
        warnings,
    })
}

fn push_bundle_entry(
    entries: &mut Vec<BundleArchiveEntry>,
    path: &str,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let current = entries.iter().map(|entry| entry.bytes.len()).sum::<usize>();
    if bytes.len() > MAX_BUNDLE_ARCHIVE_BYTES.saturating_sub(current) {
        return Err(format!(
            "session bundle uncompressed size limit exceeded ({MAX_BUNDLE_ARCHIVE_BYTES} bytes)"
        ));
    }
    entries.push(BundleArchiveEntry {
        path: path.to_string(),
        bytes,
    });
    Ok(())
}

pub(super) fn prepare_bundle_attachments(
    store_path: &Path,
    relative_paths: &[String],
) -> Result<Vec<PreparedBundleAttachment>, String> {
    if relative_paths.len() > MAX_BUNDLE_ATTACHMENTS {
        return Err(format!(
            "bundle attachment count limit exceeded ({MAX_BUNDLE_ATTACHMENTS})"
        ));
    }

    let mut seen = HashSet::new();
    let mut validated = Vec::new();
    let mut total_bytes = 0_u64;
    for relative in relative_paths {
        if !seen.insert(relative.clone()) {
            continue;
        }
        let path = resolve_log_shard_path(store_path, relative)?;
        let size = fs::metadata(&path)
            .map_err(|error| format!("failed to read bundle attachment {relative}: {error}"))?
            .len();
        if size > MAX_BUNDLE_ATTACHMENT_BYTES {
            return Err(format!(
                "bundle attachment {relative} exceeds {MAX_BUNDLE_ATTACHMENT_BYTES} byte limit"
            ));
        }
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or_else(|| "bundle attachment size overflow".to_string())?;
        if total_bytes > MAX_BUNDLE_ATTACHMENT_TOTAL_BYTES {
            return Err(format!(
                "bundle attachment total size limit exceeded ({MAX_BUNDLE_ATTACHMENT_TOTAL_BYTES} bytes)"
            ));
        }
        validated.push((relative.clone(), path, size));
    }

    validated
        .into_iter()
        .enumerate()
        .map(|(index, (source_path, path, size))| {
            let display_name = Path::new(&source_path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("attachment")
                .to_string();
            let sanitized = sanitize_log_path_segment(&display_name);
            let archive_name = if sanitized.is_empty() {
                "attachment".to_string()
            } else {
                sanitized
            };
            Ok(PreparedBundleAttachment {
                display_name,
                source_path,
                archive_path: format!("attachments/{:04}-{archive_name}", index + 1),
                bytes: read_verified_bundle_attachment(&path, size)?,
            })
        })
        .collect()
}

pub(super) fn read_verified_bundle_attachment(
    path: &Path,
    expected_size: u64,
) -> Result<Vec<u8>, String> {
    let path_lock = log_shard_lock(path)?;
    let _guard = path_lock
        .lock()
        .map_err(|_| format!("log shard lock poisoned: {}", path.display()))?;
    let file = open_bundle_attachment_file(path)?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "failed to inspect bundle attachment {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err(format!(
            "bundle attachment changed before it could be read: {}",
            path.display()
        ));
    }
    let capacity = usize::try_from(expected_size)
        .map_err(|_| "bundle attachment does not fit in memory".to_string())?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(expected_size.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            format!(
                "failed to read bundle attachment {}: {error}",
                path.display()
            )
        })?;
    if bytes.len() as u64 != expected_size {
        return Err(format!(
            "bundle attachment changed while it was being read: {}",
            path.display()
        ));
    }
    let (verified_sha256, verified_size) = sha256_file_exact(path, expected_size)?;
    if verified_size != expected_size || verified_sha256 != sha256_hex(&bytes) {
        return Err(format!(
            "bundle attachment was replaced or modified while it was being read: {}",
            path.display()
        ));
    }
    Ok(bytes)
}

pub(super) fn write_bundle_archive(
    path: &Path,
    entries: &[BundleArchiveEntry],
    modified_at: u64,
) -> Result<(), String> {
    let file = create_new_archive_file(path, "session bundle")?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = TarBuilder::new(encoder);
    for entry in entries {
        let mut header = TarHeader::new_gnu();
        header.set_size(entry.bytes.len() as u64);
        header.set_mode(0o600);
        header.set_mtime(modified_at);
        header.set_cksum();
        archive
            .append_data(&mut header, &entry.path, entry.bytes.as_slice())
            .map_err(|error| {
                format!("failed to append {} to session bundle: {error}", entry.path)
            })?;
    }
    archive
        .finish()
        .map_err(|error| format!("failed to finish session bundle tar stream: {error}"))?;
    let encoder = archive
        .into_inner()
        .map_err(|error| format!("failed to close session bundle tar stream: {error}"))?;
    let mut file = encoder
        .finish()
        .map_err(|error| format!("failed to finish session bundle compression: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush session bundle: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync session bundle: {error}"))
}

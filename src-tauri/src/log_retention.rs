use super::*;

pub(super) const LOG_RETENTION_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
const LOG_RETENTION_DATE_TOKEN: &str = "PORTMATE_RETENTION_DATE_5A8F";

pub(super) type LogRetentionChecks = Mutex<HashMap<(PathBuf, String), (u32, Instant)>>;

pub(super) static LOG_RETENTION_CHECKS: OnceLock<LogRetentionChecks> = OnceLock::new();

pub(super) fn validate_logging_retention(profile: &SessionProfile) -> Result<(), String> {
    if profile.logging.retention_days == 0 {
        return Ok(());
    }
    let template = profile.logging.path_template.as_str();
    if !template.contains("{session}") && !template.contains("{profile}") {
        return Err(
            "log retention requires {session} or {profile} in the path template".to_string(),
        );
    }
    Ok(())
}

pub(super) fn maybe_prune_expired_log_shards(
    store_path: &Path,
    profile: &SessionProfile,
) -> Result<(), String> {
    let retention_days = profile.logging.retention_days;
    if retention_days == 0 {
        clear_log_retention_check(store_path, &profile.id);
        return Ok(());
    }
    validate_logging_retention(profile)?;
    let key = (store_path.to_path_buf(), profile.id.clone());
    let checks = LOG_RETENTION_CHECKS.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let mut checks = checks
            .lock()
            .map_err(|error| format!("log retention lock poisoned: {error}"))?;
        checks.retain(|_, (_, checked)| checked.elapsed() < LOG_RETENTION_CHECK_INTERVAL);
        if checks
            .get(&key)
            .is_some_and(|(checked_days, _)| *checked_days == retention_days)
        {
            return Ok(());
        }
        checks.insert(key, (retention_days, Instant::now()));
    }
    prune_expired_log_shards_for_profile(store_path, profile, SystemTime::now()).map(|_| ())
}

pub(super) fn clear_log_retention_check(store_path: &Path, profile_id: &str) {
    if let Some(checks) = LOG_RETENTION_CHECKS.get() {
        checks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(store_path.to_path_buf(), profile_id.to_string()));
    }
}

pub(super) fn prune_expired_log_shards_for_profile(
    store_path: &Path,
    profile: &SessionProfile,
    now: SystemTime,
) -> Result<DeleteLogShardsResult, String> {
    if profile.logging.retention_days == 0 {
        return Ok(DeleteLogShardsResult {
            deleted: 0,
            bytes_deleted: 0,
        });
    }
    validate_logging_retention(profile)?;
    let retention = Duration::from_secs(u64::from(profile.logging.retention_days) * 86_400);
    let cutoff = now
        .checked_sub(retention)
        .ok_or_else(|| "log retention cutoff is outside the system clock range".to_string())?;
    let pattern = log_shard_relative_path(profile, LOG_RETENTION_DATE_TOKEN, "jsonl");
    let root = log_root(store_path);
    let mut candidates = Vec::new();
    for shard in list_log_shards_inner(store_path)? {
        let relative = Path::new(&shard.path);
        if !log_shard_matches_profile_pattern(relative, &pattern) {
            continue;
        }
        let path = resolve_log_shard_path(store_path, &shard.path)?;
        let metadata = fs::metadata(&path).map_err(|error| {
            format!(
                "failed to inspect retained log shard {}: {error}",
                path.display()
            )
        })?;
        if metadata.modified().is_ok_and(|modified| modified < cutoff) {
            candidates.push((path, metadata.len()));
        }
    }

    if candidates.is_empty() {
        return Ok(DeleteLogShardsResult {
            deleted: 0,
            bytes_deleted: 0,
        });
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("failed to resolve log root {}: {error}", root.display()))?;
    let mut deleted = 0_usize;
    let mut bytes_deleted = 0_u64;
    for (path, _) in candidates {
        let path_lock = log_shard_lock(&path)?;
        let _guard = path_lock
            .lock()
            .map_err(|_| format!("log shard lock poisoned: {}", path.display()))?;
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "failed to recheck retained log shard {}: {error}",
                    path.display()
                ));
            }
        };
        if !metadata.modified().is_ok_and(|modified| modified < cutoff) {
            continue;
        }
        fs::remove_file(&path).map_err(|error| {
            format!(
                "failed to delete expired log shard {}: {error}",
                path.display()
            )
        })?;
        deleted += 1;
        bytes_deleted = bytes_deleted.saturating_add(metadata.len());
        let mut parent = path.parent();
        while let Some(directory) = parent {
            if directory == canonical_root || !directory.starts_with(&canonical_root) {
                break;
            }
            if fs::remove_dir(directory).is_err() {
                break;
            }
            parent = directory.parent();
        }
    }
    Ok(DeleteLogShardsResult {
        deleted,
        bytes_deleted,
    })
}

fn log_shard_matches_profile_pattern(relative: &Path, pattern: &Path) -> bool {
    let mut normalized = relative.to_path_buf();
    normalized.set_extension("jsonl");
    let actual = normalized
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    let expected = pattern
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    actual.len() == expected.len()
        && actual
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| log_path_component_matches(actual, expected))
}

fn log_path_component_matches(actual: &str, pattern: &str) -> bool {
    let Some((prefix, suffix)) = pattern.split_once(LOG_RETENTION_DATE_TOKEN) else {
        return actual == pattern;
    };
    if suffix.contains(LOG_RETENTION_DATE_TOKEN)
        || !actual.starts_with(prefix)
        || !actual.ends_with(suffix)
        || actual.len() < prefix.len() + suffix.len()
    {
        return false;
    }
    let date = &actual[prefix.len()..actual.len() - suffix.len()];
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok()
}

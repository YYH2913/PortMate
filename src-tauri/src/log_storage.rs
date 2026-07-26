use super::*;

const MAX_LOG_SHARDS: usize = 10_000;
const MAX_LOG_SCAN_ENTRIES: usize = 50_000;
const MAX_LOG_DELETE_BATCH: usize = 1_000;
const DEFAULT_LOG_PREVIEW_BYTES: u64 = 64 * 1024;
const MAX_LOG_PREVIEW_BYTES: u64 = 1024 * 1024;
const DEFAULT_LOG_SHARD_SEARCH_LIMIT: u64 = 100;
const MAX_LOG_SHARD_SEARCH_LIMIT: u64 = 500;
const MAX_LOG_SHARD_SEARCH_PATHS: usize = 1_000;
const MAX_LOG_SHARD_SEARCH_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_LOG_SHARD_SEARCH_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_LOG_ARCHIVE_PATHS: usize = 1_000;
const MAX_LOG_ARCHIVE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_LOG_SEGMENT_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const LOG_RETENTION_CHECK_INTERVAL: Duration = Duration::from_secs(60 * 60);
const LOG_RETENTION_DATE_TOKEN: &str = "PORTMATE_RETENTION_DATE_5A8F";

pub(super) type LogRetentionChecks = Mutex<HashMap<(PathBuf, String), (u32, Instant)>>;
type LogShardLocks = Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>;

pub(super) static LOG_RETENTION_CHECKS: OnceLock<LogRetentionChecks> = OnceLock::new();
static LOG_SHARD_LOCKS: OnceLock<LogShardLocks> = OnceLock::new();

pub(super) fn append_log_bytes(
    store_path: &Path,
    profile: &SessionProfile,
    extension: &str,
    bytes: &[u8],
) -> Result<String, String> {
    if let Err(error) = maybe_prune_expired_log_shards(store_path, profile) {
        eprintln!("PortMate: automatic log retention failed: {error}");
    }
    let path = log_shard_path(store_path, profile, extension)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create log dir {}: {error}", parent.display()))?;
    }
    validate_log_shard_write_path(store_path, &path)?;
    let path_lock = log_shard_lock(&path)?;
    let _guard = path_lock
        .lock()
        .map_err(|_| format!("log shard lock poisoned: {}", path.display()))?;
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true).read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(&path)
        .map_err(|error| format!("failed to open log shard {}: {error}", path.display()))?;
    let offset = file
        .seek(std::io::SeekFrom::End(0))
        .map_err(|error| format!("failed to seek log shard {}: {error}", path.display()))?;
    file.write_all(bytes)
        .map_err(|error| format!("failed to append log shard {}: {error}", path.display()))?;
    file.flush()
        .map_err(|error| format!("failed to flush log shard {}: {error}", path.display()))?;
    let relative = path
        .strip_prefix(log_root(store_path))
        .unwrap_or(path.as_path())
        .display()
        .to_string();
    Ok(format!(
        "v2:{relative}:{offset}:{}:{}",
        bytes.len(),
        sha256_hex(bytes)
    ))
}

fn validate_log_shard_write_path(store_path: &Path, path: &Path) -> Result<(), String> {
    let root = log_root(store_path);
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("failed to resolve log root {}: {error}", root.display()))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("log shard has no parent directory: {}", path.display()))?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        format!(
            "failed to resolve log shard parent {}: {error}",
            parent.display()
        )
    })?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(format!("log shard escaped log root: {}", path.display()));
    }

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "log shard target is a symbolic link: {}",
            path.display()
        )),
        Ok(metadata) if !metadata.is_file() => Err(format!(
            "log shard target is not a regular file: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect log shard {}: {error}",
            path.display()
        )),
    }
}

pub(super) fn log_shard_lock(path: &Path) -> Result<Arc<Mutex<()>>, String> {
    let mut locks = LOG_SHARD_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "log shard lock registry poisoned".to_string())?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
    Ok(lock)
}

pub(super) fn log_shard_path(
    store_path: &Path,
    profile: &SessionProfile,
    extension: &str,
) -> Result<PathBuf, String> {
    let date = Utc::now().format("%Y-%m-%d").to_string();
    Ok(log_root(store_path).join(log_shard_relative_path(profile, &date, extension)))
}

pub(super) fn log_shard_relative_path(
    profile: &SessionProfile,
    date: &str,
    extension: &str,
) -> PathBuf {
    let template = profile
        .logging
        .path_template
        .trim()
        .trim_start_matches('/')
        .trim_start_matches('\\');
    let template = if template.is_empty() {
        "{profile}/{date}/{session}.jsonl"
    } else {
        template
    };
    let rendered = template
        .replace("{profile}", &profile.name)
        .replace("{group}", &profile.group)
        .replace("{session}", &profile.id)
        .replace("{date}", date);

    let mut path = PathBuf::new();
    for segment in rendered.replace('\\', "/").split('/') {
        let clean = sanitize_log_path_segment(segment);
        if !clean.is_empty() && clean != "." && clean != ".." {
            path.push(clean);
        }
    }
    if path.as_os_str().is_empty() {
        path.push(sanitize_log_path_segment(&profile.id));
    }
    path.set_extension(extension);
    path
}

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

pub(super) fn log_root(store_path: &Path) -> PathBuf {
    store_path
        .parent()
        .map(|parent| parent.join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
}

fn is_log_shard_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "raw" | "txt" | "jsonl"
            )
        })
}

fn relative_log_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| format!("log shard escaped log root: {}", path.display()))?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

pub(super) fn list_log_shards_inner(store_path: &Path) -> Result<Vec<LogShardInfo>, String> {
    let root = log_root(store_path);
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut pending = vec![root.clone()];
    let mut shards = Vec::new();
    let mut scanned = 0_usize;
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).map_err(|error| {
            format!(
                "failed to read log directory {}: {error}",
                directory.display()
            )
        })?;
        for entry in entries {
            scanned += 1;
            if scanned > MAX_LOG_SCAN_ENTRIES {
                return Err(format!(
                    "log directory entry limit exceeded ({MAX_LOG_SCAN_ENTRIES})"
                ));
            }
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read log directory entry in {}: {error}",
                    directory.display()
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                format!(
                    "failed to inspect log path {}: {error}",
                    entry.path().display()
                )
            })?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                pending.push(entry.path());
                continue;
            }
            let path = entry.path();
            if !file_type.is_file() || !is_log_shard_path(&path) {
                continue;
            }
            if shards.len() >= MAX_LOG_SHARDS {
                return Err(format!("log shard limit exceeded ({MAX_LOG_SHARDS})"));
            }
            let metadata = entry.metadata().map_err(|error| {
                format!("failed to read log metadata {}: {error}", path.display())
            })?;
            let modified_at = metadata
                .modified()
                .ok()
                .map(chrono::DateTime::<Utc>::from)
                .map(|value| value.to_rfc3339());
            shards.push(LogShardInfo {
                path: relative_log_path(&root, &path)?,
                format: path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                size: metadata.len(),
                modified_at,
            });
        }
    }
    shards.sort_by(|left, right| {
        right
            .modified_at
            .cmp(&left.modified_at)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(shards)
}

pub(super) fn resolve_log_shard_path(store_path: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = relative.trim();
    if relative.is_empty() || relative.contains('\0') || relative.contains('\\') {
        return Err("invalid log shard path".to_string());
    }
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || !is_log_shard_path(relative_path)
    {
        return Err(format!("invalid log shard path: {relative}"));
    }

    let root = log_root(store_path);
    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("failed to resolve log root {}: {error}", root.display()))?;
    let candidate = root.join(relative_path);
    let metadata = fs::symlink_metadata(&candidate)
        .map_err(|error| format!("log shard not found {relative}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("log shard is not a regular file: {relative}"));
    }
    let canonical = candidate
        .canonicalize()
        .map_err(|error| format!("failed to resolve log shard {relative}: {error}"))?;
    if !canonical.starts_with(&canonical_root) {
        return Err(format!("log shard escaped log root: {relative}"));
    }
    Ok(canonical)
}

pub(super) fn read_log_shard_inner(
    store_path: &Path,
    relative: &str,
    max_bytes: Option<u64>,
) -> Result<LogShardPreview, String> {
    let path = resolve_log_shard_path(store_path, relative)?;
    let size = fs::metadata(&path)
        .map_err(|error| format!("failed to read log metadata {relative}: {error}"))?
        .len();
    let max_bytes = max_bytes
        .unwrap_or(DEFAULT_LOG_PREVIEW_BYTES)
        .clamp(64, MAX_LOG_PREVIEW_BYTES);
    let offset = size.saturating_sub(max_bytes);
    let mut file = fs::File::open(&path)
        .map_err(|error| format!("failed to open log shard {relative}: {error}"))?;
    file.seek(std::io::SeekFrom::Start(offset))
        .map_err(|error| format!("failed to seek log shard {relative}: {error}"))?;
    let mut bytes = Vec::with_capacity((size - offset).min(max_bytes) as usize);
    file.take(max_bytes)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read log shard {relative}: {error}"))?;
    let raw = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("raw"));
    let (encoding, content) = if !raw && std::str::from_utf8(&bytes).is_ok() {
        (
            "utf8".to_string(),
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    } else {
        ("hex".to_string(), format_log_hex(&bytes, offset))
    };
    Ok(LogShardPreview {
        path: relative.to_string(),
        content,
        encoding,
        bytes_read: bytes.len() as u64,
        truncated: offset > 0,
    })
}

fn format_log_hex(bytes: &[u8], offset: u64) -> String {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(line, chunk)| {
            let hex = chunk
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let text = chunk
                .iter()
                .map(|byte| {
                    if byte.is_ascii_graphic() || *byte == b' ' {
                        char::from(*byte)
                    } else {
                        '.'
                    }
                })
                .collect::<String>();
            format!(
                "{:08X}  {:<47}  |{}|",
                offset + (line * 16) as u64,
                hex,
                text
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn delete_log_shards_inner(
    store_path: &Path,
    relative_paths: &[String],
) -> Result<DeleteLogShardsResult, String> {
    if relative_paths.is_empty() {
        return Err("select at least one log shard".to_string());
    }
    if relative_paths.len() > MAX_LOG_DELETE_BATCH {
        return Err(format!(
            "log delete batch limit exceeded ({MAX_LOG_DELETE_BATCH})"
        ));
    }
    let mut unique = HashSet::new();
    let mut validated = Vec::new();
    for relative in relative_paths {
        if unique.insert(relative.clone()) {
            let path = resolve_log_shard_path(store_path, relative)?;
            let size = fs::metadata(&path)
                .map_err(|error| format!("failed to read log metadata {relative}: {error}"))?
                .len();
            validated.push((path, size));
        }
    }

    let root = log_root(store_path)
        .canonicalize()
        .map_err(|error| format!("failed to resolve log root: {error}"))?;
    let mut bytes_deleted = 0;
    for (path, _) in &validated {
        let path_lock = log_shard_lock(path)?;
        let _guard = path_lock
            .lock()
            .map_err(|_| format!("log shard lock poisoned: {}", path.display()))?;
        let size = fs::metadata(path)
            .map_err(|error| format!("failed to read log metadata {}: {error}", path.display()))?
            .len();
        fs::remove_file(path)
            .map_err(|error| format!("failed to delete log shard {}: {error}", path.display()))?;
        bytes_deleted += size;
        let mut parent = path.parent();
        while let Some(directory) = parent {
            if directory == root || !directory.starts_with(&root) {
                break;
            }
            if fs::remove_dir(directory).is_err() {
                break;
            }
            parent = directory.parent();
        }
    }
    Ok(DeleteLogShardsResult {
        deleted: validated.len(),
        bytes_deleted,
    })
}

pub(super) fn search_log_shards_inner(
    store_path: &Path,
    request: SearchLogShardsRequest,
) -> Result<SearchLogShardsResult, String> {
    let query = request.query.trim();
    if query.is_empty() {
        return Err("log shard search query cannot be empty".to_string());
    }
    if query.chars().count() > 256 {
        return Err("log shard search query exceeds 256 characters".to_string());
    }
    if request.paths.len() > MAX_LOG_SHARD_SEARCH_PATHS {
        return Err(format!(
            "log shard search path limit exceeded ({MAX_LOG_SHARD_SEARCH_PATHS})"
        ));
    }
    let limit = request
        .limit
        .unwrap_or(DEFAULT_LOG_SHARD_SEARCH_LIMIT)
        .clamp(1, MAX_LOG_SHARD_SEARCH_LIMIT) as usize;
    let normalized_query = query.to_lowercase();
    let inventory = list_log_shards_inner(store_path)?;
    let inventory_by_path = inventory
        .iter()
        .map(|shard| (shard.path.as_str(), shard))
        .collect::<HashMap<_, _>>();
    let mut warnings = Vec::new();
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    if request.paths.is_empty() {
        selected.extend(
            inventory
                .iter()
                .filter(|shard| matches!(shard.format.as_str(), "txt" | "jsonl")),
        );
    } else {
        for path in &request.paths {
            if !seen.insert(path.as_str()) {
                continue;
            }
            let shard = inventory_by_path
                .get(path.as_str())
                .copied()
                .ok_or_else(|| format!("log shard not found or unsupported: {path}"))?;
            if !matches!(shard.format.as_str(), "txt" | "jsonl") {
                warnings.push(format!("{path}: raw shards are not text-searched"));
                continue;
            }
            selected.push(shard);
        }
    }

    let mut matches = Vec::new();
    let mut files_scanned = 0_usize;
    let mut bytes_scanned = 0_u64;
    let mut truncated = false;
    'files: for shard in selected {
        if shard.size > MAX_LOG_SHARD_SEARCH_FILE_BYTES {
            warnings.push(format!(
                "{}: file exceeds {} byte search limit",
                shard.path, MAX_LOG_SHARD_SEARCH_FILE_BYTES
            ));
            truncated = true;
            continue;
        }
        if shard.size > MAX_LOG_SHARD_SEARCH_TOTAL_BYTES.saturating_sub(bytes_scanned) {
            warnings.push(format!(
                "search stopped at {} byte total scan limit",
                MAX_LOG_SHARD_SEARCH_TOTAL_BYTES
            ));
            truncated = true;
            break;
        }
        let path = resolve_log_shard_path(store_path, &shard.path)?;
        let file = fs::File::open(&path)
            .map_err(|error| format!("failed to open log shard {}: {error}", shard.path))?;
        let mut reader = BufReader::new(file);
        files_scanned += 1;
        let mut buffer = Vec::new();
        let mut line = 0_u64;
        let mut byte_offset = 0_u64;
        loop {
            buffer.clear();
            let read = reader
                .read_until(b'\n', &mut buffer)
                .map_err(|error| format!("failed to search log shard {}: {error}", shard.path))?;
            if read == 0 {
                break;
            }
            line += 1;
            bytes_scanned += read as u64;
            let text = String::from_utf8_lossy(&buffer);
            if text.to_lowercase().contains(&normalized_query) {
                matches.push(LogShardSearchMatch {
                    path: shard.path.clone(),
                    format: shard.format.clone(),
                    line,
                    byte_offset,
                    text: truncate_for_log(&text, 600),
                });
                if matches.len() >= limit {
                    truncated = true;
                    break 'files;
                }
            }
            byte_offset += read as u64;
        }
    }

    Ok(SearchLogShardsResult {
        matches,
        files_scanned,
        bytes_scanned,
        truncated,
        warnings,
    })
}

pub(super) fn archive_log_shards_inner(
    store_path: &Path,
    request: ArchiveLogShardsRequest,
) -> Result<ArchiveLogShardsResult, String> {
    if request.paths.is_empty() {
        return Err("select at least one log shard to archive".to_string());
    }
    if request.paths.len() > MAX_LOG_ARCHIVE_PATHS {
        return Err(format!(
            "log archive path limit exceeded ({MAX_LOG_ARCHIVE_PATHS})"
        ));
    }
    let mut seen = HashSet::new();
    let mut validated = Vec::new();
    let mut source_bytes = 0_u64;
    for relative in &request.paths {
        if !seen.insert(relative.clone()) {
            continue;
        }
        let path = resolve_log_shard_path(store_path, relative)?;
        let size = fs::metadata(&path)
            .map_err(|error| format!("failed to read log metadata {relative}: {error}"))?
            .len();
        source_bytes = source_bytes
            .checked_add(size)
            .ok_or_else(|| "log archive size overflow".to_string())?;
        if source_bytes > MAX_LOG_ARCHIVE_TOTAL_BYTES {
            return Err(format!(
                "log archive source size limit exceeded ({MAX_LOG_ARCHIVE_TOTAL_BYTES} bytes)"
            ));
        }
        validated.push((relative.clone(), path, size));
    }

    let created_at = Utc::now();
    let export_dir = prepare_export_directory(store_path, "log archive")?;
    let timestamp = created_at.format("%Y%m%dT%H%M%SZ");
    let name = format!(
        "portmate-logs-{timestamp}-{}.tar.gz",
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let final_path = export_dir.join(name);
    let temp_path = path_with_appended_suffix(&final_path, ".part")?;
    if let Err(error) = write_log_shard_archive(
        &temp_path,
        &validated,
        created_at.timestamp().max(0) as u64,
        &created_at.to_rfc3339(),
    ) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    let finalized = finalize_archive_with_checksum(&temp_path, &final_path, "log archive")?;
    Ok(ArchiveLogShardsResult {
        path: final_path.display().to_string(),
        checksum_path: finalized.checksum_path.display().to_string(),
        sha256: finalized.sha256,
        size: finalized.size,
        shards: validated.len(),
        source_bytes,
    })
}

pub(super) fn read_log_bytes_ref(
    store_path: &Path,
    reference: &str,
) -> Result<(String, u64, Vec<u8>), String> {
    let parsed = parse_log_bytes_ref(reference)?;
    read_parsed_log_bytes_ref(store_path, parsed)
}

pub(super) fn read_verified_log_bytes_ref(
    store_path: &Path,
    reference: &str,
) -> Result<(String, u64, Vec<u8>), String> {
    let parsed = parse_log_bytes_ref(reference)?;
    if parsed.sha256.is_none() {
        return Err("bytesRef does not include a SHA-256 digest".to_string());
    }
    read_parsed_log_bytes_ref(store_path, parsed)
}

fn read_parsed_log_bytes_ref(
    store_path: &Path,
    parsed: ParsedLogBytesRef,
) -> Result<(String, u64, Vec<u8>), String> {
    if parsed.length > MAX_LOG_SEGMENT_BYTES {
        return Err(format!(
            "log segment exceeds {MAX_LOG_SEGMENT_BYTES} byte limit"
        ));
    }
    let path = resolve_log_shard_path(store_path, &parsed.relative)?;
    let path_lock = log_shard_lock(&path)?;
    let _guard = path_lock
        .lock()
        .map_err(|_| format!("log shard lock poisoned: {}", path.display()))?;
    let size = fs::metadata(&path)
        .map_err(|error| format!("failed to read referenced log metadata: {error}"))?
        .len();
    let end = parsed
        .offset
        .checked_add(parsed.length)
        .ok_or_else(|| "bytesRef range overflow".to_string())?;
    if end > size {
        return Err(format!(
            "bytesRef range {}..{end} exceeds shard size {size}",
            parsed.offset
        ));
    }
    let mut file = fs::File::open(&path)
        .map_err(|error| format!("failed to open referenced log shard: {error}"))?;
    file.seek(std::io::SeekFrom::Start(parsed.offset))
        .map_err(|error| format!("failed to seek referenced log shard: {error}"))?;
    let mut bytes = vec![0_u8; parsed.length as usize];
    file.read_exact(&mut bytes)
        .map_err(|error| format!("failed to read referenced log segment: {error}"))?;
    if parsed
        .sha256
        .as_deref()
        .is_some_and(|expected| expected != sha256_hex(&bytes))
    {
        return Err("bytesRef content mismatch: log shard was replaced or modified".to_string());
    }
    Ok((parsed.relative, parsed.offset, bytes))
}

pub(super) struct ParsedLogBytesRef {
    pub(super) relative: String,
    pub(super) offset: u64,
    pub(super) length: u64,
    pub(super) sha256: Option<String>,
}

pub(super) fn parse_log_bytes_ref(reference: &str) -> Result<ParsedLogBytesRef, String> {
    if let Some(reference) = reference.strip_prefix("v2:") {
        let mut parts = reference.rsplitn(4, ':');
        let sha256 = parts.next().filter(|value| {
            value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
        });
        let length = parts.next().and_then(|value| value.parse::<u64>().ok());
        let offset = parts.next().and_then(|value| value.parse::<u64>().ok());
        let relative = parts.next().filter(|value| !value.is_empty());
        if let (Some(sha256), Some(length), Some(offset), Some(relative)) =
            (sha256, length, offset, relative)
        {
            return Ok(ParsedLogBytesRef {
                relative: relative.to_string(),
                offset,
                length,
                sha256: Some(sha256.to_ascii_lowercase()),
            });
        }
    }

    let mut parts = reference.rsplitn(3, ':');
    let length = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| "invalid bytesRef length".to_string())?;
    let offset = parts
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| "invalid bytesRef offset".to_string())?;
    let relative = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "invalid bytesRef path".to_string())?;
    Ok(ParsedLogBytesRef {
        relative: relative.to_string(),
        offset,
        length,
        sha256: None,
    })
}

use super::*;

const MAX_LOG_DELETE_BATCH: usize = 1_000;
const MAX_LOG_ARCHIVE_PATHS: usize = 1_000;
const MAX_LOG_ARCHIVE_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
type LogShardLocks = Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>;

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

pub(super) fn log_root(store_path: &Path) -> PathBuf {
    store_path
        .parent()
        .map(|parent| parent.join("logs"))
        .unwrap_or_else(|| PathBuf::from("logs"))
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

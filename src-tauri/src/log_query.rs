use super::*;

const MAX_LOG_SHARDS: usize = 10_000;
const MAX_LOG_SCAN_ENTRIES: usize = 50_000;
const DEFAULT_LOG_PREVIEW_BYTES: u64 = 64 * 1024;
const MAX_LOG_PREVIEW_BYTES: u64 = 1024 * 1024;
const DEFAULT_LOG_SHARD_SEARCH_LIMIT: u64 = 100;
const MAX_LOG_SHARD_SEARCH_LIMIT: u64 = 500;
const MAX_LOG_SHARD_SEARCH_PATHS: usize = 1_000;
const MAX_LOG_SHARD_SEARCH_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_LOG_SHARD_SEARCH_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

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

use super::*;

const MAX_LOG_SEGMENT_BYTES: u64 = 4 * 1024 * 1024;

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

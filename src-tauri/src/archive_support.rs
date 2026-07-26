use super::*;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveFileManifest {
    path: String,
    size: usize,
    sha256: String,
}

pub(super) struct FinalizedArchive {
    pub(super) checksum_path: PathBuf,
    pub(super) sha256: String,
    pub(super) size: u64,
}

struct HashingReader<R> {
    inner: R,
    digest: Sha256,
    bytes_read: u64,
}

impl<R> HashingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            digest: Sha256::new(),
            bytes_read: 0,
        }
    }

    fn finish(self) -> (String, u64) {
        (format!("{:x}", self.digest.finalize()), self.bytes_read)
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.digest.update(&buffer[..read]);
        self.bytes_read += read as u64;
        Ok(read)
    }
}

pub(super) fn prepare_export_directory(store_path: &Path, label: &str) -> Result<PathBuf, String> {
    let parent = store_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| {
        format!(
            "failed to create {label} export parent directory {}: {error}",
            parent.display()
        )
    })?;
    let export_dir = parent.join("exports");
    match fs::symlink_metadata(&export_dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Err(error) = fs::create_dir(&export_dir) {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(format!(
                        "failed to create {label} export directory {}: {error}",
                        export_dir.display()
                    ));
                }
            }
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect {label} export directory {}: {error}",
                export_dir.display()
            ));
        }
    }
    let metadata = fs::symlink_metadata(&export_dir).map_err(|error| {
        format!(
            "failed to inspect {label} export directory {}: {error}",
            export_dir.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "{label} export directory must not be a symbolic link: {}",
            export_dir.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "{label} export path is not a directory: {}",
            export_dir.display()
        ));
    }
    Ok(export_dir)
}

pub(super) fn write_atomic_export_with_checksum(
    final_path: &Path,
    bytes: &[u8],
    label: &str,
) -> Result<FinalizedArchive, String> {
    let file_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid {label} path"))?;
    let temp_path = final_path.with_file_name(format!(".{file_name}.part"));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(&temp_path)
        .map_err(|error| format!("failed to create {label} {}: {error}", temp_path.display()))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!("failed to write {label}: {error}"));
    }
    drop(file);
    if let Err(error) = fs::rename(&temp_path, final_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "failed to finalize {label} {}: {error}",
            final_path.display()
        ));
    }
    let sha256 = match sha256_file(final_path) {
        Ok(sha256) => sha256,
        Err(error) => {
            let _ = fs::remove_file(final_path);
            return Err(error);
        }
    };
    let size = match fs::metadata(final_path) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            let _ = fs::remove_file(final_path);
            return Err(format!("failed to read {label} metadata: {error}"));
        }
    };
    let checksum_path = final_path.with_file_name(format!("{file_name}.sha256"));
    let checksum_temp_path = final_path.with_file_name(format!(".{file_name}.sha256.part"));
    let mut checksum_options = OpenOptions::new();
    checksum_options.create_new(true).write(true);
    #[cfg(unix)]
    checksum_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let mut checksum = match checksum_options.open(&checksum_temp_path) {
        Ok(checksum) => checksum,
        Err(error) => {
            let _ = fs::remove_file(final_path);
            return Err(format!("failed to create {label} checksum: {error}"));
        }
    };
    if let Err(error) = checksum
        .write_all(format!("{sha256}  {file_name}\n").as_bytes())
        .and_then(|_| checksum.sync_all())
    {
        let _ = fs::remove_file(final_path);
        let _ = fs::remove_file(&checksum_temp_path);
        return Err(format!("failed to write {label} checksum: {error}"));
    }
    drop(checksum);
    if let Err(error) = fs::rename(&checksum_temp_path, &checksum_path) {
        let _ = fs::remove_file(final_path);
        let _ = fs::remove_file(&checksum_temp_path);
        return Err(format!("failed to finalize {label} checksum: {error}"));
    }
    Ok(FinalizedArchive {
        checksum_path,
        sha256,
        size,
    })
}

pub(super) fn open_bundle_attachment_file(path: &Path) -> Result<fs::File, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect bundle attachment {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "bundle attachment is not a regular file: {}",
            path.display()
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options.open(path).map_err(|error| {
        format!(
            "failed to open bundle attachment {}: {error}",
            path.display()
        )
    })
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(super) fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path)
        .map_err(|error| format!("failed to open bundle for checksum: {error}"))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to read bundle for checksum: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

pub(super) fn sha256_file_exact(path: &Path, expected_size: u64) -> Result<(String, u64), String> {
    let file = open_bundle_attachment_file(path)?;
    let mut reader = HashingReader::new(file.take(expected_size.saturating_add(1)));
    std::io::copy(&mut reader, &mut std::io::sink())
        .map_err(|error| format!("failed to read file for bounded checksum: {error}"))?;
    let result = reader.finish();
    if result.1 != expected_size {
        return Err(format!(
            "file changed during checksum: read {} of {expected_size} bytes",
            result.1
        ));
    }
    Ok(result)
}

pub(super) fn path_with_appended_suffix(path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("path has no file name: {}", path.display()))?;
    let mut suffixed = file_name.to_os_string();
    suffixed.push(suffix);
    Ok(path.with_file_name(suffixed))
}

pub(super) fn write_new_synced_file(path: &Path, bytes: &[u8], label: &str) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    let mut file = options
        .open(path)
        .map_err(|error| format!("failed to create {label} {}: {error}", path.display()))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("failed to write {label} {}: {error}", path.display()))
}

pub(super) fn finalize_archive_with_checksum(
    temp_path: &Path,
    final_path: &Path,
    label: &str,
) -> Result<FinalizedArchive, String> {
    let checksum_path = path_with_appended_suffix(final_path, ".sha256")?;
    let checksum_temp_path = path_with_appended_suffix(final_path, ".sha256.part")?;
    for artifact in [
        final_path,
        checksum_path.as_path(),
        checksum_temp_path.as_path(),
    ] {
        if fs::symlink_metadata(artifact).is_ok() {
            let _ = fs::remove_file(temp_path);
            return Err(format!(
                "refusing to overwrite existing {label} artifact {}",
                artifact.display()
            ));
        }
    }
    fs::rename(temp_path, final_path).map_err(|error| {
        let _ = fs::remove_file(temp_path);
        format!(
            "failed to finalize {label} {}: {error}",
            final_path.display()
        )
    })?;
    let sha256 = match sha256_file(final_path) {
        Ok(sha256) => sha256,
        Err(error) => {
            let _ = fs::remove_file(final_path);
            return Err(error);
        }
    };
    let size = match fs::metadata(final_path) {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            let _ = fs::remove_file(final_path);
            return Err(format!("failed to read {label} metadata: {error}"));
        }
    };
    let archive_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("portmate-archive.tar.gz");
    if let Err(error) = write_new_synced_file(
        &checksum_temp_path,
        format!("{sha256}  {archive_name}\n").as_bytes(),
        &format!("{label} checksum"),
    ) {
        let _ = fs::remove_file(final_path);
        let _ = fs::remove_file(&checksum_temp_path);
        return Err(format!(
            "failed to write {label} checksum {}: {error}",
            checksum_path.display()
        ));
    }
    if let Err(error) = fs::rename(&checksum_temp_path, &checksum_path) {
        let _ = fs::remove_file(final_path);
        let _ = fs::remove_file(&checksum_temp_path);
        return Err(format!(
            "failed to finalize {label} checksum {}: {error}",
            checksum_path.display()
        ));
    }
    Ok(FinalizedArchive {
        checksum_path,
        sha256,
        size,
    })
}

pub(super) fn write_log_shard_archive(
    path: &Path,
    shards: &[(String, PathBuf, u64)],
    modified_at: u64,
    created_at: &str,
) -> Result<(), String> {
    let file = create_new_archive_file(path, "log archive")?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut archive = TarBuilder::new(encoder);
    let mut manifest_files = Vec::new();
    for (relative, source, size) in shards {
        let file = open_bundle_attachment_file(source)
            .map_err(|error| format!("failed to open log shard {relative}: {error}"))?;
        let mut reader = HashingReader::new(file.take(*size));
        let archive_path = format!("logs/{relative}");
        let mut header = TarHeader::new_gnu();
        header.set_size(*size);
        header.set_mode(0o600);
        header.set_mtime(modified_at);
        header.set_cksum();
        archive
            .append_data(&mut header, &archive_path, &mut reader)
            .map_err(|error| format!("failed to archive log shard {relative}: {error}"))?;
        let (sha256, bytes_read) = reader.finish();
        if bytes_read != *size {
            return Err(format!(
                "log shard changed while archiving {relative}: read {bytes_read} of {size} bytes"
            ));
        }
        manifest_files.push(ArchiveFileManifest {
            path: archive_path,
            size: *size as usize,
            sha256,
        });
    }
    let manifest = serde_json::json!({
        "format": "portmate-log-archive",
        "version": 1,
        "createdAt": created_at,
        "files": manifest_files,
    });
    let manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("failed to serialize log archive manifest: {error}"))?;
    let mut header = TarHeader::new_gnu();
    header.set_size(manifest_bytes.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(modified_at);
    header.set_cksum();
    archive
        .append_data(&mut header, "manifest.json", manifest_bytes.as_slice())
        .map_err(|error| format!("failed to append log archive manifest: {error}"))?;
    archive
        .finish()
        .map_err(|error| format!("failed to finish log archive tar stream: {error}"))?;
    let encoder = archive
        .into_inner()
        .map_err(|error| format!("failed to close log archive tar stream: {error}"))?;
    let mut file = encoder
        .finish()
        .map_err(|error| format!("failed to finish log archive compression: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush log archive: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync log archive: {error}"))
}

pub(super) fn create_new_archive_file(path: &Path, label: &str) -> Result<fs::File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options
        .open(path)
        .map_err(|error| format!("failed to create {label} {}: {error}", path.display()))
}

pub(super) fn sanitize_log_path_segment(segment: &str) -> String {
    let cleaned = segment
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    cleaned.trim_matches('_').to_string()
}

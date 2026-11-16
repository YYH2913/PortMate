use super::*;

pub(super) const INSTALLER_SHA256: &str =
    "682b57e1c05ef375931975c9d6ded40888f4bfdf9f4b11571e654f962f1199b6";
const MAX_INSTALLER_BYTES: u64 = 32 * 1024 * 1024;

#[cfg(windows)]
#[derive(Default)]
pub(super) struct SetupState {
    pub(super) owner: Option<String>,
    pub(super) phase: &'static str,
    pub(super) downloaded: u64,
    pub(super) total: Option<u64>,
    pub(super) error: Option<EasyTierError>,
    pub(super) installer: Option<DownloadedInstaller>,
    pub(super) cancel: Option<Arc<AtomicBool>>,
}

#[cfg(windows)]
pub(super) struct DownloadedInstaller {
    pub(super) owner: String,
    pub(super) path: PathBuf,
    // Pins are dropped before the temporary directory is removed.
    pub(super) _file: fs::File,
    pub(super) _pin: BoundDirectory,
    pub(super) _directory: tempfile::TempDir,
}

#[cfg(windows)]
pub(super) async fn download(
    root: &Path,
    owner: &str,
    shared: &Arc<Mutex<SetupState>>,
) -> Result<(), EasyTierError> {
    let cancelled = Arc::new(AtomicBool::new(false));
    {
        let mut state = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state = SetupState {
            owner: Some(owner.into()),
            phase: "downloading",
            cancel: Some(Arc::clone(&cancelled)),
            ..SetupState::default()
        };
    }
    let result = async {
        private_directory(root)?;
        let directory = tempfile::Builder::new().prefix("npcap-download-").tempdir_in(root).map_err(|e| failure("npcap-download", e))?;
        let pin = BoundDirectory::open(directory.path()).map_err(|e| failure("npcap-download", e))?;
        let path = directory.path().join("npcap-1.90.exe");
        let client = reqwest::Client::builder().https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(180))
            .build().map_err(|e| failure("npcap-download", e))?;
        let response = tokio::select! {
            response = client.get(DOWNLOAD_URL).send() => response.map_err(|e| failure("npcap-download", e))?,
            _ = wait_cancelled(&cancelled) => return Err(failure("cancelled", "Npcap download cancelled")),
        };
        receive(response, &path, INSTALLER_SHA256, &cancelled, |done, total| {
            let mut state = shared.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            state.downloaded = done; state.total = total;
        }).await?;
        let file = super::windows::pin_file(&path)?;
        verify_file(&file)?;
        tokio::select! {
            verified = super::windows::verify_signature(&path) => verified?,
            _ = wait_cancelled(&cancelled) => return Err(failure("cancelled", "Npcap download cancelled")),
        }
        if cancelled.load(Ordering::SeqCst) { return Err(failure("cancelled", "Npcap download cancelled")); }
        Ok(DownloadedInstaller { owner: owner.into(), path, _file: file, _pin: pin, _directory: directory })
    }.await;
    let mut state = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.cancel = None;
    match result {
        Ok(installer) => {
            state.installer = Some(installer);
            state.phase = "downloaded";
            Ok(())
        }
        Err(error) => {
            state.phase = "failed";
            state.error = Some(error.clone());
            Err(error)
        }
    }
}

pub(super) async fn receive(
    mut response: reqwest::Response,
    path: &Path,
    expected: &str,
    cancelled: &AtomicBool,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<(), EasyTierError> {
    if response.status() != reqwest::StatusCode::OK
        || response
            .content_length()
            .is_some_and(|size| size > MAX_INSTALLER_BYTES)
    {
        return Err(failure(
            "npcap-download",
            "Official Npcap download returned an unexpected status or size",
        ));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(path)
        .map_err(|e| failure("npcap-download", e))?;
    let mut file = tokio::fs::File::from_std(file);
    let total = response.content_length();
    let mut size = 0_u64;
    let mut hash = Sha256::new();
    progress(size, total);
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err(failure("cancelled", "Npcap download cancelled"));
        }
        let chunk = tokio::select! {
            chunk = response.chunk() => chunk.map_err(|e| failure("npcap-download", e))?,
            _ = wait_cancelled(cancelled) => return Err(failure("cancelled", "Npcap download cancelled")),
        };
        let Some(chunk) = chunk else { break };
        size = size.saturating_add(chunk.len() as u64);
        if size > MAX_INSTALLER_BYTES {
            return Err(failure(
                "npcap-download",
                "Npcap installer exceeds the download limit",
            ));
        }
        hash.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|e| failure("npcap-download", e))?;
        progress(size, total);
    }
    file.sync_all()
        .await
        .map_err(|e| failure("npcap-download", e))?;
    if size == 0
        || (total.is_some_and(|total| total != size))
        || hex_hash(hash.finalize().as_slice()) != expected
    {
        return Err(failure(
            "npcap-verification",
            "Npcap installer integrity verification failed; nothing was launched",
        ));
    }
    Ok(())
}

pub(super) fn hex_hash(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(windows)]
pub(super) fn verify_file(file: &fs::File) -> Result<(), EasyTierError> {
    let mut file = file
        .try_clone()
        .map_err(|e| failure("npcap-verification", e))?;
    file.seek(std::io::SeekFrom::Start(0))
        .map_err(|e| failure("npcap-verification", e))?;
    if file
        .metadata()
        .map_err(|e| failure("npcap-verification", e))?
        .len()
        > MAX_INSTALLER_BYTES
    {
        return Err(failure(
            "npcap-verification",
            "Npcap installer exceeds the download limit",
        ));
    }
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| failure("npcap-verification", e))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_INSTALLER_BYTES {
            return Err(failure(
                "npcap-verification",
                "Npcap installer exceeds the download limit",
            ));
        }
        hash.update(&buffer[..count]);
    }
    if total == 0 || hex_hash(&hash.finalize()) != INSTALLER_SHA256 {
        return Err(failure(
            "npcap-verification",
            "Npcap installer integrity verification failed",
        ));
    }
    Ok(())
}

async fn wait_cancelled(cancelled: &AtomicBool) {
    while !cancelled.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
#[path = "../../../../test/rust/portmate/unit/npcap_download.rs"]
mod tests;

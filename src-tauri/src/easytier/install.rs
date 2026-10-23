use super::*;
use std::io::Cursor;

const MAX_DOWNLOAD: usize = 64 * 1024 * 1024;

pub(super) fn release_asset() -> Result<(&'static str, &'static str), EasyTierError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok((
            "easytier-linux-x86_64-v2.6.4.zip",
            "61b659eaedba658fa66fe47d17e1426cdd77e5d02fa15fed447bb4357c09dfd6",
        )),
        ("windows", "x86_64") => Ok((
            "easytier-windows-x86_64-v2.6.4.zip",
            "27af91e270e554709b048bd32327fefd2dfce5062ae1e8701af7550c6f525f84",
        )),
        ("macos", "aarch64") => Ok((
            "easytier-macos-aarch64-v2.6.4.zip",
            "4be1882d1aa36d31c1d6ba0596f2cf8a097e371f8da124212324b2e0f8df7e4b",
        )),
        _ => Err(failure(
            "unsupported-platform",
            "Choose an existing EasyTier 2.6.4 directory on this architecture",
        )),
    }
}

pub(super) fn component_directory(root: &Path, config: &EasyTierConfig) -> PathBuf {
    if config.binary_directory.is_empty() {
        root.join(format!("component-{VERSION}"))
    } else {
        PathBuf::from(&config.binary_directory)
    }
}

pub(super) fn executable(directory: &Path, name: &str) -> Result<PathBuf, EasyTierError> {
    let path = directory.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    });
    let metadata = fs::symlink_metadata(&path).map_err(|_| {
        failure(
            "component-missing",
            "Install EasyTier or choose its executable directory",
        )
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(failure(
            "component-missing",
            "Expected a regular EasyTier executable",
        ));
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o111 == 0 {
        return Err(failure(
            "component-missing",
            "EasyTier executable is not executable",
        ));
    }
    Ok(path)
}

pub(super) async fn install_component(
    root: PathBuf,
    progress: tauri::ipc::Channel<InstallProgress>,
) -> Result<(), EasyTierError> {
    let (name, expected) = release_asset()?;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .user_agent(concat!("PortMate/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| failure("download-failed", e))?;
    let mut response = client
        .get(format!(
            "https://github.com/EasyTier/EasyTier/releases/download/v{VERSION}/{name}"
        ))
        .send()
        .await
        .map_err(|e| failure("download-failed", e))?
        .error_for_status()
        .map_err(|e| failure("download-failed", e))?;
    let total = response.content_length();
    if total.is_some_and(|size| size > MAX_DOWNLOAD as u64) {
        return Err(failure("invalid-download", "Download exceeds 64 MiB"));
    }
    let mut bytes = Vec::new();
    let mut last = Instant::now() - Duration::from_secs(1);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| failure("download-failed", e))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_DOWNLOAD {
            return Err(failure("invalid-download", "Download exceeds 64 MiB"));
        }
        bytes.extend_from_slice(&chunk);
        if last.elapsed() >= Duration::from_millis(100) {
            let _ = progress.send(InstallProgress {
                downloaded: bytes.len() as u64,
                total,
            });
            last = Instant::now();
        }
    }
    let _ = progress.send(InstallProgress {
        downloaded: bytes.len() as u64,
        total,
    });
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != expected {
        return Err(failure(
            "invalid-download",
            "Official archive SHA-256 does not match the pinned release",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || extract_component(&root, &bytes, cfg!(windows)))
        .await
        .map_err(|e| failure("install-failed", e))?
}

pub(super) fn extract_component(
    root: &Path,
    bytes: &[u8],
    windows: bool,
) -> Result<(), EasyTierError> {
    private_directory(root)?;
    let stage = tempfile::Builder::new()
        .prefix("install-")
        .tempdir_in(root)
        .map_err(|e| failure("install-failed", e))?;
    let names: &[&str] = if windows {
        &[
            "easytier-core.exe",
            "easytier-cli.exe",
            "wintun.dll",
            "Packet.dll",
            "WinDivert64.sys",
        ]
    } else {
        &["easytier-core", "easytier-cli"]
    };
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| failure("invalid-download", e))?;
    let mut found = HashSet::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| failure("invalid-download", e))?;
        let Some(path) = entry.enclosed_name() else {
            return Err(failure("invalid-download", "Unsafe archive path"));
        };
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !names.contains(&name) || entry.is_dir() {
            continue;
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
            || !found.insert(name.to_string())
            || entry.size() > 128 * 1024 * 1024
        {
            return Err(failure(
                "invalid-download",
                "Invalid or duplicate component file",
            ));
        }
        let output = stage.path().join(name);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|e| failure("install-failed", e))?;
        std::io::copy(&mut entry, &mut file).map_err(|e| failure("install-failed", e))?;
        #[cfg(unix)]
        fs::set_permissions(&output, fs::Permissions::from_mode(0o755))
            .map_err(|e| failure("install-failed", e))?;
    }
    if names.iter().any(|name| !found.contains(*name)) {
        return Err(failure(
            "invalid-download",
            "Archive is missing a required component",
        ));
    }
    fs::write(stage.path().join("SOURCE.txt"), format!("EasyTier {VERSION}\nhttps://github.com/EasyTier/EasyTier/tree/v{VERSION}\nLGPL-3.0\nhttps://github.com/EasyTier/EasyTier/blob/v{VERSION}/LICENSE\n"))
        .map_err(|e| failure("install-failed", e))?;
    for (name, text) in [
        (
            "LICENSE",
            include_str!("../../../licenses/EasyTier-LGPL-3.0.txt"),
        ),
        ("COPYING", include_str!("../../../licenses/GPL-3.0.txt")),
    ] {
        fs::write(stage.path().join(name), text).map_err(|e| failure("install-failed", e))?;
    }
    let target = component_directory(root, &EasyTierConfig::default());
    let backup = tempfile::Builder::new()
        .prefix("previous-")
        .tempdir_in(root)
        .map_err(|e| failure("install-failed", e))?;
    let had_previous = target.exists();
    if had_previous {
        fs::rename(&target, backup.path().join("component"))
            .map_err(|e| failure("install-failed", e))?;
    }
    if let Err(error) = fs::rename(stage.path(), &target) {
        if had_previous {
            let _ = fs::rename(backup.path().join("component"), &target);
        }
        return Err(failure("install-failed", error));
    }
    Ok(())
}

//! Platform-specific executables are part of the PortMate binary, prepared locally.
use super::*;

pub(super) struct EmbeddedFile {
    pub name: &'static str,
    pub sha256: &'static str,
    pub bytes: &'static [u8],
}

#[cfg(feature = "bundled-easytier")]
include!(concat!(env!("OUT_DIR"), "/easytier/payload.rs"));

#[cfg(not(feature = "bundled-easytier"))]
pub(super) const PAYLOAD_ID: &str = "unavailable";
#[cfg(not(feature = "bundled-easytier"))]
pub(super) static FILES: &[EmbeddedFile] = &[];

pub(super) fn notice_files() -> [(&'static str, &'static [u8]); 7] {
    [
        (
            "LICENSE",
            include_bytes!("../../../licenses/EasyTier-LGPL-3.0.txt"),
        ),
        ("COPYING", include_bytes!("../../../licenses/GPL-3.0.txt")),
        (
            "EasyTier-LGPL-3.0.txt",
            include_bytes!("../../../licenses/EasyTier-LGPL-3.0.txt"),
        ),
        (
            "GPL-3.0.txt",
            include_bytes!("../../../licenses/GPL-3.0.txt"),
        ),
        (
            "SOURCE.txt",
            include_bytes!("../../../licenses/EasyTier-NOTICE.txt"),
        ),
        (
            "Wintun-LICENSE.txt",
            include_bytes!("../../../licenses/Wintun-LICENSE.txt"),
        ),
        (
            "WinDivert-LICENSE.txt",
            include_bytes!("../../../licenses/WinDivert-LICENSE.txt"),
        ),
    ]
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
            "Bundled EasyTier executable is unavailable",
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
            "Bundled EasyTier executable is not executable",
        ));
    }
    Ok(path)
}

pub(super) fn prepare(root: &Path) -> Result<PathBuf, EasyTierError> {
    prepare_files(root, FILES, PAYLOAD_ID)
}

pub(super) fn prepare_files(
    root: &Path,
    files: &[EmbeddedFile],
    payload_id: &str,
) -> Result<PathBuf, EasyTierError> {
    if files.is_empty() {
        return Err(failure(
            "component-missing",
            "This build does not include EasyTier",
        ));
    }
    private_directory(root)?;
    let target = root.join(format!(
        "builtin-{VERSION}-{}",
        &payload_id[..12.min(payload_id.len())]
    ));
    if intact(&target, files) {
        return Ok(target);
    }
    let stage = tempfile::Builder::new()
        .prefix("prepare-")
        .tempdir_in(root)
        .map_err(|e| failure("component-failed", e))?;
    for source in files {
        let path = stage.path().join(source.name);
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o700);
        let mut file = options
            .open(&path)
            .map_err(|e| failure("component-failed", e))?;
        file.write_all(source.bytes)
            .map_err(|e| failure("component-failed", e))?;
        file.sync_all()
            .map_err(|e| failure("component-failed", e))?;
    }
    for (name, bytes) in notice_files() {
        fs::write(stage.path().join(name), bytes).map_err(|e| failure("component-failed", e))?;
    }
    let backup = tempfile::Builder::new()
        .prefix("previous-")
        .tempdir_in(root)
        .map_err(|e| failure("component-failed", e))?;
    let had_previous = fs::symlink_metadata(&target).is_ok();
    if had_previous {
        fs::rename(&target, backup.path().join("component"))
            .map_err(|e| failure("component-failed", e))?;
    }
    if let Err(error) = fs::rename(stage.path(), &target) {
        if had_previous {
            let _ = fs::rename(backup.path().join("component"), &target);
        }
        return Err(failure("component-failed", error));
    }
    Ok(target)
}

fn intact(directory: &Path, files: &[EmbeddedFile]) -> bool {
    let Ok(metadata) = fs::symlink_metadata(directory) else {
        return false;
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return false;
    }
    // A stale or user-supplied Packet.dll must not shadow the separately
    // installed, verified Npcap dependency. Notices are repaired too.
    if fs::read_dir(directory).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case("Packet.dll")
        })
    }) {
        return false;
    }
    if !notice_files().iter().all(|(name, expected)| {
        let path = directory.join(name);
        fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
            && fs::read(path).is_ok_and(|bytes| bytes == *expected)
    }) {
        return false;
    }
    files.iter().all(|source| {
        let path = directory.join(source.name);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return false;
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() != source.bytes.len() as u64
        {
            return false;
        }
        #[cfg(unix)]
        if metadata.permissions().mode() & 0o100 == 0 {
            return false;
        }
        let Ok(mut file) = fs::File::open(path) else {
            return false;
        };
        let mut hash = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        let mut remaining = metadata.len();
        while remaining > 0 {
            let Ok(count) = std::io::Read::read(&mut file, &mut buffer) else {
                return false;
            };
            if count == 0 || count as u64 > remaining {
                return false;
            }
            remaining -= count as u64;
            hash.update(&buffer[..count]);
        }
        let digest: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        digest == source.sha256
    })
}

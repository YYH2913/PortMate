use std::fs;
use std::path::Path;

pub(super) const STORE_FILE_NAME: &str = "portmate-store.sqlite3";
pub(super) const LEGACY_JSON_STORE_FILE_NAME: &str = "portmate-store.json";
pub(super) const LEGACY_APP_IDENTIFIER: &str = "dev.portmate.app";
pub(super) const PORTABLE_VAULT_FILE_NAME: &str = "portmate-vault.hold";
pub(super) const PORTABLE_VAULT_SALT_FILE_NAME: &str = "portmate-vault.salt";

const OWNED_ENTRIES: &[&str] = &[
    STORE_FILE_NAME,
    LEGACY_JSON_STORE_FILE_NAME,
    PORTABLE_VAULT_FILE_NAME,
    PORTABLE_VAULT_SALT_FILE_NAME,
    "credentials.lock",
    "portmate-ipc.json",
    "logs",
    "exports",
];

pub(super) fn migrate_legacy_app_data_dir(
    data_root: &Path,
    current_data_dir: &Path,
) -> Result<(), String> {
    let legacy_data_dir = data_root.join(LEGACY_APP_IDENTIFIER);
    if legacy_data_dir == current_data_dir || !legacy_data_dir.exists() {
        return Ok(());
    }
    validate_app_data_directory(&legacy_data_dir, "legacy")?;

    if current_data_dir.exists() {
        validate_app_data_directory(current_data_dir, "current")?;
        if app_data_directory_has_portmate_state(current_data_dir)? {
            return Err(format!(
                "both legacy and current PortMate data directories contain PortMate state; refusing to merge {} into {}",
                legacy_data_dir.display(),
                current_data_dir.display()
            ));
        }
        fs::remove_dir_all(current_data_dir).map_err(|error| {
            format!(
                "failed to remove bootstrap-only current PortMate data directory {}: {error}",
                current_data_dir.display()
            )
        })?;
    }

    if let Some(parent) = current_data_dir.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create PortMate data directory parent {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::rename(&legacy_data_dir, current_data_dir).map_err(|error| {
        format!(
            "failed to migrate PortMate data directory {} to {}: {error}",
            legacy_data_dir.display(),
            current_data_dir.display()
        )
    })
}

fn validate_app_data_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect {label} PortMate data directory {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "{label} PortMate data path must be a real directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn app_data_directory_has_portmate_state(path: &Path) -> Result<bool, String> {
    for entry in OWNED_ENTRIES {
        let entry = path.join(entry);
        if entry.try_exists().map_err(|error| {
            format!(
                "failed to inspect current PortMate data entry {}: {error}",
                entry.display()
            )
        })? {
            return Ok(true);
        }
    }
    Ok(false)
}

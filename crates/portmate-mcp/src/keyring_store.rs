use anyhow::{anyhow, Result};
use keyring_core::Entry;
use std::sync::{Mutex, OnceLock};

fn ensure_keyring_store() -> Result<()> {
    static KEYRING_INITIALIZED: OnceLock<Mutex<bool>> = OnceLock::new();
    ensure_keyring_store_with(
        KEYRING_INITIALIZED.get_or_init(|| Mutex::new(false)),
        initialize_persistent_native_keyring,
    )
}

pub(crate) fn ensure_keyring_store_with<Initialize>(
    initialized: &Mutex<bool>,
    initialize: Initialize,
) -> Result<()>
where
    Initialize: FnOnce() -> Result<()>,
{
    let mut initialized = initialized
        .lock()
        .map_err(|error| anyhow!(error.to_string()))?;
    if *initialized {
        return Ok(());
    }
    initialize()?;
    *initialized = true;
    Ok(())
}

fn initialize_persistent_native_keyring() -> Result<()> {
    initialize_persistent_native_keyring_with(|not_keyutils| {
        keyring::use_native_store(not_keyutils)
            .map_err(|error| anyhow!("system keyring initialization failed: {error}"))
    })
}

pub(crate) fn initialize_persistent_native_keyring_with<UseNative>(
    use_native: UseNative,
) -> Result<()>
where
    UseNative: FnOnce(bool) -> Result<()>,
{
    // On Linux, true selects persistent Secret Service instead of reboot-volatile keyutils.
    use_native(true)
}

fn keyring_entry(secret_ref: &str) -> Result<Entry> {
    ensure_keyring_store()?;
    let account = secret_ref
        .trim()
        .strip_prefix("keychain:")
        .unwrap_or_else(|| secret_ref.trim());
    if account.is_empty() || account.contains('\0') {
        return Err(anyhow!("invalid secretRef"));
    }
    Entry::new("PortMate", account)
        .map_err(|error| anyhow!("failed to create keyring entry: {error}"))
}

pub(crate) fn read_secret_from_keyring(secret_ref: &str) -> Result<String> {
    keyring_entry(secret_ref)?
        .get_password()
        .map_err(|error| anyhow!("failed to read keyring secret {secret_ref}: {error:?}"))
}

pub(crate) fn write_secret_to_keyring(secret_ref: &str, secret: &str) -> Result<()> {
    keyring_entry(secret_ref)?
        .set_password(secret)
        .map_err(|error| anyhow!("failed to write keyring secret {secret_ref}: {error:?}"))
}

use super::*;

pub(super) fn ensure_keyring_store() -> Result<(), String> {
    static KEYRING_INITIALIZED: OnceLock<Mutex<bool>> = OnceLock::new();
    ensure_keyring_store_with(
        KEYRING_INITIALIZED.get_or_init(|| Mutex::new(false)),
        initialize_persistent_native_keyring,
    )
}

pub(super) fn ensure_keyring_store_with<Initialize>(
    initialized: &Mutex<bool>,
    initialize: Initialize,
) -> Result<(), String>
where
    Initialize: FnOnce() -> Result<(), String>,
{
    let mut initialized = initialized.lock().map_err(|error| error.to_string())?;
    if *initialized {
        return Ok(());
    }
    initialize()?;
    *initialized = true;
    Ok(())
}

fn initialize_persistent_native_keyring() -> Result<(), String> {
    initialize_persistent_native_keyring_with(|| {
        portmate_keyring::initialize_persistent_native_store()
            .map_err(|error| format!("系统密钥库初始化失败: {error}"))
    })
}

pub(super) fn initialize_persistent_native_keyring_with<UseNative>(
    use_native: UseNative,
) -> Result<(), String>
where
    UseNative: FnOnce() -> Result<(), String>,
{
    use_native()
}

pub(super) fn write_secret_to_store(secret_ref: &str, secret: &str) -> Result<(), String> {
    if secret_ref.trim().starts_with("stronghold:") {
        write_secret_to_portable_vault(secret_ref, secret)
    } else {
        write_secret_to_keyring(secret_ref, secret)
    }
}

pub(super) fn write_new_secret(
    storage: Option<SecretStorage>,
    secret: &str,
) -> Result<String, String> {
    let preferred = storage.unwrap_or(SecretStorage::Native);
    let secret_ref = match preferred {
        SecretStorage::Native => format!("keychain:{}", Uuid::new_v4()),
        SecretStorage::Portable => format!("stronghold:{}", Uuid::new_v4()),
    };
    match write_secret_to_store(&secret_ref, secret) {
        Ok(()) => Ok(secret_ref),
        Err(native_error) if storage.is_none() && matches!(preferred, SecretStorage::Native) => {
            let fallback_ref = format!("stronghold:{}", Uuid::new_v4());
            write_secret_to_portable_vault(&fallback_ref, secret).map_err(|portable_error| {
                format!(
                    "系统密钥库写入失败: {native_error}; portable vault fallback 失败: {portable_error}"
                )
            })?;
            Ok(fallback_ref)
        }
        Err(error) => Err(error),
    }
}

pub(super) fn read_secret_from_store(secret_ref: &str) -> Result<String, String> {
    if secret_ref.trim().starts_with("stronghold:") {
        read_secret_from_portable_vault(secret_ref)
    } else {
        read_secret_from_keyring(secret_ref)
    }
}

pub(super) fn probe_secret_from_store(secret_ref: &str) -> SecretProbeResult {
    if secret_ref.trim().starts_with("stronghold:") {
        probe_secret_from_portable_vault(secret_ref)
    } else {
        probe_secret_from_keyring(secret_ref)
    }
}

pub(super) fn delete_secret_from_store(secret_ref: &str) -> Result<(), String> {
    if secret_ref.trim().starts_with("stronghold:") {
        delete_secret_from_portable_vault(secret_ref)
    } else {
        delete_secret_from_keyring(secret_ref)
    }
}

fn keyring_entry(secret_ref: &str) -> Result<Entry, String> {
    ensure_keyring_store()?;
    let account = secret_ref
        .trim()
        .strip_prefix("keychain:")
        .unwrap_or_else(|| secret_ref.trim());
    if account.is_empty() || account.contains('\0') {
        return Err("secretRef 无效".to_string());
    }
    Entry::new("PortMate", account).map_err(|error| format!("创建系统密钥库条目失败: {error}"))
}

pub(super) fn write_secret_to_keyring(secret_ref: &str, secret: &str) -> Result<(), String> {
    let entry = keyring_entry(secret_ref)?;
    entry
        .set_password(secret)
        .map_err(|error| format!("写入系统密钥库失败: {error}"))
}

pub(super) fn read_secret_from_keyring(secret_ref: &str) -> Result<String, String> {
    let entry = keyring_entry(secret_ref)?;
    entry
        .get_password()
        .map_err(|error| format!("读取系统密钥库失败: {error:?}"))
}

pub(super) fn probe_secret_from_keyring(secret_ref: &str) -> SecretProbeResult {
    let entry = match keyring_entry(secret_ref) {
        Ok(entry) => entry,
        Err(error) => return SecretProbeResult::Unavailable(error),
    };
    match entry.get_password() {
        Ok(secret) => SecretProbeResult::Present(Zeroizing::new(secret)),
        Err(keyring_core::Error::NoEntry) => SecretProbeResult::Missing,
        Err(error) => SecretProbeResult::Unavailable(format!("读取系统密钥库失败: {error:?}")),
    }
}

pub(super) fn delete_secret_from_keyring(secret_ref: &str) -> Result<(), String> {
    let entry = keyring_entry(secret_ref)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("删除系统密钥库条目失败: {error}")),
    }
}

pub(super) fn has_secret_ref(secret_ref: &str) -> bool {
    read_secret_from_store(secret_ref).is_ok()
}

pub(super) fn read_optional_secret_ref(
    secret_ref: Option<&str>,
    label: &str,
) -> Result<Option<String>, String> {
    let Some(secret_ref) = secret_ref.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    read_secret_from_store(secret_ref)
        .map(Some)
        .map_err(|error| format!("{label} 已配置 secretRef 但读取失败: {error}"))
}

fn delete_native_migration_secrets(secret_refs: &[String]) -> SecretBatchDeleteOutcome {
    let results = secret_refs
        .iter()
        .map(|secret_ref| (secret_ref.clone(), delete_secret_from_keyring(secret_ref)))
        .collect();
    SecretBatchDeleteOutcome {
        results,
        portable_vault_requires_reunlock: false,
    }
}

pub(super) fn write_profile_secret_migration_batch(
    storage: SecretStorage,
    entries: &[PreparedProfileSecretMigration],
) -> Result<bool, String> {
    match storage {
        SecretStorage::Portable => {
            let entries = entries
                .iter()
                .map(|entry| PortableVaultBatchEntry {
                    secret_ref: &entry.target_ref,
                    secret: entry.secret.as_str(),
                })
                .collect::<Vec<_>>();
            write_secret_batch_to_portable_vault(&entries)
        }
        SecretStorage::Native => {
            let mut written = Vec::new();
            for entry in entries {
                written.push(entry.target_ref.clone());
                if let Err(error) =
                    write_secret_to_keyring(&entry.target_ref, entry.secret.as_str())
                {
                    let cleanup = delete_native_migration_secrets(&written);
                    return Err(migration_error_with_cleanup(error, &cleanup));
                }
                let verification = match read_secret_from_keyring(&entry.target_ref) {
                    Ok(secret) => Zeroizing::new(secret),
                    Err(error) => {
                        let cleanup = delete_native_migration_secrets(&written);
                        return Err(migration_error_with_cleanup(
                            format!("系统密钥库目标读回验证失败: {error}"),
                            &cleanup,
                        ));
                    }
                };
                if verification.as_str() != entry.secret.as_str() {
                    let cleanup = delete_native_migration_secrets(&written);
                    return Err(migration_error_with_cleanup(
                        "系统密钥库目标读回内容不一致",
                        &cleanup,
                    ));
                }
            }
            Ok(false)
        }
    }
}

pub(super) fn delete_profile_secret_migration_batch(
    storage: SecretStorage,
    secret_refs: &[String],
) -> SecretBatchDeleteOutcome {
    match storage {
        SecretStorage::Native => delete_native_migration_secrets(secret_refs),
        SecretStorage::Portable => match delete_secret_batch_from_portable_vault(secret_refs) {
            Ok(requires_reunlock) => SecretBatchDeleteOutcome {
                results: secret_refs
                    .iter()
                    .map(|secret_ref| (secret_ref.clone(), Ok(())))
                    .collect(),
                portable_vault_requires_reunlock: requires_reunlock,
            },
            Err(error) => SecretBatchDeleteOutcome {
                results: secret_refs
                    .iter()
                    .map(|secret_ref| (secret_ref.clone(), Err(error.clone())))
                    .collect(),
                portable_vault_requires_reunlock: false,
            },
        },
    }
}

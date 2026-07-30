use super::*;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const PORTABLE_VAULT_CLIENT: &[u8] = b"portmate-secrets";

pub(super) struct PortableVaultContext {
    pub(super) snapshot_path: PathBuf,
    pub(super) salt_path: PathBuf,
    pub(super) stronghold: Mutex<Option<PortableStronghold>>,
}

pub(super) static PORTABLE_VAULT: OnceLock<PortableVaultContext> = OnceLock::new();

pub(super) fn portable_vault_status_inner() -> Result<PortableVaultStatus, String> {
    let context = portable_vault_context()?;
    let unlocked = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?
        .is_some();
    Ok(PortableVaultStatus {
        exists: context.snapshot_path.exists(),
        unlocked,
        path: context.snapshot_path.display().to_string(),
    })
}

pub(super) fn portable_vault_recovery_ready() -> Result<bool, String> {
    let context = portable_vault_context()?;
    let stronghold = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    Ok(stronghold.as_ref().is_some_and(|stronghold| {
        stronghold.snapshot_version != PortableVaultSnapshotVersion::UnknownAfterCommit
    }))
}

pub(super) fn portable_vault_context() -> Result<&'static PortableVaultContext, String> {
    PORTABLE_VAULT
        .get()
        .ok_or_else(|| "portable vault 尚未初始化".to_string())
}

pub(super) fn read_portable_vault_salt(salt_path: &Path) -> Result<Vec<u8>, String> {
    if !portable_vault_file_exists(salt_path, "salt")? {
        return Err("无法读取 portable vault salt: 文件不存在".to_string());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let file = options
        .open(salt_path)
        .map_err(|error| format!("无法读取 portable vault salt: {error}"))?;
    secure_opened_portable_vault_file(&file, "salt")?;
    let mut salt = Vec::with_capacity(portmate_kdf::SALT_LENGTH.saturating_add(1));
    file.take(portmate_kdf::SALT_LENGTH.saturating_add(1) as u64)
        .read_to_end(&mut salt)
        .map_err(|error| format!("无法读取 portable vault salt: {error}"))?;
    if salt.len() != portmate_kdf::SALT_LENGTH {
        return Err(format!(
            "portable vault salt 长度无效: expected {}, got {}",
            portmate_kdf::SALT_LENGTH,
            salt.len()
        ));
    }
    Ok(salt)
}

pub(super) fn open_portable_vault(
    snapshot_path: &Path,
    salt_path: &Path,
    password: &str,
) -> Result<PortableStronghold, String> {
    secure_portable_vault_parent(snapshot_path)?;
    let salt_lock = lock_portable_vault_snapshot(snapshot_path)?;
    let snapshot_exists = portable_vault_file_exists(snapshot_path, "snapshot")?;
    let salt_exists = portable_vault_file_exists(salt_path, "salt")?;
    if snapshot_exists && !salt_exists {
        return Err("portable vault snapshot 存在，但 salt 文件缺失，已阻止解锁".to_string());
    }
    let salt = if salt_exists {
        read_portable_vault_salt(salt_path)?
    } else {
        let mut salt = vec![0_u8; portmate_kdf::SALT_LENGTH];
        getrandom::fill(&mut salt)
            .map_err(|error| format!("无法生成 portable vault salt: {error}"))?;
        super::store_persistence::write_private_atomic_file(
            salt_path,
            &salt,
            "portable vault salt",
        )?;
        salt
    };
    drop(salt_lock);
    let mut key = portmate_kdf::derive_key(password.as_bytes(), &salt)
        .map_err(|error| format!("portable vault 密钥派生失败: {error}"))?;
    let stronghold_result = PortableStronghold::new(snapshot_path, key.to_vec());
    key.zeroize();
    let mut stronghold =
        stronghold_result.map_err(|error| format!("portable vault 解锁失败: {error}"))?;
    if stronghold.opened_existing_snapshot {
        stronghold
            .load_client(PORTABLE_VAULT_CLIENT)
            .map_err(|error| format!("portable vault client 加载失败: {error}"))?;
    } else {
        stronghold
            .create_client(PORTABLE_VAULT_CLIENT)
            .map_err(|error| format!("portable vault client 创建失败: {error}"))?;
        stronghold
            .save()
            .map_err(|error| format!("portable vault 初始化保存失败: {error}"))?;
    }
    Ok(stronghold)
}

pub(super) fn unlock_portable_vault_in(
    context: &PortableVaultContext,
    password: &str,
) -> Result<(), String> {
    let mut unlocked = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    if !context.snapshot_path.exists() && password.chars().count() < 8 {
        return Err("新建 portable vault 的主密码至少需要 8 个字符".to_string());
    }
    let stronghold = open_portable_vault(&context.snapshot_path, &context.salt_path, password)?;
    *unlocked = Some(stronghold);
    Ok(())
}

pub(super) fn rotate_portable_vault_password_in(
    context: &PortableVaultContext,
    current_password: &str,
    new_password: &str,
) -> Result<(), String> {
    if !context.snapshot_path.exists() {
        return Err("portable vault 尚未创建".to_string());
    }
    if new_password.chars().count() < 8 {
        return Err("portable vault 新主密码至少需要 8 个字符".to_string());
    }
    if current_password == new_password {
        return Err("portable vault 新主密码必须与当前密码不同".to_string());
    }

    let mut unlocked = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    let stronghold = unlocked
        .as_mut()
        .ok_or_else(|| "portable vault 已锁定".to_string())?;
    let verification =
        open_portable_vault(&context.snapshot_path, &context.salt_path, current_password)
            .map_err(|error| format!("portable vault 当前主密码验证失败: {error}"))?;
    drop(verification);

    let salt = read_portable_vault_salt(&context.salt_path)?;
    let mut key = portmate_kdf::derive_key(new_password.as_bytes(), &salt)
        .map_err(|error| format!("portable vault 新密钥派生失败: {error}"))?;
    let result = stronghold.rekey(key.to_vec());
    key.zeroize();
    result
}

pub(super) fn portable_vault_account(secret_ref: &str) -> Result<&str, String> {
    let account = secret_ref
        .trim()
        .strip_prefix("stronghold:")
        .ok_or_else(|| "portable secretRef 必须使用 stronghold: 前缀".to_string())?;
    if account.is_empty() || account.contains('\0') {
        return Err("secretRef 无效".to_string());
    }
    Ok(account)
}

pub(super) fn write_secret_to_portable_vault(secret_ref: &str, secret: &str) -> Result<(), String> {
    let context = portable_vault_context()?;
    write_secret_to_portable_vault_in(context, secret_ref, secret)
}

pub(super) fn write_secret_to_portable_vault_in(
    context: &PortableVaultContext,
    secret_ref: &str,
    secret: &str,
) -> Result<(), String> {
    let account = portable_vault_account(secret_ref)?;
    let mut stronghold = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    let stronghold = stronghold
        .as_mut()
        .ok_or_else(|| "portable vault 已锁定".to_string())?;
    let client = stronghold
        .get_client(PORTABLE_VAULT_CLIENT)
        .map_err(|error| format!("portable vault client 不可用: {error}"))?;
    let store = client.store();
    let old_value = store
        .insert(
            account.as_bytes().to_vec(),
            secret.as_bytes().to_vec(),
            None,
        )
        .map_err(|error| format!("写入 portable vault 失败: {error}"))?;
    if let Err(error) = stronghold.save() {
        match old_value {
            Some(old_value) => {
                let _ = store.insert(account.as_bytes().to_vec(), old_value, None);
            }
            None => {
                let _ = store.delete(account.as_bytes());
            }
        }
        return Err(format!("保存 portable vault snapshot 失败: {error}"));
    }
    Ok(())
}

pub(super) fn read_secret_from_portable_vault(secret_ref: &str) -> Result<String, String> {
    let context = portable_vault_context()?;
    read_secret_from_portable_vault_in(context, secret_ref)
}

pub(super) fn read_secret_from_portable_vault_in(
    context: &PortableVaultContext,
    secret_ref: &str,
) -> Result<String, String> {
    let account = portable_vault_account(secret_ref)?;
    let stronghold = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    let stronghold = stronghold
        .as_ref()
        .ok_or_else(|| "portable vault 已锁定".to_string())?;
    stronghold.ensure_snapshot_current()?;
    let client = stronghold
        .get_client(PORTABLE_VAULT_CLIENT)
        .map_err(|error| format!("portable vault client 不可用: {error}"))?;
    let value = client
        .store()
        .get(account.as_bytes())
        .map_err(|error| format!("读取 portable vault 失败: {error}"))?
        .ok_or_else(|| "portable vault 中不存在该 secretRef".to_string())?;
    String::from_utf8(value).map_err(|_| "portable vault secret 不是有效 UTF-8".to_string())
}

pub(super) fn probe_secret_from_portable_vault(secret_ref: &str) -> SecretProbeResult {
    let context = match portable_vault_context() {
        Ok(context) => context,
        Err(error) => return SecretProbeResult::Unavailable(error),
    };
    let account = match portable_vault_account(secret_ref) {
        Ok(account) => account,
        Err(error) => return SecretProbeResult::Unavailable(error),
    };
    let stronghold = match context.stronghold.lock() {
        Ok(stronghold) => stronghold,
        Err(error) => return SecretProbeResult::Unavailable(error.to_string()),
    };
    let Some(stronghold) = stronghold.as_ref() else {
        return SecretProbeResult::Unavailable("portable vault 已锁定".to_string());
    };
    if let Err(error) = stronghold.ensure_snapshot_current() {
        return SecretProbeResult::Unavailable(error);
    }
    let client = match stronghold.get_client(PORTABLE_VAULT_CLIENT) {
        Ok(client) => client,
        Err(error) => {
            return SecretProbeResult::Unavailable(format!(
                "portable vault client 不可用: {error}"
            ));
        }
    };
    match client.store().get(account.as_bytes()) {
        Ok(Some(value)) => match String::from_utf8(value) {
            Ok(value) => SecretProbeResult::Present(Zeroizing::new(value)),
            Err(_) => {
                SecretProbeResult::Unavailable("portable vault secret 不是有效 UTF-8".to_string())
            }
        },
        Ok(None) => SecretProbeResult::Missing,
        Err(error) => SecretProbeResult::Unavailable(format!("读取 portable vault 失败: {error}")),
    }
}

pub(super) fn delete_secret_from_portable_vault(secret_ref: &str) -> Result<(), String> {
    let context = portable_vault_context()?;
    delete_secret_from_portable_vault_in(context, secret_ref)
}

pub(super) fn delete_secret_from_portable_vault_in(
    context: &PortableVaultContext,
    secret_ref: &str,
) -> Result<(), String> {
    let account = portable_vault_account(secret_ref)?;
    let mut stronghold = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    let stronghold = stronghold
        .as_mut()
        .ok_or_else(|| "portable vault 已锁定".to_string())?;
    let client = stronghold
        .get_client(PORTABLE_VAULT_CLIENT)
        .map_err(|error| format!("portable vault client 不可用: {error}"))?;
    let store = client.store();
    let old_value = store
        .delete(account.as_bytes())
        .map_err(|error| format!("删除 portable vault secret 失败: {error}"))?;
    if let Err(error) = stronghold.save() {
        if let Some(old_value) = old_value {
            let _ = store.insert(account.as_bytes().to_vec(), old_value, None);
        }
        return Err(format!("保存 portable vault snapshot 失败: {error}"));
    }
    Ok(())
}

pub(super) struct PortableVaultBatchEntry<'a> {
    pub(super) secret_ref: &'a str,
    pub(super) secret: &'a str,
}

fn restore_portable_vault_values(
    store: &iota_stronghold::Store,
    old_values: &[(Vec<u8>, Option<Vec<u8>>)],
) {
    for (account, old_value) in old_values.iter().rev() {
        match old_value {
            Some(old_value) => {
                let _ = store.insert(account.clone(), old_value.clone(), None);
            }
            None => {
                let _ = store.delete(account);
            }
        }
    }
}

pub(super) fn write_secret_batch_to_portable_vault(
    entries: &[PortableVaultBatchEntry<'_>],
) -> Result<bool, String> {
    let context = portable_vault_context()?;
    write_secret_batch_to_portable_vault_in(context, entries)
}

pub(super) fn write_secret_batch_to_portable_vault_in(
    context: &PortableVaultContext,
    entries: &[PortableVaultBatchEntry<'_>],
) -> Result<bool, String> {
    let mut accounts = Vec::with_capacity(entries.len());
    let mut unique = HashSet::new();
    for entry in entries {
        let account = portable_vault_account(entry.secret_ref)?
            .as_bytes()
            .to_vec();
        if !unique.insert(account.clone()) {
            return Err(format!(
                "portable vault 批量写入包含重复 secretRef: {}",
                entry.secret_ref
            ));
        }
        accounts.push(account);
    }
    let mut stronghold = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    let stronghold = stronghold
        .as_mut()
        .ok_or_else(|| "portable vault 已锁定".to_string())?;
    stronghold.ensure_snapshot_current()?;
    let client = stronghold
        .get_client(PORTABLE_VAULT_CLIENT)
        .map_err(|error| format!("portable vault client 不可用: {error}"))?;
    let store = client.store();
    let mut old_values = Vec::with_capacity(entries.len());
    for (entry, account) in entries.iter().zip(accounts) {
        match store.insert(account.clone(), entry.secret.as_bytes().to_vec(), None) {
            Ok(old_value) => old_values.push((account, old_value)),
            Err(error) => {
                restore_portable_vault_values(&store, &old_values);
                return Err(format!("写入 portable vault 失败: {error}"));
            }
        }
    }
    if let Err(error) = stronghold.save() {
        restore_portable_vault_values(&store, &old_values);
        return Err(format!("保存 portable vault snapshot 失败: {error}"));
    }
    Ok(stronghold.snapshot_version == PortableVaultSnapshotVersion::UnknownAfterCommit)
}

pub(super) fn delete_secret_batch_from_portable_vault(
    secret_refs: &[String],
) -> Result<bool, String> {
    let context = portable_vault_context()?;
    delete_secret_batch_from_portable_vault_in(context, secret_refs)
}

pub(super) fn delete_secret_batch_from_portable_vault_in(
    context: &PortableVaultContext,
    secret_refs: &[String],
) -> Result<bool, String> {
    let mut accounts = Vec::with_capacity(secret_refs.len());
    let mut unique = HashSet::new();
    for secret_ref in secret_refs {
        let account = portable_vault_account(secret_ref)?.as_bytes().to_vec();
        if !unique.insert(account.clone()) {
            return Err(format!(
                "portable vault 批量删除包含重复 secretRef: {secret_ref}"
            ));
        }
        accounts.push(account);
    }
    let mut stronghold = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    let stronghold = stronghold
        .as_mut()
        .ok_or_else(|| "portable vault 已锁定".to_string())?;
    stronghold.ensure_snapshot_current()?;
    let client = stronghold
        .get_client(PORTABLE_VAULT_CLIENT)
        .map_err(|error| format!("portable vault client 不可用: {error}"))?;
    let store = client.store();
    let mut old_values = Vec::with_capacity(accounts.len());
    for account in accounts {
        match store.delete(&account) {
            Ok(old_value) => old_values.push((account, old_value)),
            Err(error) => {
                restore_portable_vault_values(&store, &old_values);
                return Err(format!("删除 portable vault secret 失败: {error}"));
            }
        }
    }
    if let Err(error) = stronghold.save() {
        restore_portable_vault_values(&store, &old_values);
        return Err(format!("保存 portable vault snapshot 失败: {error}"));
    }
    Ok(stronghold.snapshot_version == PortableVaultSnapshotVersion::UnknownAfterCommit)
}

pub(super) fn ensure_portable_vault_ready_for_migration() -> Result<(), String> {
    let context = portable_vault_context()?;
    let stronghold = context
        .stronghold
        .lock()
        .map_err(|error| error.to_string())?;
    let stronghold = stronghold
        .as_ref()
        .ok_or_else(|| "portable vault 已锁定，请先解锁再迁移凭据".to_string())?;
    stronghold.ensure_snapshot_current()
}

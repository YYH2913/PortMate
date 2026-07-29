use super::*;

#[test]
fn portable_vault_batch_write_and_delete_commit_once_and_reopen() {
    let root = std::env::temp_dir().join(format!("portmate-stronghold-batch-{}", Uuid::new_v4()));
    let snapshot_path = root.join(PORTABLE_VAULT_FILE_NAME);
    let salt_path = root.join(PORTABLE_VAULT_SALT_FILE_NAME);
    let context = PortableVaultContext {
        snapshot_path: snapshot_path.clone(),
        salt_path: salt_path.clone(),
        stronghold: Mutex::new(Some(
            open_portable_vault(&snapshot_path, &salt_path, "correct horse").unwrap(),
        )),
    };
    let entries = [
        PortableVaultBatchEntry {
            secret_ref: "stronghold:first",
            secret: "first-private-value",
        },
        PortableVaultBatchEntry {
            secret_ref: "stronghold:second",
            secret: "second-private-value",
        },
    ];
    assert!(!write_secret_batch_to_portable_vault_in(&context, &entries).unwrap());
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:first").unwrap(),
        "first-private-value"
    );
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:second").unwrap(),
        "second-private-value"
    );
    assert!(!delete_secret_batch_from_portable_vault_in(
        &context,
        &["stronghold:first".to_string()]
    )
    .unwrap());
    context.stronghold.lock().unwrap().take();
    let reopened = open_portable_vault(&snapshot_path, &salt_path, "correct horse").unwrap();
    *context.stronghold.lock().unwrap() = Some(reopened);
    assert!(read_secret_from_portable_vault_in(&context, "stronghold:first").is_err());
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:second").unwrap(),
        "second-private-value"
    );
    context.stronghold.lock().unwrap().take();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn portable_vault_batch_commit_failure_restores_all_in_memory_values() {
    let root =
        std::env::temp_dir().join(format!("portmate-stronghold-batch-fail-{}", Uuid::new_v4()));
    let snapshot_path = root.join(PORTABLE_VAULT_FILE_NAME);
    let salt_path = root.join(PORTABLE_VAULT_SALT_FILE_NAME);
    let context = PortableVaultContext {
        snapshot_path: snapshot_path.clone(),
        salt_path: salt_path.clone(),
        stronghold: Mutex::new(Some(
            open_portable_vault(&snapshot_path, &salt_path, "correct horse").unwrap(),
        )),
    };
    write_secret_to_portable_vault_in(&context, "stronghold:existing", "preserved").unwrap();
    let blocked_parent = root.join("not-a-directory");
    fs::write(&blocked_parent, b"block snapshot commit").unwrap();
    {
        let mut stronghold = context.stronghold.lock().unwrap();
        stronghold.as_mut().unwrap().path =
            SnapshotPath::from_path(blocked_parent.join("vault.hold"));
    }
    let entries = [
        PortableVaultBatchEntry {
            secret_ref: "stronghold:first-new",
            secret: "first-private-value",
        },
        PortableVaultBatchEntry {
            secret_ref: "stronghold:second-new",
            secret: "second-private-value",
        },
    ];
    assert!(write_secret_batch_to_portable_vault_in(&context, &entries)
        .unwrap_err()
        .contains("保存 portable vault"));
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:existing").unwrap(),
        "preserved"
    );
    assert!(read_secret_from_portable_vault_in(&context, "stronghold:first-new").is_err());
    assert!(read_secret_from_portable_vault_in(&context, "stronghold:second-new").is_err());
    context.stronghold.lock().unwrap().take();
    let reopened = open_portable_vault(&snapshot_path, &salt_path, "correct horse").unwrap();
    *context.stronghold.lock().unwrap() = Some(reopened);
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:existing").unwrap(),
        "preserved"
    );
    assert!(read_secret_from_portable_vault_in(&context, "stronghold:first-new").is_err());
    context.stronghold.lock().unwrap().take();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn portable_stronghold_vault_encrypts_and_reopens_records() {
    let root = std::env::temp_dir().join(format!("portmate-stronghold-{}", Uuid::new_v4()));
    let snapshot_path = root.join(PORTABLE_VAULT_FILE_NAME);
    let salt_path = root.join(PORTABLE_VAULT_SALT_FILE_NAME);
    let secret = b"portable-private-key-material";

    let context = PortableVaultContext {
        snapshot_path: snapshot_path.clone(),
        salt_path: salt_path.clone(),
        stronghold: Mutex::new(Some(
            open_portable_vault(&snapshot_path, &salt_path, "correct horse").unwrap(),
        )),
    };
    write_secret_to_portable_vault_in(
        &context,
        "stronghold:identity-1",
        std::str::from_utf8(secret).unwrap(),
    )
    .unwrap();
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:identity-1").unwrap(),
        std::str::from_utf8(secret).unwrap()
    );
    let salt_before = fs::read(&salt_path).unwrap();
    let snapshot = fs::read(&snapshot_path).unwrap();
    assert!(!snapshot
        .windows(secret.len())
        .any(|window| window == secret));
    assert!(open_portable_vault(&snapshot_path, &salt_path, "wrong password").is_err());

    let wrong_current =
        rotate_portable_vault_password_in(&context, "wrong password", "new correct horse")
            .unwrap_err();
    assert!(wrong_current.contains("当前主密码验证失败"));
    assert!(
        rotate_portable_vault_password_in(&context, "correct horse", "short")
            .unwrap_err()
            .contains("至少需要 8 个字符")
    );
    assert!(
        rotate_portable_vault_password_in(&context, "correct horse", "correct horse")
            .unwrap_err()
            .contains("必须与当前密码不同")
    );
    rotate_portable_vault_password_in(&context, "correct horse", "new correct horse").unwrap();
    assert_eq!(fs::read(&salt_path).unwrap(), salt_before);
    assert!(unlock_portable_vault_in(&context, "correct horse").is_err());
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:identity-1").unwrap(),
        std::str::from_utf8(secret).unwrap()
    );

    context.stronghold.lock().unwrap().take();
    assert!(open_portable_vault(&snapshot_path, &salt_path, "correct horse").is_err());
    assert!(
        rotate_portable_vault_password_in(&context, "new correct horse", "third password")
            .unwrap_err()
            .contains("已锁定")
    );

    let reopened = open_portable_vault(&snapshot_path, &salt_path, "new correct horse").unwrap();
    *context.stronghold.lock().unwrap() = Some(reopened);
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:identity-1").unwrap(),
        std::str::from_utf8(secret).unwrap()
    );
    delete_secret_from_portable_vault_in(&context, "stronghold:identity-1").unwrap();
    assert!(read_secret_from_portable_vault_in(&context, "stronghold:identity-1").is_err());
    context.stronghold.lock().unwrap().take();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn portable_vault_rekey_failure_keeps_old_snapshot_and_provider() {
    let root = std::env::temp_dir().join(format!("portmate-rekey-fail-{}", Uuid::new_v4()));
    let snapshot_path = root.join(PORTABLE_VAULT_FILE_NAME);
    let salt_path = root.join(PORTABLE_VAULT_SALT_FILE_NAME);
    let context = PortableVaultContext {
        snapshot_path: snapshot_path.clone(),
        salt_path: salt_path.clone(),
        stronghold: Mutex::new(Some(
            open_portable_vault(&snapshot_path, &salt_path, "current password").unwrap(),
        )),
    };
    write_secret_to_portable_vault_in(&context, "stronghold:identity-1", "private material")
        .unwrap();
    let blocked_parent = root.join("not-a-directory");
    fs::write(&blocked_parent, b"block snapshot commit").unwrap();
    {
        let mut stronghold = context.stronghold.lock().unwrap();
        stronghold.as_mut().unwrap().path =
            SnapshotPath::from_path(blocked_parent.join("vault.hold"));
    }

    let error =
        rotate_portable_vault_password_in(&context, "current password", "replacement password")
            .unwrap_err();
    assert!(error.contains("换密提交失败"));
    assert_eq!(
        read_secret_from_portable_vault_in(&context, "stronghold:identity-1").unwrap(),
        "private material"
    );
    context.stronghold.lock().unwrap().take();
    assert!(open_portable_vault(&snapshot_path, &salt_path, "current password").is_ok());
    assert!(open_portable_vault(&snapshot_path, &salt_path, "replacement password").is_err());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn portable_vault_stale_instance_cannot_overwrite_rotated_snapshot() {
    let root = std::env::temp_dir().join(format!("portmate-rekey-stale-{}", Uuid::new_v4()));
    let snapshot_path = root.join(PORTABLE_VAULT_FILE_NAME);
    let salt_path = root.join(PORTABLE_VAULT_SALT_FILE_NAME);
    let current = PortableVaultContext {
        snapshot_path: snapshot_path.clone(),
        salt_path: salt_path.clone(),
        stronghold: Mutex::new(Some(
            open_portable_vault(&snapshot_path, &salt_path, "current password").unwrap(),
        )),
    };
    write_secret_to_portable_vault_in(&current, "stronghold:identity-1", "original").unwrap();
    let stale = PortableVaultContext {
        snapshot_path: snapshot_path.clone(),
        salt_path: salt_path.clone(),
        stronghold: Mutex::new(Some(
            open_portable_vault(&snapshot_path, &salt_path, "current password").unwrap(),
        )),
    };

    rotate_portable_vault_password_in(&current, "current password", "replacement password")
        .unwrap();
    let error =
        write_secret_to_portable_vault_in(&stale, "stronghold:identity-2", "stale process write")
            .unwrap_err();
    assert!(error.contains("另一 PortMate 实例修改"));
    assert!(open_portable_vault(&snapshot_path, &salt_path, "current password").is_err());

    stale.stronghold.lock().unwrap().take();
    current.stronghold.lock().unwrap().take();
    let reopened = PortableVaultContext {
        snapshot_path: snapshot_path.clone(),
        salt_path: salt_path.clone(),
        stronghold: Mutex::new(Some(
            open_portable_vault(&snapshot_path, &salt_path, "replacement password").unwrap(),
        )),
    };
    assert_eq!(
        read_secret_from_portable_vault_in(&reopened, "stronghold:identity-1").unwrap(),
        "original"
    );
    assert!(read_secret_from_portable_vault_in(&reopened, "stronghold:identity-2").is_err());
    reopened
        .stronghold
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .snapshot_version = PortableVaultSnapshotVersion::UnknownAfterCommit;
    assert!(
        read_secret_from_portable_vault_in(&reopened, "stronghold:identity-1")
            .unwrap_err()
            .contains("重新解锁")
    );
    reopened.stronghold.lock().unwrap().take();

    let _ = fs::remove_dir_all(root);
}

#[test]
fn portable_secret_refs_require_the_stronghold_prefix_and_account() {
    assert_eq!(
        portable_vault_account(" stronghold:identity-1 ").unwrap(),
        "identity-1"
    );
    assert!(portable_vault_account("keychain:identity-1").is_err());
    assert!(portable_vault_account("stronghold:").is_err());
}

#[test]
fn portable_vault_does_not_replace_a_missing_snapshot_salt() {
    let root = std::env::temp_dir().join(format!("portmate-stronghold-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let snapshot_path = root.join(PORTABLE_VAULT_FILE_NAME);
    let salt_path = root.join(PORTABLE_VAULT_SALT_FILE_NAME);
    fs::write(&snapshot_path, b"encrypted snapshot placeholder").unwrap();
    let error = match open_portable_vault(&snapshot_path, &salt_path, "correct horse") {
        Ok(_) => panic!("snapshot without salt must not unlock"),
        Err(error) => error,
    };
    assert!(error.contains("salt 文件缺失"));
    assert!(!salt_path.exists());
    let _ = fs::remove_dir_all(root);
}

const PORTABLE_VAULT_PROBE_PHASE_ENV: &str = "PORTMATE_PORTABLE_VAULT_PROBE_PHASE";
const PORTABLE_VAULT_PROBE_ROOT_ENV: &str = "PORTMATE_PORTABLE_VAULT_PROBE_ROOT";
const PORTABLE_VAULT_PROBE_PASSWORD_ENV: &str = "PORTMATE_PORTABLE_VAULT_PROBE_PASSWORD";
const PORTABLE_VAULT_PROBE_SECRET_REF_ENV: &str = "PORTMATE_PORTABLE_VAULT_PROBE_SECRET_REF";
const PORTABLE_VAULT_PROBE_SECRET_ENV: &str = "PORTMATE_PORTABLE_VAULT_PROBE_SECRET";
const PORTABLE_VAULT_PROBE_ROTATED_SECRET_ENV: &str =
    "PORTMATE_PORTABLE_VAULT_PROBE_ROTATED_SECRET";
const PORTABLE_VAULT_PROBE_TEST_NAME: &str =
    "tests::portable_vault_tests::portable_vault_cross_process_fault_matrix";
const PORTABLE_VAULT_PROBE_SECRET_BYTES: usize = 1_200;

#[derive(Clone)]
struct PortableVaultProbeConfig {
    root: PathBuf,
    password: String,
    secret_ref: String,
    secret: String,
    rotated_secret: String,
}

impl PortableVaultProbeConfig {
    fn new(root: PathBuf, label: &str) -> Self {
        Self {
            root,
            password: format!(
                "portmate-portable-vault-password-{label}-{}",
                Uuid::new_v4()
            ),
            secret_ref: format!("stronghold:portable-probe-{label}-{}", Uuid::new_v4()),
            secret: portable_vault_probe_secret("initial"),
            rotated_secret: portable_vault_probe_secret("rotated"),
        }
    }

    fn from_environment() -> Result<Self, String> {
        let root = std::env::var_os(PORTABLE_VAULT_PROBE_ROOT_ENV)
            .map(PathBuf::from)
            .ok_or_else(|| format!("{PORTABLE_VAULT_PROBE_ROOT_ENV} is missing"))?;
        Ok(Self {
            root,
            password: required_portable_vault_probe_environment(PORTABLE_VAULT_PROBE_PASSWORD_ENV)?,
            secret_ref: required_portable_vault_probe_environment(
                PORTABLE_VAULT_PROBE_SECRET_REF_ENV,
            )?,
            secret: required_portable_vault_probe_environment(PORTABLE_VAULT_PROBE_SECRET_ENV)?,
            rotated_secret: required_portable_vault_probe_environment(
                PORTABLE_VAULT_PROBE_ROTATED_SECRET_ENV,
            )?,
        })
    }

    fn snapshot_path(&self) -> PathBuf {
        self.root.join(PORTABLE_VAULT_FILE_NAME)
    }

    fn salt_path(&self) -> PathBuf {
        self.root.join(PORTABLE_VAULT_SALT_FILE_NAME)
    }

    fn lock_path(&self) -> PathBuf {
        self.root.join(format!("{PORTABLE_VAULT_FILE_NAME}.lock"))
    }

    fn child_secret_ref(&self) -> String {
        format!("{}-child", self.secret_ref)
    }

    fn stale_secret_ref(&self) -> String {
        format!("{}-stale", self.secret_ref)
    }
}

#[test]
fn portable_vault_cross_process_fault_matrix() {
    if let Ok(phase) = std::env::var(PORTABLE_VAULT_PROBE_PHASE_ENV) {
        let config = PortableVaultProbeConfig::from_environment().unwrap();
        run_portable_vault_probe_phase(&phase, &config).unwrap();
        return;
    }

    let temp = tempfile::Builder::new()
        .prefix("portmate-portable-vault-probe-")
        .tempdir()
        .unwrap();

    let crud = PortableVaultProbeConfig::new(temp.path().join("crud"), "crud");
    for phase in [
        "write",
        "expect-wrong-password",
        "verify-update",
        "verify-delete",
    ] {
        run_portable_vault_probe_child(phase, &crud).unwrap();
    }

    verify_portable_vault_integrity_failures(temp.path()).unwrap();
    verify_portable_vault_commit_failure(temp.path()).unwrap();
    verify_portable_vault_cross_process_conflict(temp.path()).unwrap();
    #[cfg(unix)]
    verify_portable_vault_unix_file_security(temp.path()).unwrap();

    println!(
        "PortMate portable vault probe passed on {} ({}-byte cross-process secret)",
        std::env::consts::OS,
        PORTABLE_VAULT_PROBE_SECRET_BYTES
    );
}

fn run_portable_vault_probe_phase(
    phase: &str,
    config: &PortableVaultProbeConfig,
) -> Result<(), String> {
    match phase {
        "write" => {
            let context = open_portable_vault_probe_context(config)?;
            expect_portable_vault_probe_missing(&context, &config.secret_ref)?;
            write_secret_to_portable_vault_in(&context, &config.secret_ref, &config.secret)?;
            expect_portable_vault_probe_secret(&context, &config.secret_ref, &config.secret)
        }
        "expect-wrong-password" => {
            let wrong_password = format!("wrong-{}", config.password);
            match open_portable_vault(
                &config.snapshot_path(),
                &config.salt_path(),
                &wrong_password,
            ) {
                Ok(_) => Err("portable vault unexpectedly accepted a wrong password".to_string()),
                Err(_) => Ok(()),
            }
        }
        "verify-update" => {
            let context = open_portable_vault_probe_context(config)?;
            expect_portable_vault_probe_secret(&context, &config.secret_ref, &config.secret)?;
            write_secret_to_portable_vault_in(
                &context,
                &config.secret_ref,
                &config.rotated_secret,
            )?;
            expect_portable_vault_probe_secret(&context, &config.secret_ref, &config.rotated_secret)
        }
        "verify-delete" => {
            let context = open_portable_vault_probe_context(config)?;
            expect_portable_vault_probe_secret(
                &context,
                &config.secret_ref,
                &config.rotated_secret,
            )?;
            delete_secret_from_portable_vault_in(&context, &config.secret_ref)?;
            expect_portable_vault_probe_missing(&context, &config.secret_ref)?;
            delete_secret_from_portable_vault_in(&context, &config.secret_ref)?;
            expect_portable_vault_probe_missing(&context, &config.secret_ref)
        }
        "conflict-update" => {
            let context = open_portable_vault_probe_context(config)?;
            expect_portable_vault_probe_secret(&context, &config.secret_ref, &config.secret)?;
            write_secret_to_portable_vault_in(
                &context,
                &config.child_secret_ref(),
                &config.rotated_secret,
            )
        }
        _ => Err(format!("unsupported portable vault probe phase: {phase}")),
    }
}

fn run_portable_vault_probe_child(
    phase: &str,
    config: &PortableVaultProbeConfig,
) -> Result<(), String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("resolve portable vault test executable failed: {error}"))?;
    let mut child = Command::new(executable)
        .args([
            PORTABLE_VAULT_PROBE_TEST_NAME,
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(PORTABLE_VAULT_PROBE_PHASE_ENV, phase)
        .env(PORTABLE_VAULT_PROBE_ROOT_ENV, &config.root)
        .env(PORTABLE_VAULT_PROBE_PASSWORD_ENV, &config.password)
        .env(PORTABLE_VAULT_PROBE_SECRET_REF_ENV, &config.secret_ref)
        .env(PORTABLE_VAULT_PROBE_SECRET_ENV, &config.secret)
        .env(
            PORTABLE_VAULT_PROBE_ROTATED_SECRET_ENV,
            &config.rotated_secret,
        )
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("start portable vault probe phase {phase} failed: {error}"))?;
    wait_for_portable_vault_probe_child(&mut child, phase)
}

fn wait_for_portable_vault_probe_child(
    child: &mut std::process::Child,
    phase: &str,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "portable vault probe phase {phase} exited with {status}"
                ));
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "portable vault probe phase {phase} exceeded its 90-second deadline"
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "wait for portable vault probe phase {phase} failed: {error}"
                ));
            }
        }
    }
}

fn open_portable_vault_probe_context(
    config: &PortableVaultProbeConfig,
) -> Result<PortableVaultContext, String> {
    Ok(PortableVaultContext {
        snapshot_path: config.snapshot_path(),
        salt_path: config.salt_path(),
        stronghold: Mutex::new(Some(open_portable_vault(
            &config.snapshot_path(),
            &config.salt_path(),
            &config.password,
        )?)),
    })
}

fn expect_portable_vault_probe_secret(
    context: &PortableVaultContext,
    secret_ref: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = read_secret_from_portable_vault_in(context, secret_ref)?;
    if actual != expected {
        return Err(format!(
            "portable vault probe secret content mismatch for {secret_ref}"
        ));
    }
    Ok(())
}

fn expect_portable_vault_probe_missing(
    context: &PortableVaultContext,
    secret_ref: &str,
) -> Result<(), String> {
    match read_secret_from_portable_vault_in(context, secret_ref) {
        Ok(_) => Err(format!(
            "portable vault probe secret unexpectedly exists: {secret_ref}"
        )),
        Err(error) if error.contains("不存在该 secretRef") => Ok(()),
        Err(error) => Err(format!(
            "portable vault probe missing check failed for {secret_ref}: {error}"
        )),
    }
}

fn verify_portable_vault_integrity_failures(root: &Path) -> Result<(), String> {
    let source = PortableVaultProbeConfig::new(root.join("integrity-source"), "integrity");
    let context = open_portable_vault_probe_context(&source)?;
    write_secret_to_portable_vault_in(&context, &source.secret_ref, &source.secret)?;
    drop(context);
    let snapshot = fs::read(source.snapshot_path())
        .map_err(|error| format!("read portable vault probe snapshot failed: {error}"))?;
    if snapshot
        .windows(source.secret.len())
        .any(|window| window == source.secret.as_bytes())
    {
        return Err("portable vault probe snapshot contains plaintext secret".to_string());
    }
    let salt = fs::read(source.salt_path())
        .map_err(|error| format!("read portable vault probe salt failed: {error}"))?;

    let corrupt = PortableVaultProbeConfig {
        root: root.join("integrity-corrupt"),
        ..source.clone()
    };
    fs::create_dir_all(&corrupt.root)
        .map_err(|error| format!("create corrupt portable vault probe root failed: {error}"))?;
    fs::write(corrupt.snapshot_path(), b"corrupt portable vault snapshot")
        .map_err(|error| format!("write corrupt portable vault probe snapshot failed: {error}"))?;
    fs::write(corrupt.salt_path(), &salt)
        .map_err(|error| format!("write corrupt portable vault probe salt failed: {error}"))?;
    if open_portable_vault(
        &corrupt.snapshot_path(),
        &corrupt.salt_path(),
        &corrupt.password,
    )
    .is_ok()
    {
        return Err("portable vault probe accepted a corrupt snapshot".to_string());
    }
    if fs::read(corrupt.snapshot_path()).ok().as_deref()
        != Some(b"corrupt portable vault snapshot".as_slice())
    {
        return Err("portable vault probe replaced corrupt snapshot evidence".to_string());
    }

    let missing_salt = PortableVaultProbeConfig {
        root: root.join("integrity-missing-salt"),
        ..source
    };
    fs::create_dir_all(&missing_salt.root)
        .map_err(|error| format!("create missing-salt portable vault root failed: {error}"))?;
    fs::write(missing_salt.snapshot_path(), snapshot)
        .map_err(|error| format!("write missing-salt snapshot failed: {error}"))?;
    let error = match open_portable_vault(
        &missing_salt.snapshot_path(),
        &missing_salt.salt_path(),
        &missing_salt.password,
    ) {
        Ok(_) => return Err("portable vault probe accepted a snapshot without salt".to_string()),
        Err(error) => error,
    };
    if !error.contains("salt 文件缺失") || missing_salt.salt_path().exists() {
        return Err("portable vault probe did not preserve missing-salt evidence".to_string());
    }
    Ok(())
}

fn verify_portable_vault_commit_failure(root: &Path) -> Result<(), String> {
    let config = PortableVaultProbeConfig::new(root.join("commit-failure"), "commit-failure");
    let context = open_portable_vault_probe_context(&config)?;
    write_secret_to_portable_vault_in(&context, &config.secret_ref, &config.secret)?;
    let blocked_parent = config.root.join("not-a-directory");
    fs::write(&blocked_parent, b"block portable vault commit")
        .map_err(|error| format!("create portable vault commit blocker failed: {error}"))?;
    {
        let mut stronghold = context
            .stronghold
            .lock()
            .map_err(|error| error.to_string())?;
        stronghold.as_mut().unwrap().path =
            SnapshotPath::from_path(blocked_parent.join(PORTABLE_VAULT_FILE_NAME));
    }
    let attempted_ref = config.stale_secret_ref();
    let error = write_secret_to_portable_vault_in(&context, &attempted_ref, &config.rotated_secret)
        .unwrap_err();
    if !error.contains("保存 portable vault snapshot 失败") {
        return Err(format!(
            "portable vault commit failure returned the wrong error: {error}"
        ));
    }
    drop(context);
    let reopened = open_portable_vault_probe_context(&config)?;
    expect_portable_vault_probe_secret(&reopened, &config.secret_ref, &config.secret)?;
    expect_portable_vault_probe_missing(&reopened, &attempted_ref)
}

fn verify_portable_vault_cross_process_conflict(root: &Path) -> Result<(), String> {
    let config = PortableVaultProbeConfig::new(root.join("cross-process-conflict"), "conflict");
    let stale = open_portable_vault_probe_context(&config)?;
    write_secret_to_portable_vault_in(&stale, &config.secret_ref, &config.secret)?;
    run_portable_vault_probe_child("conflict-update", &config)?;
    let stale_ref = config.stale_secret_ref();
    let error =
        write_secret_to_portable_vault_in(&stale, &stale_ref, &config.rotated_secret).unwrap_err();
    if !error.contains("另一 PortMate 实例修改") {
        return Err(format!(
            "portable vault cross-process conflict returned the wrong error: {error}"
        ));
    }
    drop(stale);
    let reopened = open_portable_vault_probe_context(&config)?;
    expect_portable_vault_probe_secret(&reopened, &config.secret_ref, &config.secret)?;
    expect_portable_vault_probe_secret(
        &reopened,
        &config.child_secret_ref(),
        &config.rotated_secret,
    )?;
    expect_portable_vault_probe_missing(&reopened, &stale_ref)
}

#[cfg(unix)]
fn verify_portable_vault_unix_file_security(root: &Path) -> Result<(), String> {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let config = PortableVaultProbeConfig::new(root.join("unix-private"), "unix-private");
    let context = open_portable_vault_probe_context(&config)?;
    write_secret_to_portable_vault_in(&context, &config.secret_ref, &config.secret)?;
    drop(context);
    for (path, expected_mode) in [
        (config.root.clone(), 0o700),
        (config.snapshot_path(), 0o600),
        (config.salt_path(), 0o600),
        (config.lock_path(), 0o600),
    ] {
        let mode = fs::symlink_metadata(&path)
            .map_err(|error| format!("inspect portable vault permissions failed: {error}"))?
            .permissions()
            .mode()
            & 0o777;
        if mode != expected_mode {
            return Err(format!(
                "portable vault path {} has mode {mode:o}, expected {expected_mode:o}",
                path.display()
            ));
        }
    }

    let protected = root.join("portable-vault-protected.txt");
    fs::write(&protected, b"must remain unchanged")
        .map_err(|error| format!("write portable vault protected file failed: {error}"))?;
    for (label, target) in [
        ("snapshot", PORTABLE_VAULT_FILE_NAME),
        ("salt", PORTABLE_VAULT_SALT_FILE_NAME),
        ("lock", "portmate-vault.hold.lock"),
    ] {
        let linked = PortableVaultProbeConfig::new(root.join(format!("unix-{label}-link")), label);
        fs::create_dir_all(&linked.root)
            .map_err(|error| format!("create portable vault symlink root failed: {error}"))?;
        symlink(&protected, linked.root.join(target))
            .map_err(|error| format!("create portable vault {label} symlink failed: {error}"))?;
        if open_portable_vault(
            &linked.snapshot_path(),
            &linked.salt_path(),
            &linked.password,
        )
        .is_ok()
        {
            return Err(format!("portable vault accepted a {label} symlink"));
        }
        if fs::read(&protected).ok().as_deref() != Some(b"must remain unchanged".as_slice()) {
            return Err(format!(
                "portable vault {label} symlink modified the protected target"
            ));
        }
    }

    for (label, target) in [
        ("snapshot", PORTABLE_VAULT_FILE_NAME),
        ("salt", PORTABLE_VAULT_SALT_FILE_NAME),
        ("lock", "portmate-vault.hold.lock"),
    ] {
        let hard_linked =
            PortableVaultProbeConfig::new(root.join(format!("unix-{label}-hard-link")), label);
        fs::create_dir_all(&hard_linked.root)
            .map_err(|error| format!("create portable vault hard-link root failed: {error}"))?;
        fs::hard_link(&protected, hard_linked.root.join(target))
            .map_err(|error| format!("create portable vault {label} hard link failed: {error}"))?;
        if open_portable_vault(
            &hard_linked.snapshot_path(),
            &hard_linked.salt_path(),
            &hard_linked.password,
        )
        .is_ok()
        {
            return Err(format!(
                "portable vault accepted a multiply-linked {label} file"
            ));
        }
        if fs::read(&protected).ok().as_deref() != Some(b"must remain unchanged".as_slice()) {
            return Err(format!(
                "portable vault {label} hard-link probe modified the protected target"
            ));
        }
    }

    let real_parent = root.join("unix-real-parent");
    fs::create_dir_all(&real_parent)
        .map_err(|error| format!("create portable vault real parent failed: {error}"))?;
    let linked_parent = root.join("unix-parent-link");
    symlink(&real_parent, &linked_parent)
        .map_err(|error| format!("create portable vault parent symlink failed: {error}"))?;
    let parent_linked = PortableVaultProbeConfig::new(linked_parent, "parent-link");
    if open_portable_vault(
        &parent_linked.snapshot_path(),
        &parent_linked.salt_path(),
        &parent_linked.password,
    )
    .is_ok()
    {
        return Err("portable vault accepted a symlinked parent directory".to_string());
    }
    Ok(())
}

fn required_portable_vault_probe_environment(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{name} is missing"))
}

fn portable_vault_probe_secret(label: &str) -> String {
    let mut secret = format!("portmate-portable-vault-{label}-{}", Uuid::new_v4());
    secret.extend(std::iter::repeat_n(
        'x',
        PORTABLE_VAULT_PROBE_SECRET_BYTES - secret.len(),
    ));
    secret
}

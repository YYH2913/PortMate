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

    #[cfg(unix)]
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
    #[cfg(windows)]
    verify_portable_vault_windows_file_security(temp.path()).unwrap();

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

#[cfg(windows)]
fn verify_portable_vault_windows_file_security(root: &Path) -> Result<(), String> {
    let protected = root.join("portable-vault-windows-protected.txt");
    fs::write(&protected, b"must remain unchanged")
        .map_err(|error| format!("write portable vault protected file failed: {error}"))?;
    for (label, target) in [
        ("snapshot", PORTABLE_VAULT_FILE_NAME),
        ("salt", PORTABLE_VAULT_SALT_FILE_NAME),
        ("lock", "portmate-vault.hold.lock"),
    ] {
        let hard_linked =
            PortableVaultProbeConfig::new(root.join(format!("windows-{label}-hard-link")), label);
        fs::create_dir_all(&hard_linked.root)
            .map_err(|error| format!("create portable vault hard-link root failed: {error}"))?;
        let linked_path = hard_linked.root.join(target);
        fs::hard_link(&protected, &linked_path)
            .map_err(|error| format!("create portable vault {label} hard link failed: {error}"))?;
        let error = portable_vault_file_exists(&linked_path, label)
            .expect_err("portable vault accepted a multiply-linked Windows file");
        if !error.contains("多个硬链接") {
            return Err(format!(
                "portable vault {label} hard-link probe returned the wrong error: {error}"
            ));
        }
        if fs::read(&protected).ok().as_deref() != Some(b"must remain unchanged".as_slice()) {
            return Err(format!(
                "portable vault {label} hard-link probe modified the protected target"
            ));
        }
    }

    let real_parent = root.join("windows-real-parent");
    fs::create_dir_all(&real_parent)
        .map_err(|error| format!("create portable vault real parent failed: {error}"))?;
    let linked_parent = root.join("windows-parent-junction");
    let result = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&linked_parent)
        .arg(&real_parent)
        .output()
        .map_err(|error| format!("start Windows junction command failed: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "create portable vault Windows parent junction failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    let parent_linked = PortableVaultProbeConfig::new(linked_parent, "parent-junction");
    let error = open_portable_vault(
        &parent_linked.snapshot_path(),
        &parent_linked.salt_path(),
        &parent_linked.password,
    )
    .err()
    .expect("portable vault accepted a Windows parent junction");
    if !error.contains("真实目录") {
        return Err(format!(
            "portable vault parent-junction probe returned the wrong error: {error}"
        ));
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

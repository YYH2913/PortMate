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

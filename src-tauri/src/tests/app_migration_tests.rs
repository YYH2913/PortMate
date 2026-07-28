use super::*;

#[test]
fn legacy_app_identifier_data_directory_migrates_atomically() {
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join(LEGACY_APP_IDENTIFIER);
    let current = root.path().join("dev.portmate.desktop");
    fs::create_dir_all(legacy.join("logs")).unwrap();
    fs::write(legacy.join(STORE_FILE_NAME), b"store").unwrap();
    fs::write(legacy.join("logs/session.txt"), b"log").unwrap();
    fs::create_dir_all(current.join("mediakeys/v1")).unwrap();
    fs::write(current.join("mediakeys/v1/salt"), b"bootstrap").unwrap();

    migrate_legacy_app_data_dir(root.path(), &current).unwrap();

    assert!(!legacy.exists());
    assert_eq!(fs::read(current.join(STORE_FILE_NAME)).unwrap(), b"store");
    assert_eq!(fs::read(current.join("logs/session.txt")).unwrap(), b"log");
}

#[test]
fn legacy_app_identifier_migration_refuses_to_merge_two_live_stores() {
    let root = tempfile::tempdir().unwrap();
    let legacy = root.path().join(LEGACY_APP_IDENTIFIER);
    let current = root.path().join("dev.portmate.desktop");
    fs::create_dir_all(&legacy).unwrap();
    fs::create_dir_all(&current).unwrap();
    fs::write(legacy.join(STORE_FILE_NAME), b"legacy").unwrap();
    fs::write(current.join(STORE_FILE_NAME), b"current").unwrap();

    let error = migrate_legacy_app_data_dir(root.path(), &current).unwrap_err();

    assert!(error.contains("refusing to merge"), "{error}");
    assert_eq!(fs::read(legacy.join(STORE_FILE_NAME)).unwrap(), b"legacy");
    assert_eq!(fs::read(current.join(STORE_FILE_NAME)).unwrap(), b"current");
}

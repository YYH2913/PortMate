use super::*;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[test]
fn native_no_replace_rename_handles_missing_targets_and_never_swaps_existing_entries() {
    for directory in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let existing = root.path().join("existing");
        let target = root.path().join("new-target");
        if directory {
            fs::create_dir(&source).unwrap();
            fs::write(source.join("payload"), b"source").unwrap();
            fs::create_dir(&existing).unwrap();
            fs::write(existing.join("payload"), b"existing").unwrap();
        } else {
            fs::write(&source, b"source").unwrap();
            fs::write(&existing, b"existing").unwrap();
        }
        assert!(rename_local_path_without_overwrite(&source, &existing).is_err());
        let payload = |path: &Path| {
            if directory {
                path.join("payload")
            } else {
                path.to_path_buf()
            }
        };
        assert_eq!(fs::read(payload(&source)).unwrap(), b"source");
        assert_eq!(fs::read(payload(&existing)).unwrap(), b"existing");
        rename_local_path_without_overwrite(&source, &target).unwrap();
        assert!(!source.exists());
        assert_eq!(fs::read(payload(&target)).unwrap(), b"source");
        assert_eq!(fs::read(payload(&existing)).unwrap(), b"existing");
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn native_no_replace_rename_moves_only_the_final_symlink() {
    let root = tempfile::tempdir().unwrap();
    let protected = root.path().join("protected");
    let link = root.path().join("link");
    let target = root.path().join("renamed-link");
    fs::write(&protected, b"protected").unwrap();
    std::os::unix::fs::symlink(&protected, &link).unwrap();
    assert!(rename_local_path_without_overwrite(&link, &protected).is_err());
    assert_eq!(fs::read_link(&link).unwrap(), protected);
    rename_local_path_without_overwrite(&link, &target).unwrap();
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(fs::read_link(&target).unwrap(), protected);
    assert_eq!(fs::read(&protected).unwrap(), b"protected");
}

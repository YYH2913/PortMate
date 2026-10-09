#[test]
fn file_manager_local_paths_reject_foreign_roots_and_filesystem_roots() {
    let foreign = if cfg!(windows) {
        "/tmp/portmate-foreign"
    } else {
        r"C:\Users\operator\foreign"
    };
    assert!(validate_native_local_path(foreign).is_err());
    assert!(validate_local_mutating_path(foreign).is_err());
    assert!(validate_local_drop_destination(foreign).is_err());
    assert!(list_local_files(foreign).is_err());
    assert!(local_file_properties(foreign).is_err());

    let filesystem_root = if cfg!(windows) { r"C:\" } else { "/" };
    assert!(validate_local_mutating_path(filesystem_root).is_err());
    assert!(validate_local_mutating_path("~").is_err());
    assert_eq!(
        validate_native_local_path("nested/child").unwrap(),
        expand_identity_path("nested/child")
    );
    assert!(validate_local_mutating_path("nested/../child").is_err());
    assert!(validate_local_mutating_path("nested/./child").is_err());
}

#[test]
fn local_tilde_paths_follow_native_home_and_separator_rules() {
    let home = Path::new("/home/operator");
    assert_eq!(expand_identity_path_with_home("~", Some(home), false), home);
    assert_eq!(
        expand_identity_path_with_home("~/.ssh/id_ed25519", Some(home), false),
        home.join(".ssh/id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home("~//.ssh/id_ed25519", Some(home), false),
        home.join(".ssh/id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home(r"~\.ssh\id_ed25519", Some(home), true),
        home.join(r".ssh\id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home(r"~\\.ssh\id_ed25519", Some(home), true),
        home.join(r".ssh\id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home(r"~/C:\Windows", Some(home), true),
        PathBuf::from(r"~/C:\Windows")
    );
    assert_eq!(
        expand_identity_path_with_home(r"~\.ssh\id_ed25519", Some(home), false),
        PathBuf::from(r"~\.ssh\id_ed25519")
    );
    assert_eq!(
        expand_identity_path_with_home("~/relative", None, true),
        PathBuf::from("~/relative")
    );
}

#[test]
fn local_mutations_protect_the_platform_native_home_directory() {
    let root = tempfile::tempdir().unwrap();
    let unix_home = root.path().join("unix-home");
    let windows_profile = root.path().join("windows-profile");
    fs::create_dir_all(&unix_home).unwrap();
    fs::create_dir_all(&windows_profile).unwrap();

    let windows_home =
        preferred_native_home_path(Some(unix_home.clone()), Some(windows_profile.clone()), true)
            .unwrap();
    assert_eq!(windows_home, windows_profile);
    assert!(validate_local_mutating_path_with_home(
        windows_profile.to_str().unwrap(),
        Some(&windows_home),
    )
    .unwrap_err()
    .contains("用户主目录"));
    assert!(validate_local_mutating_path_with_home(
        unix_home.to_str().unwrap(),
        Some(&windows_home),
    )
    .is_ok());

    let unix_native_home = preferred_native_home_path(
        Some(unix_home.clone()),
        Some(windows_profile.clone()),
        false,
    )
    .unwrap();
    assert_eq!(unix_native_home, unix_home);
    assert!(validate_local_mutating_path_with_home(
        unix_home.to_str().unwrap(),
        Some(&unix_native_home),
    )
    .unwrap_err()
    .contains("用户主目录"));
    assert_eq!(
        preferred_native_home_path(None, Some(windows_profile.clone()), false),
        Some(windows_profile.clone())
    );
    assert_eq!(
        preferred_native_home_path(Some(unix_home.clone()), None, true),
        Some(unix_home)
    );
}

#[test]
fn file_manager_remote_mutating_paths_reject_parent_components() {
    for path in [
        "/tmp/..",
        "/tmp/./file",
        "nested/../outside",
        "../outside",
        "//",
        "~",
    ] {
        assert!(
            validate_remote_mutating_path(path).is_err(),
            "unsafe remote path was accepted: {path}"
        );
        assert!(normalize_remote_batch_source(path).is_err());
        assert!(validate_remote_drop_destination(path).is_err());
    }
    assert!(validate_remote_mutating_path("/").is_err());
    assert!(normalize_remote_batch_source("/").is_err());
    assert!(validate_remote_drop_destination("/").is_ok());
    assert!(validate_remote_drop_destination("/tmp/portmate/").is_ok());
    assert_eq!(
        validate_remote_mutating_path("/tmp/portmate/file").unwrap(),
        "/tmp/portmate/file"
    );
}

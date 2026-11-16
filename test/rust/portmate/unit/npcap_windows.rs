use super::*;

#[test]
fn interactive_setup_has_no_silent_switches_and_pins_the_official_installer() {
    assert!(LAUNCH_INTERACTIVE.contains("-Verb RunAs -PassThru -Wait"));
    assert!(!LAUNCH_INTERACTIVE.contains("-ArgumentList"));
    assert!(!LAUNCH_INTERACTIVE.contains("/S"));
    assert!(LAUNCH_INTERACTIVE.contains(setup::INSTALLER_SHA256));
    assert!(VERIFY_SIGNATURE.contains("Get-AuthenticodeSignature -LiteralPath"));
    assert!(VERIFY_SIGNATURE.contains("'Valid'"));
    assert!(VERIFY_SIGNATURE.contains("-cnotcontains $publisher"));
}

#[tokio::test]
async fn a_system_executable_is_not_accepted_as_an_npcap_publisher() {
    let executable = system_directory()
        .unwrap()
        .join("WindowsPowerShell/v1.0/powershell.exe");
    let _file = pin_file(&executable).unwrap();
    assert_eq!(
        verify_signature(&executable).await.unwrap_err().code,
        "npcap-verification"
    );
}

#[test]
fn pinned_files_deny_writes_until_the_lease_is_released() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("local-test.dll");
    fs::write(&path, b"fixture").unwrap();
    let file = pin_file(&path).unwrap();
    assert!(OpenOptions::new().write(true).open(&path).is_err());
    drop(file);
    assert!(OpenOptions::new().write(true).open(&path).is_ok());
}

#[tokio::test]
async fn setup_is_owner_bound_and_installation_blocks_networking_and_app_updates() {
    let directory = tempfile::tempdir().unwrap();
    let installer_root = tempfile::tempdir_in(directory.path()).unwrap();
    let path = installer_root.path().join("never-execute.exe");
    fs::write(&path, b"test-only, not executable").unwrap();
    let installer = DownloadedInstaller {
        owner: "main".into(),
        path: path.clone(),
        _file: pin_file(&path).unwrap(),
        _pin: BoundDirectory::open(installer_root.path()).unwrap(),
        _directory: installer_root,
    };
    let shared = Arc::new(Mutex::new(SetupState {
        owner: Some("main".into()),
        phase: "downloaded",
        installer: Some(installer),
        ..SetupState::default()
    }));
    let state = crate::tests::test_app_state(
        crate::tests::test_shell_profile(),
        directory.path().join("store.sqlite3"),
    );
    let runtime = EasyTierRuntime::default();
    assert_eq!(
        launch_installer("workspace-other", Arc::clone(&shared), &state, &runtime)
            .await
            .unwrap_err()
            .code,
        "npcap-setup"
    );
    assert!(shared.lock().unwrap().installer.is_some());
    let dependency = NpcapRuntime {
        state: Arc::clone(&shared),
    };
    dependency.cancel_owner("workspace-other");
    assert!(path.exists());
    dependency.cancel_owner("main");
    assert!(!path.exists());
    shared.lock().unwrap().phase = "installing";
    assert_eq!(
        dependency.ensure_idle().unwrap_err().code,
        "npcap-installing"
    );
}

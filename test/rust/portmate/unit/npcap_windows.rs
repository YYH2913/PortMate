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

fn fake_system(root: &Path) -> PathBuf {
    let system = root.join("Windows/System32");
    fs::create_dir_all(system.join("Npcap")).unwrap();
    fs::create_dir_all(system.join("drivers")).unwrap();
    fs::write(system.join("Npcap/Packet.dll"), b"local-fixture-only").unwrap();
    fs::write(system.join("drivers/npcap.sys"), b"local-fixture-only").unwrap();
    system
}

fn fake_run(root: &Path, system: PathBuf) -> RunLease {
    let directory = tempfile::tempdir_in(root).unwrap();
    RunLease {
        installation: pin_installation(system).unwrap(),
        _binaries: Vec::new(),
        _pin: BoundDirectory::open(directory.path()).unwrap(),
        _directory: directory,
    }
}

#[test]
fn loads_installed_npcap_in_place_without_an_application_local_copy() {
    let root = tempfile::tempdir().unwrap();
    let system = fake_system(root.path());
    let run = fake_run(root.path(), system.clone());
    let mut command = tokio::process::Command::new("never-execute.exe");
    command.env("PATH", "untrusted-inherited-directory");
    run.configure_command(&mut command, run._directory.path())
        .unwrap();
    let path = command
        .as_std()
        .get_envs()
        .find(|(name, _)| *name == "PATH")
        .unwrap()
        .1
        .unwrap();
    assert_eq!(
        std::env::split_paths(path).collect::<Vec<_>>(),
        vec![system.join("Npcap"), system.clone()]
    );
    assert_eq!(command.as_std().get_current_dir(), Some(system.as_path()));
    assert!(!run._directory.path().join("Packet.dll").exists());
    assert!(OpenOptions::new()
        .write(true)
        .open(system.join("Npcap/Packet.dll"))
        .is_err());
    fs::write(run._directory.path().join("Packet.dll"), b"shadow").unwrap();
    assert_eq!(
        run.configure_command(&mut command, run._directory.path())
            .unwrap_err()
            .code,
        "npcap-conflict"
    );
}

#[test]
fn rejects_conflicting_system_dlls_and_search_path_changes() {
    let root = tempfile::tempdir().unwrap();
    let system = fake_system(root.path());
    let shadow = system.join("Packet.dll");
    fs::write(&shadow, b"conflicting-winpcap").unwrap();
    assert!(
        matches!(pin_installation(system.clone()), Err(error) if error.code == "npcap-conflict")
    );
    fs::write(&shadow, b"local-fixture-only").unwrap();
    let run = fake_run(root.path(), system.clone());
    let mut command = tokio::process::Command::new("never-execute.exe");
    run.configure_command(&mut command, run._directory.path())
        .unwrap();
    assert!(OpenOptions::new().write(true).open(&shadow).is_err());
    fs::write(system.parent().unwrap().join("Packet.dll"), b"new-shadow").unwrap();
    assert_eq!(
        run.configure_command(&mut command, run._directory.path())
            .unwrap_err()
            .code,
        "npcap-conflict"
    );
}

#[test]
fn directory_redirection_cannot_replace_the_pinned_installation() {
    let root = tempfile::tempdir().unwrap();
    let system = fake_system(root.path());
    let run = fake_run(root.path(), system.clone());
    assert!(fs::rename(system.join("Npcap"), system.join("Npcap-old")).is_err());
    let mut command = tokio::process::Command::new("never-execute.exe");
    fs::write(
        run._directory.path().join("easytier-core.exe.local"),
        b"redirect",
    )
    .unwrap();
    assert!(run
        .configure_command(&mut command, run._directory.path())
        .is_err());
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

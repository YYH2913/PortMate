use super::*;

fn elevated_profile() -> SessionProfile {
    let mut profile = crate::tests::test_shell_profile();
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        unreachable!()
    };
    shell.elevated = true;
    profile
}

#[test]
fn saved_shell_preference_is_not_manual_desktop_consent() {
    let profile = elevated_profile();
    assert_eq!(
        authorize_launch(&profile, None).unwrap_err(),
        MANUAL_CONSENT_REQUIRED
    );
    let ConnectionConfig::Shell(shell) = &profile.connection else {
        unreachable!()
    };
    assert!(authorize_launch(&profile, Some(shell)).is_ok());
    let mut changed = shell.clone();
    for field in ["program", "args", "cwd", "elevated"] {
        match field {
            "program" => changed.program.push_str("-different"),
            "args" => changed.args.push("--different".into()),
            "cwd" => changed.cwd = Some("different".into()),
            _ => changed.elevated = false,
        }
        assert_eq!(
            authorize_launch(&profile, Some(&changed)).unwrap_err(),
            CONFIGURATION_CHANGED
        );
        changed = shell.clone();
    }
    let mut other = crate::tests::test_shell_profile();
    other.connection = ConnectionConfig::Tcp(TcpConnection::default());
    assert_eq!(
        authorize_launch(&other, Some(shell)).unwrap_err(),
        CONFIGURATION_CHANGED
    );
    assert!(authorize_launch(&crate::tests::test_shell_profile(), None).is_ok());
    assert_eq!(
        authorize_launch(&crate::tests::test_shell_profile(), Some(shell)).unwrap_err(),
        CONFIGURATION_CHANGED
    );
}

#[test]
fn old_profiles_default_to_normal_and_elevated_profiles_roundtrip() {
    let shell: ShellConnection = serde_json::from_value(serde_json::json!({
        "program": "", "args": [], "cwd": null,
    }))
    .unwrap();
    assert!(!shell.elevated);
    let ConnectionConfig::Shell(shell) = elevated_profile().connection else {
        unreachable!()
    };
    let json = serde_json::to_value(&shell).unwrap();
    assert_eq!(json["elevated"], true);
    assert_eq!(
        serde_json::from_value::<ShellConnection>(json).unwrap(),
        shell
    );
}

#[test]
fn generic_open_cannot_authorize_elevation_or_change_runtime_status() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let profile = elevated_profile();
        let state =
            crate::tests::test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        let before = state.store.lock().unwrap().summaries()[0].runtime.status;
        assert!(
            serde_json::from_value::<crate::OpenSessionRequest>(serde_json::json!({
                "sessionId": profile.id, "authorizeShellElevation": true,
            }))
            .is_err()
        );
        let request: crate::OpenSessionRequest = serde_json::from_value(serde_json::json!({
            "sessionId": profile.id, "credentialHandle": null,
        }))
        .unwrap();
        let error = crate::session_open::open_session_inner(
            state.clone(),
            request.session_id,
            crate::session_open::SessionOpenCredentials::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, MANUAL_CONSENT_REQUIRED);
        assert!(state.shell.lock().unwrap().is_empty());
        assert_eq!(
            state.store.lock().unwrap().summaries()[0].runtime.status,
            before
        );
        assert!(crate::shell_transport::prepare_shell_session(profile).is_err());
    });
}

#[test]
fn privilege_wrappers_keep_each_argument_without_shell_interpolation() {
    let args: Vec<String> = [
        "",
        "two words",
        "a\"b",
        "C:\\path with spaces\\",
        "& echo injected",
        "; touch /tmp/no",
        "$HOME",
        "--new",
        "中文",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let program = Path::new("C:\\Program Files\\PowerShell\\7\\pwsh.exe");
    let windows = windows_arguments(program, &args);
    assert_eq!(
        &windows[..7],
        [
            "--direct",
            "--inline",
            "--close",
            "--wait",
            "--integrity",
            "High",
            "--"
        ]
        .map(OsString::from)
    );
    assert_eq!(windows[7], program.as_os_str());
    assert_eq!(
        &windows[8..],
        &args.iter().map(OsString::from).collect::<Vec<_>>()
    );
    let sudo = sudo_arguments(Path::new("/bin/sh"), &args);
    assert_eq!(
        &sudo[..7],
        [
            "-k",
            "-H",
            "-u",
            "root",
            "-p",
            "PortMate sudo password: ",
            "--"
        ]
        .map(OsString::from)
    );
    assert_eq!(sudo[7], "/bin/sh");
    assert_eq!(
        &sudo[8..],
        &args.iter().map(OsString::from).collect::<Vec<_>>()
    );
}

#[test]
fn relative_executable_paths_cannot_resolve_against_session_working_directory() {
    for path in ["./cmd.exe", "nested/shell", "../sh"] {
        assert!(resolve_elevated_program(Path::new(path)).is_err(), "{path}");
    }
    let root = tempfile::tempdir().unwrap();
    assert!(validate_elevated_program(root.path()).is_err());
    assert!(validate_elevated_program(&root.path().join("missing.exe")).is_err());
}

#[cfg(unix)]
#[test]
fn unix_elevation_uses_only_system_sudo_and_rejects_nonexecutables() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("not-executable");
    fs::write(&file, "#!/bin/sh\n").unwrap();
    assert!(validate_elevated_program(&file).is_err());
    let program = validate_elevated_program(Path::new("/bin/sh")).unwrap();
    assert!(program.is_absolute());
    let result = prepare_launch(
        &ShellLaunchPaths { program, cwd: None },
        &["-l".into()],
        None,
    );
    if Path::new("/usr/bin/sudo").exists() || Path::new("/bin/sudo").exists() {
        let plan = result.unwrap();
        assert!(
            [Path::new("/usr/bin/sudo"), Path::new("/bin/sudo")].contains(&plan.program.as_path())
        );
        assert!(plan.resources.is_none());
        assert_eq!(plan.args.last().unwrap(), "-l");
    } else {
        assert!(result.err().unwrap().contains("sudo"));
    }
}

#[cfg(unix)]
#[test]
fn unix_elevation_preserves_shell_symlink_name_and_argv_zero_semantics() {
    let root = tempfile::tempdir().unwrap();
    let alias = root.path().join("rbash");
    std::os::unix::fs::symlink("/bin/sh", &alias).unwrap();
    // Canonicalizing sh/rbash to bash can change POSIX or restricted mode.
    assert_eq!(validate_elevated_program(&alias).unwrap(), alias);
}

#[cfg(windows)]
#[test]
fn windows_elevation_rejects_network_device_and_script_targets() {
    for path in [
        r"\\server\share\cmd.exe",
        r"\\?\UNC\server\share\cmd.exe",
        r"\\.\C:\cmd.exe",
        r"C:cmd.exe",
        "cmd.exe",
    ] {
        assert!(
            require_local_windows_path(Path::new(path)).is_err(),
            "{path}"
        );
    }
    assert!(require_local_windows_path(Path::new(r"C:\Windows\System32\cmd.exe")).is_ok());
    let root = tempfile::tempdir().unwrap();
    for extension in ["bat", "cmd", "ps1"] {
        let script = root.path().join(format!("script.{extension}"));
        fs::write(&script, "echo should not run").unwrap();
        assert!(validate_elevated_program(&script).is_err());
    }
    assert!(resolve_elevated_program(Path::new("cmd.exe"))
        .unwrap()
        .is_absolute());
}

#[cfg(windows)]
#[test]
fn windows_helper_pinning_rejects_tampering_and_denies_replacement() {
    let root = tempfile::tempdir().unwrap();
    let helper = root.path().join("helper.exe");
    fs::write(&helper, b"MZfixture").unwrap();
    let digest = portmate_core::encode_hex(&Sha256::digest(b"MZfixture"));
    assert!(pin_verified_helper(&helper, "bad digest").is_err());
    let pinned = pin_verified_helper(&helper, &digest).unwrap();
    assert!(fs::write(&helper, b"tampered").is_err());
    assert!(fs::remove_file(&helper).is_err());
    assert!(fs::rename(&helper, root.path().join("replaced.exe")).is_err());
    drop(pinned);
    fs::remove_file(helper).unwrap();
}

#[cfg(all(windows, feature = "bundled-shell-elevation"))]
#[test]
fn bundled_helper_is_verified_offline_and_lease_removes_its_directory() {
    let root = tempfile::tempdir().unwrap();
    let (helper, resources) = materialize_helper(Some(root.path())).unwrap();
    assert_eq!(fs::read(&helper).unwrap(), HELPER_BYTES);
    assert!(helper.with_file_name("gsudo-MIT.txt").is_file());
    assert!(OpenOptions::new().write(true).open(&helper).is_err());
    let directory = helper.parent().unwrap().to_path_buf();
    drop(resources);
    assert!(!directory.exists());
}

#[cfg(all(windows, feature = "bundled-shell-elevation"))]
#[test]
fn bundled_helper_runs_without_installation_or_elevation_for_version_query() {
    use std::os::windows::process::CommandExt;
    let root = tempfile::tempdir().unwrap();
    let (helper, _lease) = materialize_helper(Some(root.path())).unwrap();
    // Never trigger interactive UAC in CI. This still verifies PE architecture,
    // loader compatibility, the offline payload and sharing-compatible execution.
    let output = std::process::Command::new(helper)
        .arg("--version")
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("2.6.1"),
        "{output:?}"
    );
}

#[cfg(windows)]
#[test]
fn windows_elevated_cwd_is_local_and_does_not_trigger_gsudo_network_wrapping() {
    let root = tempfile::tempdir().unwrap();
    let dos = local_windows_working_directory(root.path()).unwrap();
    assert!(dos.is_dir() && dos.is_absolute());
    assert!(!dos.to_string_lossy().starts_with(r"\\"));
    assert_eq!(
        fs::canonicalize(&dos).unwrap(),
        fs::canonicalize(root.path()).unwrap()
    );
}

#[test]
fn authorized_but_unavailable_elevated_shell_records_consent_and_never_falls_back() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let mut profile = elevated_profile();
        let ConnectionConfig::Shell(shell) = &mut profile.connection else {
            unreachable!()
        };
        shell.program = root.path().join("missing-shell.exe").display().to_string();
        shell.args = vec!["sensitive-argument-must-not-enter-audit".into()];
        let consent = shell.clone();
        let state =
            crate::tests::test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        let error = crate::session_open::open_session_inner(
            state.clone(),
            profile.id.clone(),
            crate::session_open::SessionOpenCredentials {
                authorized_shell: Some(consent),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("executable is unavailable"), "{error}");
        assert!(state.shell.lock().unwrap().is_empty());
        let store = state.store.lock().unwrap();
        assert_eq!(store.summaries()[0].runtime.status, SessionStatus::Error);
        let audit = store
            .audit
            .iter()
            .find(|record| record.action == "open_elevated_shell")
            .unwrap();
        assert_eq!(audit.session_id.as_deref(), Some(profile.id.as_str()));
        assert_eq!(audit.actor, "desktop-user");
        assert_eq!(audit.decision, "authorized");
        assert!(!serde_json::to_string(audit)
            .unwrap()
            .contains("sensitive-argument"));
    });
}

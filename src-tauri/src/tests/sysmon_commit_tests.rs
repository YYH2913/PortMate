#[test]
fn sysmon_commit_failure_rolls_back_snapshot_and_success_event() {
    let root =
        std::env::temp_dir().join(format!("portmate-sysmon-commit-failure-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let blocked_parent = root.join("not-a-directory");
    fs::write(&blocked_parent, b"blocked").unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(
        profile.clone(),
        blocked_parent.join("portmate-store.sqlite3"),
    );

    let error =
        commit_sysmon_snapshot(&state, &profile.id, test_sysmon_snapshot(&profile.id)).unwrap_err();

    assert!(error.contains("无法判定 Store 提交是否生效"), "{error}");
    let store = state.store.lock().unwrap();
    assert!(store.sysmon.is_empty());
    assert!(store.events.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn sysmon_commit_rejects_a_profile_deleted_during_collection() {
    let profile = test_shell_profile();
    let state = test_app_state(
        profile.clone(),
        PathBuf::from("sysmon-deleted-profile-test.sqlite3"),
    );
    {
        let mut store = state.store.lock().unwrap();
        store.profiles.clear();
        store.runtimes.clear();
    }

    let error =
        commit_sysmon_snapshot(&state, &profile.id, test_sysmon_snapshot(&profile.id)).unwrap_err();

    assert!(error.contains("unknown session"));
    let store = state.store.lock().unwrap();
    assert!(store.sysmon.is_empty());
    assert!(store.events.is_empty());
}

#[test]
fn sysmon_commit_rejects_a_local_sample_after_the_profile_becomes_remote() {
    let local_profile = test_shell_profile();
    let state = test_app_state(
        local_profile.clone(),
        PathBuf::from("sysmon-target-change-test.sqlite3"),
    );
    let mut remote_profile = test_ssh_profile();
    remote_profile.id = local_profile.id.clone();
    remote_profile.name = local_profile.name.clone();
    {
        let mut store = state.store.lock().unwrap();
        store.upsert_profile(remote_profile);
    }

    let error = commit_sysmon_snapshot(
        &state,
        &local_profile.id,
        test_sysmon_snapshot(&local_profile.id),
    )
    .unwrap_err();

    assert!(error.contains("目标已从本机变为 SSH"), "{error}");
    let store = state.store.lock().unwrap();
    assert!(store.sysmon.is_empty());
    assert!(store.events.is_empty());
}

#[test]
fn sysmon_commit_rejects_a_missing_or_replaced_ssh_runtime() {
    let profile = test_ssh_profile();
    let state = test_app_state(
        profile.clone(),
        PathBuf::from("sysmon-runtime-change-test.sqlite3"),
    );

    let error = commit_sysmon_snapshot_for_target(
        &state,
        &profile.id,
        test_sysmon_snapshot(&profile.id),
        &SysmonCollectionTarget::Ssh("runtime-before-sample".to_string()),
    )
    .unwrap_err();

    assert!(error.contains("runtime 在 Sysmon 采样期间已变化"), "{error}");
    let store = state.store.lock().unwrap();
    assert!(store.sysmon.is_empty());
    assert!(store.events.is_empty());
}

#[test]
fn sysmon_collection_target_requires_the_exact_ssh_runtime() {
    let profile = test_ssh_profile();
    let target = SysmonCollectionTarget::Ssh("runtime-1".to_string());

    assert!(validate_sysmon_collection_target(&profile, Some("runtime-1"), &target).is_ok());
    let error = validate_sysmon_collection_target(&profile, Some("runtime-2"), &target)
        .unwrap_err();
    assert!(error.contains("runtime 在 Sysmon 采样期间已变化"), "{error}");
}

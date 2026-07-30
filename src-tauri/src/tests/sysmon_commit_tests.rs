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


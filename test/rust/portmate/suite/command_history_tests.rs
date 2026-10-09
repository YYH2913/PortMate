use super::*;
use crate::command_history_commands::{record_command_submission, CommandSubmissionSource};

#[test]
fn desktop_and_mcp_submissions_share_persisted_policy_and_canonical_history() {
    let temp = canonical_test_tempdir();
    let path = temp.path().join("history.sqlite3");
    let profile = test_shell_profile();
    let id = Some(profile.id.clone());
    let state = test_app_state(profile, path.clone());
    record_command_submission(
        &state,
        "ignored\r".into(),
        id.clone(),
        CommandSubmissionSource::McpSendText,
    )
    .unwrap();
    assert!(!path.exists(), "disabled history performed disk I/O");
    {
        let mut store = state.store.lock().unwrap();
        store.command_history_policy = portmate_core::CommandHistoryPolicy {
            enabled: true,
            limit: 2,
            retention_days: 1,
        };
    }
    record_command_submission(
        &state,
        "desktop".into(),
        id.clone(),
        CommandSubmissionSource::Interactive,
    )
    .unwrap();
    record_command_submission(
        &state,
        "mcp".into(),
        id.clone(),
        CommandSubmissionSource::McpRunCommand,
    )
    .unwrap();
    let before = fs::read(&path).unwrap();
    record_command_submission(
        &state,
        "partial".into(),
        id.clone(),
        CommandSubmissionSource::McpSendText,
    )
    .unwrap();
    assert_eq!(
        fs::read(&path).unwrap(),
        before,
        "unsubmitted text performed disk I/O"
    );
    record_command_submission(
        &state,
        "submitted\r\nunfinished".into(),
        id,
        CommandSubmissionSource::McpSendText,
    )
    .unwrap();
    let persisted = load_store_sqlite(&path).unwrap();
    assert_eq!(
        persisted
            .command_history
            .iter()
            .map(|e| e.command.as_str())
            .collect::<Vec<_>>(),
        ["submitted", "mcp"]
    );
    assert_eq!(persisted.command_history_policy.limit, 2);
    assert_eq!(persisted.command_history_policy.retention_days, 1);
    assert!(persisted.command_history_policy.enabled);
}

#[test]
fn history_persistence_failure_does_not_publish_an_in_memory_commit() {
    let temp = canonical_test_tempdir();
    let path = temp.path().join("directory.sqlite3");
    fs::create_dir(&path).unwrap();
    let profile = test_shell_profile();
    let id = Some(profile.id.clone());
    let state = test_app_state(profile, path);
    state.store.lock().unwrap().command_history_policy.enabled = true;
    assert!(
        record_command_submission(&state, "failed".into(), id, CommandSubmissionSource::Paste)
            .is_err()
    );
    assert!(state.store.lock().unwrap().command_history.is_empty());
    assert_eq!(state.store.lock().unwrap().command_history_revision, 0);
}

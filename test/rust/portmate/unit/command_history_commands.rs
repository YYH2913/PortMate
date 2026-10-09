use super::*;

#[test]
fn mcp_text_requires_complete_submissions_and_rejects_editor_input() {
    use CommandSubmissionSource::*;
    for text in [
        "echo partial",
        "\r",
        "   \n",
        "abc\x1b[D\n",
        "\x1b[200~one\ntwo\n\x1b[201~",
    ] {
        assert!(submitted_commands(text, McpSendText).is_empty(), "{text:?}");
    }
    assert_eq!(
        submitted_commands("one\r\ntwo\nunfinished", McpSendText),
        ["one", "two"]
    );
    assert_eq!(
        submitted_commands("echo submitted", McpRunCommand),
        ["echo submitted"]
    );
}

#[test]
fn all_submission_sources_share_disabled_limit_and_retention_policy() {
    let mut store = SessionStore::default();
    let now = 10 * 86_400_000;
    configure_history_policy(&mut store, true, 2, 1, now).unwrap();
    for (command, timestamp) in [
        ("expired", now - 2 * 86_400_000),
        ("first", now - 1),
        ("second", now),
        ("third", now),
    ] {
        append_submissions(
            &mut store,
            submitted_commands(command, CommandSubmissionSource::McpRunCommand),
            None,
            timestamp,
        )
        .unwrap();
    }
    assert_eq!(
        store
            .command_history
            .iter()
            .map(|e| e.command.as_str())
            .collect::<Vec<_>>(),
        ["third", "second"]
    );
    configure_history_policy(&mut store, false, 2, 1, now).unwrap();
    let revision = store.command_history_revision;
    for source in [
        CommandSubmissionSource::Interactive,
        CommandSubmissionSource::Paste,
        CommandSubmissionSource::FreeInput,
        CommandSubmissionSource::QuickCommand,
        CommandSubmissionSource::SendPanel,
        CommandSubmissionSource::McpRunCommand,
        CommandSubmissionSource::McpSendText,
        CommandSubmissionSource::SyncBroadcast,
    ] {
        append_submissions(
            &mut store,
            submitted_commands("private\r", source),
            None,
            now,
        )
        .unwrap();
    }
    assert!(store.command_history.is_empty());
    assert_eq!(store.command_history_revision, revision);
    let restored: SessionStore =
        serde_json::from_str(&serde_json::to_string(&store).unwrap()).unwrap();
    assert_eq!(
        restored.command_history_policy,
        store.command_history_policy
    );
}

#[test]
fn retention_zero_keeps_old_entries_and_policy_update_prunes_them() {
    let mut store = SessionStore::default();
    configure_history_policy(&mut store, true, 10, 0, 1).unwrap();
    assert!(!store.command_history_migrated);
    append_submissions(&mut store, vec!["old".into()], None, 1).unwrap();
    let now = 3 * 86_400_000;
    append_submissions(&mut store, vec!["new".into()], None, now).unwrap();
    assert_eq!(store.command_history.len(), 2);
    configure_history_policy(&mut store, true, 10, 1, now).unwrap();
    assert_eq!(store.command_history.len(), 1);
    assert_eq!(store.command_history[0].command, "new");
    let policy = store.command_history_policy.clone();
    assert!(configure_history_policy(&mut store, true, 0, 1, now).is_err());
    assert_eq!(store.command_history_policy, policy);
}

#[test]
fn empty_legacy_history_migration_is_persistence_free() {
    let store = SessionStore::default();
    assert!(should_skip_empty_migration(&store, &[]));

    let mut store_with_history = SessionStore::default();
    store_with_history
        .command_history
        .push(portmate_core::CommandHistoryEntry {
            command: "git status".to_string(),
            recorded_at: 1,
            session_id: None,
        });
    assert!(!should_skip_empty_migration(&store_with_history, &[]));

    let mut migrated_store = SessionStore::default();
    migrated_store.command_history_migrated = true;
    assert!(!should_skip_empty_migration(&migrated_store, &[]));
}

use super::*;

#[test]
fn export_command_types_keep_stable_serde_contract() {
    let request: ExportSessionBundleArchiveRequest =
        serde_json::from_value(serde_json::json!({ "sessionId": "shell-a" })).unwrap();
    assert_eq!(request.session_id, "shell-a");
    assert!(request.redact_secrets);
    assert!(!request.include_raw_logs);
    assert!(request.attachment_paths.is_empty());

    assert_eq!(
        serde_json::to_value(TerminalTextExportSource::Selection).unwrap(),
        serde_json::json!("selection")
    );
    assert_eq!(
        serde_json::from_value::<TerminalTextExportSource>(serde_json::json!("buffer")).unwrap(),
        TerminalTextExportSource::Buffer
    );

    let search: SearchLogShardsRequest = serde_json::from_value(serde_json::json!({
        "query": "disconnect"
    }))
    .unwrap();
    assert!(search.paths.is_empty());
    assert!(search.limit.is_none());
}

#[test]
fn mcp_audit_export_is_atomic_exact_and_checksummed() {
    let root = canonical_test_temp_path("portmate-mcp-audit-export");
    fs::create_dir_all(&root).unwrap();
    let audit = vec![
        AuditRecord {
            id: "audit-a".to_string(),
            ts: Utc::now(),
            actor: "mcp:alpha".to_string(),
            action: "send_text".to_string(),
            session_id: Some("edge".to_string()),
            decision: "succeeded".to_string(),
            details: BTreeMap::from([("scope".to_string(), "write-input".to_string())]),
        },
        AuditRecord {
            id: "audit-b".to_string(),
            ts: Utc::now(),
            actor: "mcp:beta".to_string(),
            action: "create_tunnel".to_string(),
            session_id: Some("lab".to_string()),
            decision: "denied".to_string(),
            details: BTreeMap::from([("scope".to_string(), "tunnel".to_string())]),
        },
        AuditRecord {
            id: "audit-c".to_string(),
            ts: Utc::now(),
            actor: "mcp:gamma".to_string(),
            action: "list_sessions".to_string(),
            session_id: None,
            decision: "authorized".to_string(),
            details: BTreeMap::from([("scope".to_string(), "read-sessions".to_string())]),
        },
    ];
    let result = export_mcp_audit_inner(
        &root.join("portmate-store.sqlite3"),
        &audit,
        ExportMcpAuditRequest {
            record_ids: vec!["audit-c".to_string(), "audit-a".to_string()],
        },
    )
    .unwrap();
    assert_eq!(result.records, 2);
    assert_eq!(result.sha256, sha256_file(Path::new(&result.path)).unwrap());
    assert!(fs::read_to_string(&result.checksum_path)
        .unwrap()
        .starts_with(&result.sha256));

    let lines = fs::read_to_string(&result.path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["format"], "portmate-mcp-audit");
    assert_eq!(lines[0]["recordCount"], 2);
    assert_eq!(lines[0]["containsSecretBodies"], false);
    assert_eq!(lines[1]["record"]["id"], "audit-c");
    assert_eq!(lines[2]["record"]["id"], "audit-a");
    assert!(!fs::read_to_string(&result.path)
        .unwrap()
        .contains("audit-b"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&result.checksum_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    let duplicate = export_mcp_audit_inner(
        &root.join("portmate-store.sqlite3"),
        &audit,
        ExportMcpAuditRequest {
            record_ids: vec!["audit-a".to_string(), "audit-a".to_string()],
        },
    )
    .unwrap_err();
    assert!(duplicate.contains("duplicate"));
    let stale = export_mcp_audit_inner(
        &root.join("portmate-store.sqlite3"),
        &audit,
        ExportMcpAuditRequest {
            record_ids: vec!["missing".to_string()],
        },
    )
    .unwrap_err();
    assert!(stale.contains("refresh"));

    assert!(fs::read_dir(root.join("exports"))
        .unwrap()
        .all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".part")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mcp_audit_delete_is_validated_transactional_and_persistent() {
    let root = canonical_test_temp_path("portmate-mcp-audit-delete");
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    let mut store = SessionStore::default();
    for id in ["audit-a", "audit-b", "audit-c"] {
        store.record_audit(AuditRecord {
            id: id.to_string(),
            ts: Utc::now(),
            actor: "mcp:test".to_string(),
            action: "list_sessions".to_string(),
            session_id: None,
            decision: "authorized".to_string(),
            details: BTreeMap::new(),
        });
    }
    save_store(&store_path, &store).unwrap();

    let remaining = commit_store_mutation(&mut store, &store_path, |next_store| {
        delete_mcp_audit_from_store(
            next_store,
            &DeleteMcpAuditRequest {
                record_ids: vec!["audit-b".to_string()],
                all: false,
            },
        )
    })
    .unwrap();
    assert_eq!(
        remaining
            .iter()
            .map(|record| record.id.as_str())
            .collect::<Vec<_>>(),
        ["audit-a", "audit-c"]
    );
    assert_eq!(load_store_sqlite(&store_path).unwrap().audit.len(), 2);

    let before_invalid = store.audit.clone();
    let duplicate = delete_mcp_audit_from_store(
        &mut store,
        &DeleteMcpAuditRequest {
            record_ids: vec!["audit-a".to_string(), "audit-a".to_string()],
            all: false,
        },
    )
    .unwrap_err();
    assert!(duplicate.contains("duplicate"));
    assert_eq!(store.audit, before_invalid);

    let stale = delete_mcp_audit_from_store(
        &mut store,
        &DeleteMcpAuditRequest {
            record_ids: vec!["missing".to_string()],
            all: false,
        },
    )
    .unwrap_err();
    assert!(stale.contains("refresh"));
    assert_eq!(store.audit, before_invalid);

    let invalid_shape = delete_mcp_audit_from_store(
        &mut store,
        &DeleteMcpAuditRequest {
            record_ids: vec!["audit-a".to_string()],
            all: true,
        },
    )
    .unwrap_err();
    assert!(invalid_shape.contains("combine"));
    assert_eq!(store.audit, before_invalid);

    let empty_selection = delete_mcp_audit_from_store(
        &mut store,
        &DeleteMcpAuditRequest {
            record_ids: Vec::new(),
            all: false,
        },
    )
    .unwrap_err();
    assert!(empty_selection.contains("at least one"));
    assert_eq!(store.audit, before_invalid);

    let cleared = commit_store_mutation(&mut store, &store_path, |next_store| {
        delete_mcp_audit_from_store(
            next_store,
            &DeleteMcpAuditRequest {
                record_ids: Vec::new(),
                all: true,
            },
        )
    })
    .unwrap();
    assert!(cleared.is_empty());
    assert!(load_store_sqlite(&store_path).unwrap().audit.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn terminal_text_export_is_atomic_bounded_and_checksummed() {
    let root = canonical_test_temp_path("portmate-terminal-export");
    fs::create_dir_all(&root).unwrap();
    let request = ExportTerminalTextRequest {
        session_id: "../shell export".to_string(),
        view_id: "view-mirror".to_string(),
        source: TerminalTextExportSource::Buffer,
        text: "prompt$ echo 终端\n终端\n".to_string(),
        destination_directory: None,
        destination_path: None,
        overwrite: false,
    };
    let result =
        export_terminal_text_inner(&root.join("portmate-store.sqlite3"), request.clone()).unwrap();
    assert_eq!(result.session_id, request.session_id);
    assert_eq!(result.view_id, request.view_id);
    assert_eq!(result.source, TerminalTextExportSource::Buffer);
    assert_eq!(fs::read_to_string(&result.path).unwrap(), request.text);
    assert_eq!(result.size as usize, request.text.len());
    assert_eq!(result.sha256, sha256_file(Path::new(&result.path)).unwrap());
    assert!(fs::read_to_string(&result.checksum_path)
        .unwrap()
        .starts_with(&result.sha256));
    let export_dir = root.join("exports").canonicalize().unwrap();
    assert_eq!(Path::new(&result.path).parent().unwrap(), export_dir);
    assert!(Path::new(&result.path)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .contains("shell_export"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            fs::metadata(&result.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&result.checksum_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(fs::read_dir(&export_dir).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".part")));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn terminal_text_export_rejects_a_symlinked_exports_directory() {
    let root = canonical_test_tempdir();
    let outside = canonical_test_tempdir();
    let exports = root.path().join("exports");
    std::os::unix::fs::symlink(outside.path(), &exports).unwrap();

    let error = export_terminal_text_inner(
        &root.path().join("portmate-store.sqlite3"),
        ExportTerminalTextRequest {
            session_id: "shell-a".to_string(),
            view_id: "view-a".to_string(),
            source: TerminalTextExportSource::Buffer,
            text: "sensitive terminal output".to_string(),
            destination_directory: None,
            destination_path: None,
            overwrite: false,
        },
    )
    .unwrap_err();

    assert!(error.contains("symbolic link"), "{error}");
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
}

#[test]
fn terminal_text_export_rejects_empty_invalid_and_oversized_requests() {
    let mut request = ExportTerminalTextRequest {
        session_id: "shell-a".to_string(),
        view_id: "view-a".to_string(),
        source: TerminalTextExportSource::Selection,
        text: "selected".to_string(),
        destination_directory: None,
        destination_path: None,
        overwrite: false,
    };
    assert!(validate_terminal_text_export_request(&request, 8).is_ok());
    request.text.push('!');
    assert!(validate_terminal_text_export_request(&request, 8)
        .unwrap_err()
        .contains("8 byte limit"));
    request.text.clear();
    assert!(validate_terminal_text_export_request(&request, 8)
        .unwrap_err()
        .contains("empty"));
    request.text = "text".to_string();
    request.view_id = "bad\nview".to_string();
    assert!(validate_terminal_text_export_request(&request, 8)
        .unwrap_err()
        .contains("view id"));
    request.view_id = "view-a".to_string();
    request.session_id.clear();
    assert!(validate_terminal_text_export_request(&request, 8)
        .unwrap_err()
        .contains("session id"));
    request.session_id = "bad\nsession".to_string();
    assert!(validate_terminal_text_export_request(&request, 8)
        .unwrap_err()
        .contains("session id"));
}

#[test]
fn terminal_text_export_request_keeps_legacy_destination_defaults() {
    let request: ExportTerminalTextRequest = serde_json::from_value(serde_json::json!({
        "sessionId": "shell-a",
        "viewId": "view-a",
        "source": "buffer",
        "text": "legacy request"
    }))
    .unwrap();
    assert!(request.destination_directory.is_none());
    assert!(request.destination_path.is_none());
    assert!(!request.overwrite);
}

#[test]
fn terminal_text_export_uses_a_configured_existing_directory() {
    let root = canonical_test_tempdir();
    let destination = root.path().join("terminal-exports");
    fs::create_dir(&destination).unwrap();
    let request = ExportTerminalTextRequest {
        session_id: "shell-a".to_string(),
        view_id: "view-a".to_string(),
        source: TerminalTextExportSource::Buffer,
        text: "configured destination".to_string(),
        destination_directory: Some(destination.display().to_string()),
        destination_path: None,
        overwrite: false,
    };

    let result = export_terminal_text_inner(&root.path().join("store.sqlite3"), request).unwrap();
    assert_eq!(Path::new(&result.path).parent().unwrap(), destination);
    assert_eq!(
        fs::read_to_string(result.path).unwrap(),
        "configured destination"
    );
}

#[test]
fn terminal_text_export_writes_and_overwrites_an_explicit_file() {
    let root = canonical_test_tempdir();
    let destination = root.path().join("chosen.txt");
    let mut request = ExportTerminalTextRequest {
        session_id: "shell-a".to_string(),
        view_id: "view-a".to_string(),
        source: TerminalTextExportSource::Buffer,
        text: "first export".to_string(),
        destination_directory: None,
        destination_path: Some(destination.display().to_string()),
        overwrite: false,
    };

    let first =
        export_terminal_text_inner(&root.path().join("store.sqlite3"), request.clone()).unwrap();
    assert_eq!(Path::new(&first.path), destination);
    assert_eq!(fs::read_to_string(&destination).unwrap(), "first export");
    let refusal = export_terminal_text_inner(&root.path().join("store.sqlite3"), request.clone())
        .unwrap_err();
    assert!(refusal.contains("refusing to overwrite"), "{refusal}");

    request.text = "replacement export".to_string();
    request.overwrite = true;
    let replacement =
        export_terminal_text_inner(&root.path().join("store.sqlite3"), request).unwrap();
    assert_eq!(
        fs::read_to_string(&destination).unwrap(),
        "replacement export"
    );
    assert_eq!(replacement.sha256, sha256_file(&destination).unwrap());
    assert!(fs::read_to_string(replacement.checksum_path)
        .unwrap()
        .starts_with(&replacement.sha256));
    assert!(fs::read_dir(root.path()).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".part")));
}

#[test]
fn terminal_text_export_rejects_ambiguous_or_relative_destinations() {
    let root = canonical_test_tempdir();
    let mut request = ExportTerminalTextRequest {
        session_id: "shell-a".to_string(),
        view_id: "view-a".to_string(),
        source: TerminalTextExportSource::Buffer,
        text: "destination validation".to_string(),
        destination_directory: None,
        destination_path: Some("relative.txt".to_string()),
        overwrite: false,
    };
    let relative = export_terminal_text_inner(&root.path().join("store.sqlite3"), request.clone())
        .unwrap_err();
    assert!(relative.contains("must be absolute"), "{relative}");

    request.destination_path = Some(format!("{}/./chosen.txt", root.path().display()));
    let current_component =
        export_terminal_text_inner(&root.path().join("store.sqlite3"), request.clone())
            .unwrap_err();
    assert!(
        current_component.contains(". or .. components"),
        "{current_component}"
    );
    request.destination_path = Some(format!("{}/child/../chosen.txt", root.path().display()));
    let parent_component =
        export_terminal_text_inner(&root.path().join("store.sqlite3"), request.clone())
            .unwrap_err();
    assert!(
        parent_component.contains(". or .. components"),
        "{parent_component}"
    );

    request.destination_directory = Some(root.path().display().to_string());
    let ambiguous = validate_terminal_text_export_request(&request, 1024).unwrap_err();
    assert!(
        ambiguous.contains("either a destination directory"),
        "{ambiguous}"
    );

    request.destination_path = None;
    request.overwrite = true;
    let invalid_overwrite = validate_terminal_text_export_request(&request, 1024).unwrap_err();
    assert!(
        invalid_overwrite.contains("explicit destination path"),
        "{invalid_overwrite}"
    );

    let orphan_target = root.path().join("orphan.txt");
    let orphan_checksum = root.path().join("orphan.txt.sha256");
    fs::write(&orphan_checksum, "unrelated checksum").unwrap();
    request.destination_directory = None;
    request.destination_path = Some(orphan_target.display().to_string());
    let orphan_error =
        export_terminal_text_inner(&root.path().join("store.sqlite3"), request).unwrap_err();
    assert!(
        orphan_error.contains("refusing to overwrite"),
        "{orphan_error}"
    );
    assert!(!orphan_target.exists());
    assert_eq!(
        fs::read_to_string(orphan_checksum).unwrap(),
        "unrelated checksum"
    );
}

#[cfg(unix)]
#[test]
fn terminal_text_export_rejects_symlinked_custom_destinations() {
    let root = canonical_test_tempdir();
    let outside = canonical_test_tempdir();
    let linked_directory = root.path().join("linked-exports");
    std::os::unix::fs::symlink(outside.path(), &linked_directory).unwrap();
    let request = ExportTerminalTextRequest {
        session_id: "shell-a".to_string(),
        view_id: "view-a".to_string(),
        source: TerminalTextExportSource::Buffer,
        text: "must stay local".to_string(),
        destination_directory: Some(linked_directory.display().to_string()),
        destination_path: None,
        overwrite: false,
    };
    let directory_error =
        export_terminal_text_inner(&root.path().join("store.sqlite3"), request).unwrap_err();
    assert!(
        directory_error.contains("symbolic link"),
        "{directory_error}"
    );

    let target = root.path().join("chosen.txt");
    let outside_file = outside.path().join("outside.txt");
    fs::write(&outside_file, "outside").unwrap();
    std::os::unix::fs::symlink(&outside_file, &target).unwrap();
    let target_error = export_terminal_text_inner(
        &root.path().join("store.sqlite3"),
        ExportTerminalTextRequest {
            session_id: "shell-a".to_string(),
            view_id: "view-a".to_string(),
            source: TerminalTextExportSource::Buffer,
            text: "replacement".to_string(),
            destination_directory: None,
            destination_path: Some(target.display().to_string()),
            overwrite: true,
        },
    )
    .unwrap_err();
    assert!(target_error.contains("symbolic link"), "{target_error}");
    assert_eq!(fs::read_to_string(outside_file).unwrap(), "outside");
}

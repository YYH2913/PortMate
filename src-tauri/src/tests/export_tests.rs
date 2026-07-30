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
    let root = std::env::temp_dir().join(format!("portmate-mcp-audit-export-{}", Uuid::new_v4()));
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
fn terminal_text_export_is_atomic_bounded_and_checksummed() {
    let root = std::env::temp_dir().join(format!("portmate-terminal-export-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let request = ExportTerminalTextRequest {
        session_id: "../shell export".to_string(),
        view_id: "view-mirror".to_string(),
        source: TerminalTextExportSource::Buffer,
        text: "prompt$ echo 终端\n终端\n".to_string(),
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
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let exports = root.path().join("exports");
    std::os::unix::fs::symlink(outside.path(), &exports).unwrap();

    let error = export_terminal_text_inner(
        &root.path().join("portmate-store.sqlite3"),
        ExportTerminalTextRequest {
            session_id: "shell-a".to_string(),
            view_id: "view-a".to_string(),
            source: TerminalTextExportSource::Buffer,
            text: "sensitive terminal output".to_string(),
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

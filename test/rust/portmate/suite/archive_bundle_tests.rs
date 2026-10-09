#[test]
fn session_bundle_archive_enforces_redaction_and_verifies_checksums() {
    let root = std::env::temp_dir().join(format!("portmate-bundle-test-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let mut profile = test_shell_profile();
    profile.logging.path_template = "/home/operator/private-logs/{session}.raw".to_string();
    profile.transfer.default_local_dir = Some("/home/operator/private-downloads".to_string());
    let ConnectionConfig::Shell(shell) = &mut profile.connection else {
        unreachable!("test profile should use a shell connection");
    };
    shell.cwd = Some("/home/operator/private-shell-cwd".to_string());
    shell.args = vec!["--password".to_string(), "opaque-shell-secret".to_string()];
    let session_id = profile.id.clone();
    let secret = b"password=hunter2";
    let bytes_ref = append_log_bytes(&store_path, &profile, "raw", secret).unwrap();
    let log_relative = parse_log_bytes_ref(&bytes_ref).unwrap().relative;
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    store
        .record_stream_event_with_bytes_ref(
            &session_id,
            EventDirection::Inbound,
            EventStream::Stdout,
            String::from_utf8_lossy(secret),
            Some(bytes_ref.clone()),
        )
        .unwrap();
    let attachment_root = log_root(&store_path).join("attachments");
    fs::create_dir_all(attachment_root.join("nested")).unwrap();
    fs::write(attachment_root.join("report.txt"), b"primary report").unwrap();
    fs::write(
        attachment_root.join("nested/report.txt"),
        b"secondary report",
    )
    .unwrap();
    let signing_key = SigningKey::from_bytes(&[0x42; 32]);

    let redacted = export_session_bundle_archive_inner(
        &store_path,
        &store,
        ExportSessionBundleArchiveRequest {
            session_id: session_id.clone(),
            redact_secrets: true,
            include_raw_logs: true,
            attachment_paths: Vec::new(),
        },
        &signing_key,
    )
    .unwrap();
    assert!(redacted.redacted);
    assert_eq!(redacted.raw_log_segments, 0);
    assert!(redacted
        .warnings
        .iter()
        .any(|warning| warning.contains("omitted")));
    assert_eq!(
        sha256_file(Path::new(&redacted.path)).unwrap(),
        redacted.sha256
    );
    assert!(fs::read_to_string(&redacted.checksum_path)
        .unwrap()
        .contains(&redacted.sha256));
    assert!(redacted.checksum_path.ends_with(".tar.gz.sha256"));
    assert!(redacted.signature_path.ends_with(".tar.gz.sig.json"));
    assert!(!redacted.checksum_path.contains(".tar.tar.gz"));
    let redacted_entries = read_test_bundle_entries(Path::new(&redacted.path));
    assert!(redacted_entries.contains_key("bundle.json"));
    assert!(redacted_entries.contains_key("events.jsonl"));
    assert!(redacted_entries.contains_key("diagnostics.json"));
    assert!(redacted_entries.contains_key("manifest.json"));
    assert!(!redacted_entries
        .keys()
        .any(|path| path.starts_with("log-segments/")));
    let redacted_text = String::from_utf8_lossy(&redacted_entries["bundle.json"]);
    let redacted_events = String::from_utf8_lossy(&redacted_entries["events.jsonl"]);
    let redacted_diagnostics: serde_json::Value =
        serde_json::from_slice(&redacted_entries["diagnostics.json"]).unwrap();
    assert!(!redacted_text.contains("hunter2"));
    assert!(!redacted_text.contains(&bytes_ref));
    assert!(!redacted_events.contains(&bytes_ref));
    assert_eq!(
        redacted_diagnostics["availableLogShards"],
        serde_json::json!([])
    );
    assert!(
        !String::from_utf8_lossy(&redacted_entries["diagnostics.json"]).contains(&log_relative)
    );
    for sensitive in [
        "/home/operator/private-logs/{session}.raw",
        "/home/operator/private-downloads",
        "/home/operator/private-shell-cwd",
        "opaque-shell-secret",
    ] {
        assert!(!redacted_text.contains(sensitive));
    }

    let manifest: serde_json::Value =
        serde_json::from_slice(&redacted_entries["manifest.json"]).unwrap();
    for file in manifest["files"].as_array().unwrap() {
        let path = file["path"].as_str().unwrap();
        assert_eq!(
            file["sha256"].as_str().unwrap(),
            sha256_hex(&redacted_entries[path])
        );
    }

    let plain = export_session_bundle_archive_inner(
        &store_path,
        &store,
        ExportSessionBundleArchiveRequest {
            session_id,
            redact_secrets: false,
            include_raw_logs: true,
            attachment_paths: vec![
                "attachments/report.txt".to_string(),
                "attachments/nested/report.txt".to_string(),
                "attachments/report.txt".to_string(),
            ],
        },
        &signing_key,
    )
    .unwrap();
    assert!(!plain.redacted);
    assert_eq!(plain.raw_log_segments, 1);
    assert_eq!(plain.attachments, 2);
    assert_eq!(plain.signature_algorithm, "Ed25519");
    assert_eq!(
        plain.signing_public_key,
        BASE64_STANDARD.encode(signing_key.verifying_key().to_bytes())
    );
    let plain_entries = read_test_bundle_entries(Path::new(&plain.path));
    let plain_bundle = String::from_utf8_lossy(&plain_entries["bundle.json"]);
    let plain_diagnostics: serde_json::Value =
        serde_json::from_slice(&plain_entries["diagnostics.json"]).unwrap();
    assert!(plain_bundle.contains(&bytes_ref));
    assert!(plain_bundle.contains("/home/operator/private-shell-cwd"));
    assert!(plain_bundle.contains("opaque-shell-secret"));
    assert!(plain_diagnostics["availableLogShards"]
        .as_array()
        .is_some_and(|shards| shards.iter().any(|shard| shard["path"] == log_relative)));
    let raw_entry = plain_entries
        .iter()
        .find(|(path, _)| path.starts_with("log-segments/"))
        .unwrap();
    assert_eq!(raw_entry.1, secret);
    assert!(String::from_utf8_lossy(&plain_entries["events.jsonl"]).contains("hunter2"));
    assert_eq!(
        plain_entries["attachments/0001-report.txt"],
        b"primary report"
    );
    assert_eq!(
        plain_entries["attachments/0002-report.txt"],
        b"secondary report"
    );

    let plain_manifest: serde_json::Value =
        serde_json::from_slice(&plain_entries["manifest.json"]).unwrap();
    assert_eq!(plain_manifest["version"], 2);
    let attachments = plain_manifest["attachments"].as_array().unwrap();
    assert_eq!(attachments.len(), 2);
    assert_eq!(attachments[0]["displayName"], "report.txt");
    assert_eq!(attachments[0]["sourcePath"], "attachments/report.txt");
    assert_eq!(attachments[1]["archivePath"], "attachments/0002-report.txt");
    for attachment in attachments {
        let path = attachment["archivePath"].as_str().unwrap();
        assert_eq!(
            attachment["sha256"].as_str().unwrap(),
            sha256_hex(&plain_entries[path])
        );
    }

    let signature_text = fs::read_to_string(&plain.signature_path).unwrap();
    assert!(!signature_text.contains(&BASE64_STANDARD.encode(signing_key.to_bytes())));
    let signature_document: serde_json::Value = serde_json::from_str(&signature_text).unwrap();
    assert_eq!(signature_document["format"], "portmate-detached-signature");
    assert_eq!(signature_document["algorithm"], "Ed25519");
    assert_eq!(signature_document["archiveSha256"], plain.sha256);
    assert_eq!(signature_document["archiveSize"], plain.size);
    let signed_payload = BASE64_STANDARD
        .decode(signature_document["signedPayloadBase64"].as_str().unwrap())
        .unwrap();
    let archive_name = Path::new(&plain.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap();
    assert_eq!(
        signed_payload,
        bundle_signature_payload(
            archive_name,
            &plain.sha256,
            plain.size,
            signature_document["createdAt"].as_str().unwrap(),
        )
    );
    let public_key = BASE64_STANDARD
        .decode(signature_document["publicKeyBase64"].as_str().unwrap())
        .unwrap();
    let public_key = <[u8; 32]>::try_from(public_key).unwrap();
    let signature = BASE64_STANDARD
        .decode(signature_document["signatureBase64"].as_str().unwrap())
        .unwrap();
    let signature = ed25519_dalek::Signature::from_slice(&signature).unwrap();
    ed25519_dalek::VerifyingKey::from_bytes(&public_key)
        .unwrap()
        .verify_strict(&signed_payload, &signature)
        .unwrap();

    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_bundle_attachments_reject_unsafe_paths_and_size_changes() {
    let root = std::env::temp_dir().join(format!(
        "portmate-bundle-attachment-test-{}",
        Uuid::new_v4()
    ));
    let store_path = root.join("portmate-store.sqlite3");
    let logs = log_root(&store_path);
    fs::create_dir_all(&logs).unwrap();
    fs::write(logs.join("valid.txt"), b"valid").unwrap();
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    let mut store = SessionStore::default();
    store.upsert_profile(profile);
    let signing_key = SigningKey::from_bytes(&[0x24; 32]);

    let redacted_error = export_session_bundle_archive_inner(
        &store_path,
        &store,
        ExportSessionBundleArchiveRequest {
            session_id: session_id.clone(),
            redact_secrets: true,
            include_raw_logs: false,
            attachment_paths: vec!["valid.txt".to_string()],
        },
        &signing_key,
    )
    .unwrap_err();
    assert!(redacted_error.contains("not redacted"));

    let traversal_error = export_session_bundle_archive_inner(
        &store_path,
        &store,
        ExportSessionBundleArchiveRequest {
            session_id: session_id.clone(),
            redact_secrets: false,
            include_raw_logs: false,
            attachment_paths: vec!["../outside.txt".to_string()],
        },
        &signing_key,
    )
    .unwrap_err();
    assert!(traversal_error.contains("invalid log shard path"));

    let too_many = vec!["valid.txt".to_string(); MAX_BUNDLE_ATTACHMENTS + 1];
    let count_error = prepare_bundle_attachments(&store_path, &too_many).unwrap_err();
    assert!(count_error.contains("count limit"));

    let oversized = logs.join("oversized.raw");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(MAX_BUNDLE_ATTACHMENT_BYTES + 1)
        .unwrap();
    let size_error =
        prepare_bundle_attachments(&store_path, &["oversized.raw".to_string()]).unwrap_err();
    assert!(size_error.contains("byte limit"));

    let changing = logs.join("changing.jsonl");
    fs::write(&changing, b"one").unwrap();
    fs::write(&changing, b"changed").unwrap();
    let change_error = read_verified_bundle_attachment(&changing, 3).unwrap_err();
    assert!(change_error.contains("changed"));

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(logs.join("valid.txt"), logs.join("linked.txt")).unwrap();
        let symlink_error =
            prepare_bundle_attachments(&store_path, &["linked.txt".to_string()]).unwrap_err();
        assert!(symlink_error.contains("not a regular file"));
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_shard_management_lists_previews_and_deletes_safely() {
    let root = std::env::temp_dir().join(format!("portmate-log-manager-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let logs = log_root(&store_path);
    let nested = logs.join("profile/2026-07-12");
    fs::create_dir_all(&nested).unwrap();
    let text_path = nested.join("session.txt");
    let raw_path = nested.join("session.raw");
    fs::write(&text_path, "a".repeat(200)).unwrap();
    fs::write(&raw_path, [0_u8, 0xff, b'A', b' ']).unwrap();
    fs::write(nested.join("ignored.md"), b"not a shard").unwrap();

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&raw_path, nested.join("linked.raw")).unwrap();
    }

    let shards = list_log_shards_inner(&store_path).unwrap();
    assert_eq!(shards.len(), 2);
    assert!(shards
        .iter()
        .any(|shard| shard.path == "profile/2026-07-12/session.txt" && shard.size == 200));
    assert!(shards
        .iter()
        .any(|shard| shard.path == "profile/2026-07-12/session.raw" && shard.format == "raw"));

    let text =
        read_log_shard_inner(&store_path, "profile/2026-07-12/session.txt", Some(64)).unwrap();
    assert_eq!(text.encoding, "utf8");
    assert_eq!(text.bytes_read, 64);
    assert!(text.truncated);
    assert_eq!(text.content, "a".repeat(64));

    let raw = read_log_shard_inner(&store_path, "profile/2026-07-12/session.raw", None).unwrap();
    assert_eq!(raw.encoding, "hex");
    assert!(raw.content.contains("00 FF 41 20"));

    let batch_error = delete_log_shards_inner(
        &store_path,
        &[
            "profile/2026-07-12/session.txt".to_string(),
            "../outside.raw".to_string(),
        ],
    )
    .unwrap_err();
    assert!(batch_error.contains("invalid log shard path"));
    assert!(
        text_path.exists(),
        "validation failure deleted a valid shard"
    );
    assert!(read_log_shard_inner(&store_path, "/etc/passwd.raw", None).is_err());

    let deleted = delete_log_shards_inner(
        &store_path,
        &[
            "profile/2026-07-12/session.txt".to_string(),
            "profile/2026-07-12/session.txt".to_string(),
        ],
    )
    .unwrap();
    assert_eq!(deleted.deleted, 1);
    assert_eq!(deleted.bytes_deleted, 200);
    assert!(!text_path.exists());
    assert!(raw_path.exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_shard_search_filters_text_formats_and_reports_limits() {
    let root = std::env::temp_dir().join(format!("portmate-log-search-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let logs = log_root(&store_path);
    fs::create_dir_all(&logs).unwrap();
    fs::write(
        logs.join("session.txt"),
        b"normal line\nDevice ERROR at startup\nlast line\n",
    )
    .unwrap();
    fs::write(
        logs.join("session.jsonl"),
        br#"{"text":"another error from jsonl"}
{"text":"ok"}
"#,
    )
    .unwrap();
    fs::write(logs.join("session.raw"), b"binary ERROR bytes").unwrap();

    let result = search_log_shards_inner(
        &store_path,
        SearchLogShardsRequest {
            query: "error".to_string(),
            paths: Vec::new(),
            limit: Some(10),
        },
    )
    .unwrap();
    assert_eq!(result.matches.len(), 2);
    assert_eq!(result.files_scanned, 2);
    assert!(!result.truncated);
    assert!(result
        .matches
        .iter()
        .any(|item| item.path == "session.txt" && item.line == 2 && item.byte_offset == 12));
    assert!(result
        .matches
        .iter()
        .any(|item| item.path == "session.jsonl" && item.line == 1));

    let limited = search_log_shards_inner(
        &store_path,
        SearchLogShardsRequest {
            query: "error".to_string(),
            paths: Vec::new(),
            limit: Some(1),
        },
    )
    .unwrap();
    assert_eq!(limited.matches.len(), 1);
    assert!(limited.truncated);

    let raw_only = search_log_shards_inner(
        &store_path,
        SearchLogShardsRequest {
            query: "error".to_string(),
            paths: vec!["session.raw".to_string()],
            limit: None,
        },
    )
    .unwrap();
    assert!(raw_only.matches.is_empty());
    assert_eq!(raw_only.files_scanned, 0);
    assert!(raw_only.warnings[0].contains("not text-searched"));

    assert!(search_log_shards_inner(
        &store_path,
        SearchLogShardsRequest {
            query: "x".repeat(257),
            paths: Vec::new(),
            limit: None,
        },
    )
    .is_err());
    assert!(search_log_shards_inner(
        &store_path,
        SearchLogShardsRequest {
            query: "error".to_string(),
            paths: vec!["../outside.txt".to_string()],
            limit: None,
        },
    )
    .is_err());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_shard_archive_streams_verified_files_without_deleting_sources() {
    let root = std::env::temp_dir().join(format!("portmate-log-archive-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let logs = log_root(&store_path);
    fs::create_dir_all(logs.join("nested")).unwrap();
    let text = b"first line\nsecond line\n";
    let raw = [0_u8, 0xff, 0x7f, b'A'];
    fs::write(logs.join("session.txt"), text).unwrap();
    fs::write(logs.join("nested/session.raw"), raw).unwrap();

    let archived = archive_log_shards_inner(
        &store_path,
        ArchiveLogShardsRequest {
            paths: vec![
                "session.txt".to_string(),
                "nested/session.raw".to_string(),
                "session.txt".to_string(),
            ],
        },
    )
    .unwrap();
    assert_eq!(archived.shards, 2);
    assert_eq!(archived.source_bytes, (text.len() + raw.len()) as u64);
    assert_eq!(
        sha256_file(Path::new(&archived.path)).unwrap(),
        archived.sha256
    );
    assert!(fs::read_to_string(&archived.checksum_path)
        .unwrap()
        .contains(&archived.sha256));
    assert!(archived.checksum_path.ends_with(".tar.gz.sha256"));
    assert!(!archived.checksum_path.contains(".tar.tar.gz"));
    assert!(logs.join("session.txt").exists());
    assert!(logs.join("nested/session.raw").exists());

    let entries = read_test_bundle_entries(Path::new(&archived.path));
    assert_eq!(entries["logs/session.txt"], text);
    assert_eq!(entries["logs/nested/session.raw"], raw);
    let manifest: serde_json::Value = serde_json::from_slice(&entries["manifest.json"]).unwrap();
    assert_eq!(manifest["format"], "portmate-log-archive");
    for file in manifest["files"].as_array().unwrap() {
        let path = file["path"].as_str().unwrap();
        assert_eq!(file["sha256"].as_str().unwrap(), sha256_hex(&entries[path]));
    }

    assert!(archive_log_shards_inner(
        &store_path,
        ArchiveLogShardsRequest {
            paths: vec!["../outside.raw".to_string()],
        },
    )
    .is_err());

    let _ = fs::remove_dir_all(root);
}

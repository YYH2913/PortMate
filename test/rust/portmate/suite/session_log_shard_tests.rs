#[test]
fn bytes_ref_detects_recreated_shards_and_reads_legacy_refs() {
    let root = std::env::temp_dir().join(format!("portmate-log-ref-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let profile = test_shell_profile();
    let old_ref = append_log_bytes(&store_path, &profile, "raw", b"AAAA").unwrap();
    let parsed = parse_log_bytes_ref(&old_ref).unwrap();
    let legacy_ref = format!("{}:0:4", parsed.relative);
    assert_eq!(
        read_log_bytes_ref(&store_path, &legacy_ref).unwrap().2,
        b"AAAA"
    );
    let ambiguous_path = log_root(&store_path).join("v2:legacy.raw");
    fs::write(&ambiguous_path, b"CCCC").unwrap();
    assert_eq!(
        read_log_bytes_ref(&store_path, "v2:legacy.raw:0:4")
            .unwrap()
            .2,
        b"CCCC"
    );

    delete_log_shards_inner(&store_path, std::slice::from_ref(&parsed.relative)).unwrap();
    let new_ref = append_log_bytes(&store_path, &profile, "raw", b"BBBB").unwrap();
    let error = read_log_bytes_ref(&store_path, &old_ref).unwrap_err();
    assert!(
        error.contains("content mismatch"),
        "unexpected error: {error}"
    );
    assert_eq!(
        read_log_bytes_ref(&store_path, &new_ref).unwrap().2,
        b"BBBB"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn append_log_bytes_serializes_concurrent_writers() {
    let root = std::env::temp_dir().join(format!("portmate-log-race-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let mut profile = test_shell_profile();
    profile.logging.path_template = "shared/{date}/transport.jsonl".to_string();
    let barrier = Arc::new(std::sync::Barrier::new(48));
    let results = Arc::new(Mutex::new(Vec::new()));

    std::thread::scope(|scope| {
        for index in 0_u8..48 {
            let barrier = Arc::clone(&barrier);
            let results = Arc::clone(&results);
            let store_path = store_path.clone();
            let profile = profile.clone();
            scope.spawn(move || {
                let payload = vec![index, 0xff, 0x00, 0x80, index.wrapping_add(1)];
                barrier.wait();
                let reference = append_log_bytes(&store_path, &profile, "raw", &payload).unwrap();
                results.lock().unwrap().push((payload, reference));
            });
        }
    });

    let results = results.lock().unwrap();
    assert_eq!(results.len(), 48);
    let mut ranges = Vec::new();
    for (expected, reference) in results.iter() {
        let (_, offset, actual) = read_log_bytes_ref(&store_path, reference).unwrap();
        assert_eq!(&actual, expected);
        ranges.push((offset, offset + actual.len() as u64));
    }
    ranges.sort_unstable();
    let mut expected_offset = 0_u64;
    for (start, end) in ranges {
        assert_eq!(start, expected_offset);
        expected_offset = end;
    }
    assert_eq!(expected_offset, 48 * 5);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_retention_prunes_only_expired_profile_shards() {
    let root = std::env::temp_dir().join(format!("portmate-log-retention-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let now = SystemTime::now();
    let mut profile = test_shell_profile();
    profile.logging.retention_days = 30;
    let empty = prune_expired_log_shards_for_profile(&store_path, &profile, now).unwrap();
    assert_eq!(empty.deleted, 0);
    let old_path =
        log_root(&store_path).join(log_shard_relative_path(&profile, "2026-05-01", "raw"));
    let fresh_path =
        log_root(&store_path).join(log_shard_relative_path(&profile, "2026-07-01", "txt"));
    let mut other = profile.clone();
    other.id = "session:2".to_string();
    other.name = "Other Device".to_string();
    let other_path =
        log_root(&store_path).join(log_shard_relative_path(&other, "2026-05-01", "jsonl"));
    for path in [&old_path, &fresh_path, &other_path] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"data").unwrap();
    }
    fs::File::options()
        .write(true)
        .open(&old_path)
        .unwrap()
        .set_modified(now - Duration::from_secs(31 * 86_400))
        .unwrap();
    fs::File::options()
        .write(true)
        .open(&fresh_path)
        .unwrap()
        .set_modified(now - Duration::from_secs(29 * 86_400))
        .unwrap();
    fs::File::options()
        .write(true)
        .open(&other_path)
        .unwrap()
        .set_modified(now - Duration::from_secs(60 * 86_400))
        .unwrap();

    let result = prune_expired_log_shards_for_profile(&store_path, &profile, now).unwrap();
    assert_eq!(result.deleted, 1);
    assert_eq!(result.bytes_deleted, 4);
    assert!(!old_path.exists());
    assert!(fresh_path.exists());
    assert!(other_path.exists());

    profile.logging.path_template = "{date}/shared.jsonl".to_string();
    assert!(validate_logging_retention(&profile).is_err());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_retention_check_registry_replaces_changes_and_reclaims_entries() {
    let root =
        std::env::temp_dir().join(format!("portmate-log-retention-cache-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let key = (store_path.clone(), "retention-session".to_string());
    let stale_key = (root.join("stale.sqlite3"), "stale-session".to_string());
    let mut profile = test_shell_profile();
    profile.id = key.1.clone();
    profile.logging.retention_days = 30;
    let checks = LOG_RETENTION_CHECKS.get_or_init(|| Mutex::new(HashMap::new()));
    checks.lock().unwrap().insert(
        stale_key.clone(),
        (
            7,
            Instant::now() - LOG_RETENTION_CHECK_INTERVAL - Duration::from_secs(1),
        ),
    );

    maybe_prune_expired_log_shards(&store_path, &profile).unwrap();
    {
        let checks = checks.lock().unwrap();
        assert_eq!(checks.get(&key).map(|(days, _)| *days), Some(30));
        assert!(!checks.contains_key(&stale_key));
    }

    profile.logging.retention_days = 31;
    maybe_prune_expired_log_shards(&store_path, &profile).unwrap();
    assert_eq!(
        checks.lock().unwrap().get(&key).map(|(days, _)| *days),
        Some(31)
    );

    profile.logging.retention_days = 0;
    maybe_prune_expired_log_shards(&store_path, &profile).unwrap();
    assert!(!checks.lock().unwrap().contains_key(&key));

    profile.logging.retention_days = 30;
    maybe_prune_expired_log_shards(&store_path, &profile).unwrap();
    clear_log_retention_check(&store_path, &profile.id);
    assert!(!checks.lock().unwrap().contains_key(&key));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn log_truncation_preserves_utf8_boundaries() {
    assert_eq!(truncate_for_log("  short message  ", 20), "short message");
    assert_eq!(truncate_for_log("传输失败详情", 5), "传...");
    assert_eq!(truncate_for_log("传输失败详情", 6), "传输...");
}

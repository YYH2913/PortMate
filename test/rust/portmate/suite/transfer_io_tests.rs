use super::*;

#[test]
fn transfer_throttle_delay_respects_rate_limit() {
    assert!(transfer_throttle_delay(None, 1024, Duration::ZERO).is_none());
    assert!(transfer_throttle_delay(Some(0), 1024, Duration::ZERO).is_none());
    assert!(transfer_throttle_delay(Some(1024), 0, Duration::ZERO).is_none());

    assert_eq!(
        transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(1)),
        Some(Duration::from_secs(1))
    );
    assert!(transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(2)).is_none());
    assert!(transfer_throttle_delay(Some(1024), 2048, Duration::from_secs(3)).is_none());
}

#[test]
fn transfer_throttle_wait_is_async_and_cancellable() {
    tauri::async_runtime::block_on(async {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = TransferProgressContext {
            state: test_app_state(
                test_shell_profile(),
                PathBuf::from("transfer-throttle-test.sqlite3"),
            ),
            task_id: "unused-transfer".to_string(),
            cancel: Arc::clone(&cancel),
            last_emit: Arc::new(Mutex::new(Instant::now())),
            started: Instant::now(),
            rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
            rate_limit_bytes_per_second: Some(1024),
        };
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.store(true, Ordering::SeqCst);
        });

        let started = Instant::now();
        let error = progress.throttle(1024).await.unwrap_err();
        assert_eq!(error, TRANSFER_CANCELLED_MESSAGE);
        assert!(started.elapsed() < Duration::from_millis(500));
        cancel_task.await.unwrap();
    });
}

#[test]
fn local_resume_part_helpers_keep_stable_offsets() {
    let root = canonical_test_temp_path("portmate-resume-test");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("image.bin");
    let part = local_resume_part_path(&target);
    assert_eq!(
        part.file_name().unwrap().to_string_lossy(),
        "image.bin.portmate-part"
    );

    fs::write(&part, b"abc").unwrap();
    assert_eq!(local_resume_offset(&part, 10).unwrap(), 3);
    assert!(part.exists());

    fs::write(&part, b"too-long").unwrap();
    assert_eq!(local_resume_offset(&part, 3).unwrap(), 0);
    assert!(!part.exists());

    fs::write(&part, b"complete").unwrap();
    fs::write(&target, b"old").unwrap();
    finalize_local_resume_file(&part, &target).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"complete");
    assert!(!part.exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_resume_requires_a_matching_source_prefix() {
    let root = canonical_test_tempdir();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    state
        .store
        .lock()
        .unwrap()
        .transfers
        .push(test_transfer_task(&profile.id, TransferStatus::Running));
    let progress = test_transfer_progress_context(
        &state,
        "transfer-commit-test",
        Arc::new(AtomicBool::new(false)),
    );
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    let part = local_resume_part_path(&target);
    fs::write(&source, b"abcdef").unwrap();

    fs::write(&part, b"abc").unwrap();
    let mut source_file = open_local_transfer_source(&source, "source").unwrap().0;
    assert_eq!(
        local_resume_offset_matching_local_source(&mut source_file, &part, 6, &progress,).unwrap(),
        3
    );
    let mut suffix = Vec::new();
    source_file.read_to_end(&mut suffix).unwrap();
    assert_eq!(suffix, b"def");

    fs::write(&part, b"xyz").unwrap();
    let mut source_file = open_local_transfer_source(&source, "source").unwrap().0;
    assert_eq!(
        local_resume_offset_matching_local_source(&mut source_file, &part, 6, &progress,).unwrap(),
        0
    );
    let mut full = Vec::new();
    source_file.read_to_end(&mut full).unwrap();
    assert_eq!(full, b"abcdef");
}

#[test]
fn sftp_prefix_read_errors_distinguish_short_file_and_io_failure() {
    assert!(!prefix_read_mismatch_or_error(
        std::io::Error::from(std::io::ErrorKind::UnexpectedEof),
        "SFTP 远端断点文件",
    )
    .unwrap());

    let error = prefix_read_mismatch_or_error(
        std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        "SFTP 远端断点文件",
    )
    .unwrap_err();
    assert!(error.contains("读取SFTP 远端断点文件前缀失败"), "{error}");
}

#[test]
fn local_transfer_creates_empty_files_and_rejects_source_shrink() {
    let root = canonical_test_tempdir();
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
    state
        .store
        .lock()
        .unwrap()
        .transfers
        .push(test_transfer_task(&profile.id, TransferStatus::Running));

    tauri::async_runtime::block_on(async {
        let empty_source = root.path().join("empty-source.bin");
        let empty_target = root.path().join("empty-target.bin");
        fs::write(&empty_source, []).unwrap();
        let empty_progress = test_transfer_progress_context(
            &state,
            "transfer-commit-test",
            Arc::new(AtomicBool::new(false)),
        );
        let copied = copy_local_file_for_transfer(
            empty_source.to_str().unwrap(),
            empty_target.to_str().unwrap(),
            &empty_progress,
        )
        .await
        .unwrap();
        assert_eq!(copied, 0);
        assert_eq!(fs::metadata(&empty_target).unwrap().len(), 0);

        let source = root.path().join("shrinking-source.bin");
        let target = root.path().join("shrinking-target.bin");
        fs::write(&source, vec![b'x'; 256 * 1024]).unwrap();
        let part = local_resume_part_path(&target);
        let progress = TransferProgressContext {
            state: state.clone(),
            task_id: "transfer-commit-test".to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            last_emit: Arc::new(Mutex::new(Instant::now())),
            started: Instant::now(),
            rate_baseline_bytes: Arc::new(AtomicU64::new(0)),
            rate_limit_bytes_per_second: Some(512 * 1024),
        };
        let source_path = source.display().to_string();
        let target_path = target.display().to_string();
        let copy = tokio::spawn(async move {
            copy_local_file_for_transfer(&source_path, &target_path, &progress).await
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if fs::metadata(&part).is_ok_and(|metadata| metadata.len() >= 64 * 1024) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("local transfer did not write its first block");
        OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap()
            .set_len(64 * 1024)
            .unwrap();

        let error = copy.await.unwrap().unwrap_err();
        assert_eq!(
            error,
            "local transfer size mismatch: copied 65536, expected 262144"
        );
        assert!(!target.exists());
        assert_eq!(fs::metadata(&part).unwrap().len(), 64 * 1024);
    });
}

#[cfg(unix)]
#[test]
fn local_transfer_writes_reject_symlink_targets() {
    let root = canonical_test_temp_path("portmate-transfer-links");
    fs::create_dir_all(&root).unwrap();
    let protected = root.join("protected.bin");
    fs::write(&protected, b"protected").unwrap();

    let destination = root.join("destination.bin");
    std::os::unix::fs::symlink(&protected, &destination).unwrap();
    let error =
        write_local_transfer_file(destination.to_str().unwrap(), b"replacement").unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let part = root.join("destination.bin.portmate-part");
    std::os::unix::fs::symlink(&protected, &part).unwrap();
    let error = local_resume_offset(&part, 128).unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let temp = root.join("new.part");
    fs::write(&temp, b"new payload").unwrap();
    let final_link = root.join("final.bin");
    std::os::unix::fs::symlink(&protected, &final_link).unwrap();
    let error = finalize_local_resume_file(&temp, &final_link).unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");
    assert!(temp.exists());

    let protected_dir = root.join("protected-dir");
    fs::create_dir(&protected_dir).unwrap();
    let linked_dir = root.join("linked-dir");
    std::os::unix::fs::symlink(&protected_dir, &linked_dir).unwrap();
    let nested_target = linked_dir.join("nested.bin");
    let error = write_local_transfer_file(nested_target.to_str().unwrap(), b"nested").unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert!(!protected_dir.join("nested.bin").exists());

    let copy_source = root.join("copy-source.bin");
    fs::write(&copy_source, b"copy payload").unwrap();
    let copied_target = linked_dir.join("copied.bin");
    let profile = test_shell_profile();
    let state = test_app_state(profile.clone(), root.join("store.sqlite3"));
    state
        .store
        .lock()
        .unwrap()
        .transfers
        .push(test_transfer_task(&profile.id, TransferStatus::Running));
    let progress = test_transfer_progress_context(
        &state,
        "transfer-commit-test",
        Arc::new(AtomicBool::new(false)),
    );
    let error = tauri::async_runtime::block_on(copy_local_file_for_transfer(
        copy_source.to_str().unwrap(),
        copied_target.to_str().unwrap(),
        &progress,
    ))
    .unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert!(!protected_dir.join("copied.bin").exists());

    let scp_target = linked_dir.join("scp.bin");
    let error = prepare_local_transfer_target_path(&scp_target, "SCP 本地目标路径").unwrap_err();
    assert!(error.contains("符号链接"), "{error}");
    assert!(!protected_dir.join("scp.bin").exists());

    let missing_parent = root.join("missing-parent").join("payload.bin");
    prepare_local_transfer_target_path(&missing_parent, "本地传输目标路径").unwrap();
    assert!(root.join("missing-parent").is_dir());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn streamed_modem_output_preserves_internal_eof_and_discards_final_padding() {
    let root = canonical_test_temp_path("portmate-modem-output");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("received.bin");
    let mut output =
        PendingLocalTransferOutput::create(&target, "测试 Modem 本地目标文件").unwrap();
    let mut trailing_padding = 0_u64;
    let mut bytes_written = 0_u64;

    append_modem_data_without_trailing_padding(
        &mut output,
        b"before\x1a\x1a",
        &mut trailing_padding,
        &mut bytes_written,
    )
    .unwrap();
    append_modem_data_without_trailing_padding(
        &mut output,
        b"\x1aafter\x1a\x1a",
        &mut trailing_padding,
        &mut bytes_written,
    )
    .unwrap();
    output.finish().unwrap();

    let expected = b"before\x1a\x1a\x1aafter";
    assert_eq!(bytes_written, expected.len() as u64);
    assert_eq!(fs::read(&target).unwrap(), expected);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn streamed_modem_output_respects_ymodem_declared_length() {
    let root = canonical_test_temp_path("portmate-ymodem-output");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("received.bin");
    let mut output =
        PendingLocalTransferOutput::create(&target, "测试 Modem 本地目标文件").unwrap();
    let mut bytes_written = 0_u64;

    append_modem_data_with_size_limit(&mut output, b"abcdef", 4, &mut bytes_written).unwrap();
    append_modem_data_with_size_limit(&mut output, b"more", 4, &mut bytes_written).unwrap();
    output.finish().unwrap();

    assert_eq!(bytes_written, 4);
    assert_eq!(fs::read(&target).unwrap(), b"abcd");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn streamed_modem_output_removes_temp_file_when_not_finalized() {
    let root = canonical_test_temp_path("portmate-modem-temp");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("received.bin");
    let mut output =
        PendingLocalTransferOutput::create(&target, "测试 Modem 本地目标文件").unwrap();
    let temp = output.temp.clone();
    output.file_mut().unwrap().write_all(b"partial").unwrap();
    drop(output);

    assert!(!target.exists());
    assert!(!temp.exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn local_transfer_sources_reject_symlinks() {
    let root = canonical_test_temp_path("portmate-transfer-sources");
    fs::create_dir_all(&root).unwrap();
    let protected = root.join("protected.bin");
    fs::write(&protected, b"protected").unwrap();
    let source = root.join("source.bin");
    std::os::unix::fs::symlink(&protected, &source).unwrap();

    assert!(open_local_transfer_source(&source, "source").is_err());
    let error = match modem_direction(&StartTransferRequest {
        session_id: "session".to_string(),
        protocol: TransferProtocol::Xmodem,
        source: source.display().to_string(),
        destination: "remote:/tmp/source.bin".to_string(),
    }) {
        Err(error) => error,
        Ok(_) => panic!("symbolic link source was accepted"),
    };
    assert!(error.contains("符号链接"), "{error}");
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let _ = fs::remove_dir_all(root);
}

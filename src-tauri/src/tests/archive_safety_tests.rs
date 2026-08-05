#[cfg(unix)]
#[test]
fn archive_writes_reject_symlink_sources_outputs_and_existing_artifacts() {
    let root = std::env::temp_dir().join(format!("portmate-archive-paths-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let protected = root.join("protected.bin");
    fs::write(&protected, b"protected").unwrap();
    let source_link = root.join("source.txt");
    std::os::unix::fs::symlink(&protected, &source_link).unwrap();
    let archive_path = root.join("logs.tar.gz.part");
    let source_error = write_log_shard_archive(
        &archive_path,
        &[("source.txt".to_string(), source_link, 9)],
        0,
        "2026-07-22T00:00:00Z",
    )
    .unwrap_err();
    assert!(source_error.contains("not a regular file"));
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let output_link = root.join("bundle.tar.gz.part");
    std::os::unix::fs::symlink(&protected, &output_link).unwrap();
    let output_error = write_bundle_archive(&output_link, &[], 0).unwrap_err();
    assert!(output_error.contains("failed to create session bundle"));
    assert_eq!(fs::read(&protected).unwrap(), b"protected");

    let final_path = root.join("existing.tar.gz");
    let temp_path = root.join("existing.tar.gz.part");
    fs::write(&final_path, b"existing").unwrap();
    fs::write(&temp_path, b"new archive").unwrap();
    let final_error = finalize_archive_with_checksum(&temp_path, &final_path, "log archive")
        .err()
        .expect("existing archive artifact must be rejected");
    assert!(final_error.contains("refusing to overwrite"));
    assert_eq!(fs::read(&final_path).unwrap(), b"existing");
    assert!(!temp_path.exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn signed_bundle_finalization_refuses_existing_artifacts_without_partial_output() {
    let root =
        std::env::temp_dir().join(format!("portmate-bundle-finalize-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let final_path = root.join("bundle.tar.gz");
    let temp_path = root.join("bundle.tar.gz.part");
    let signature_temp_path = path_with_appended_suffix(&final_path, ".sig.json.part").unwrap();
    fs::write(&temp_path, b"archive").unwrap();
    fs::write(&signature_temp_path, b"owned by another export").unwrap();

    let error = finalize_signed_bundle_archive(
        &temp_path,
        &final_path,
        "test bundle",
        &SigningKey::from_bytes(&[0x11; 32]),
        "2026-07-16T00:00:00Z",
    )
    .unwrap_err();
    assert!(error.contains("refusing to overwrite"));
    assert!(!temp_path.exists());
    assert!(!final_path.exists());
    assert!(signature_temp_path.exists());
    assert!(!path_with_appended_suffix(&final_path, ".sha256")
        .unwrap()
        .exists());
    assert!(!path_with_appended_suffix(&final_path, ".sig.json")
        .unwrap()
        .exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn bundle_signing_key_decoder_rejects_malformed_key_material() {
    assert!(decode_bundle_signing_key("not-base64").is_err());
    assert!(decode_bundle_signing_key(&BASE64_STANDARD.encode([7_u8; 31])).is_err());
    assert!(decode_bundle_signing_key(&BASE64_STANDARD.encode([7_u8; 32])).is_ok());
}

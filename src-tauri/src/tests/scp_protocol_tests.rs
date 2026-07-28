use super::*;

#[test]
fn remote_copy_markers_parse_latest_size_and_done() {
    let output = b"noise\n__PORTMATE_SIZE__1024\nother\n__PORTMATE_DONE__1024\n";
    assert_eq!(
        remote_copy_markers(output),
        RemoteCopyMarkers {
            total: Some(1024),
            resume_candidate: None,
            resume: None,
            progress: None,
            done: Some(1024)
        }
    );
}

#[test]
fn remote_copy_markers_parse_latest_progress() {
    let output = b"__PORTMATE_SIZE__4096\n__PORTMATE_RESUME_CANDIDATE__512\n__PORTMATE_RESUME__512\n__PORTMATE_PROGRESS__512\n__PORTMATE_PROGRESS__2048\n__PORTMATE_DONE__4096\n";
    assert_eq!(
        remote_copy_markers(output),
        RemoteCopyMarkers {
            total: Some(4096),
            resume_candidate: Some(512),
            resume: Some(512),
            progress: Some(2048),
            done: Some(4096)
        }
    );
}

#[test]
fn remote_copy_markers_require_monotonic_consistent_progress() {
    let reported = RemoteCopyMarkers {
        total: Some(4096),
        resume_candidate: None,
        resume: Some(512),
        progress: Some(2048),
        done: None,
    };
    validate_remote_copy_markers(&reported, &reported).unwrap();
    validate_remote_copy_markers(
        &RemoteCopyMarkers {
            progress: Some(3072),
            ..reported
        },
        &reported,
    )
    .unwrap();

    for (markers, expected) in [
        (
            RemoteCopyMarkers {
                total: Some(8192),
                ..reported
            },
            "size marker changed",
        ),
        (
            RemoteCopyMarkers {
                resume: Some(1024),
                ..reported
            },
            "resume marker changed",
        ),
        (
            RemoteCopyMarkers {
                progress: Some(1024),
                ..reported
            },
            "progress marker moved backwards",
        ),
        (
            RemoteCopyMarkers {
                progress: Some(8192),
                ..reported
            },
            "progress marker 8192 exceeds size 4096",
        ),
        (
            RemoteCopyMarkers {
                done: Some(3072),
                ..reported
            },
            "done marker 3072 does not match size 4096",
        ),
    ] {
        let error = validate_remote_copy_markers(&markers, &reported).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn remote_copy_command_polls_progress_and_cleans_background_copy() {
    let command = remote_copy_command("/tmp/source file.bin", "/tmp/o'clock.bin");
    assert!(command.contains("__PORTMATE_RESUME__%s"));
    assert!(command.contains("__PORTMATE_PROGRESS__%s"));
    assert!(command.contains("trap cleanup INT TERM HUP EXIT"));
    assert!(command.contains("kill \"$pid\""));
    assert!(command.contains("remote_name=${src##*/}"));
    assert!(command.contains("case \"$dst\" in */)"));
    assert!(command.contains("part=\"${target%/*}/${target##*/}.portmate-part\""));
    assert!(command.contains("portable_path()"));
    assert!(command.contains("src=$(portable_path \"$src\")"));
    assert!(command.contains("head -c \"$current\" \"$src\" | cmp -s - \"$part\""));
    assert!(command.contains("tail -c +$((offset + 1)) \"$src\" >> \"$part\""));
    assert!(command.contains("mv -f \"$part\" \"$target\""));
    assert!(!command.contains(" -- \"$src\""));
    assert!(!command.contains("mv -f --"));
    assert!(command.contains("src='/tmp/source file.bin'"));
    assert!(command.contains("dst='/tmp/o'\\''clock.bin'"));
}

#[cfg(unix)]
#[test]
fn remote_copy_command_verifies_existing_part_file_prefix() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    let target = root.path().join("target.bin");
    let part = root.path().join("target.bin.portmate-part");
    fs::write(&source, b"abcdef").unwrap();

    for (prefix, expected_resume) in [(b"abc".as_slice(), 3), (b"xyz".as_slice(), 0)] {
        fs::write(&part, prefix).unwrap();
        let command = remote_copy_command(source.to_str().unwrap(), target.to_str().unwrap());
        let output = Command::new("sh").arg("-c").arg(command).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(fs::read(&target).unwrap(), b"abcdef");
        assert!(!part.exists());
        let markers = remote_copy_markers(&output.stdout);
        assert_eq!(markers.total, Some(6));
        assert_eq!(markers.resume, Some(expected_resume));
        assert_eq!(markers.done, Some(6));
    }
}

#[cfg(unix)]
#[test]
fn remote_copy_command_handles_dash_prefixed_relative_paths() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("-source.bin");
    let target = root.path().join("-target.bin");
    let part = root.path().join("-target.bin.portmate-part");
    fs::write(&source, b"abcdef").unwrap();
    fs::write(&part, b"abc").unwrap();

    let output = Command::new("sh")
        .arg("-c")
        .arg(remote_copy_command("-source.bin", "-target.bin"))
        .current_dir(root.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"abcdef");
    assert!(!part.exists());
    assert_eq!(remote_copy_markers(&output.stdout).resume, Some(3));
}

#[cfg(unix)]
#[test]
fn remote_copy_command_rejects_symbolic_sources_and_targets() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    let protected = root.path().join("protected.bin");
    let source_link = root.path().join("source-link.bin");
    let target_link = root.path().join("target-link.bin");
    let part_link = root.path().join("target.bin.portmate-part");
    fs::write(&source, b"payload").unwrap();
    fs::write(&protected, b"protected").unwrap();

    for (input, destination, expected) in [
        (&source_link, root.path().join("target.bin"), "source"),
        (&source, target_link.clone(), "target"),
        (&source, root.path().join("target.bin"), "part"),
    ] {
        if expected == "source" {
            std::os::unix::fs::symlink(&source, input).unwrap();
        } else if expected == "target" {
            std::os::unix::fs::symlink(&protected, &destination).unwrap();
        } else {
            std::os::unix::fs::symlink(&protected, &part_link).unwrap();
        }

        let command = remote_copy_command(input.to_str().unwrap(), destination.to_str().unwrap());
        let output = Command::new("sh").arg("-c").arg(command).output().unwrap();
        assert!(!output.status.success(), "{expected} symlink was accepted");
        assert_eq!(fs::read(&protected).unwrap(), b"protected");

        for path in [&source_link, &target_link, &part_link] {
            let _ = fs::remove_file(path);
        }
    }
}

#[test]
fn scp_upload_command_uses_resume_receiver() {
    let command = scp_upload_command("/tmp/upload dir/", "local o'clock.bin", 8192);
    assert!(command.contains("dst='/tmp/upload dir/'"));
    assert!(command.contains("source_name='local o'\\''clock.bin'"));
    assert!(command.contains("total=8192"));
    assert!(command.contains("__PORTMATE_RESUME__%s"));
    assert!(command.contains("__PORTMATE_PROGRESS__%s"));
    assert!(command.contains("case \"$dst\" in */)"));
    assert!(command.contains("part=\"${target%/*}/${target##*/}.portmate-part\""));
    assert!(command.contains("__PORTMATE_RESUME_CANDIDATE__%s"));
    assert!(command.contains("IFS= read -r expected_prefix_sha256"));
    assert!(command.contains("file_size()"));
    assert!(command.contains("part_sha256()"));
    assert!(command.contains("command -v shasum"));
    assert!(command.contains("command -v sha256"));
    assert!(command.contains("__PORTMATE_PREFIX_SHA256__$actual_prefix_sha256"));
    assert!(command.contains("cat >> \"$part\" || exit 1"));
    assert!(command.contains("portable_path()"));
    assert!(command.contains("target=$(portable_path \"$target\")"));
    assert!(command.contains("mv -f \"$part\" \"$target\""));
    assert!(!command.contains("mv -f --"));
    assert!(command.contains("final_target=$(file_size \"$target\")"));
}

#[test]
fn scp_upload_command_resumes_existing_part_file() {
    let root = std::env::temp_dir().join(format!("portmate-scp-upload-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let target = root.join("upload.bin");
    let part = root.join("upload.bin.portmate-part");
    fs::write(&part, b"abc").unwrap();

    let mut destination = root.to_string_lossy().to_string();
    destination.push('/');
    let command = scp_upload_command(&destination, "upload.bin", 6);
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let prefix_hash = format!("{:x}", Sha256::digest(b"abc"));
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("__PORTMATE_PREFIX_SHA256__{prefix_hash}\n").as_bytes())
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"def").unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"abcdef");
    assert!(!part.exists());
    let markers = remote_copy_markers(&output.stdout);
    assert_eq!(markers.total, Some(6));
    assert_eq!(markers.resume, Some(3));
    assert_eq!(markers.done, Some(6));

    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn scp_upload_command_handles_dash_prefixed_target() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("-source.bin");
    let command = scp_upload_command(".", "-source.bin", 6);
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(root.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"abcdef").unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"abcdef");
    assert!(!root.path().join("-source.bin.portmate-part").exists());
    let markers = remote_copy_markers(&output.stdout);
    assert_eq!(markers.resume, Some(0));
    assert_eq!(markers.done, Some(6));
}

#[test]
fn scp_upload_command_rewrites_mismatched_existing_part_file() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("upload.bin");
    let part = root.path().join("upload.bin.portmate-part");
    fs::write(&part, b"xyz").unwrap();

    let command = scp_upload_command(root.path().to_str().unwrap(), "upload.bin", 6);
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let prefix_hash = format!("{:x}", Sha256::digest(b"abc"));
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("__PORTMATE_PREFIX_SHA256__{prefix_hash}\nabcdef").as_bytes())
        .unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"abcdef");
    assert!(!part.exists());
    let markers = remote_copy_markers(&output.stdout);
    assert_eq!(markers.total, Some(6));
    assert_eq!(markers.resume_candidate, Some(3));
    assert_eq!(markers.resume, Some(0));
    assert_eq!(markers.done, Some(6));
}

#[test]
fn scp_upload_command_resets_unverified_existing_part_file() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("upload.bin");
    let part = root.path().join("upload.bin.portmate-part");
    fs::write(&part, b"abc").unwrap();

    let command = scp_upload_command(root.path().to_str().unwrap(), "upload.bin", 6);
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"unexpected-response\nabcdef")
        .unwrap();
    drop(child.stdin.take());

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&target).unwrap(), b"abcdef");
    let markers = remote_copy_markers(&output.stdout);
    assert_eq!(markers.resume_candidate, Some(3));
    assert_eq!(markers.resume, Some(0));
}

#[test]
fn scp_source_prefix_sha256_hashes_exact_prefix_and_restores_reader() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    fs::write(&source, b"abcdef").unwrap();
    let mut file = open_local_transfer_source(&source, "source").unwrap().0;
    let state = test_app_state(test_shell_profile(), root.path().join("store.sqlite3"));
    let progress = test_transfer_progress_context(
        &state,
        "transfer-commit-test",
        Arc::new(AtomicBool::new(false)),
    );

    assert_eq!(
        scp_source_prefix_sha256(&mut file, 3, &progress).unwrap(),
        format!("{:x}", Sha256::digest(b"abc"))
    );
    let mut suffix = Vec::new();
    file.read_to_end(&mut suffix).unwrap();
    assert_eq!(suffix, b"def");
}

#[test]
fn scp_upload_source_tail_check_rejects_growth() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    fs::write(&source, b"abcdef").unwrap();
    let mut file = open_local_transfer_source(&source, "source").unwrap().0;
    file.seek(std::io::SeekFrom::Start(3)).unwrap();

    assert_eq!(
        ensure_scp_source_has_not_grown(&mut file).unwrap_err(),
        "SCP 本地源文件在传输中增长，已保留断点文件且未提升目标文件"
    );

    file.seek(std::io::SeekFrom::End(0)).unwrap();
    ensure_scp_source_has_not_grown(&mut file).unwrap();
}

#[cfg(unix)]
#[test]
fn scp_upload_command_rejects_symbolic_targets() {
    let root = tempfile::tempdir().unwrap();
    let protected = root.path().join("protected.bin");
    let target = root.path().join("target.bin");
    let part = root.path().join("target.bin.portmate-part");
    fs::write(&protected, b"protected").unwrap();

    for link in [&target, &part] {
        std::os::unix::fs::symlink(&protected, link).unwrap();
        let command = scp_upload_command(root.path().to_str().unwrap(), "target.bin", 7);
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success(), "symlink target was accepted");
        assert_eq!(fs::read(&protected).unwrap(), b"protected");
        fs::remove_file(link).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn scp_download_command_rejects_symbolic_sources() {
    let root = tempfile::tempdir().unwrap();
    let protected = root.path().join("protected.bin");
    let source_link = root.path().join("source-link.bin");
    fs::write(&protected, b"protected").unwrap();
    std::os::unix::fs::symlink(&protected, &source_link).unwrap();

    let command = scp_download_command(source_link.to_str().unwrap());
    assert!(command.contains("[ -L \"$source\" ]"));
    assert!(command.contains("exec scp -f \"$source\""));
    let output = Command::new("sh").arg("-c").arg(command).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read(&protected).unwrap(), b"protected");
}

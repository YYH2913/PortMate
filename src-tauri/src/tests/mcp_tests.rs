use super::*;

#[test]
fn invalid_mcp_identifiers_are_bounded_before_audit() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-mcp-invalid-identifiers-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let invalid_client = format!("{}\nsecret-client", "x".repeat(1024));
        let error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: invalid_client.clone(),
                trusted_write: true,
                command: "open_session".to_string(),
                args: serde_json::json!({ "sessionId": "bad\nsession" }),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("session ID"));
        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].actor, "<invalid-client-id>");
        assert_eq!(audit[0].session_id, None);
        let encoded = serde_json::to_string(&audit).unwrap();
        assert!(!encoded.contains("secret-client"));
        assert!(!encoded.contains("bad\\nsession"));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn denied_and_invalid_mcp_writes_are_audited_without_arguments() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-denied-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();

        let denied = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "readonly-client".to_string(),
                trusted_write: false,
                command: "open_session".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "password": "denied-password-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(denied.contains("does not permit"));

        let invalid = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "trusted-client".to_string(),
                trusted_write: true,
                command: "run_command".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "command": 42,
                    "password": "invalid-password-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(invalid.contains("missing string argument `command`"));

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 2);
        assert_eq!(audit[0].actor, "readonly-client");
        assert_eq!(audit[0].action, "open_session");
        assert_eq!(audit[0].decision, "denied");
        assert_eq!(audit[0].session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(
            audit[0].details.get("scope").map(String::as_str),
            Some("manage-sessions")
        );
        assert_eq!(audit[1].actor, "trusted-client");
        assert_eq!(audit[1].action, "run_command");
        assert_eq!(audit[1].decision, "invalid");
        assert_eq!(
            audit[1].details.get("scope").map(String::as_str),
            Some("write-input")
        );
        let encoded = serde_json::to_string(&audit).unwrap();
        assert!(!encoded.contains("denied-password-secret"));
        assert!(!encoded.contains("invalid-password-secret"));
        assert!(!encoded.contains("\"command\":42"));

        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn mcp_transfer_writes_require_at_least_one_remote_side_and_never_audit_paths() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-transfer-route-{}", Uuid::new_v4()));
        let state = test_app_state(test_ssh_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        let request = |source: &str, destination: &str| IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "trusted-transfer-client".to_string(),
            trusted_write: true,
            command: "start_transfer".to_string(),
            args: serde_json::json!({
                "sessionId": session_id,
                "protocol": "sftp",
                "source": source,
                "destination": destination
            }),
        };

        for (source, destination) in [
            ("/home/operator/private-a", "/tmp/private-b"),
            ("/home/operator/private-a\0secret", "remote:/srv/private-b"),
        ] {
            let error = handle_ipc_request(state.clone(), request(source, destination))
                .await
                .unwrap_err();
            assert!(
                error.contains("at least one remote:/ssh:/load: endpoint")
                    || error.contains("NUL-free"),
                "{error}"
            );
        }

        validate_mcp_transfer_route(&StartTransferRequest {
            session_id: session_id.clone(),
            protocol: TransferProtocol::Scp,
            source: "remote:/srv/private-source".to_string(),
            destination: "ssh:/srv/private-destination".to_string(),
        })
        .unwrap();

        validate_mcp_transfer_route(&StartTransferRequest {
            session_id: session_id.clone(),
            protocol: TransferProtocol::Ymodem,
            source: "/home/operator/firmware.bin".to_string(),
            destination: "load:loady?address=0x80000000&baud=115200".to_string(),
        })
        .unwrap();
        assert!(validate_mcp_transfer_route(&StartTransferRequest {
            session_id: session_id.clone(),
            protocol: TransferProtocol::Ymodem,
            source: "remote:/srv/firmware.bin".to_string(),
            destination: "load:loady".to_string(),
        })
        .is_err());

        let local_transfer = TransferTask {
            id: "local-transfer".to_string(),
            session_id: session_id.clone(),
            protocol: TransferProtocol::Sftp,
            source: "/home/operator/private-local-source".to_string(),
            destination: "/tmp/private-local-destination".to_string(),
            bytes_total: 10,
            bytes_done: 0,
            status: TransferStatus::Failed,
            message: Some("failed".to_string()),
            started_at: None,
            finished_at: Some(Utc::now()),
            average_bytes_per_second: None,
        };
        state
            .store
            .lock()
            .unwrap()
            .record_transfer(local_transfer.clone());
        let retry_error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "trusted-transfer-client".to_string(),
                trusted_write: true,
                command: "retry_transfer".to_string(),
                args: serde_json::json!({ "transferId": local_transfer.id }),
            },
        )
        .await
        .unwrap_err();
        assert!(retry_error.contains("at least one remote:/ssh:/load: endpoint"));

        let store = state.store.lock().unwrap();
        assert_eq!(store.audit.len(), 3);
        assert!(store.audit.iter().all(|record| record.decision == "invalid"
            && record.details.get("scope").map(String::as_str) == Some("transfer")));
        let encoded = serde_json::to_string(&store.audit).unwrap();
        for sensitive in [
            "private-a",
            "private-b",
            "private-local-source",
            "private-local-destination",
            "/home/operator",
            "/srv/",
        ] {
            assert!(!encoded.contains(sensitive), "audit leaked {sensitive}");
        }
        drop(store);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn mcp_content_transfer_validates_payload_and_stages_without_exposing_content() {
    let root = std::env::temp_dir().join(format!("portmate-mcp-content-{}", Uuid::new_v4()));
    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let valid = StartMcpContentTransferRequest {
        session_id: "session:1".to_string(),
        protocol: TransferProtocol::Xmodem,
        file_name: "firmware.bin".to_string(),
        content_base64: BASE64_STANDARD.encode(b"secret firmware bytes"),
        destination: "load:loadx".to_string(),
    };
    validate_mcp_content_transfer_request(&valid).unwrap();
    for file_name in ["../firmware.bin", "nested/firmware.bin", "C:firmware.bin"] {
        let mut invalid = valid.clone();
        invalid.file_name = file_name.to_string();
        assert!(validate_mcp_content_transfer_request(&invalid).is_err());
    }
    let mut invalid = valid.clone();
    invalid.content_base64 = "not-base64".to_string();
    assert!(validate_mcp_content_transfer_request(&invalid)
        .unwrap_err()
        .contains("valid standard Base64"));

    let maximum_payload = vec![0xa5; portmate_core::MAX_MCP_CONTENT_TRANSFER_BYTES];
    let mut maximum = valid.clone();
    maximum.file_name = "maximum.bin".to_string();
    maximum.content_base64 = BASE64_STANDARD.encode(&maximum_payload);
    validate_mcp_content_transfer_request(&maximum).unwrap();
    let (_, maximum_path) = stage_mcp_content_transfer(&state, &maximum).unwrap();
    assert_eq!(
        fs::metadata(&maximum_path).unwrap().len(),
        maximum_payload.len() as u64
    );
    cleanup_staged_mcp_content_path(&maximum_path);
    assert!(!maximum_path.exists());

    let mut oversized = maximum;
    oversized.content_base64 = BASE64_STANDARD.encode(vec![
        0xa5;
        portmate_core::MAX_MCP_CONTENT_TRANSFER_BYTES
            + 1
    ]);
    assert!(validate_mcp_content_transfer_request(&oversized)
        .unwrap_err()
        .contains("decoded limit"));

    let (source, staging_path) = stage_mcp_content_transfer(&state, &valid).unwrap();
    assert!(source.contains(".mcp-transfer-staging"));
    assert_eq!(fs::read(&staging_path).unwrap(), b"secret firmware bytes");
    assert!(!serde_json::to_string(&state.store.lock().unwrap().audit)
        .unwrap()
        .contains("secret firmware bytes"));
    let task_id = "staged-task";
    state
        .mcp_content_transfer_staging
        .lock()
        .unwrap()
        .insert(task_id.to_string(), staging_path.clone());
    cleanup_mcp_content_transfer_staging(&state, task_id);
    assert!(!staging_path.exists());
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn mcp_content_transfer_rejects_a_symlinked_staging_root() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("portmate-mcp-inline-link-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    symlink(&outside, root.join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)).unwrap();
    let request = StartMcpContentTransferRequest {
        session_id: "session:1".to_string(),
        protocol: TransferProtocol::Xmodem,
        file_name: "firmware.bin".to_string(),
        content_base64: BASE64_STANDARD.encode(b"must not leave the private root"),
        destination: "load:loadx".to_string(),
    };
    assert!(stage_mcp_content_transfer(&state, &request)
        .unwrap_err()
        .contains("invalid MCP content staging directory"));
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mcp_chunked_content_upload_is_owned_verified_and_copied_before_transfer() {
    let root =
        std::env::temp_dir().join(format!("portmate-mcp-chunked-content-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let upload_id = Uuid::new_v4().to_string();
    let upload_dir = root
        .join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)
        .join(MCP_CONTENT_UPLOADS_DIRECTORY)
        .join(&upload_id);
    fs::create_dir_all(&upload_dir).unwrap();
    let payload = b"chunked content held by a remote MCP client";
    let metadata = McpContentUploadMetadata {
        version: MCP_CONTENT_UPLOAD_METADATA_VERSION,
        upload_id: upload_id.clone(),
        client_id: "chunk-owner".to_string(),
        session_id: "session:1".to_string(),
        protocol: TransferProtocol::Xmodem,
        file_name: "firmware.bin".to_string(),
        size_bytes: payload.len() as u64,
        sha256: format!("{:x}", Sha256::digest(payload)),
        destination: "load:loadx".to_string(),
        created_at_unix_seconds: 1,
    };
    fs::write(
        upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    fs::write(upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE), payload).unwrap();

    assert!(
        load_mcp_content_upload_metadata(&state, "wrong-client", &upload_id)
            .unwrap_err()
            .contains("unknown or unavailable")
    );
    let loaded = load_mcp_content_upload_metadata(&state, "chunk-owner", &upload_id).unwrap();
    assert_eq!(loaded, metadata);
    let (source, staging_path) = stage_mcp_content_upload(&state, &loaded).unwrap();
    assert_eq!(source, staging_path.display().to_string());
    assert_eq!(fs::read(&staging_path).unwrap(), payload);
    assert_ne!(
        staging_path,
        upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE)
    );
    fs::remove_file(&staging_path).unwrap();
    fs::remove_dir(staging_path.parent().unwrap()).unwrap();

    fs::write(
        upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE),
        vec![b'x'; payload.len()],
    )
    .unwrap();
    assert!(stage_mcp_content_upload(&state, &loaded)
        .unwrap_err()
        .contains("SHA-256 mismatch"));
    let non_upload_entries = fs::read_dir(root.join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name() != MCP_CONTENT_UPLOADS_DIRECTORY)
        .count();
    assert_eq!(non_upload_entries, 0);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn mcp_chunked_content_upload_rejects_symlinked_payloads() {
    use std::os::unix::fs::symlink;

    let root = std::env::temp_dir().join(format!("portmate-mcp-upload-link-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let upload_id = Uuid::new_v4().to_string();
    let upload_dir = root
        .join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)
        .join(MCP_CONTENT_UPLOADS_DIRECTORY)
        .join(&upload_id);
    fs::create_dir_all(&upload_dir).unwrap();
    let target = root.join("outside.bin");
    fs::write(&target, b"outside").unwrap();
    let metadata = McpContentUploadMetadata {
        version: MCP_CONTENT_UPLOAD_METADATA_VERSION,
        upload_id: upload_id.clone(),
        client_id: "link-owner".to_string(),
        session_id: "session:1".to_string(),
        protocol: TransferProtocol::Xmodem,
        file_name: "firmware.bin".to_string(),
        size_bytes: 7,
        sha256: format!("{:x}", Sha256::digest(b"outside")),
        destination: "load:loadx".to_string(),
        created_at_unix_seconds: 1,
    };
    fs::write(
        upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE),
        serde_json::to_vec(&metadata).unwrap(),
    )
    .unwrap();
    symlink(&target, upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE)).unwrap();
    assert!(stage_mcp_content_upload(&state, &metadata)
        .unwrap_err()
        .contains("invalid MCP content upload payload"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mcp_chunked_content_upload_enters_the_authorized_transfer_queue() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-upload-queue-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let upload_id = Uuid::new_v4().to_string();
        let upload_dir = root
            .join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)
            .join(MCP_CONTENT_UPLOADS_DIRECTORY)
            .join(&upload_id);
        fs::create_dir_all(&upload_dir).unwrap();
        let payload = b"queued MCP upload";
        let metadata = McpContentUploadMetadata {
            version: MCP_CONTENT_UPLOAD_METADATA_VERSION,
            upload_id: upload_id.clone(),
            client_id: "queue-client".to_string(),
            session_id: "session:1".to_string(),
            protocol: TransferProtocol::Xmodem,
            file_name: "queued.bin".to_string(),
            size_bytes: payload.len() as u64,
            sha256: format!("{:x}", Sha256::digest(payload)),
            destination: "load:loadx".to_string(),
            created_at_unix_seconds: 1,
        };
        fs::write(
            upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        fs::write(upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE), payload).unwrap();

        let value = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "queue-client".to_string(),
                trusted_write: true,
                command: "start_content_upload_transfer".to_string(),
                args: serde_json::json!({ "uploadId": upload_id }),
            },
        )
        .await
        .unwrap();
        let returned: TransferTask = serde_json::from_value(value).unwrap();
        assert_eq!(returned.session_id, "session:1");
        assert_eq!(returned.protocol, TransferProtocol::Xmodem);
        assert_eq!(returned.destination, "<redacted-path>");
        {
            let store = state.store.lock().unwrap();
            let queued = store.transfer_by_id(&returned.id).unwrap();
            assert!(queued.source.contains(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY));
            assert_eq!(queued.destination, "load:loadx");
            let audit = store
                .audit
                .iter()
                .find(|record| record.action == "start_content_upload_transfer")
                .unwrap();
            assert_eq!(audit.decision, "succeeded");
            assert_eq!(
                audit.details.get("scope").map(String::as_str),
                Some("transfer")
            );
            assert!(!serde_json::to_string(audit).unwrap().contains("queued.bin"));
        }
        let terminal = wait_for_transfer_terminal_state(&state, &returned.id).await;
        assert!(matches!(
            terminal.status,
            TransferStatus::Completed | TransferStatus::Failed | TransferStatus::Cancelled
        ));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn mcp_write_revalidation_rejects_changed_targets_and_revoked_grants() {
    let root = std::env::temp_dir().join(format!("portmate-mcp-write-recheck-{}", Uuid::new_v4()));
    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let owner_session = state.store.lock().unwrap().profiles[0].id.clone();
    let transfer = TransferTask {
        id: "rechecked-transfer".to_string(),
        session_id: owner_session.clone(),
        protocol: TransferProtocol::Sftp,
        source: "/home/operator/private-source".to_string(),
        destination: "remote:/srv/private-target".to_string(),
        bytes_total: 100,
        bytes_done: 1,
        status: TransferStatus::Running,
        message: Some("running".to_string()),
        started_at: Some(Utc::now()),
        finished_at: None,
        average_bytes_per_second: None,
    };
    {
        let mut store = state.store.lock().unwrap();
        store.record_transfer(transfer.clone());
        store.grants.push(McpGrant {
            client_id: "rechecked-client".to_string(),
            name: "Rechecked client".to_string(),
            scopes: vec![McpScope::Transfer],
            allowed_sessions: vec![owner_session.clone()],
            confirm_writes: true,
            expires_at: None,
            revoked_at: None,
        });
    }
    let request = IpcRequest {
        token: "authenticated-token".to_string(),
        client_id: "rechecked-client".to_string(),
        trusted_write: false,
        command: "cancel_transfer".to_string(),
        args: serde_json::json!({ "transferId": transfer.id }),
    };

    revalidate_ipc_write_target(&state, &request, McpScope::Transfer, &owner_session, false)
        .unwrap();
    state
        .store
        .lock()
        .unwrap()
        .transfers
        .iter_mut()
        .find(|item| item.id == transfer.id)
        .unwrap()
        .session_id = "changed-session".to_string();
    assert!(revalidate_ipc_write_target(
        &state,
        &request,
        McpScope::Transfer,
        &owner_session,
        false,
    )
    .unwrap_err()
    .contains("target changed"));

    {
        let mut store = state.store.lock().unwrap();
        store
            .transfers
            .iter_mut()
            .find(|item| item.id == transfer.id)
            .unwrap()
            .session_id = owner_session.clone();
        store.grants[0].revoked_at = Some(Utc::now());
    }
    assert!(revalidate_ipc_write_target(
        &state,
        &request,
        McpScope::Transfer,
        &owner_session,
        false,
    )
    .unwrap_err()
    .contains("grant changed"));

    {
        let mut store = state.store.lock().unwrap();
        store.grants.clear();
    }
    let trusted_request = IpcRequest {
        trusted_write: true,
        ..request.clone()
    };
    assert!(revalidate_ipc_write_target(
        &state,
        &trusted_request,
        McpScope::Transfer,
        &owner_session,
        false,
    )
    .unwrap_err()
    .contains("grant changed"));
    revalidate_ipc_write_target(
        &state,
        &trusted_request,
        McpScope::Transfer,
        &owner_session,
        true,
    )
    .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stale_mcp_transfer_authorization_is_rejected_at_each_commit_point() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-transfer-commit-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let client_id = "stale-transfer-client";
        let running = TransferTask {
            id: "stale-transfer-running".to_string(),
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Xmodem,
            source: root.join("running.bin").display().to_string(),
            destination: "load:loadx".to_string(),
            bytes_total: 100,
            bytes_done: 1,
            status: TransferStatus::Running,
            message: Some("running".to_string()),
            started_at: Some(Utc::now()),
            finished_at: None,
            average_bytes_per_second: None,
        };
        let failed = TransferTask {
            id: "stale-transfer-failed".to_string(),
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Xmodem,
            source: root.join("failed.bin").display().to_string(),
            destination: "load:loadx".to_string(),
            bytes_total: 100,
            bytes_done: 1,
            status: TransferStatus::Failed,
            message: Some("failed".to_string()),
            started_at: Some(Utc::now()),
            finished_at: Some(Utc::now()),
            average_bytes_per_second: None,
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut store = state.store.lock().unwrap();
            store.record_transfer(running.clone());
            store.record_transfer(failed.clone());
            store.grants.push(McpGrant {
                client_id: client_id.to_string(),
                name: "Stale transfer client".to_string(),
                scopes: vec![McpScope::Transfer],
                allowed_sessions: vec![profile.id.clone()],
                confirm_writes: false,
                expires_at: None,
                revoked_at: None,
            });
        }
        state.transfer_cancellations.lock().unwrap().insert(
            running.id.clone(),
            Arc::new(TransferCancellation::with_flag(Arc::clone(&cancelled))),
        );

        let upload_id = Uuid::new_v4().to_string();
        let upload_dir = root
            .join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY)
            .join(MCP_CONTENT_UPLOADS_DIRECTORY)
            .join(&upload_id);
        fs::create_dir_all(&upload_dir).unwrap();
        let upload_payload = b"original resumable MCP upload";
        let upload_metadata = McpContentUploadMetadata {
            version: MCP_CONTENT_UPLOAD_METADATA_VERSION,
            upload_id: upload_id.clone(),
            client_id: client_id.to_string(),
            session_id: profile.id.clone(),
            protocol: TransferProtocol::Xmodem,
            file_name: "uploaded.bin".to_string(),
            size_bytes: upload_payload.len() as u64,
            sha256: format!("{:x}", Sha256::digest(upload_payload)),
            destination: "load:loadx".to_string(),
            created_at_unix_seconds: 1,
        };
        fs::write(
            upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE),
            serde_json::to_vec(&upload_metadata).unwrap(),
        )
        .unwrap();
        fs::write(
            upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE),
            upload_payload,
        )
        .unwrap();

        let execution = McpWriteExecutionContext::Generic;
        let authorization =
            McpWriteAuthorizationContext::new(McpScope::Transfer, profile.id.clone(), false);
        state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());

        let requests = [
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: client_id.to_string(),
                trusted_write: false,
                command: "start_transfer".to_string(),
                args: serde_json::json!({
                    "sessionId": profile.id,
                    "protocol": "xmodem",
                    "source": root.join("direct.bin").display().to_string(),
                    "destination": "load:loadx",
                }),
            },
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: client_id.to_string(),
                trusted_write: false,
                command: "start_content_transfer".to_string(),
                args: serde_json::json!({
                    "sessionId": profile.id,
                    "protocol": "xmodem",
                    "fileName": "inline.bin",
                    "contentBase64": BASE64_STANDARD.encode(b"inline content must be rolled back"),
                    "destination": "load:loadx",
                }),
            },
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: client_id.to_string(),
                trusted_write: false,
                command: "start_content_upload_transfer".to_string(),
                args: serde_json::json!({ "uploadId": upload_id }),
            },
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: client_id.to_string(),
                trusted_write: false,
                command: "cancel_transfer".to_string(),
                args: serde_json::json!({ "transferId": running.id }),
            },
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: client_id.to_string(),
                trusted_write: false,
                command: "retry_transfer".to_string(),
                args: serde_json::json!({ "transferId": failed.id }),
            },
        ];
        for request in requests {
            let command = request.command.clone();
            let error = execute_ipc_request_with_context(
                state.clone(),
                request,
                &execution,
                &authorization,
            )
            .await
            .unwrap_err();
            assert!(
                error.contains("grant changed"),
                "{command} returned an unexpected error: {error}"
            );
        }

        assert!(!cancelled.load(Ordering::SeqCst));
        assert_eq!(
            state.transfer_task_slots.available_permits(),
            MAX_ACTIVE_TRANSFER_TASKS
        );
        assert_eq!(state.transfer_cancellations.lock().unwrap().len(), 1);
        assert!(state
            .mcp_content_transfer_staging
            .lock()
            .unwrap()
            .is_empty());
        let store = state.store.lock().unwrap();
        assert_eq!(store.transfers.len(), 2);
        assert_eq!(
            store.transfer_by_id(&running.id).unwrap().status,
            TransferStatus::Running
        );
        assert_eq!(
            store.transfer_by_id(&failed.id).unwrap().status,
            TransferStatus::Failed
        );
        assert!(store.events.is_empty());
        drop(store);

        assert_eq!(
            fs::read(upload_dir.join(MCP_CONTENT_UPLOAD_PAYLOAD_FILE)).unwrap(),
            upload_payload
        );
        assert!(upload_dir.join(MCP_CONTENT_UPLOAD_METADATA_FILE).is_file());
        let non_upload_entries = fs::read_dir(root.join(MCP_CONTENT_UPLOAD_STAGING_DIRECTORY))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name() != MCP_CONTENT_UPLOADS_DIRECTORY)
            .count();
        assert_eq!(non_upload_entries, 0);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn queued_mcp_input_revalidates_grants_at_the_outbound_commit_point() {
    tauri::async_runtime::block_on(async {
        for (index, command) in [
            "send_text",
            "send_key",
            "run_command",
            "attach_tmux",
            "run_custom_script",
        ]
        .into_iter()
        .enumerate()
        {
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::time::timeout(Duration::from_millis(300), socket.read_u8()).await
            });
            let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: false,
                ..Default::default()
            }));
            let root = std::env::temp_dir().join(format!(
                "portmate-mcp-outbound-commit-{index}-{}",
                Uuid::new_v4()
            ));
            fs::create_dir_all(&root).unwrap();
            let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
            let stream = TcpStream::connect(address).await.unwrap();
            let (_reader, writer) = stream.into_split();
            let (tap, _) = broadcast::channel(8);
            state.tcp.lock().unwrap().insert(
                profile.id.clone(),
                TcpRuntime {
                    runtime_id: Uuid::new_v4().to_string(),
                    writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
                    tap,
                    closed: Arc::new(AtomicBool::new(false)),
                    telnet: None,
                },
            );

            let client_id = format!("queued-input-{index}");
            let scope = if command == "run_custom_script" {
                McpScope::RunScripts
            } else {
                McpScope::WriteInput
            };
            let script_timestamp = Utc::now();
            let script = CustomScript {
                id: Uuid::new_v4().to_string(),
                name: "Queued MCP script".to_string(),
                description: "Grant revocation regression".to_string(),
                content: "must-not-run".to_string(),
                allow_all_sessions: false,
                allowed_session_ids: vec![profile.id.clone()],
                mcp_enabled: true,
                created_at: script_timestamp,
                updated_at: script_timestamp,
            };
            let args = match command {
                "send_text" => serde_json::json!({
                    "sessionId": profile.id.clone(),
                    "text": "must-not-send",
                }),
                "send_key" => serde_json::json!({
                    "sessionId": profile.id.clone(),
                    "key": "Enter",
                }),
                "run_command" => serde_json::json!({
                    "sessionId": profile.id.clone(),
                    "command": "must-not-run",
                }),
                "attach_tmux" => serde_json::json!({
                    "sessionId": profile.id.clone(),
                    "target": "main",
                }),
                "run_custom_script" => serde_json::json!({
                    "sessionId": profile.id.clone(),
                    "scriptId": script.id.clone(),
                }),
                _ => unreachable!(),
            };
            {
                let mut store = state.store.lock().unwrap();
                if command == "run_custom_script" {
                    store.custom_scripts.push(script);
                }
                store.grants.push(McpGrant {
                    client_id: client_id.clone(),
                    name: "Queued input".to_string(),
                    scopes: vec![scope],
                    allowed_sessions: vec![profile.id.clone()],
                    confirm_writes: false,
                    expires_at: None,
                    revoked_at: None,
                });
            }

            let lane = outbound_lane(&state.store_path, &profile.id).unwrap();
            let lane_guard = lane.lock().await;
            let request = IpcRequest {
                token: "authenticated-token".to_string(),
                client_id,
                trusted_write: false,
                command: command.to_string(),
                args,
            };
            let run_state = state.clone();
            let run = tokio::spawn(async move { handle_ipc_request(run_state, request).await });
            tokio::time::timeout(Duration::from_secs(2), async {
                while Arc::strong_count(&lane) < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("{command} did not enter the outbound queue"));

            state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());
            drop(lane_guard);

            let error = run.await.unwrap().unwrap_err();
            assert!(
                error.contains("grant changed"),
                "{command} returned an unexpected error: {error}"
            );
            assert!(
                server.await.unwrap().is_err(),
                "{command} wrote bytes after its grant was revoked"
            );
            let store = state.store.lock().unwrap();
            assert!(store.events.is_empty());
            assert_eq!(
                store
                    .audit
                    .iter()
                    .find(|record| record.action == command)
                    .map(|record| record.decision.as_str()),
                Some("failed")
            );
            drop(store);
            let _ = fs::remove_dir_all(root);
        }
    });
}

#[test]
fn queued_mcp_open_session_revalidates_its_grant_before_connecting() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-open-commit-{}", Uuid::new_v4()));
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "queued-open-client".to_string(),
            name: "Queued open client".to_string(),
            scopes: vec![McpScope::ManageSessions],
            allowed_sessions: vec![profile.id.clone()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let lane = session_lifecycle_lane(&state, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let request = IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "queued-open-client".to_string(),
            trusted_write: false,
            command: "open_session".to_string(),
            args: serde_json::json!({ "sessionId": profile.id }),
        };
        let run_state = state.clone();
        let run = tokio::spawn(async move { handle_ipc_request(run_state, request).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&lane) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("open_session did not enter the lifecycle queue");

        state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());
        drop(lane_guard);

        let error = run.await.unwrap().unwrap_err();
        assert!(error.contains("grant changed"), "{error}");
        assert!(state.shell.lock().unwrap().is_empty());
        let store = state.store.lock().unwrap();
        assert_eq!(
            store.summaries()[0].runtime.status,
            SessionStatus::Disconnected
        );
        assert!(store.events.is_empty());
        assert_eq!(
            store
                .audit
                .iter()
                .find(|record| record.action == "open_session")
                .map(|record| record.decision.as_str()),
            Some("failed")
        );
        drop(store);
        assert_eq!(
            state.session_open_slots.available_permits(),
            MAX_CONCURRENT_SESSION_OPENS
        );
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn queued_mcp_close_session_revalidates_its_grant_before_disconnect() {
    let _runtime_guard = shared_runtime_test_guard();
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.path().join("portmate-store.sqlite3"));
        let opened = open_session_inner(
            state.clone(),
            profile.id.clone(),
            SessionOpenCredentials::default(),
        )
        .await
        .unwrap();
        assert_eq!(opened.runtime.status, SessionStatus::Connected);
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "queued-close-client".to_string(),
            name: "Queued close client".to_string(),
            scopes: vec![McpScope::ManageSessions],
            allowed_sessions: vec![profile.id.clone()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let lane = session_lifecycle_lane(&state, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let request = IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "queued-close-client".to_string(),
            trusted_write: false,
            command: "close_session".to_string(),
            args: serde_json::json!({ "sessionId": profile.id }),
        };
        let run_state = state.clone();
        let run = tokio::spawn(async move { handle_ipc_request(run_state, request).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&lane) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("close_session did not enter the lifecycle queue");

        state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());
        drop(lane_guard);

        let error = run.await.unwrap().unwrap_err();
        assert!(error.contains("grant changed"), "{error}");
        assert!(state.shell.lock().unwrap().contains_key(&profile.id));
        {
            let store = state.store.lock().unwrap();
            assert_eq!(
                store.summaries()[0].runtime.status,
                SessionStatus::Connected
            );
            assert_eq!(
                store
                    .audit
                    .iter()
                    .find(|record| record.action == "close_session")
                    .map(|record| record.decision.as_str()),
                Some("failed")
            );
        }

        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
    });
}

#[test]
fn authorized_mcp_close_finishes_after_cancelling_a_pending_open() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let profile = test_shell_profile();
        let state = test_app_state(profile.clone(), root.path().join("portmate-store.sqlite3"));
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "pending-open-close-client".to_string(),
            name: "Pending open close client".to_string(),
            scopes: vec![McpScope::ManageSessions],
            allowed_sessions: vec![profile.id.clone()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });
        let pending_open = register_session_open_cancellation(&state, &profile.id).unwrap();
        let lane = session_lifecycle_lane(&state, &profile.id).unwrap();
        let lane_guard = lane.lock().await;
        let request = IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "pending-open-close-client".to_string(),
            trusted_write: false,
            command: "close_session".to_string(),
            args: serde_json::json!({ "sessionId": profile.id }),
        };
        let run_state = state.clone();
        let run = tokio::spawn(async move { handle_ipc_request(run_state, request).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while !pending_open.is_cancelled() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("close_session did not cancel the pending open before waiting");

        state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());
        drop(lane_guard);

        let closed = run.await.unwrap().unwrap();
        assert_eq!(
            closed
                .get("runtime")
                .and_then(|runtime| runtime.get("status"))
                .and_then(serde_json::Value::as_str),
            Some("disconnected")
        );
        let store = state.store.lock().unwrap();
        assert_eq!(
            store
                .audit
                .iter()
                .find(|record| record.action == "close_session")
                .map(|record| record.decision.as_str()),
            Some("succeeded")
        );
        drop(store);
        drop(pending_open);
        assert_eq!(
            state.session_open_slots.available_permits(),
            MAX_CONCURRENT_SESSION_OPENS
        );
    });
}

#[test]
fn mcp_cancel_transfer_authorizes_the_recorded_session_not_client_arguments() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-transfer-owner-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let owner_session = state.store.lock().unwrap().profiles[0].id.clone();
        let mut other = test_shell_profile();
        other.id = "other-session".to_string();
        other.name = "Other session".to_string();
        let transfer = TransferTask {
            id: "owned-transfer".to_string(),
            session_id: owner_session.clone(),
            protocol: TransferProtocol::Sftp,
            source: "/home/operator/private-source".to_string(),
            destination: "remote:/srv/private-target".to_string(),
            bytes_total: 100,
            bytes_done: 1,
            status: TransferStatus::Running,
            message: Some("running".to_string()),
            started_at: Some(Utc::now()),
            finished_at: None,
            average_bytes_per_second: None,
        };
        {
            let mut store = state.store.lock().unwrap();
            store.upsert_profile(other);
            store.record_transfer(transfer.clone());
            store.grants.push(McpGrant {
                client_id: "other-only".to_string(),
                name: "Other only".to_string(),
                scopes: vec![McpScope::Transfer],
                allowed_sessions: vec!["other-session".to_string()],
                confirm_writes: false,
                expires_at: None,
                revoked_at: None,
            });
        }

        let denied = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "other-only".to_string(),
                trusted_write: false,
                command: "cancel_transfer".to_string(),
                args: serde_json::json!({
                    "transferId": transfer.id,
                    "sessionId": "other-session"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(denied.contains(&owner_session));
        let store = state.store.lock().unwrap();
        assert_eq!(
            store.transfer_by_id(&transfer.id).unwrap().status,
            TransferStatus::Running
        );
        assert_eq!(store.audit.len(), 1);
        assert_eq!(
            store.audit[0].session_id.as_deref(),
            Some(owner_session.as_str())
        );
        assert_eq!(store.audit[0].decision, "denied");
        assert!(!serde_json::to_string(&store.audit)
            .unwrap()
            .contains("private-source"));
        drop(store);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn mcp_stop_tunnel_authorizes_runtime_owner_and_tunnel_reads_are_scoped() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-tunnel-owner-{}", Uuid::new_v4()));
        let state = test_app_state(test_ssh_profile(), root.join("portmate-store.sqlite3"));
        let owner_session = state.store.lock().unwrap().profiles[0].id.clone();
        let mut other = test_ssh_profile();
        other.id = "other-ssh-session".to_string();
        other.name = "Other SSH".to_string();
        let tunnel = TunnelSpec {
            id: "owned-tunnel".to_string(),
            label: "token=route-label-secret".to_string(),
            mode: TunnelMode::Dynamic,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_080,
            target_host: String::new(),
            target_port: 0,
            route_rules: Vec::new(),
            enabled: true,
        };
        {
            let mut store = state.store.lock().unwrap();
            store.upsert_profile(other);
            store.grants.push(McpGrant {
                client_id: "route-client".to_string(),
                name: "Route client".to_string(),
                scopes: vec![McpScope::ReadTunnels, McpScope::Tunnel],
                allowed_sessions: vec!["other-ssh-session".to_string()],
                confirm_writes: false,
                expires_at: None,
                revoked_at: None,
            });
        }
        let closed = Arc::new(AtomicBool::new(false));
        state.tunnels.lock().unwrap().insert(
            tunnel.id.clone(),
            TunnelRuntime {
                session_id: owner_session.clone(),
                ssh_runtime_id: "ssh-runtime-owner".to_string(),
                spec: tunnel.clone(),
                metrics: Arc::new(TunnelMetrics::default()),
                closed: Arc::clone(&closed),
                listener_worker: TunnelListenerWorker::completed(),
            },
        );
        let request = |command: &str, args: serde_json::Value| IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "route-client".to_string(),
            trusted_write: false,
            command: command.to_string(),
            args,
        };

        assert!(handle_ipc_request(
            state.clone(),
            request(
                "list_tunnels",
                serde_json::json!({ "sessionId": owner_session }),
            ),
        )
        .await
        .unwrap_err()
        .contains("ReadTunnels"));
        let listed = handle_ipc_request(
            state.clone(),
            request(
                "list_tunnels",
                serde_json::json!({ "sessionId": "other-ssh-session" }),
            ),
        )
        .await
        .unwrap();
        assert_eq!(listed, serde_json::json!([]));

        let denied = handle_ipc_request(
            state.clone(),
            request(
                "stop_tunnel",
                serde_json::json!({
                    "tunnelId": tunnel.id,
                    "sessionId": "other-ssh-session"
                }),
            ),
        )
        .await
        .unwrap_err();
        assert!(denied.contains(&owner_session));
        assert!(!closed.load(Ordering::SeqCst));
        assert!(state.tunnels.lock().unwrap().contains_key(&tunnel.id));
        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].session_id.as_deref(), Some(owner_session.as_str()));
        assert_eq!(audit[0].decision, "denied");
        assert!(!serde_json::to_string(&audit)
            .unwrap()
            .contains("route-label-secret"));
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn queued_mcp_stop_tunnel_revalidates_its_grant_before_stopping() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-tunnel-commit-{}", Uuid::new_v4()));
        let state = test_app_state(test_ssh_profile(), root.join("portmate-store.sqlite3"));
        let owner_session = state.store.lock().unwrap().profiles[0].id.clone();
        let tunnel = TunnelSpec {
            id: "queued-stop-tunnel".to_string(),
            label: "Queued stop tunnel".to_string(),
            mode: TunnelMode::Dynamic,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_081,
            target_host: String::new(),
            target_port: 0,
            route_rules: Vec::new(),
            enabled: true,
        };
        let closed = Arc::new(AtomicBool::new(false));
        state.tunnels.lock().unwrap().insert(
            tunnel.id.clone(),
            TunnelRuntime {
                session_id: owner_session.clone(),
                ssh_runtime_id: "ssh-runtime-owner".to_string(),
                spec: tunnel.clone(),
                metrics: Arc::new(TunnelMetrics::default()),
                closed: Arc::clone(&closed),
                listener_worker: TunnelListenerWorker::completed(),
            },
        );
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "queued-tunnel-client".to_string(),
            name: "Queued tunnel client".to_string(),
            scopes: vec![McpScope::Tunnel],
            allowed_sessions: vec![owner_session],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let lane = tunnel_lifecycle_lane(&state, &tunnel.id).unwrap();
        let lane_guard = lane.lock().await;
        let request = IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "queued-tunnel-client".to_string(),
            trusted_write: false,
            command: "stop_tunnel".to_string(),
            args: serde_json::json!({ "tunnelId": tunnel.id }),
        };
        let run_state = state.clone();
        let run = tokio::spawn(async move { handle_ipc_request(run_state, request).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while Arc::strong_count(&lane) < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("stop_tunnel did not enter the lifecycle queue");

        state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());
        drop(lane_guard);

        let error = run.await.unwrap().unwrap_err();
        assert!(error.contains("grant changed"), "{error}");
        assert!(!closed.load(Ordering::SeqCst));
        assert!(state.tunnels.lock().unwrap().contains_key(&tunnel.id));
        let store = state.store.lock().unwrap();
        assert_eq!(
            store
                .audit
                .iter()
                .find(|record| record.action == "stop_tunnel")
                .map(|record| record.decision.as_str()),
            Some("failed")
        );
        drop(store);
        let _ = fs::remove_dir_all(root);
    });
}

#[cfg(unix)]
#[test]
fn mcp_and_remote_server_tunnel_staging_failures_are_rolled_back() {
    if Command::new("ssh-keygen").arg("-V").output().is_err() {
        eprintln!("skipping MCP tunnel commit test: ssh-keygen is not installed");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let host_key = root.path().join("ssh_host_ed25519_key");
    generate_ed25519_test_key(&host_key);

    tauri::async_runtime::block_on(async {
        let profile = test_ssh_profile();
        let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        let username = "portmate-mcp-tunnel-user";
        let secret = "PortMate MCP tunnel secret";
        let (port, counters, server_task) =
            spawn_mixed_auth_test_server(&host_key, username, secret).await;
        let remote_forwards = Arc::new(Mutex::new(HashMap::new()));
        let handler = PortMateSshHandler {
            profile_id: profile.id.clone(),
            host: "127.0.0.1".to_string(),
            port,
            alias: None,
            policy: portmate_core::HostKeyPolicy {
                mode: HostKeyMode::TrustOnFirstUse,
                alias: None,
                trust_scope: HostKeyScope::Profile,
                allow_rotation: false,
                check_ip: false,
            },
            host_keys: state.store.lock().unwrap().host_keys.clone(),
            one_time_host_key_ids: Vec::new(),
            observed_key: Arc::new(Mutex::new(None)),
            host_key_error: Arc::new(Mutex::new(None)),
            remote_forwards: Arc::clone(&remote_forwards),
        };
        let mut ssh = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            handler,
        )
        .await
        .unwrap();
        assert!(ssh
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let terminal = SshBackendChannel::from_russh(ssh.channel_open_session().await.unwrap());
        let (_reader, writer) = terminal.split();
        let handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_russh(ssh)));
        let (tap, _) = broadcast::channel(1);
        let (_reader_finished_sender, reader_finished) = tokio::sync::oneshot::channel();
        state.ssh.lock().unwrap().insert(
            profile.id.clone(),
            SshRuntime {
                runtime_id: "mcp-tunnel-runtime".to_string(),
                profile_snapshot: ssh_health::ssh_health_profile_snapshot(&profile).unwrap(),
                backend: SshBackendKind::Russh,
                auth_method: AuthMethod::Password,
                handle,
                sftp: Arc::new(tokio::sync::Mutex::new(None)),
                jump_handles: Vec::new(),
                writer: Arc::new(tokio::sync::Mutex::new(writer)),
                tap,
                remote_forwards: Arc::clone(&remote_forwards),
                remote_forward_acceptor_started: Arc::new(AtomicBool::new(false)),
                agent_forwarder_finished: None,
                transport_bridge_finished: None,
                closed: Arc::new(AtomicBool::new(false)),
                terminal_channel_open: Arc::new(AtomicBool::new(true)),
                reader_finished,
            },
        );
        state
            .store
            .lock()
            .unwrap()
            .set_runtime_status(&profile.id, SessionStatus::Connected)
            .unwrap();

        let occupied_spec = TunnelSpec {
            id: "occupied-remote-route".to_string(),
            label: "Occupied remote route".to_string(),
            mode: TunnelMode::Remote,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 41_922,
            target_host: "127.0.0.1".to_string(),
            target_port: 9,
            route_rules: Vec::new(),
            enabled: true,
        };
        let occupied_metrics = Arc::new(TunnelMetrics::default());
        let occupied_target = TunnelForwardTarget {
            spec: occupied_spec.clone(),
            metrics: occupied_metrics,
            connection_slots: Arc::clone(&state.tunnel_connection_slots),
        };
        {
            let mut forwards = remote_forwards.lock().unwrap();
            forwards.insert(
                remote_forward_key(&occupied_spec.bind_host, occupied_spec.bind_port),
                occupied_target.clone(),
            );
            forwards.insert(
                remote_forward_port_key(occupied_spec.bind_port),
                occupied_target,
            );
        }
        let occupied_error = create_tunnel_inner(
            &state,
            CreateTunnelRequest {
                session_id: profile.id.clone(),
                mode: TunnelMode::Remote,
                bind_host: occupied_spec.bind_host.clone(),
                bind_port: occupied_spec.bind_port,
                target_host: "127.0.0.1".to_string(),
                target_port: 9,
                route_rules: Vec::new(),
                label: None,
            },
        )
        .await
        .unwrap_err();
        assert!(
            occupied_error.contains("route already registered"),
            "{occupied_error}"
        );
        assert_eq!(counters.remote_forward_requests.load(Ordering::SeqCst), 0);
        assert_eq!(remote_forwards.lock().unwrap().len(), 2);
        remote_forwards.lock().unwrap().clear();

        let client_id = "stale-create-tunnel-client";
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: client_id.to_string(),
            name: "Stale create tunnel client".to_string(),
            scopes: vec![McpScope::Tunnel],
            allowed_sessions: vec![profile.id.clone()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });
        let execution = McpWriteExecutionContext::Generic;
        let authorization =
            McpWriteAuthorizationContext::new(McpScope::Tunnel, profile.id.clone(), false);
        state.store.lock().unwrap().grants[0].revoked_at = Some(Utc::now());

        let reservation = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let local_port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let request = |mode: &str, bind_port: u16| IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: client_id.to_string(),
            trusted_write: false,
            command: "create_tunnel".to_string(),
            args: serde_json::json!({
                "sessionId": profile.id,
                "mode": mode,
                "bindHost": "127.0.0.1",
                "bindPort": bind_port,
                "targetHost": "127.0.0.1",
                "targetPort": 9,
                "routeRules": [],
            }),
        };

        let local_error = execute_ipc_request_with_context(
            state.clone(),
            request("local", local_port),
            &execution,
            &authorization,
        )
        .await
        .unwrap_err();
        assert!(local_error.contains("grant changed"), "{local_error}");
        let rebound = TcpListener::bind(("127.0.0.1", local_port))
            .await
            .expect("stale MCP local tunnel retained its staged listener");
        drop(rebound);

        let remote_error = execute_ipc_request_with_context(
            state.clone(),
            request("remote", 41_923),
            &execution,
            &authorization,
        )
        .await
        .unwrap_err();
        assert!(remote_error.contains("grant changed"), "{remote_error}");
        assert_eq!(counters.remote_forward_requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            counters.remote_forward_cancellations.load(Ordering::SeqCst),
            1
        );

        let assigned_port_error = create_tunnel_inner(
            &state,
            CreateTunnelRequest {
                session_id: profile.id.clone(),
                mode: TunnelMode::Remote,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
                target_host: "127.0.0.1".to_string(),
                target_port: 9,
                route_rules: Vec::new(),
                label: None,
            },
        )
        .await
        .unwrap_err();
        assert!(
            assigned_port_error.contains("did not return an assigned port"),
            "{assigned_port_error}"
        );
        assert_eq!(counters.remote_forward_requests.load(Ordering::SeqCst), 2);
        assert_eq!(
            counters.remote_forward_cancellations.load(Ordering::SeqCst),
            2
        );
        assert_eq!(
            counters
                .last_remote_forward_cancellation_port
                .load(Ordering::SeqCst),
            0
        );

        counters
            .remote_forward_assigned_port
            .store(41_924, Ordering::SeqCst);
        let assigned_then_revoked_error = execute_ipc_request_with_context(
            state.clone(),
            request("remote", 0),
            &execution,
            &authorization,
        )
        .await
        .unwrap_err();
        assert!(
            assigned_then_revoked_error.contains("grant changed"),
            "{assigned_then_revoked_error}"
        );
        assert_eq!(counters.remote_forward_requests.load(Ordering::SeqCst), 3);
        assert_eq!(
            counters.remote_forward_cancellations.load(Ordering::SeqCst),
            3
        );
        assert_eq!(
            counters
                .last_remote_forward_cancellation_port
                .load(Ordering::SeqCst),
            41_924
        );
        assert!(remote_forwards.lock().unwrap().is_empty());
        assert!(state.tunnels.lock().unwrap().is_empty());
        {
            let store = state.store.lock().unwrap();
            let stored_profile = store.profile(&profile.id).unwrap();
            let tunnels = match &stored_profile.connection {
                ConnectionConfig::Ssh(ssh) => &ssh.tunnels,
                _ => unreachable!(),
            };
            assert!(tunnels.is_empty());
            assert!(store.events.is_empty());
        }

        let timeout_handle = Arc::clone(&state.ssh.lock().unwrap()[&profile.id].handle);
        counters
            .remote_forward_delay_millis
            .store(100, Ordering::SeqCst);
        let timeout_result = listen_remote_tunnel_forward_with_timeout(
            &timeout_handle,
            "127.0.0.1".to_string(),
            41_925,
            Duration::from_millis(20),
            "test remote forward",
        )
        .await;
        let timeout_error = match timeout_result {
            Ok(_) => panic!("remote forward request unexpectedly completed"),
            Err(error) => error,
        };
        assert!(timeout_error.contains("timed out"), "{timeout_error}");
        counters
            .remote_forward_delay_millis
            .store(0, Ordering::SeqCst);

        let cancel_handler = PortMateSshHandler {
            profile_id: profile.id.clone(),
            host: "127.0.0.1".to_string(),
            port,
            alias: None,
            policy: portmate_core::HostKeyPolicy {
                mode: HostKeyMode::TrustOnFirstUse,
                alias: None,
                trust_scope: HostKeyScope::Profile,
                allow_rotation: false,
                check_ip: false,
            },
            host_keys: state.store.lock().unwrap().host_keys.clone(),
            one_time_host_key_ids: Vec::new(),
            observed_key: Arc::new(Mutex::new(None)),
            host_key_error: Arc::new(Mutex::new(None)),
            remote_forwards: Arc::new(Mutex::new(HashMap::new())),
        };
        let mut cancel_ssh = client::connect(
            Arc::new(client::Config::default()),
            ("127.0.0.1", port),
            cancel_handler,
        )
        .await
        .unwrap();
        assert!(cancel_ssh
            .authenticate_password(username, secret)
            .await
            .unwrap()
            .success());
        let cancel_handle = Arc::new(tokio::sync::Mutex::new(SshBackendSession::from_russh(
            cancel_ssh,
        )));
        let cancellation_count = counters.remote_forward_cancellations.load(Ordering::SeqCst);
        counters
            .remote_forward_cancel_delay_millis
            .store(100, Ordering::SeqCst);
        let cancel_error = cancel_remote_tunnel_forward_with_timeout(
            &cancel_handle,
            "127.0.0.1".to_string(),
            41_926,
            Duration::from_millis(20),
            "test remote forward cancel",
        )
        .await
        .unwrap_err();
        assert!(cancel_error.contains("timed out"), "{cancel_error}");
        assert_eq!(
            counters.remote_forward_cancellations.load(Ordering::SeqCst),
            cancellation_count + 1
        );
        counters
            .remote_forward_cancel_delay_millis
            .store(0, Ordering::SeqCst);
        server_task.abort();
    });
}

#[test]
fn trusted_mcp_input_uses_client_actor_and_exact_tool_audit() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 8];
            socket.read_exact(&mut received).await.unwrap();
            received
        });
        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-input-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let stream = TcpStream::connect(address).await.unwrap();
        let (_reader, writer) = stream.into_split();
        let (tap, _) = broadcast::channel(8);
        state.tcp.lock().unwrap().insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: Uuid::new_v4().to_string(),
                writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
                tap,
                closed: Arc::new(AtomicBool::new(false)),
                telnet: None,
            },
        );

        let key_event: SessionEvent = serde_json::from_value(
            handle_ipc_request(
                state.clone(),
                IpcRequest {
                    token: "authenticated-token".to_string(),
                    client_id: "mcp-e2e-client".to_string(),
                    trusted_write: true,
                    command: "send_key".to_string(),
                    args: serde_json::json!({
                        "sessionId": profile.id,
                        "key": "Enter"
                    }),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();
        let command_event: SessionEvent = serde_json::from_value(
            handle_ipc_request(
                state.clone(),
                IpcRequest {
                    token: "authenticated-token".to_string(),
                    client_id: "mcp-e2e-client".to_string(),
                    trusted_write: true,
                    command: "run_command".to_string(),
                    args: serde_json::json!({
                        "sessionId": profile.id,
                        "command": "status"
                    }),
                },
            )
            .await
            .unwrap(),
        )
        .unwrap();

        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("TCP server timed out")
            .expect("TCP server failed");
        assert_eq!(&received, b"\rstatus\n");
        for event in [&key_event, &command_event] {
            assert_eq!(
                event.annotations.get("actor").map(String::as_str),
                Some("mcp-e2e-client")
            );
        }
        assert!(!key_event.annotations.contains_key("commandId"));
        assert!(command_event.annotations.contains_key("commandId"));
        assert_eq!(
            command_event
                .annotations
                .get("commandState")
                .map(String::as_str),
            Some("started")
        );

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(
            audit.len(),
            2,
            "MCP input must not add implicit send_text audits"
        );
        assert_eq!(audit[0].action, "send_key");
        assert_eq!(audit[1].action, "run_command");
        assert!(audit.iter().all(|record| record.actor == "mcp-e2e-client"));
        assert!(audit.iter().all(|record| record.decision == "succeeded"));
        assert!(audit.iter().all(|record| {
            record.details.get("trustedBootstrap").map(String::as_str) == Some("true")
        }));
        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn failed_mcp_write_finalizes_audit_without_secret_arguments() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-audit-failed-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        let error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "failed-client".to_string(),
                trusted_write: true,
                command: "run_command".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "command": "password=failed-command-secret"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(!error.is_empty());
        assert!(!error.contains("failed-command-secret"));

        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].actor, "failed-client");
        assert_eq!(audit[0].action, "run_command");
        assert_eq!(audit[0].decision, "failed");
        assert!(!serde_json::to_string(&audit)
            .unwrap()
            .contains("failed-command-secret"));
        let persisted = load_store_sqlite(&state.store_path).unwrap();
        assert_eq!(persisted.audit, audit);

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn applied_mcp_audit_keeps_final_truth_when_persistence_fails() {
    let mut store = SessionStore::default();
    store.record_audit(AuditRecord {
        id: "audit-applied".to_string(),
        ts: Utc::now(),
        actor: "mcp:test-client".to_string(),
        action: "send_text".to_string(),
        session_id: Some("session-1".to_string()),
        decision: "authorized".to_string(),
        details: BTreeMap::new(),
    });

    let error = finish_applied_mcp_write_audit_with(
        &mut store,
        "audit-applied",
        "succeeded",
        Some("approved"),
        |_| Err("disk full".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();

    assert_eq!(error, "disk full");
    let audit = store
        .audit
        .iter()
        .find(|record| record.id == "audit-applied")
        .unwrap();
    assert_eq!(audit.decision, "succeeded");
    assert_eq!(
        audit.details.get("approval").map(String::as_str),
        Some("approved")
    );
    assert_eq!(
        audit
            .details
            .get("finalizationPersistence")
            .map(String::as_str),
        Some("degraded")
    );
}

#[test]
fn applied_mcp_audit_accepts_a_verified_post_commit_error() {
    let mut store = SessionStore::default();
    store.record_audit(AuditRecord {
        id: "audit-verified".to_string(),
        ts: Utc::now(),
        actor: "mcp:test-client".to_string(),
        action: "run_command".to_string(),
        session_id: Some("session-1".to_string()),
        decision: "authorized".to_string(),
        details: BTreeMap::new(),
    });

    finish_applied_mcp_write_audit_with(
        &mut store,
        "audit-verified",
        "failed",
        None,
        |_| Err("post-commit version read failed".to_string()),
        |_| Ok(true),
    )
    .unwrap();

    let audit = store
        .audit
        .iter()
        .find(|record| record.id == "audit-verified")
        .unwrap();
    assert_eq!(audit.decision, "failed");
    assert!(!audit.details.contains_key("finalizationPersistence"));
}

#[test]
fn log_query_limit_matches_mcp_schema_bounds() {
    assert_eq!(bounded_log_query_limit(None), 100);
    assert_eq!(bounded_log_query_limit(Some(0)), 1);
    assert_eq!(bounded_log_query_limit(Some(600)), 600);
    assert_eq!(bounded_log_query_limit(Some(u64::MAX)), 1000);
}

#[test]
fn mcp_http_config_uses_bridge_token_ref_and_loopback_endpoint() {
    let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
    let store_path = Path::new("/home/operator/PortMate Data/portmate-store.sqlite3");
    let config =
        build_mcp_http_config_for_request(true, executable, store_path, McpHttpSettings::default())
            .unwrap();
    assert_eq!(config.token_ref, MCP_HTTP_TOKEN_REF);
    assert_eq!(config.endpoint, "http://127.0.0.1:8787/mcp");
    assert_eq!(config.client_endpoint, "http://127.0.0.1:8787/mcp");
    assert_eq!(config.settings, McpHttpSettings::default());
    assert!(!config.remote_access);
    assert!(config.token_available);
    assert!(config.start_command.contains("PORTMATE_MCP_HTTP=1"));
    assert_eq!(config.executable, executable.to_string_lossy());
    assert_eq!(config.store_path, store_path.to_string_lossy());
    assert!(config
        .start_command
        .contains("/opt/PortMate/bin/portmate-mcp"));
    assert!(config
        .start_command
        .contains("'/home/operator/PortMate Data/portmate-store.sqlite3'"));
    assert!(!config.start_command.contains("cargo run"));
    assert!(!config.start_command.contains(MCP_HTTP_TOKEN_REF));
    assert!(!config.start_command.contains("keychain:"));
    assert!(!config.start_command.contains("PORTMATE_MCP_HTTP_TOKEN"));
    assert!(!config.start_command.contains("example-token-body"));
    #[cfg(not(windows))]
    assert_eq!(
        config.start_command,
        "PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3' PORTMATE_MCP_HTTP=1 PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787' PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787,http://localhost:8787' PORTMATE_MCP_CLIENT_ID='portmate-local' PORTMATE_MCP_HTTP_ALLOW_REMOTE=0 PORTMATE_MCP_TRUSTED=0 '/opt/PortMate/bin/portmate-mcp' --http"
    );
    #[cfg(windows)]
    assert_eq!(
        config.start_command,
        "$env:PORTMATE_STORE_PATH='/home/operator/PortMate Data/portmate-store.sqlite3'; $env:PORTMATE_MCP_HTTP='1'; $env:PORTMATE_MCP_HTTP_ADDR='127.0.0.1:8787'; $env:PORTMATE_MCP_HTTP_ORIGINS='http://127.0.0.1:8787,http://localhost:8787'; $env:PORTMATE_MCP_CLIENT_ID='portmate-local'; $env:PORTMATE_MCP_HTTP_ALLOW_REMOTE='0'; $env:PORTMATE_MCP_TRUSTED='0'; & '/opt/PortMate/bin/portmate-mcp' --http"
    );
}

#[test]
fn mcp_http_config_supports_explicit_remote_listeners_and_validates_origins() {
    let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
    let store_path = Path::new("/tmp/portmate-store.sqlite3");
    let remote = McpHttpSettings {
        listen_host: "0.0.0.0".to_string(),
        client_host: "192.168.33.222".to_string(),
        port: 9888,
        allowed_origins: vec!["https://console.example.test".to_string()],
        client_id: "automation-client".to_string(),
        trusted: true,
        allow_remote: true,
    };
    let config = build_mcp_http_config_for_request(false, executable, store_path, remote).unwrap();
    assert_eq!(config.endpoint, "http://0.0.0.0:9888/mcp");
    assert_eq!(config.client_endpoint, "http://192.168.33.222:9888/mcp");
    assert!(config.remote_access);
    assert!(config
        .start_command
        .contains("PORTMATE_MCP_HTTP_ALLOW_REMOTE"));
    assert!(config.start_command.contains("PORTMATE_MCP_TRUSTED"));
    assert!(config.start_command.contains("automation-client"));
    assert!(config.start_command.contains("console.example.test"));

    let mut denied = config.settings.clone();
    denied.allow_remote = false;
    assert!(normalize_mcp_http_settings(denied)
        .unwrap_err()
        .contains("explicit remote access"));

    let invalid_origin = McpHttpSettings {
        allowed_origins: vec!["https://console.example.test/path".to_string()],
        ..McpHttpSettings::default()
    };
    assert!(normalize_mcp_http_settings(invalid_origin)
        .unwrap_err()
        .contains("scheme and authority"));

    let invalid_scheme = McpHttpSettings {
        allowed_origins: vec!["ftp://console.example.test".to_string()],
        ..McpHttpSettings::default()
    };
    assert!(normalize_mcp_http_settings(invalid_scheme)
        .unwrap_err()
        .contains("HTTP(S)"));

    let invalid_client_host = McpHttpSettings {
        client_host: "0.0.0.0".to_string(),
        ..McpHttpSettings::default()
    };
    assert!(normalize_mcp_http_settings(invalid_client_host)
        .unwrap_err()
        .contains("cannot be an unspecified"));

    let loopback = McpHttpSettings {
        allow_remote: true,
        ..McpHttpSettings::default()
    };
    assert!(
        !normalize_mcp_http_settings(loopback)
            .unwrap()
            .0
            .allow_remote
    );

    let ipv6_loopback = McpHttpSettings {
        listen_host: "[::1]".to_string(),
        port: 9889,
        allowed_origins: Vec::new(),
        ..McpHttpSettings::default()
    };
    let ipv6_config =
        build_mcp_http_config_for_request(false, executable, store_path, ipv6_loopback).unwrap();
    assert_eq!(ipv6_config.settings.listen_host, "::1");
    assert_eq!(ipv6_config.client_endpoint, "http://127.0.0.1:9889/mcp");
    assert_eq!(
        ipv6_config.settings.allowed_origins,
        vec!["http://[::1]:9889"]
    );
    assert_eq!(ipv6_config.endpoint, "http://[::1]:9889/mcp");
    assert!(!ipv6_config.remote_access);
    assert!(ipv6_config
        .start_command
        .contains("PORTMATE_MCP_HTTP_ADDR='[::1]:9889'"));

    let ipv6_wildcard = McpHttpSettings {
        listen_host: "::".to_string(),
        port: 9890,
        allowed_origins: Vec::new(),
        allow_remote: true,
        ..McpHttpSettings::default()
    };
    let ipv6_wildcard_config =
        build_mcp_http_config_for_request(false, executable, store_path, ipv6_wildcard).unwrap();
    assert_eq!(ipv6_wildcard_config.settings.listen_host, "::");
    assert_eq!(
        ipv6_wildcard_config.settings.allowed_origins,
        vec!["http://127.0.0.1:9890", "http://localhost:9890"]
    );
    assert_eq!(ipv6_wildcard_config.endpoint, "http://[::]:9890/mcp");
    assert!(ipv6_wildcard_config.remote_access);
    assert!(ipv6_wildcard_config
        .start_command
        .contains("PORTMATE_MCP_HTTP_ADDR='[::]:9890'"));
}

#[test]
fn managed_mcp_http_command_uses_saved_settings_without_exposing_the_token() {
    let executable = Path::new("/opt/PortMate/bin/portmate-mcp");
    let store_path = Path::new("/tmp/PortMate Data/portmate-store.sqlite3");
    let config = build_mcp_http_config_for_request(
        true,
        executable,
        store_path,
        McpHttpSettings {
            listen_host: "0.0.0.0".to_string(),
            client_host: "mcp.example.test".to_string(),
            port: 9911,
            allowed_origins: vec!["https://console.example.test".to_string()],
            client_id: "managed-client".to_string(),
            trusted: true,
            allow_remote: true,
        },
    )
    .unwrap();
    let command = mcp_http_process_command(executable, store_path, &config);
    let args = command
        .get_args()
        .map(|value| value.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let env = command
        .get_envs()
        .map(|(name, value)| {
            (
                name.to_string_lossy().to_string(),
                value.map(|value| value.to_string_lossy().to_string()),
            )
        })
        .collect::<HashMap<_, _>>();

    assert_eq!(command.get_program(), executable);
    assert_eq!(args, vec!["--http"]);
    assert_eq!(
        env["PORTMATE_STORE_PATH"].as_deref(),
        Some("/tmp/PortMate Data/portmate-store.sqlite3")
    );
    assert_eq!(
        env["PORTMATE_MCP_HTTP_ADDR"].as_deref(),
        Some("0.0.0.0:9911")
    );
    assert_eq!(
        env["PORTMATE_MCP_HTTP_ORIGINS"].as_deref(),
        Some("https://console.example.test")
    );
    assert_eq!(
        env["PORTMATE_MCP_CLIENT_ID"].as_deref(),
        Some("managed-client")
    );
    assert_eq!(env["PORTMATE_MCP_HTTP_ALLOW_REMOTE"].as_deref(), Some("1"));
    assert_eq!(env["PORTMATE_MCP_TRUSTED"].as_deref(), Some("1"));
    assert_eq!(
        env["PORTMATE_MCP_PARENT_PID"].as_deref(),
        Some(std::process::id().to_string().as_str())
    );
    assert_eq!(env.get("PORTMATE_MCP_HTTP_TOKEN"), Some(&None));
}

#[test]
fn managed_mcp_http_ready_probe_rejects_an_unrelated_listener() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0_u8; 512];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
    });

    assert!(!probe_mcp_http_ready(address));
    server.join().unwrap();
}

#[test]
fn managed_mcp_http_runtime_reports_ready_stops_and_retains_bounded_failures() {
    let _guard = shared_runtime_test_guard();
    let root = std::env::temp_dir().join(format!("portmate-mcp-runtime-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
    let reservation = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = reservation.local_addr().unwrap();
    drop(reservation);

    let mut fixture = managed_mcp_http_fixture_command(address, false);
    install_test_mcp_http_process(
        &state,
        &mut fixture,
        format!("http://{address}/mcp"),
        address,
    )
    .unwrap();
    let running = wait_for_managed_mcp_http_phase(&state, McpHttpRuntimePhase::Running);
    assert_eq!(
        running.endpoint.as_deref(),
        Some(format!("http://{address}/mcp").as_str())
    );
    assert!(running.pid.is_some());
    assert!(running.started_at.is_some());
    let gate_error = lock_stopped_mcp_http_runtime(&state, "保存配置")
        .err()
        .unwrap();
    assert!(gate_error.contains("请先停止"));
    assert_eq!(
        stop_mcp_http_runtime_inner(&state).unwrap().phase,
        McpHttpRuntimePhase::Stopped
    );
    assert_eq!(
        mcp_http_runtime_status_inner(&state).unwrap().phase,
        McpHttpRuntimePhase::Stopped
    );

    let failed_address = std::net::SocketAddr::from(([127, 0, 0, 1], address.port()));
    let mut failed_fixture = managed_mcp_http_fixture_command(failed_address, true);
    install_test_mcp_http_process(
        &state,
        &mut failed_fixture,
        format!("http://{failed_address}/mcp"),
        failed_address,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let failure = loop {
        let status = mcp_http_runtime_status_inner(&state).unwrap();
        if status.phase == McpHttpRuntimePhase::Failed
            && status
                .message
                .as_deref()
                .is_some_and(|message| message.contains("fixture startup rejected"))
        {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "managed MCP HTTP failure was not reported: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(failure.pid, None);
    assert!(failure.message.unwrap().chars().count() <= 1_024);
    stop_mcp_http_runtime_inner(&state).unwrap();
    let _ = fs::remove_dir_all(root);
}

fn managed_mcp_http_fixture_command(address: std::net::SocketAddr, fail: bool) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "tests::mcp_tests::managed_mcp_http_fixture",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("PORTMATE_TEST_MCP_HTTP_ADDRESS", address.to_string())
        .env("PORTMATE_TEST_MCP_HTTP_FAIL", if fail { "1" } else { "0" });
    command
}

fn wait_for_managed_mcp_http_phase(
    state: &AppState,
    phase: McpHttpRuntimePhase,
) -> McpHttpRuntimeStatus {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = mcp_http_runtime_status_inner(state).unwrap();
        if status.phase == phase {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "managed MCP HTTP phase did not become {phase:?}: {status:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore]
fn managed_mcp_http_fixture() {
    let Some(address) = std::env::var("PORTMATE_TEST_MCP_HTTP_ADDRESS")
        .ok()
        .and_then(|value| value.parse::<std::net::SocketAddr>().ok())
    else {
        return;
    };
    if std::env::var("PORTMATE_TEST_MCP_HTTP_FAIL").as_deref() == Ok("1") {
        eprintln!("fixture startup rejected");
        return;
    }
    let listener = std::net::TcpListener::bind(address).unwrap();
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let mut request = [0_u8; 512];
                if stream.read(&mut request).is_ok() {
                    let _ = stream.write_all(
                        b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nMCP-Protocol-Version: 2025-06-18\r\nConnection: close\r\n\r\n",
                    );
                }
            }
            Err(_) => break,
        }
    }
}

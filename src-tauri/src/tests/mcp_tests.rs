use super::*;

#[test]
fn empty_mcp_grant_store_requires_trusted_bootstrap() {
    let mut store = SessionStore::default();
    assert!(!mcp_scope_allowed(
        &store,
        "portmate-local",
        false,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(mcp_scope_allowed(
        &store,
        "portmate-local",
        true,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "",
        true,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "bad\nclient",
        true,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_scope_allowed(
        &store,
        &"x".repeat(MAX_MCP_GRANT_CLIENT_ID_BYTES + 1),
        true,
        McpScope::WriteInput,
        "session-1",
    ));

    store.grants.push(McpGrant {
        client_id: "granted-client".to_string(),
        name: "Granted client".to_string(),
        scopes: vec![McpScope::WriteInput],
        allowed_sessions: vec!["session-1".to_string()],
        confirm_writes: false,
        expires_at: None,
        revoked_at: None,
    });
    assert!(mcp_scope_allowed(
        &store,
        " granted-client ",
        false,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_scope_allowed(
        &store,
        "ungranted-client",
        true,
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::WriteInput,
        "session-1",
    ));
    store.grants[0].confirm_writes = true;
    assert!(mcp_write_confirmation_required(
        &store,
        " granted-client ",
        McpScope::WriteInput,
        "session-1",
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::ReadLogs,
        "session-1",
    ));
    assert!(!mcp_write_confirmation_required(
        &store,
        "granted-client",
        McpScope::WriteInput,
        "other-session",
    ));
}

#[test]
fn mcp_grant_validation_normalizes_and_rejects_ambiguous_inputs() {
    let grant = McpGrant {
        client_id: "  ops-client  ".to_string(),
        name: "  Operations  ".to_string(),
        scopes: vec![McpScope::ReadSessions, McpScope::WriteInput],
        allowed_sessions: vec!["  edge  ".to_string(), "lab".to_string()],
        confirm_writes: true,
        expires_at: None,
        revoked_at: None,
    };
    let normalized = normalize_mcp_grant(grant.clone()).unwrap();
    assert_eq!(normalized.client_id, "ops-client");
    assert_eq!(normalized.name, "Operations");
    assert_eq!(normalized.allowed_sessions, ["edge", "lab"]);

    let mut invalid = grant.clone();
    invalid.client_id = " \n ".to_string();
    assert!(normalize_mcp_grant(invalid)
        .unwrap_err()
        .contains("client ID"));

    let mut invalid = grant.clone();
    invalid.scopes = vec![McpScope::ReadLogs, McpScope::ReadLogs];
    assert!(normalize_mcp_grant(invalid)
        .unwrap_err()
        .contains("duplicate scopes"));

    let mut invalid = grant;
    invalid.allowed_sessions = vec![" edge ".to_string(), "edge".to_string()];
    assert!(normalize_mcp_grant(invalid)
        .unwrap_err()
        .contains("duplicate session IDs"));
}

#[test]
fn mcp_grant_mutations_change_memory_only_after_persistence_succeeds() {
    let mut store = SessionStore::default();
    store.grants.push(McpGrant {
        client_id: "ops-client".to_string(),
        name: "Old grant".to_string(),
        scopes: vec![McpScope::ReadSessions],
        allowed_sessions: Vec::new(),
        confirm_writes: true,
        expires_at: None,
        revoked_at: None,
    });
    let updated = McpGrant {
        client_id: "ops-client".to_string(),
        name: "Updated grant".to_string(),
        scopes: vec![McpScope::ReadSessions, McpScope::WriteInput],
        allowed_sessions: vec!["edge".to_string()],
        confirm_writes: true,
        expires_at: None,
        revoked_at: None,
    };
    let before = serde_json::to_value(&store).unwrap();

    let error = commit_store_mutation_with(
        &mut store,
        |next_store| upsert_mcp_grant_in_store(next_store, updated.clone()),
        |next_store| {
            assert_eq!(next_store.grants[0].name, "Updated grant");
            Err("store conflict".to_string())
        },
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "store conflict");
    assert_eq!(serde_json::to_value(&store).unwrap(), before);

    commit_store_mutation_with(
        &mut store,
        |next_store| upsert_mcp_grant_in_store(next_store, updated),
        |_| Ok(()),
        |_| panic!("successful persistence must not be reverified"),
    )
    .unwrap();
    assert_eq!(store.grants[0].name, "Updated grant");

    let error = commit_store_mutation_with(
        &mut store,
        |next_store| Ok(revoke_mcp_grant_from_store(next_store, "ops-client")),
        |_| Err("disk full".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "disk full");
    assert_eq!(store.grants[0].client_id, "ops-client");
}

#[test]
fn mcp_approval_queue_is_bounded_one_shot_and_times_out_closed() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-approval-queue-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        let request = build_mcp_approval_request(
            "approval-client",
            "run_command",
            &session_id,
            McpScope::WriteInput,
        )
        .unwrap();
        let (event_tx, event_rx) = tokio::sync::oneshot::channel();
        let waiter_state = state.clone();
        let waiter = tokio::spawn(async move {
            await_mcp_approval_with_emitter(
                &waiter_state,
                request,
                Duration::from_secs(1),
                move |event| {
                    event_tx
                        .send(event.clone())
                        .map_err(|_| "approval event receiver closed".to_string())
                },
            )
            .await
        });
        let emitted = event_rx.await.unwrap();
        let encoded = serde_json::to_value(&emitted).unwrap();
        assert_eq!(encoded.as_object().unwrap().len(), 7);
        assert_eq!(
            list_mcp_approvals_inner(&state).unwrap(),
            std::slice::from_ref(&emitted)
        );
        respond_mcp_approval_inner(&state, &emitted.id, true).unwrap();
        assert_eq!(waiter.await.unwrap().unwrap(), McpApprovalOutcome::Approved);
        assert!(list_mcp_approvals_inner(&state).unwrap().is_empty());
        assert!(respond_mcp_approval_inner(&state, &emitted.id, true)
            .unwrap_err()
            .contains("no longer pending"));

        let timed_out = build_mcp_approval_request(
            "approval-client",
            "create_tunnel",
            &session_id,
            McpScope::Tunnel,
        )
        .unwrap();
        let timed_out_id = timed_out.id.clone();
        assert_eq!(
            await_mcp_approval_with_emitter(&state, timed_out, Duration::from_millis(10), |_| Ok(
                ()
            ),)
            .await
            .unwrap(),
            McpApprovalOutcome::TimedOut
        );
        assert!(list_mcp_approvals_inner(&state).unwrap().is_empty());
        assert!(respond_mcp_approval_inner(&state, &timed_out_id, false).is_err());

        {
            let mut pending = state.pending_mcp_approvals.lock().unwrap();
            for _ in 0..MAX_PENDING_MCP_APPROVALS {
                let request = build_mcp_approval_request(
                    "capacity-client",
                    "start_transfer",
                    &session_id,
                    McpScope::Transfer,
                )
                .unwrap();
                let (response, _receiver) = tokio::sync::oneshot::channel();
                pending.insert(request.id.clone(), PendingMcpApproval { request, response });
            }
        }
        let overflow = build_mcp_approval_request(
            "capacity-client",
            "start_transfer",
            &session_id,
            McpScope::Transfer,
        )
        .unwrap();
        let emitted_overflow = Arc::new(AtomicBool::new(false));
        let emitted_flag = Arc::clone(&emitted_overflow);
        let error = await_mcp_approval_with_emitter(
            &state,
            overflow,
            Duration::from_millis(10),
            move |_| {
                emitted_flag.store(true, Ordering::SeqCst);
                Ok(())
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("queue limit"));
        assert!(!emitted_overflow.load(Ordering::SeqCst));
        state.pending_mcp_approvals.lock().unwrap().clear();
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn confirming_mcp_grant_fails_closed_before_execution_without_ui() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!(
            "portmate-mcp-approval-unavailable-{}",
            Uuid::new_v4()
        ));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "confirming-client".to_string(),
            name: "Confirming client".to_string(),
            scopes: vec![McpScope::ManageSessions],
            allowed_sessions: vec![session_id.clone()],
            confirm_writes: true,
            expires_at: None,
            revoked_at: None,
        });

        let error = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "confirming-client".to_string(),
                trusted_write: false,
                command: "open_session".to_string(),
                args: serde_json::json!({
                    "sessionId": session_id,
                    "password": "must-not-enter-approval-or-audit"
                }),
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("approval was unavailable"));
        assert!(state.shell.lock().unwrap().is_empty());
        let store = state.store.lock().unwrap();
        assert_eq!(store.runtimes[0].status, SessionStatus::Disconnected);
        assert_eq!(store.audit.len(), 1);
        assert_eq!(store.audit[0].decision, "denied");
        assert_eq!(
            store.audit[0].details.get("approval").map(String::as_str),
            Some("unavailable")
        );
        assert_eq!(
            store.audit[0]
                .details
                .get("approvalRequired")
                .map(String::as_str),
            Some("true")
        );
        assert!(!serde_json::to_string(&store.audit)
            .unwrap()
            .contains("must-not-enter-approval-or-audit"));
        drop(store);
        let _ = fs::remove_dir_all(root);
    });
}

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

async fn exchange_test_ipc(state: AppState, token: &str, raw: Vec<u8>) -> IpcResponse {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let token = token.to_string();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        handle_ipc_client(state, token, stream).await;
    });
    let mut client = TcpStream::connect(address).await.unwrap();
    client.write_all(&raw).await.unwrap();
    client.shutdown().await.unwrap();
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    server.await.unwrap();
    serde_json::from_slice(&response).unwrap()
}

#[test]
fn ipc_payload_reader_times_out_incomplete_clients() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let _idle_client = TcpStream::connect(address).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();

        let error = read_ipc_payload(&mut server, 128, Duration::from_millis(25))
            .await
            .unwrap_err();
        assert!(error.contains("timed out after 25 ms"));
    });
}

#[test]
fn ipc_connection_limit_rejects_excess_and_releases_completed_slots() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!("portmate-ipc-slots-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let slots = Arc::new(tokio::sync::Semaphore::new(1));

        let first_client = TcpStream::connect(address).await.unwrap();
        let (first_server, _) = listener.accept().await.unwrap();
        assert!(
            spawn_ipc_client(
                state.clone(),
                "expected-token".to_string(),
                first_server,
                Arc::clone(&slots),
            )
            .await
        );
        assert_eq!(slots.available_permits(), 0);

        let mut rejected_client = TcpStream::connect(address).await.unwrap();
        let (rejected_server, _) = listener.accept().await.unwrap();
        assert!(
            !spawn_ipc_client(
                state,
                "expected-token".to_string(),
                rejected_server,
                Arc::clone(&slots),
            )
            .await
        );
        let mut response = Vec::new();
        rejected_client.read_to_end(&mut response).await.unwrap();
        let response: IpcResponse = serde_json::from_slice(&response).unwrap();
        assert!(!response.ok);
        assert!(response.error.as_deref().is_some_and(|error| {
            error.contains("connection limit reached")
                && error.contains(&MAX_IPC_CONNECTIONS.to_string())
        }));

        drop(first_client);
        let restored =
            tokio::time::timeout(Duration::from_secs(1), Arc::clone(&slots).acquire_owned())
                .await
                .expect("IPC connection slot was not released")
                .unwrap();
        drop(restored);
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn ipc_rejects_invalid_tokens_and_oversized_payloads_without_audit() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!("portmate-ipc-bounds-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        let invalid_token_request = serde_json::to_vec(&IpcRequest {
            token: "wrong-token".to_string(),
            client_id: "unauthenticated-client".to_string(),
            trusted_write: true,
            command: "run_command".to_string(),
            args: serde_json::json!({
                "sessionId": session_id,
                "command": "password=must-not-be-audited"
            }),
        })
        .unwrap();

        let invalid_token =
            exchange_test_ipc(state.clone(), "expected-token", invalid_token_request).await;
        assert!(!invalid_token.ok);
        assert_eq!(invalid_token.error.as_deref(), Some("invalid IPC token"));
        assert!(state.store.lock().unwrap().audit.is_empty());

        let oversized = exchange_test_ipc(
            state.clone(),
            "expected-token",
            vec![b' '; MAX_IPC_REQUEST_BYTES + 1],
        )
        .await;
        assert!(!oversized.ok);
        assert!(oversized
            .error
            .as_deref()
            .is_some_and(|error| error.contains("1048576-byte limit")));
        assert!(state.store.lock().unwrap().audit.is_empty());
        assert!(!state.store_path.exists());

        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn ipc_endpoint_file_is_private_atomic_and_does_not_follow_symlinks() {
    let root = std::env::temp_dir().join(format!("portmate-ipc-endpoint-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let endpoint_path = root.join("portmate-ipc.json");
    fs::write(&endpoint_path, b"old endpoint").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o644)).unwrap();
    }
    let endpoint = IpcEndpointFile {
        addr: "127.0.0.1:43123".to_string(),
        token: Some("fallback-token".to_string()),
        token_ref: None,
        store_path: root.join("portmate-store.sqlite3").display().to_string(),
    };

    write_ipc_endpoint_file(&endpoint_path, &endpoint).unwrap();
    let persisted: IpcEndpointFile =
        serde_json::from_slice(&fs::read(&endpoint_path).unwrap()).unwrap();
    assert_eq!(persisted.addr, endpoint.addr);
    assert_eq!(persisted.token, endpoint.token);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&endpoint_path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let protected = root.join("protected-data");
        fs::write(&protected, b"must remain unchanged").unwrap();
        fs::remove_file(&endpoint_path).unwrap();
        std::os::unix::fs::symlink(&protected, &endpoint_path).unwrap();
        write_ipc_endpoint_file(&endpoint_path, &endpoint).unwrap();
        assert_eq!(fs::read(&protected).unwrap(), b"must remain unchanged");
        assert!(!fs::symlink_metadata(&endpoint_path)
            .unwrap()
            .file_type()
            .is_symlink());

        let locked = root.join("locked");
        fs::create_dir_all(&locked).unwrap();
        let locked_endpoint = locked.join("portmate-ipc.json");
        fs::write(&locked_endpoint, b"previous valid endpoint").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        let error = write_ipc_endpoint_file(&locked_endpoint, &endpoint).unwrap_err();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(error.contains("temporary MCP IPC endpoint"));
        assert_eq!(
            fs::read(&locked_endpoint).unwrap(),
            b"previous valid endpoint"
        );
    }
    assert!(fs::read_dir(&root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ipc_endpoint_publication_retires_only_valid_previous_token_refs() {
    let root = std::env::temp_dir().join(format!("portmate-ipc-publication-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let endpoint_path = root.join("portmate-ipc.json");
    let store_path = root.join("portmate-store.sqlite3");
    let other_store_path = root.join("other-store.sqlite3");
    fs::write(&store_path, b"store").unwrap();
    fs::write(&other_store_path, b"other").unwrap();
    let state = test_app_state(test_shell_profile(), store_path.clone());
    let replacement = IpcEndpointFile {
        addr: "127.0.0.1:43124".to_string(),
        token: Some("replacement-inline-token".to_string()),
        token_ref: None,
        store_path: store_path.display().to_string(),
    };
    let previous_token_ref = format!("keychain:ipc-{}", Uuid::new_v4());
    let previous = IpcEndpointFile {
        addr: "127.0.0.1:43123".to_string(),
        token: None,
        token_ref: Some(previous_token_ref.clone()),
        store_path: store_path.display().to_string(),
    };

    write_ipc_endpoint_file(&endpoint_path, &previous).unwrap();
    assert_eq!(
        publish_ipc_endpoint(&state, &endpoint_path, &replacement).unwrap(),
        Some(previous_token_ref.clone())
    );

    let mut unsafe_previous = previous.clone();
    unsafe_previous.store_path = other_store_path.display().to_string();
    write_ipc_endpoint_file(&endpoint_path, &unsafe_previous).unwrap();
    assert_eq!(
        publish_ipc_endpoint(&state, &endpoint_path, &replacement).unwrap(),
        None
    );

    unsafe_previous.store_path = store_path.display().to_string();
    unsafe_previous.token_ref = Some("keychain:ipc-not-a-uuid".to_string());
    write_ipc_endpoint_file(&endpoint_path, &unsafe_previous).unwrap();
    assert_eq!(
        publish_ipc_endpoint(&state, &endpoint_path, &replacement).unwrap(),
        None
    );
    assert!(!valid_ipc_token_ref("keychain:ipc-not-a-uuid"));
    assert!(!valid_ipc_token_ref(&format!(
        "keychain:ipc-{}",
        Uuid::new_v4().hyphenated().to_string().to_uppercase()
    )));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        write_ipc_endpoint_file(&endpoint_path, &previous).unwrap();
        fs::set_permissions(&endpoint_path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            publish_ipc_endpoint(&state, &endpoint_path, &replacement).unwrap(),
            None
        );

        let symlink_target = root.join("symlink-target.json");
        write_ipc_endpoint_file(&symlink_target, &previous).unwrap();
        fs::remove_file(&endpoint_path).unwrap();
        std::os::unix::fs::symlink(&symlink_target, &endpoint_path).unwrap();
        assert_eq!(
            publish_ipc_endpoint(&state, &endpoint_path, &replacement).unwrap(),
            None
        );
        assert_eq!(
            read_private_ipc_endpoint_file(&symlink_target)
                .unwrap()
                .unwrap(),
            previous
        );

        let locked = root.join("locked");
        fs::create_dir_all(&locked).unwrap();
        let locked_endpoint = locked.join("portmate-ipc.json");
        write_ipc_endpoint_file(&locked_endpoint, &previous).unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o500)).unwrap();
        assert!(publish_ipc_endpoint(&state, &locked_endpoint, &replacement).is_err());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            read_private_ipc_endpoint_file(&locked_endpoint)
                .unwrap()
                .unwrap(),
            previous
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ipc_endpoint_shutdown_removes_only_its_publication_and_own_token() {
    let root = std::env::temp_dir().join(format!("portmate-ipc-shutdown-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let store_path = root.join("portmate-store.sqlite3");
    fs::write(&store_path, b"store").unwrap();

    let matching_path = root.join("matching-ipc.json");
    let matching_state = test_app_state(test_shell_profile(), store_path.clone());
    let matching_token_ref = format!("keychain:ipc-{}", Uuid::new_v4());
    let matching = IpcEndpointFile {
        addr: "127.0.0.1:43123".to_string(),
        token: None,
        token_ref: Some(matching_token_ref.clone()),
        store_path: store_path.display().to_string(),
    };
    publish_ipc_endpoint(&matching_state, &matching_path, &matching).unwrap();
    let mut deleted = Vec::new();
    assert!(retire_ipc_publication_with(&matching_state, |token_ref| {
        deleted.push(token_ref.to_string());
        Ok(())
    })
    .is_empty());
    assert!(!matching_path.exists());
    assert_eq!(deleted, vec![matching_token_ref]);

    let replaced_path = root.join("replaced-ipc.json");
    let replaced_state = test_app_state(test_shell_profile(), store_path.clone());
    let own_token_ref = format!("keychain:ipc-{}", Uuid::new_v4());
    let own = IpcEndpointFile {
        addr: "127.0.0.1:43124".to_string(),
        token: None,
        token_ref: Some(own_token_ref.clone()),
        store_path: store_path.display().to_string(),
    };
    publish_ipc_endpoint(&replaced_state, &replaced_path, &own).unwrap();
    let newer = IpcEndpointFile {
        addr: "127.0.0.1:43125".to_string(),
        token: Some("newer-inline-token".to_string()),
        token_ref: None,
        store_path: store_path.display().to_string(),
    };
    write_ipc_endpoint_file(&replaced_path, &newer).unwrap();
    let mut deleted = Vec::new();
    assert!(retire_ipc_publication_with(&replaced_state, |token_ref| {
        deleted.push(token_ref.to_string());
        Ok(())
    })
    .is_empty());
    assert_eq!(
        read_private_ipc_endpoint_file(&replaced_path)
            .unwrap()
            .unwrap(),
        newer
    );
    assert_eq!(deleted, vec![own_token_ref]);

    let inline_path = root.join("inline-ipc.json");
    let inline_state = test_app_state(test_shell_profile(), store_path.clone());
    let inline = IpcEndpointFile {
        addr: "127.0.0.1:43126".to_string(),
        token: Some("inline-token".to_string()),
        token_ref: None,
        store_path: store_path.display().to_string(),
    };
    publish_ipc_endpoint(&inline_state, &inline_path, &inline).unwrap();
    assert!(retire_ipc_publication_with(&inline_state, |_| {
        panic!("inline endpoint must not schedule keyring deletion")
    })
    .is_empty());
    assert!(!inline_path.exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn mcp_ipc_reads_enforce_grants_and_reject_unlisted_commands() {
    tauri::async_runtime::block_on(async {
        let root = std::env::temp_dir().join(format!("portmate-mcp-read-scope-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = state.store.lock().unwrap().profiles[0].id.clone();
        state.store.lock().unwrap().grants.push(McpGrant {
            client_id: "scoped-reader".to_string(),
            name: "Scoped reader".to_string(),
            scopes: vec![McpScope::ReadSessions],
            allowed_sessions: vec![session_id.clone()],
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });

        let sessions = handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "scoped-reader".to_string(),
                trusted_write: false,
                command: "list_sessions".to_string(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        assert_eq!(sessions.as_array().unwrap().len(), 1);

        let read_screen = || IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "scoped-reader".to_string(),
            trusted_write: false,
            command: "read_screen".to_string(),
            args: serde_json::json!({ "sessionId": session_id }),
        };
        assert!(handle_ipc_request(state.clone(), read_screen())
            .await
            .unwrap_err()
            .contains("ReadLogs"));
        assert!(handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "unknown-reader".to_string(),
                trusted_write: false,
                command: "list_sessions".to_string(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap_err()
        .contains("ReadSessions"));
        assert!(handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "bad\nreader".to_string(),
                trusted_write: false,
                command: "list_sessions".to_string(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap_err()
        .contains("ReadSessions"));
        assert!(handle_ipc_request(
            state.clone(),
            IpcRequest {
                token: "authenticated-token".to_string(),
                client_id: "scoped-reader".to_string(),
                trusted_write: false,
                command: "list_files".to_string(),
                args: serde_json::json!({ "path": "/" }),
            },
        )
        .await
        .unwrap_err()
        .contains("unsupported IPC command"));

        state.store.lock().unwrap().grants[0]
            .scopes
            .push(McpScope::ReadLogs);
        assert!(handle_ipc_request(state.clone(), read_screen())
            .await
            .is_ok());
        let _ = fs::remove_dir_all(root);
    });
}

#[test]
fn mcp_ipc_reads_redact_profiles_and_complete_event_metadata() {
    tauri::async_runtime::block_on(async {
        let root =
            std::env::temp_dir().join(format!("portmate-mcp-read-redaction-{}", Uuid::new_v4()));
        let state = test_app_state(test_shell_profile(), root.join("portmate-store.sqlite3"));
        let session_id = {
            let mut store = state.store.lock().unwrap();
            let session_id = store.profiles[0].id.clone();
            let ConnectionConfig::Shell(shell) = &mut store.profiles[0].connection else {
                panic!("test profile should use a shell connection");
            };
            shell.args = vec!["--password".to_string(), "opaque-shell-secret".to_string()];
            shell.cwd = Some("/home/operator/private-shell-cwd".to_string());
            store.profiles[0].logging.path_template =
                "/home/operator/private-logs/{session}.raw".to_string();
            store.profiles[0].transfer.default_local_dir =
                Some("/home/operator/private-downloads".to_string());
            store.runtimes[0].cwd = Some("/home/operator/runtime-cwd".to_string());
            store
                .record_event(
                    &session_id,
                    EventDirection::Inbound,
                    EventStream::Stdout,
                    Some("password=event-secret".to_string()),
                    Some("v2:/home/operator/private-logs/raw:0:12:digest".to_string()),
                    BTreeMap::from([(
                        "diagnostic".to_string(),
                        "token=annotation-secret".to_string(),
                    )]),
                )
                .unwrap();
            session_id
        };
        let raw_store = serde_json::to_string(&*state.store.lock().unwrap()).unwrap();
        let request = |command: &str, args: serde_json::Value| IpcRequest {
            token: "authenticated-token".to_string(),
            client_id: "redaction-reader".to_string(),
            trusted_write: false,
            command: command.to_string(),
            args,
        };

        let surfaces = [
            handle_ipc_request(
                state.clone(),
                request("list_sessions", serde_json::json!({})),
            )
            .await
            .unwrap(),
            handle_ipc_request(
                state.clone(),
                request(
                    "read_screen",
                    serde_json::json!({ "sessionId": session_id }),
                ),
            )
            .await
            .unwrap(),
            handle_ipc_request(
                state.clone(),
                request("tail_log", serde_json::json!({ "sessionId": session_id })),
            )
            .await
            .unwrap(),
            handle_ipc_request(
                state.clone(),
                request(
                    "search_logs",
                    serde_json::json!({
                        "query": "password",
                        "sessionId": session_id
                    }),
                ),
            )
            .await
            .unwrap(),
        ];
        let encoded = surfaces
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        for sensitive in [
            "opaque-shell-secret",
            "/home/operator/private-shell-cwd",
            "/home/operator/private-logs/{session}.raw",
            "/home/operator/private-downloads",
            "/home/operator/runtime-cwd",
            "event-secret",
            "annotation-secret",
            "v2:/home/operator/private-logs/raw:0:12:digest",
        ] {
            assert!(
                !encoded.contains(sensitive),
                "IPC response leaked {sensitive}"
            );
        }
        assert!(encoded.contains("<redacted>"));
        assert!(encoded.contains("Bench/Device"));
        assert_eq!(
            serde_json::to_string(&*state.store.lock().unwrap()).unwrap(),
            raw_store
        );
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

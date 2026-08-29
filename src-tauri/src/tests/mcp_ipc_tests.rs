use super::*;

#[test]
fn ipc_identity_is_bound_to_the_desktop_selected_client() {
    let root = std::env::temp_dir().join(format!("portmate-ipc-identity-{}", Uuid::new_v4()));
    let state = test_app_state(test_ssh_profile(), root.join("portmate-store.sqlite3"));
    {
        let mut store = state.store.lock().unwrap();
        store.mcp_http_settings.client_id = "selected-client".to_string();
        store.grants.push(McpGrant {
            client_id: "selected-client".to_string(),
            name: "Selected client".to_string(),
            scopes: vec![McpScope::ReadSessions],
            allowed_sessions: Vec::new(),
            confirm_writes: false,
            expires_at: None,
            revoked_at: None,
        });
    }
    let mut impersonated = IpcRequest {
        token: "token".to_string(),
        client_id: "other-client".to_string(),
        trusted_write: true,
        command: "list_sessions".to_string(),
        args: serde_json::json!({}),
    };
    assert!(bind_ipc_request_identity(&state, &mut impersonated)
        .unwrap_err()
        .contains("selected-client"));

    impersonated.client_id = " selected-client ".to_string();
    bind_ipc_request_identity(&state, &mut impersonated).unwrap();
    assert_eq!(impersonated.client_id, "selected-client");
    assert!(!impersonated.trusted_write);
    let _ = fs::remove_dir_all(root);
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
fn desktop_ipc_uses_a_rotating_owner_only_endpoint_credential() {
    let store_path = std::env::temp_dir().join("portmate-store.sqlite3");
    let endpoint = inline_ipc_endpoint("127.0.0.1:43123", "rotating-token", &store_path);

    assert_eq!(endpoint.addr, "127.0.0.1:43123");
    assert_eq!(endpoint.token.as_deref(), Some("rotating-token"));
    assert_eq!(endpoint.token_ref, None);
    assert_eq!(endpoint.store_path, store_path.display().to_string());
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

        let bridge_sized_request = serde_json::to_vec(&IpcRequest {
            token: "wrong-token".to_string(),
            client_id: "unauthenticated-client".to_string(),
            trusted_write: true,
            command: "start_transfer".to_string(),
            args: serde_json::json!({
                "sessionId": session_id,
                "protocol": "xmodem",
                "source": {
                    "kind": "mcp",
                    "fileName": "firmware.bin",
                    "contentBase64": BASE64_STANDARD.encode(vec![
                        0xa5;
                        portmate_core::MAX_MCP_CONTENT_TRANSFER_BYTES
                    ])
                },
                "destination": "load:loadx"
            }),
        })
        .unwrap();
        assert!(bridge_sized_request.len() > 1024 * 1024);
        assert!(bridge_sized_request.len() <= MAX_IPC_REQUEST_BYTES);
        let bridge_sized =
            exchange_test_ipc(state.clone(), "expected-token", bridge_sized_request).await;
        assert!(!bridge_sized.ok);
        assert_eq!(bridge_sized.error.as_deref(), Some("invalid IPC token"));
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
            .is_some_and(|error| error.contains(&format!("{MAX_IPC_REQUEST_BYTES}-byte limit"))));
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

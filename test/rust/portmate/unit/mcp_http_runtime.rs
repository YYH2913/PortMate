use super::*;

#[test]
fn background_expiry_retries_token_failure_after_status_removed_the_process() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = crate::tests::test_app_state(
            crate::tests::test_shell_profile(),
            root.path().join("store.sqlite3"),
        );
        state.store.lock().unwrap().mcp_http_settings.client_id = "expired-client".into();
        let child = Command::new("/bin/sleep").arg("60").spawn().unwrap();
        let owner = install_mcp_http_process(
            &mut state.mcp_http_process.lock().unwrap(),
            child,
            "http://127.0.0.1:1".into(),
            "127.0.0.1:1".parse().unwrap(),
        );
        let failed = invalidate_expired_mcp_http_runtime_with(
            &state,
            None,
            || true,
            || Err("injected token retirement failure".into()),
        );
        assert!(failed.is_err());
        assert!(state.mcp_http_process.lock().unwrap().process.is_none());
        let token_present = Arc::new(AtomicBool::new(true));
        let retries = Arc::new(AtomicUsize::new(0));
        let token = Arc::clone(&token_present);
        let attempts = Arc::clone(&retries);
        spawn_mcp_http_expiry_worker_with(state.clone(), owner, move |state, owner| {
            invalidate_expired_mcp_http_runtime_with(
                state,
                owner,
                || token.load(Ordering::SeqCst),
                || {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    token.store(false, Ordering::SeqCst);
                    Ok(())
                },
            )
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            while token_present.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(retries.load(Ordering::SeqCst), 1);
        assert!(state
            .mcp_http_process
            .lock()
            .unwrap()
            .token_retirement_owner
            .is_none());
    });
}

#[test]
fn expired_managed_process_is_stopped_without_a_status_or_ui_caller() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let state = crate::tests::test_app_state(
            crate::tests::test_shell_profile(),
            root.path().join("store.sqlite3"),
        );
        {
            let mut store = state.store.lock().unwrap();
            store.mcp_http_settings.client_id = "expiry-client".into();
            store.grants.push(McpGrant {
                client_id: "expiry-client".into(),
                name: "test".into(),
                scopes: vec![McpScope::ReadSessions],
                allowed_sessions: vec![],
                confirm_writes: false,
                expires_at: Some(Utc::now() + chrono::Duration::milliseconds(50)),
                revoked_at: None,
            });
        }
        let child = Command::new("/bin/sleep").arg("60").spawn().unwrap();
        let owner = {
            let mut registry = state.mcp_http_process.lock().unwrap();
            install_mcp_http_process(
                &mut registry,
                child,
                "http://127.0.0.1:1".into(),
                "127.0.0.1:1".parse().unwrap(),
            )
        };
        spawn_mcp_http_expiry_worker(state.clone(), owner);
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if state.mcp_http_process.lock().unwrap().process.is_none() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        // No call to status/status_for_owner occurs in this test. Token
        // providers are not mocked as real OS-keyring or HTTP validation.
    });
}

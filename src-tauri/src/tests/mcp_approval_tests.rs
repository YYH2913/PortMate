use super::*;

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
fn transfer_and_route_lifecycle_actions_build_expected_approval_scopes() {
    assert_eq!(
        ipc_read_scope("mcp_http_runtime_status"),
        Some(McpScope::ReadMcp)
    );
    assert_eq!(
        ipc_write_scope("restart_mcp_http"),
        Some(McpScope::ManageMcp)
    );
    for (action, scope, label) in [
        ("serial_send_break", McpScope::WriteInput, "write-input"),
        ("send_bytes", McpScope::WriteInput, "write-input"),
        ("cancel_transfer", McpScope::Transfer, "transfer"),
        ("start_transfer", McpScope::Transfer, "transfer"),
        ("tftp", McpScope::Transfer, "transfer"),
        ("retry_transfer", McpScope::Transfer, "transfer"),
        (
            "start_content_upload_transfer",
            McpScope::Transfer,
            "transfer",
        ),
        ("stop_tunnel", McpScope::Tunnel, "tunnel"),
        ("restart_mcp_http", McpScope::ManageMcp, "manage-mcp"),
    ] {
        let request = build_mcp_approval_request("ops-client", action, "session-1", scope).unwrap();
        assert_eq!(request.action, action);
        assert_eq!(request.scope, label);
        assert_eq!(request.session_id, "session-1");
    }
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

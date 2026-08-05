#[test]
fn one_key_completion_writes_value_with_prompt_audit_without_readable_text() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let expected = b"private-value\r".to_vec();
        let expected_len = expected.len();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = vec![0_u8; expected_len];
            socket.read_exact(&mut received).await.unwrap();
            let _ = release_rx.await;
            received
        });

        let profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        let root = std::env::temp_dir().join(format!("portmate-one-key-send-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();
        let (prompt_event_id, one_key_updated_at) = {
            let mut store = state.store.lock().unwrap();
            let now = Utc::now();
            store.one_keys.push(OneKeyCredential {
                id: "onekey:completion".to_string(),
                label: "Completion".to_string(),
                kind: OneKeyKind::Account,
                username: "operator".to_string(),
                password_secret_ref: Some("keychain:completion".to_string()),
                passphrase_secret_ref: None,
                identity: None,
                session_ids: vec![profile.id.clone()],
                created_at: now,
                updated_at: now,
            });
            let prompt_event_id = store
                .record_stream_event(
                    &profile.id,
                    EventDirection::Inbound,
                    EventStream::Stdout,
                    "Password:",
                )
                .unwrap()
                .id;
            (prompt_event_id, now)
        };
        let validation = OneKeyPromptValidation {
            one_key_id: "onekey:completion".to_string(),
            one_key_updated_at,
            field: OneKeyField::Password,
            prompt_event_id: prompt_event_id.clone(),
        };

        let event = send_one_key_value(
            state.session_io(),
            &profile.id,
            "private-value",
            "one-key-completion",
            Some(&prompt_event_id),
            Some(&validation),
        )
        .await
        .unwrap();
        assert!(event.text.is_none());
        assert_eq!(
            event.annotations.get("origin").map(String::as_str),
            Some("one-key-completion")
        );
        assert_eq!(
            event.annotations.get("relatedEventId").map(String::as_str),
            Some(prompt_event_id.as_str())
        );
        assert!(!serde_json::to_string(&event)
            .unwrap()
            .contains("private-value"));
        close_session_inner(&state, profile.id.clone())
            .await
            .unwrap();
        let _ = release_tx.send(());
        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("OneKey loopback server timed out")
            .expect("OneKey loopback server failed");
        assert_eq!(received, expected);
        let _ = fs::remove_dir_all(root);
    });
}

use super::*;

#[test]
fn trigger_actions_dispatch_all_runtime_effects() {
    let mut profile = test_shell_profile();
    let session_id = profile.id.clone();
    profile.triggers = vec![portmate_core::TriggerSpec {
        id: "panic-trigger".to_string(),
        label: "Panic".to_string(),
        matcher: portmate_core::TriggerMatcher::Contains {
            text: "panic".to_string(),
            case_sensitive: false,
        },
        actions: vec![
            TriggerAction::Highlight {
                color: "#f87171".to_string(),
            },
            TriggerAction::Notification {
                message: "panic detected".to_string(),
            },
            TriggerAction::CustomLink {
                url_template: "https://example.test/search?q={text}".to_string(),
            },
            TriggerAction::Sound {
                name: "alert".to_string(),
            },
            TriggerAction::SendText {
                text: "status\n".to_string(),
            },
            TriggerAction::LocalCommand {
                command: "echo panic".to_string(),
            },
            TriggerAction::TimelineMark {
                label: "panic-mark".to_string(),
            },
        ],
        enabled: true,
    }];
    let mut store = SessionStore::default();
    store.upsert_profile(profile);

    let (dispatch, changed) = apply_trigger_actions_locked(&mut store, &session_id, "KERNEL PANIC");
    assert!(changed);
    assert_eq!(dispatch.send_texts, vec!["status\n"]);
    assert_eq!(dispatch.local_commands, vec!["echo panic"]);
    assert_eq!(dispatch.effects.len(), 4);
    assert_eq!(dispatch.effects[0].kind, "highlight");
    assert_eq!(dispatch.effects[0].value, "#f87171");
    assert_eq!(dispatch.effects[1].kind, "notification");
    assert_eq!(dispatch.effects[2].kind, "custom-link");
    assert_eq!(
        dispatch.effects[2].value,
        "https://example.test/search?q=KERNEL PANIC"
    );
    assert_eq!(dispatch.effects[3].kind, "sound");
    assert_eq!(dispatch.effects[3].value, "alert");
    assert!(store.timeline.iter().any(|mark| mark.label == "panic-mark"));
}

#[test]
fn trigger_custom_link_expansion_is_streamed_and_unicode_bounded() {
    assert_eq!(
        render_trigger_custom_link("https://example.test/?q={text}", " status "),
        ("https://example.test/?q=status".to_string(), false)
    );

    let template = "{text}".repeat(600);
    let matched = "界".repeat(MAX_TRIGGER_CUSTOM_LINK_CHARACTERS + 1);
    let (rendered, truncated) = render_trigger_custom_link(&template, &matched);
    assert!(truncated);
    assert_eq!(rendered.chars().count(), MAX_TRIGGER_CUSTOM_LINK_CHARACTERS);
    assert!(rendered.chars().all(|character| character == '界'));
}

#[cfg(not(windows))]
#[tokio::test]
async fn trigger_shell_command_captures_exit_status_and_bounds_output() {
    let (code, stdout, stderr) = run_shell_command_bounded(
        "printf stdout; printf stderr >&2; exit 7",
        Duration::from_secs(2),
        64,
        64,
    )
    .await
    .unwrap();
    assert_eq!(code, 7);
    assert_eq!(stdout, "stdout");
    assert_eq!(stderr, "stderr");

    let error = run_shell_command_bounded("printf 12345", Duration::from_secs(2), 4, 64)
        .await
        .unwrap_err();
    assert_eq!(error, "trigger command stdout exceeded 4 byte limit");
}

#[cfg(not(windows))]
#[tokio::test]
async fn trigger_shell_command_timeout_returns_promptly() {
    let started = Instant::now();
    let error = run_shell_command_bounded("sleep 5", Duration::from_millis(100), 64, 64)
        .await
        .unwrap_err();
    assert_eq!(error, "trigger command timed out after 100 ms");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn trigger_command_saturation_skips_execution_with_diagnostic() {
    let root = std::env::temp_dir().join(format!("portmate-trigger-command-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    let state = test_app_state(profile, store_path.clone());
    let permits = (0..MAX_TRIGGER_COMMAND_CONCURRENCY)
        .map(|_| {
            Arc::clone(&state.trigger_command_slots)
                .try_acquire_owned()
                .unwrap()
        })
        .collect::<Vec<_>>();

    spawn_trigger_commands(
        Arc::clone(&state.store),
        store_path,
        Arc::clone(&state.trigger_command_slots),
        session_id.clone(),
        vec![
            "this command must not run".to_string(),
            "neither should this".to_string(),
        ],
    );

    let store = state.store.lock().unwrap();
    assert!(store.events.iter().any(|event| {
        event.session_id == session_id
            && event.text.as_deref().is_some_and(|text| {
                text.contains("concurrent command limit reached (4)")
                    && text.contains("skipped 2 actions")
            })
    }));
    drop(store);
    drop(permits);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn trigger_send_text_dispatch_bounds_batches_and_reports_saturation() {
    let root = std::env::temp_dir().join(format!("portmate-trigger-send-{}", Uuid::new_v4()));
    let store_path = root.join("portmate-store.sqlite3");
    let mut profile = test_shell_profile();
    let session_id = profile.id.clone();
    let mut triggers: Vec<portmate_core::TriggerSpec> = (0..3)
        .map(|trigger_index| portmate_core::TriggerSpec {
            id: format!("send-{trigger_index}"),
            label: format!("Send {trigger_index}"),
            matcher: portmate_core::TriggerMatcher::Contains {
                text: "MATCH".to_string(),
                case_sensitive: true,
            },
            actions: (0..if trigger_index < 2 { 16 } else { 1 })
                .map(|action_index| TriggerAction::SendText {
                    text: format!("{trigger_index}-{action_index}"),
                })
                .collect(),
            enabled: true,
        })
        .collect();
    triggers.extend((0..2).map(|trigger_index| {
        portmate_core::TriggerSpec {
            id: format!("command-{trigger_index}"),
            label: format!("Command {trigger_index}"),
            matcher: portmate_core::TriggerMatcher::Contains {
                text: "MATCH".to_string(),
                case_sensitive: true,
            },
            actions: (0..if trigger_index == 0 { 8 } else { 1 })
                .map(|action_index| TriggerAction::LocalCommand {
                    command: format!("command-{trigger_index}-{action_index}"),
                })
                .collect(),
            enabled: true,
        }
    }));
    profile.triggers = triggers;
    let state = test_app_state(profile, store_path.clone());

    let (dispatch, changed) = {
        let mut store = state.store.lock().unwrap();
        apply_trigger_actions_locked(&mut store, &session_id, "MATCH")
    };
    assert!(changed);
    assert_eq!(dispatch.send_texts.len(), MAX_TRIGGER_SEND_TEXTS_PER_BATCH);
    assert_eq!(
        dispatch.local_commands.len(),
        MAX_TRIGGER_LOCAL_COMMANDS_PER_BATCH
    );
    assert!(state.store.lock().unwrap().events.iter().any(|event| {
        event.text.as_deref().is_some_and(|text| {
            text.contains("send_text batch limit") && text.contains("skipped 1 actions")
        })
    }));
    assert!(state.store.lock().unwrap().events.iter().any(|event| {
        event.text.as_deref().is_some_and(|text| {
            text.contains("local-command batch limit") && text.contains("skipped 1 actions")
        })
    }));

    let permits = (0..MAX_TRIGGER_SEND_BATCH_CONCURRENCY)
        .map(|_| {
            Arc::clone(&state.trigger_send_batch_slots)
                .try_acquire_owned()
                .unwrap()
        })
        .collect::<Vec<_>>();
    spawn_trigger_send_text_batch(
        state.session_io(),
        session_id,
        "runtime-current".to_string(),
        vec!["must-not-run".to_string()],
    );
    assert!(state.store.lock().unwrap().events.iter().any(|event| {
        event
            .text
            .as_deref()
            .is_some_and(|text| text.contains("concurrent batch limit reached (8)"))
    }));
    drop(permits);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn trigger_send_text_preserves_batch_order_and_rejects_stale_runtime() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut received = [0_u8; 11];
            socket.read_exact(&mut received).await.unwrap();
            received
        });
        let mut profile = test_tcp_profile(ConnectionConfig::Tcp(portmate_core::TcpConnection {
            host: "127.0.0.1".to_string(),
            port: address.port(),
            reconnect: false,
            ..Default::default()
        }));
        profile.triggers = vec![portmate_core::TriggerSpec {
            id: "stale-trigger".to_string(),
            label: "Stale".to_string(),
            matcher: portmate_core::TriggerMatcher::Contains {
                text: "STALE".to_string(),
                case_sensitive: true,
            },
            actions: vec![TriggerAction::TimelineMark {
                label: "must-not-run".to_string(),
            }],
            enabled: true,
        }];
        profile.logging.enabled = true;
        profile.logging.raw = true;
        profile.logging.text = true;
        profile.logging.jsonl = true;
        let root = std::env::temp_dir().join(format!("portmate-trigger-order-{}", Uuid::new_v4()));
        let store_path = root.join("portmate-store.sqlite3");
        let state = test_app_state(profile.clone(), store_path.clone());
        let stream = TcpStream::connect(address).await.unwrap();
        let (_reader, writer) = stream.into_split();
        let (tap, _) = broadcast::channel(8);
        state.tcp.lock().unwrap().insert(
            profile.id.clone(),
            TcpRuntime {
                runtime_id: "runtime-current".to_string(),
                writer: Arc::new(tokio::sync::Mutex::new(box_tcp_write_half(writer))),
                tap,
                closed: Arc::new(AtomicBool::new(false)),
                telnet: None,
            },
        );
        let io = state.session_io();

        send_trigger_text_inner(&io, &profile.id, "runtime-current", "first")
            .await
            .unwrap();
        send_trigger_text_inner(&io, &profile.id, "runtime-current", "second")
            .await
            .unwrap();
        let received = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("trigger send server timed out")
            .expect("trigger send server failed");
        assert_eq!(&received, b"firstsecond");
        let audit = state.store.lock().unwrap().audit.clone();
        assert_eq!(
            audit
                .iter()
                .filter(|record| {
                    record.actor == "trigger" && record.action == "trigger_send_text"
                })
                .count(),
            2
        );

        state
            .tcp
            .lock()
            .unwrap()
            .get_mut(&profile.id)
            .unwrap()
            .runtime_id = "runtime-replacement".to_string();

        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            let worker_io = io.clone();
            let session_id = profile.id.clone();
            scope.spawn(move || {
                assert_eq!(
                    with_current_session_runtime_store(
                        &worker_io,
                        &session_id,
                        "runtime-replacement",
                        |_| {
                            entered_tx.send(()).unwrap();
                            release_rx.recv().unwrap();
                        },
                    )
                    .unwrap(),
                    Some(())
                );
            });
            entered_rx.recv().unwrap();
            assert!(matches!(
                io.runtimes.tcp.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            assert!(matches!(
                io.store.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ));
            release_tx.send(()).unwrap();
        });
        state
            .tcp
            .lock()
            .unwrap()
            .get_mut(&profile.id)
            .unwrap()
            .runtime_id = "runtime-newest".to_string();

        let error = send_trigger_text_inner(&io, &profile.id, "runtime-current", "stale")
            .await
            .unwrap_err();
        assert_eq!(error, "触发动作来源连接已关闭或被新连接替换");
        record_channel_bytes(
            &io,
            &profile.id,
            Some("runtime-current"),
            EventStream::Stdout,
            b"STALE",
            "STALE".to_string(),
        );
        assert!(!record_runtime_system_event(
            &io,
            &profile.id,
            "runtime-current",
            "PortMate: STALE-SYSTEM".to_string(),
            "stale test system event",
        ));
        assert!(record_outbound_control_event_for_runtime(
            &io,
            &profile.id,
            "runtime-current",
            b"STALE-CONTROL",
            "stale-test",
            None,
            true,
        )
        .is_none());
        let store = state.store.lock().unwrap();
        assert!(store.timeline.is_empty());
        assert!(!store.events.iter().any(|event| {
            event.session_id == profile.id && event.text.as_deref() == Some("STALE")
        }));
        assert!(!store
            .screen(&profile.id)
            .is_some_and(|screen| screen.contains("STALE")));
        assert!(!store.events.iter().any(|event| {
            event
                .annotations
                .get("origin")
                .is_some_and(|origin| origin == "stale-test")
        }));
        drop(store);
        for extension in ["raw", "txt", "jsonl"] {
            let path = log_shard_path(&store_path, &profile, extension).unwrap();
            if let Ok(contents) = fs::read(path) {
                assert!(!contents
                    .windows(b"STALE".len())
                    .any(|part| part == b"STALE"));
            }
        }

        state.tcp.lock().unwrap().remove(&profile.id);
        let _ = fs::remove_dir_all(root);
    });
}

use super::*;

#[cfg(unix)]
#[test]
fn external_tcp_telnet_server_compatibility() {
    let Ok(label) = std::env::var("PORTMATE_COMPAT_SOCKET_LABEL") else {
        eprintln!("skipping external TCP/Telnet compatibility test: matrix environment is not set");
        return;
    };
    let host = std::env::var("PORTMATE_COMPAT_SOCKET_HOST").unwrap();
    let port = std::env::var("PORTMATE_COMPAT_SOCKET_PORT")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let protocol = std::env::var("PORTMATE_COMPAT_SOCKET_PROTOCOL").unwrap();
    let mode = std::env::var("PORTMATE_COMPAT_SOCKET_MODE").unwrap();
    let tls_enabled = std::env::var("PORTMATE_COMPAT_SOCKET_TLS")
        .unwrap_or_else(|_| "false".to_string())
        .parse::<bool>()
        .unwrap();
    let tls_server_name = std::env::var("PORTMATE_COMPAT_SOCKET_TLS_SERVER_NAME")
        .ok()
        .filter(|value| !value.is_empty());
    let tls_accept_invalid_cert = std::env::var("PORTMATE_COMPAT_SOCKET_TLS_ACCEPT_INVALID_CERT")
        .unwrap_or_else(|_| "false".to_string())
        .parse::<bool>()
        .unwrap();
    let expected_rejected_option = std::env::var("PORTMATE_COMPAT_SOCKET_EXPECT_REJECTED_OPTION")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| value.parse::<u8>().unwrap());
    let verify_naws_pty = std::env::var("PORTMATE_COMPAT_SOCKET_VERIFY_NAWS_PTY")
        .unwrap_or_else(|_| "false".to_string())
        .parse::<bool>()
        .unwrap();
    let root = std::env::temp_dir().join(format!(
        "portmate-external-socket-compat-{}-{}",
        label,
        Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();

    tauri::async_runtime::block_on(async {
        let tcp = TcpConnection {
            host,
            port,
            reconnect: false,
            telnet_binary: true,
            telnet_naws: true,
            tls_enabled,
            tls_server_name,
            tls_accept_invalid_cert,
            ..Default::default()
        };
        let connection = match protocol.as_str() {
            "tcp" => ConnectionConfig::Tcp(tcp),
            "telnet" => ConnectionConfig::Telnet(tcp),
            other => panic!("unsupported external socket protocol: {other}"),
        };
        let mut profile = test_tcp_profile(connection);
        if expected_rejected_option.is_some() {
            profile.logging.enabled = true;
            profile.logging.raw = true;
            profile.logging.text = false;
            profile.logging.jsonl = false;
        }
        profile.terminal.term = "xterm-256color".to_string();
        profile.terminal.cols = 120;
        profile.terminal.rows = 40;
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        let opened = open_tcp_session(&state, profile.clone())
            .await
            .unwrap_or_else(|error| panic!("{label} open failed: {error}"));
        assert_eq!(opened.runtime.status, SessionStatus::Connected);

        match (protocol.as_str(), mode.as_str()) {
            ("telnet", "shell") => {
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        let negotiation_ready =
                            state.store.lock().unwrap().events.iter().any(|event| {
                                event.session_id == profile.id
                                    && event.direction == EventDirection::Outbound
                                    && event.stream == EventStream::Control
                                    && event
                                        .annotations
                                        .get("origin")
                                        .is_some_and(|origin| origin == "telnet-negotiation")
                            });
                        if negotiation_ready {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("{label} negotiation timed out"));
                if let Some(option) = expected_rejected_option {
                    tokio::time::timeout(Duration::from_secs(10), async {
                        loop {
                            let references = state
                                .store
                                .lock()
                                .unwrap()
                                .events
                                .iter()
                                .filter(|event| {
                                    event.session_id == profile.id
                                        && event.direction == EventDirection::Outbound
                                        && event.stream == EventStream::Control
                                        && event
                                            .annotations
                                            .get("origin")
                                            .is_some_and(|origin| origin == "telnet-negotiation")
                                })
                                .filter_map(|event| event.bytes_ref.clone())
                                .collect::<Vec<_>>();
                            if references.iter().any(|reference| {
                                read_log_bytes_ref(&state.store_path, reference).is_ok_and(
                                    |(_, _, bytes)| {
                                        bytes.windows(3).any(|window| {
                                            window == [TELNET_IAC, TELNET_WONT, option]
                                        })
                                    },
                                )
                            }) {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    })
                    .await
                    .unwrap_or_else(|_| panic!("{label} did not reject Telnet option {option}"));
                }
                send_text_inner(
                    state.session_io(),
                    profile.id.clone(),
                    "printf '__PORTMATE_TELNET_COMPAT__\\n'\n".to_string(),
                )
                .await
                .unwrap_or_else(|error| panic!("{label} shell write failed: {error}"));
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if state
                            .store
                            .lock()
                            .unwrap()
                            .screen(&profile.id)
                            .is_some_and(|screen| screen.contains("__PORTMATE_TELNET_COMPAT__"))
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("{label} shell output timed out"));
                if verify_naws_pty {
                    tokio::time::timeout(Duration::from_secs(10), async {
                        loop {
                            let negotiated = state
                                .tcp
                                .lock()
                                .unwrap()
                                .get(&profile.id)
                                .and_then(|runtime| runtime.telnet.as_ref())
                                .is_some_and(|telnet| {
                                    telnet.naws_negotiated.load(Ordering::SeqCst)
                                });
                            if negotiated {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    })
                    .await
                    .unwrap_or_else(|_| panic!("{label} did not negotiate Telnet NAWS"));
                }
                resize_session_inner(&state, profile.id.clone(), 132, 43)
                    .await
                    .unwrap_or_else(|error| panic!("{label} resize failed: {error}"));
                if verify_naws_pty {
                    assert!(
                        state.store.lock().unwrap().events.iter().any(|event| {
                            event.session_id == profile.id
                                && event.direction == EventDirection::Outbound
                                && event.stream == EventStream::Control
                                && event
                                    .annotations
                                    .get("origin")
                                    .is_some_and(|origin| origin == "telnet-naws")
                        }),
                        "{label} did not send Telnet NAWS resize"
                    );
                    send_text_inner(
                        state.session_io(),
                        profile.id.clone(),
                        "printf '__PORTMATE_TELNET_SIZE__'; stty size; printf '__PORTMATE_TELNET_SIZE_DONE__\\n'\n"
                            .to_string(),
                    )
                    .await
                    .unwrap_or_else(|error| panic!("{label} size probe failed: {error}"));
                    tokio::time::timeout(Duration::from_secs(10), async {
                        loop {
                            if state.store.lock().unwrap().screen(&profile.id).is_some_and(
                                |screen| {
                                    screen.contains("__PORTMATE_TELNET_SIZE__")
                                        && screen.contains("43 132")
                                        && screen.contains("__PORTMATE_TELNET_SIZE_DONE__")
                                },
                            ) {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                    })
                    .await
                    .unwrap_or_else(|_| panic!("{label} did not apply Telnet NAWS resize"));
                }
                send_text_inner(state.session_io(), profile.id.clone(), "exit\n".to_string())
                    .await
                    .unwrap_or_else(|error| panic!("{label} shell exit failed: {error}"));
            }
            ("tcp", "echo") => {
                let mut tap = state
                    .tcp
                    .lock()
                    .unwrap()
                    .get(&profile.id)
                    .unwrap()
                    .tap
                    .subscribe();
                let raw = vec![0x00, 0x01, TELNET_IAC, 0x7f, b'P', b'M'];
                send_bytes_inner(state.session_io(), profile.id.clone(), raw.clone())
                    .await
                    .unwrap_or_else(|error| panic!("{label} raw write failed: {error}"));
                let echoed = tokio::time::timeout(Duration::from_secs(5), tap.recv())
                    .await
                    .unwrap_or_else(|_| panic!("{label} raw echo timed out"))
                    .unwrap_or_else(|error| panic!("{label} raw echo tap failed: {error}"));
                assert_eq!(echoed, raw, "{label} changed raw TCP bytes");
                close_session_inner(&state, profile.id.clone())
                    .await
                    .unwrap_or_else(|error| panic!("{label} close failed: {error}"));
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            ("tcp", "burst-close") => {
                tokio::time::timeout(Duration::from_secs(10), async {
                    loop {
                        if state
                            .store
                            .lock()
                            .unwrap()
                            .screen(&profile.id)
                            .is_some_and(|screen| screen.contains("__PORTMATE_TCP_BURST__"))
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("{label} burst marker timed out"));
                let bytes = state
                    .store
                    .lock()
                    .unwrap()
                    .events
                    .iter()
                    .filter(|event| {
                        event.session_id == profile.id
                            && event.direction == EventDirection::Inbound
                            && event.stream == EventStream::Stdout
                    })
                    .map(|event| event.text.as_deref().map_or(0, |text| text.len() as u64))
                    .sum::<u64>();
                assert!(bytes >= 256 * 1024, "{label} lost burst bytes: {bytes}");
            }
            ("tcp", "close") => {
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        if state
                            .store
                            .lock()
                            .unwrap()
                            .screen(&profile.id)
                            .is_some_and(|screen| screen.contains("__PORTMATE_TCP_CLOSE__"))
                        {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .unwrap_or_else(|_| panic!("{label} close marker timed out"));
            }
            pair => panic!("unsupported external socket compatibility case: {pair:?}"),
        }

        if mode != "echo" {
            let disconnected = tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let runtime = state
                        .store
                        .lock()
                        .unwrap()
                        .summaries()
                        .into_iter()
                        .find(|summary| summary.profile.id == profile.id)
                        .unwrap()
                        .runtime;
                    if runtime.status == SessionStatus::Disconnected {
                        break runtime;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap_or_else(|_| panic!("{label} did not settle to disconnected"));
            let reason = disconnected.last_disconnect_reason.unwrap_or_default();
            if protocol == "telnet" {
                assert!(
                    reason.starts_with("Telnet ")
                        && (reason.contains("socket closed")
                            || reason.contains("negotiation reply failed")),
                    "{label} unexpected disconnect reason: {reason}"
                );
            } else {
                assert!(
                    reason.contains("socket closed"),
                    "{label} unexpected disconnect reason: {reason}"
                );
            }
        }
    });

    let _ = fs::remove_dir_all(root);
}

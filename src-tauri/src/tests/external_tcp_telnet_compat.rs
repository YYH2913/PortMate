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
                resize_session_inner(&state, profile.id.clone(), 132, 43)
                    .await
                    .unwrap_or_else(|error| panic!("{label} resize failed: {error}"));
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

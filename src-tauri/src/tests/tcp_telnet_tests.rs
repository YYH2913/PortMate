use super::*;
use crate::session_terminal::terminate_command_for_protocol;

#[test]
fn telnet_negotiator_filters_iac_and_replies() {
    let profile = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection::default()));
    let mut negotiator = TelnetNegotiator::new(TelnetRuntimeState::from_profile(&profile).unwrap());
    let (output, replies) = negotiator.filter(&[
        b'h',
        b'i',
        TELNET_IAC,
        TELNET_WILL,
        TELNET_OPT_ECHO,
        TELNET_IAC,
        TELNET_DO,
        TELNET_OPT_TERMINAL_TYPE,
    ]);
    assert_eq!(output, b"hi");
    assert_eq!(
        replies,
        vec![
            vec![TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO],
            vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_TERMINAL_TYPE],
        ]
    );
}

#[test]
fn telnet_binary_negotiation_controls_directional_nvt_decoding() {
    let profile = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection::default()));
    let runtime = TelnetRuntimeState::from_profile(&profile).unwrap();
    let mut negotiator = TelnetNegotiator::new(Arc::clone(&runtime));

    let (binary, replies) =
        negotiator.filter(&[TELNET_IAC, TELNET_WILL, TELNET_OPT_BINARY, b'\r', 0]);
    assert_eq!(binary, [b'\r', 0]);
    assert_eq!(replies, [vec![TELNET_IAC, TELNET_DO, TELNET_OPT_BINARY]]);
    assert!(runtime.remote_binary.load(Ordering::SeqCst));

    let (nvt, replies) = negotiator.filter(&[TELNET_IAC, TELNET_WONT, TELNET_OPT_BINARY, b'\r', 0]);
    assert_eq!(nvt, b"\r");
    assert_eq!(replies, [vec![TELNET_IAC, TELNET_DONT, TELNET_OPT_BINARY]]);
    assert!(!runtime.remote_binary.load(Ordering::SeqCst));

    let (_, replies) = negotiator.filter(&[
        TELNET_IAC,
        TELNET_DO,
        TELNET_OPT_BINARY,
        TELNET_IAC,
        TELNET_DONT,
        TELNET_OPT_BINARY,
    ]);
    assert_eq!(
        replies,
        [
            vec![TELNET_IAC, TELNET_WILL, TELNET_OPT_BINARY],
            vec![TELNET_IAC, TELNET_WONT, TELNET_OPT_BINARY],
        ]
    );
    assert!(!runtime.local_binary.load(Ordering::SeqCst));
}

#[test]
fn telnet_disabled_binary_and_naws_options_are_rejected() {
    let profile = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection {
        telnet_binary: false,
        telnet_naws: false,
        ..Default::default()
    }));
    let runtime = TelnetRuntimeState::from_profile(&profile).unwrap();
    let mut negotiator = TelnetNegotiator::new(Arc::clone(&runtime));
    let (_, replies) = negotiator.filter(&[
        TELNET_IAC,
        TELNET_DO,
        TELNET_OPT_BINARY,
        TELNET_IAC,
        TELNET_WILL,
        TELNET_OPT_BINARY,
        TELNET_IAC,
        TELNET_DO,
        TELNET_OPT_NAWS,
    ]);
    assert_eq!(
        replies,
        [
            vec![TELNET_IAC, TELNET_WONT, TELNET_OPT_BINARY],
            vec![TELNET_IAC, TELNET_DONT, TELNET_OPT_BINARY],
            vec![TELNET_IAC, TELNET_WONT, TELNET_OPT_NAWS],
        ]
    );
    assert!(!runtime.local_binary.load(Ordering::SeqCst));
    assert!(!runtime.remote_binary.load(Ordering::SeqCst));
    assert!(!runtime.naws_negotiated.load(Ordering::SeqCst));
}

#[test]
fn telnet_naws_escapes_iac_dimension_bytes() {
    assert_eq!(
        telnet_naws_message(255, 511),
        [
            TELNET_IAC,
            TELNET_SB,
            TELNET_OPT_NAWS,
            0,
            TELNET_IAC,
            TELNET_IAC,
            1,
            TELNET_IAC,
            TELNET_IAC,
            TELNET_IAC,
            TELNET_SE,
        ]
    );
}

#[test]
fn telnet_outbound_text_uses_crlf() {
    assert_eq!(encode_telnet_outbound_text("show\n", false), "show\r\n");
    assert_eq!(encode_telnet_outbound_text("show\r\n", false), "show\r\n");
    assert_eq!(
        encode_telnet_outbound_text("a\rb\r\nc\n", false),
        "a\r\0b\r\nc\r\n"
    );
    assert_eq!(encode_telnet_outbound_text("ÿ\n", false), "ÿ\r\n");
    assert_eq!(encode_telnet_outbound_text("show\n", true), "show\n");
    assert_eq!(
        terminal_key_sequence_for_protocol("Enter", true).unwrap(),
        "\r\n"
    );
    assert_eq!(
        terminal_key_sequence_for_protocol("Enter", false).unwrap(),
        "\r"
    );
    assert_eq!(
        terminate_command_for_protocol("show\r".to_string(), true),
        "show\r\n"
    );
    assert_eq!(
        terminate_command_for_protocol("show\r".to_string(), false),
        "show\r"
    );
    assert_eq!(
        encode_telnet_outbound_bytes(&[0x01, TELNET_IAC]),
        vec![0x01, TELNET_IAC, TELNET_IAC]
    );
}

#[test]
fn telnet_negotiator_handles_fragmented_nvt_and_subnegotiation() {
    let profile = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection::default()));
    let mut negotiator = TelnetNegotiator::new(TelnetRuntimeState::from_profile(&profile).unwrap());
    let (first, replies) = negotiator.filter(b"left\r");
    assert_eq!(first, b"left");
    assert!(replies.is_empty());

    let (second, replies) =
        negotiator.filter(&[0, b'|', b'\r', b'\n', TELNET_IAC, TELNET_SB, 99, TELNET_IAC]);
    assert_eq!(second, b"\r|\r\n");
    assert!(replies.is_empty());
    let (escaped, replies) = negotiator.filter(&[TELNET_IAC]);
    assert!(escaped.is_empty());
    assert!(replies.is_empty());
    assert_eq!(negotiator.subnegotiation, [99, TELNET_IAC]);
    let (third, replies) = negotiator.filter(&[7, TELNET_IAC, TELNET_SE, b'!']);
    assert_eq!(third, b"!");
    assert!(replies.is_empty());

    let (last, replies) = negotiator.filter(b"tail\r");
    assert_eq!(last, b"tail");
    assert!(replies.is_empty());
    assert_eq!(negotiator.finish(), b"\r");
}

#[test]
fn telnet_runtime_handles_fragmented_negotiation_and_nvt_data() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            for chunk in [
                b"left\r".as_slice(),
                &[0, b'|', b'\r'],
                &[b'\n', TELNET_IAC],
                &[TELNET_WILL],
                &[
                    TELNET_OPT_ECHO,
                    TELNET_IAC,
                    TELNET_SB,
                    TELNET_OPT_TERMINAL_TYPE,
                ],
            ] {
                socket.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }

            let mut option_reply = [0_u8; 3];
            socket.read_exact(&mut option_reply).await.unwrap();
            assert_eq!(option_reply, [TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO]);
            for chunk in [&[TELNET_TTYPE_SEND, TELNET_IAC][..], &[TELNET_SE, b'!'][..]] {
                socket.write_all(chunk).await.unwrap();
                tokio::task::yield_now().await;
            }
            let mut terminal_reply = vec![0_u8; 20];
            socket.read_exact(&mut terminal_reply).await.unwrap();
            assert_eq!(
                terminal_reply,
                [
                    [
                        TELNET_IAC,
                        TELNET_SB,
                        TELNET_OPT_TERMINAL_TYPE,
                        TELNET_TTYPE_IS
                    ]
                    .as_slice(),
                    b"xterm-256color".as_slice(),
                    [TELNET_IAC, TELNET_SE].as_slice(),
                ]
                .concat()
            );
            socket.write_all(b"tail\r").await.unwrap();
        });

        let mut profile =
            test_tcp_profile(ConnectionConfig::Telnet(portmate_core::TcpConnection {
                host: "127.0.0.1".to_string(),
                port: address.port(),
                reconnect: false,
                ..Default::default()
            }));
        profile.logging.enabled = true;
        profile.logging.raw = true;
        let root = std::env::temp_dir().join(format!("portmate-telnet-test-{}", Uuid::new_v4()));
        let state = test_app_state(profile.clone(), root.join("portmate-store.sqlite3"));
        open_tcp_session(&state, profile.clone()).await.unwrap();

        tokio::time::timeout(TEST_RUNTIME_TRANSITION_TIMEOUT, server)
            .await
            .expect("fragmented Telnet server timed out")
            .expect("fragmented Telnet server failed");
        tokio::time::timeout(TEST_RUNTIME_TRANSITION_TIMEOUT, async {
            loop {
                let summary = state
                    .store
                    .lock()
                    .unwrap()
                    .summaries()
                    .into_iter()
                    .find(|summary| summary.profile.id == profile.id)
                    .unwrap();
                if summary.runtime.status == SessionStatus::Disconnected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Telnet runtime did not close after EOF");
        let stdout = state
            .store
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| event.session_id == profile.id && event.stream == EventStream::Stdout)
            .filter_map(|event| event.text.as_deref())
            .collect::<String>();
        assert_eq!(stdout, "left\r|\r\n!tail\r");
        assert!(!stdout.contains('\0'));
        let option_reply = [TELNET_IAC, TELNET_DO, TELNET_OPT_ECHO];
        let terminal_reply = [
            [
                TELNET_IAC,
                TELNET_SB,
                TELNET_OPT_TERMINAL_TYPE,
                TELNET_TTYPE_IS,
            ]
            .as_slice(),
            b"xterm-256color".as_slice(),
            [TELNET_IAC, TELNET_SE].as_slice(),
        ]
        .concat();
        let expected_raw = [
            b"left\r".as_slice(),
            &[0, b'|', b'\r'],
            &[b'\n', TELNET_IAC],
            &[TELNET_WILL],
            &[
                TELNET_OPT_ECHO,
                TELNET_IAC,
                TELNET_SB,
                TELNET_OPT_TERMINAL_TYPE,
            ],
            option_reply.as_slice(),
            &[TELNET_TTYPE_SEND, TELNET_IAC],
            &[TELNET_SE, b'!'],
            terminal_reply.as_slice(),
            b"tail\r".as_slice(),
        ]
        .concat();
        let raw_path = log_shard_path(&state.store_path, &profile, "raw").unwrap();
        assert_eq!(fs::read(raw_path).unwrap(), expected_raw);
        let control_bytes = state
            .store
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|event| {
                event.session_id == profile.id
                    && event.direction == EventDirection::Outbound
                    && event.stream == EventStream::Control
            })
            .map(|event| {
                read_log_bytes_ref(&state.store_path, event.bytes_ref.as_deref().unwrap())
                    .unwrap()
                    .2
            })
            .collect::<Vec<_>>();
        assert_eq!(control_bytes, vec![option_reply.to_vec(), terminal_reply]);
        let _ = fs::remove_dir_all(root);
    });
}

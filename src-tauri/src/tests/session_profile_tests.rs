use super::*;

#[test]
fn terminal_resize_metadata_changes_memory_only_after_persistence_succeeds() {
    let mut store = SessionStore::default();
    let profile = test_shell_profile();
    let session_id = profile.id.clone();
    let original_size = (profile.terminal.cols, profile.terminal.rows);
    store.upsert_profile(profile);

    let error = commit_store_mutation_with(
        &mut store,
        |next_store| resize_session_profile_in_store(next_store, &session_id, 132, 43),
        |next_store| {
            let profile = next_store.profile(&session_id).unwrap();
            assert_eq!((profile.terminal.cols, profile.terminal.rows), (132, 43));
            Err("store conflict".to_string())
        },
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error, "store conflict");
    let profile = store.profile(&session_id).unwrap();
    assert_eq!(
        (profile.terminal.cols, profile.terminal.rows),
        original_size
    );

    let summary = commit_store_mutation_with(
        &mut store,
        |next_store| resize_session_profile_in_store(next_store, &session_id, 132, 43),
        |_| Err("post-commit version read failed".to_string()),
        |_| Ok(true),
    )
    .unwrap();
    assert_eq!(
        (summary.profile.terminal.cols, summary.profile.terminal.rows),
        (132, 43)
    );
    let profile = store.profile(&session_id).unwrap();
    assert_eq!((profile.terminal.cols, profile.terminal.rows), (132, 43));
}

#[test]
fn session_profile_normalization_bounds_metadata_and_terminal_settings() {
    let mut profile = test_shell_profile();
    profile.id = format!(" \0edge\n{} ", "界".repeat(300));
    profile.name = format!(" \0Router\n{} ", "界".repeat(200));
    profile.group = format!(" Lab\u{0085}{} ", "g".repeat(300));
    profile.tags = std::iter::once(" edge ".to_string())
        .chain(std::iter::once("edge".to_string()))
        .chain((0..40).map(|index| format!("tag-{index}-{}", "x".repeat(80))))
        .collect();
    profile.terminal.term = "xterm\nmalformed".to_string();
    profile.terminal.rows = 0;
    profile.terminal.cols = u16::MAX;
    profile.terminal.scrollback = u32::MAX;
    profile.terminal.font_family = "bad\0font".to_string();
    profile.terminal.font_size = 0;
    profile.terminal.theme = " graphite ".to_string();
    profile.terminal.background_opacity = 0;
    profile.triggers = vec![
        portmate_core::TriggerSpec {
            id: "valid-trigger".to_string(),
            label: "Valid".to_string(),
            matcher: portmate_core::TriggerMatcher::Contains {
                text: "match".to_string(),
                case_sensitive: true,
            },
            actions: vec![TriggerAction::TimelineMark {
                label: "mark".to_string(),
            }],
            enabled: true,
        },
        portmate_core::TriggerSpec {
            id: "invalid\ntrigger".to_string(),
            label: "Invalid".to_string(),
            matcher: portmate_core::TriggerMatcher::Contains {
                text: "match".to_string(),
                case_sensitive: true,
            },
            actions: Vec::new(),
            enabled: true,
        },
    ];
    let normalized = normalize_session_profile(profile.clone());
    assert_eq!(
        normalized.id.chars().count(),
        MAX_SESSION_PROFILE_ID_CHARACTERS
    );
    assert!(normalized.id.starts_with("edge"));
    assert!(!normalized.id.chars().any(char::is_control));
    assert_eq!(
        normalized.name.chars().count(),
        MAX_SESSION_PROFILE_NAME_CHARACTERS
    );
    assert!(normalized.name.starts_with("Router"));
    assert!(!normalized.name.chars().any(char::is_control));
    assert_eq!(
        normalized.group.chars().count(),
        MAX_SESSION_PROFILE_GROUP_CHARACTERS
    );
    assert_eq!(normalized.tags.len(), MAX_SESSION_PROFILE_TAGS);
    assert_eq!(normalized.tags[0], "edge");
    assert!(normalized
        .tags
        .iter()
        .all(|tag| tag.chars().count() <= MAX_SESSION_PROFILE_TAG_CHARACTERS));
    assert_eq!(
        normalized.tags.iter().collect::<HashSet<_>>().len(),
        normalized.tags.len()
    );
    assert_eq!(normalized.terminal.term, DEFAULT_TERMINAL_NAME);
    assert_eq!(normalized.terminal.rows, MIN_TERMINAL_ROWS);
    assert_eq!(normalized.terminal.cols, MAX_TERMINAL_COLS);
    assert_eq!(normalized.terminal.scrollback, MAX_TERMINAL_SCROLLBACK);
    assert_eq!(
        normalized.terminal.font_family,
        DEFAULT_TERMINAL_FONT_FAMILY
    );
    assert_eq!(normalized.terminal.font_size, MIN_TERMINAL_FONT_SIZE);
    assert_eq!(normalized.terminal.theme, "graphite");
    assert_eq!(
        normalized.terminal.background_opacity,
        MIN_TERMINAL_BACKGROUND_OPACITY
    );
    assert_eq!(normalized.triggers.len(), 1);
    assert_eq!(normalized.triggers[0].id, "valid-trigger");

    let mut fallback = profile.clone();
    fallback.name = "\0\n".to_string();
    assert_eq!(
        normalize_session_profile(fallback).name,
        normalized_profile_metadata_text(
            &normalized_session_profile_id(&profile.id),
            MAX_SESSION_PROFILE_NAME_CHARACTERS,
        )
    );

    profile.terminal.theme = "future-or-corrupt-theme".to_string();
    assert_eq!(
        normalize_session_profile(profile).terminal.theme,
        DEFAULT_TERMINAL_THEME
    );
}

#[test]
fn ssh_auth_success_hint_respects_the_current_policy() {
    let mut profile = test_ssh_profile();
    let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
        panic!("test profile must be SSH");
    };
    ssh.identity_policy.auth_order = vec![AuthMethod::Password, AuthMethod::PublicKey];
    ssh.identity_policy.last_successful = Some(AuthMethod::PublicKey);
    assert_eq!(
        ordered_auth_methods(ssh),
        vec![AuthMethod::PublicKey, AuthMethod::Password]
    );

    ssh.identity_policy.last_successful = Some(AuthMethod::KeyboardInteractive);
    assert_eq!(
        ordered_auth_methods(ssh),
        vec![AuthMethod::Password, AuthMethod::PublicKey]
    );

    ssh.identity_policy.record_success = false;
    ssh.identity_policy.last_successful = Some(AuthMethod::PublicKey);
    assert_eq!(
        ordered_auth_methods(ssh),
        vec![AuthMethod::Password, AuthMethod::PublicKey]
    );
    let normalized = normalize_session_profile(profile);
    let ConnectionConfig::Ssh(ssh) = normalized.connection else {
        panic!("normalized test profile must remain SSH");
    };
    assert_eq!(ssh.identity_policy.last_successful, None);
}

#[test]
fn active_session_rejects_cross_transport_profile_changes() {
    let current = test_shell_profile();
    let mut next = current.clone();
    next.kind = SessionKind::Serial;
    next.connection = ConnectionConfig::Serial(portmate_core::SerialConnection {
        port: "/dev/ttyUSB0".to_string(),
        baud_rate: 115_200,
        data_bits: 8,
        stop_bits: 1,
        parity: "none".to_string(),
        flow_control: "none".to_string(),
        dtr: false,
        rts: false,
        reconnect: true,
        reconnect_delay_ms: portmate_core::DEFAULT_SERIAL_RECONNECT_DELAY_MS,
        receive_idle_timeout_enabled: false,
        receive_idle_timeout_seconds: portmate_core::DEFAULT_SERIAL_RECEIVE_IDLE_TIMEOUT_SECONDS,
    });

    for status in [
        SessionStatus::Connecting,
        SessionStatus::Connected,
        SessionStatus::Reconnecting,
    ] {
        let error =
            validate_profile_transport_change(Some(&current), &next, Some(status)).unwrap_err();
        assert!(error.contains("切换到 Serial 前请先关闭会话"));
    }
    for status in [
        SessionStatus::Disconnected,
        SessionStatus::Blocked,
        SessionStatus::Error,
    ] {
        validate_profile_transport_change(Some(&current), &next, Some(status)).unwrap();
    }

    let mut same_transport = current.clone();
    if let ConnectionConfig::Shell(shell) = &mut same_transport.connection {
        shell.program = "/bin/bash".to_string();
    }
    validate_profile_transport_change(
        Some(&current),
        &same_transport,
        Some(SessionStatus::Connected),
    )
    .unwrap();
    validate_profile_transport_change(None, &next, None).unwrap();
}

#[test]
fn proxy_secret_usage_counts_all_supported_profile_kinds() {
    fn shared_proxy() -> ProxyConfig {
        ProxyConfig {
            enabled: true,
            username: "proxy-user".to_string(),
            password_secret_ref: Some(" keychain:shared-proxy ".to_string()),
            ..ProxyConfig::default()
        }
    }

    let mut ssh = test_ssh_profile();
    if let ConnectionConfig::Ssh(connection) = &mut ssh.connection {
        connection.proxy = shared_proxy();
    }
    let mut tmux = test_ssh_profile();
    tmux.id = "tmux-session-1".to_string();
    tmux.kind = SessionKind::Tmux;
    let ConnectionConfig::Ssh(mut tmux_connection) = tmux.connection else {
        unreachable!();
    };
    tmux_connection.proxy = shared_proxy();
    tmux.connection = ConnectionConfig::Tmux(tmux_connection);
    let tcp = test_tcp_profile(ConnectionConfig::Tcp(TcpConnection {
        proxy: shared_proxy(),
        ..TcpConnection::default()
    }));
    let mut telnet = test_tcp_profile(ConnectionConfig::Telnet(TcpConnection {
        proxy: shared_proxy(),
        ..TcpConnection::default()
    }));
    telnet.id = "telnet-session-1".to_string();

    let mut store = SessionStore::default();
    for profile in [ssh, tmux, tcp, telnet] {
        store.upsert_profile(profile);
    }
    assert_eq!(secret_ref_usage_count(&store, "keychain:shared-proxy"), 4);
    assert!(store
        .profiles
        .iter()
        .all(|profile| { profile_secret_refs(profile).contains("keychain:shared-proxy") }));
}

#[test]
fn proxy_password_updates_store_only_a_secret_reference() {
    let mut profile = test_tcp_profile(ConnectionConfig::Tcp(TcpConnection {
        proxy: ProxyConfig {
            enabled: true,
            kind: ProxyKind::Socks5,
            username: "proxy-user".to_string(),
            ..ProxyConfig::default()
        },
        ..TcpConnection::default()
    }));
    let written = std::cell::RefCell::new(None::<String>);
    let generated = apply_proxy_password_update_with_io(
        &mut profile,
        Some(ProxyPasswordUpdate::Set {
            password: "private-proxy-password".to_string(),
            storage: None,
        }),
        |storage, password| {
            assert!(storage.is_none());
            written.replace(Some(password.to_string()));
            Ok("keychain:proxy-password".to_string())
        },
    )
    .unwrap();
    assert_eq!(generated.as_deref(), Some("keychain:proxy-password"));
    assert_eq!(written.borrow().as_deref(), Some("private-proxy-password"));
    let proxy = profile_proxy(&profile).unwrap();
    assert_eq!(
        proxy.password_secret_ref.as_deref(),
        Some("keychain:proxy-password")
    );
    let serialized = serde_json::to_string(&profile).unwrap();
    assert!(!serialized.contains("private-proxy-password"));

    let credentials = resolve_proxy_credentials_with(proxy, |secret_ref| {
        assert_eq!(secret_ref, "keychain:proxy-password");
        Ok("private-proxy-password".to_string())
    })
    .unwrap()
    .unwrap();
    assert_eq!(credentials.username, "proxy-user");
    assert_eq!(credentials.password.as_str(), "private-proxy-password");

    apply_proxy_password_update_with_io(&mut profile, Some(ProxyPasswordUpdate::Clear), |_, _| {
        panic!("clearing a proxy password must not write a secret")
    })
    .unwrap();
    assert!(profile_proxy(&profile)
        .unwrap()
        .password_secret_ref
        .is_none());

    assert!(
        validate_proxy_credentials(ProxyKind::HttpConnect, "bad:user", "password")
            .unwrap_err()
            .contains("冒号")
    );
    assert!(
        validate_proxy_credentials(ProxyKind::Socks5, &"u".repeat(256), "password")
            .unwrap_err()
            .contains("1-255")
    );
    assert!(
        validate_proxy_credentials(ProxyKind::Socks5, "proxy-user", &"p".repeat(256))
            .unwrap_err()
            .contains("1-255")
    );
}

use super::*;

#[test]
fn ssh_reconnect_enabled_reads_ssh_and_tmux_profiles() {
    let mut profile = test_ssh_profile();
    assert!(ssh_reconnect_enabled(&profile));
    assert_eq!(
        ssh_reconnect_delay(&profile),
        Duration::from_millis(portmate_core::DEFAULT_SSH_RECONNECT_DELAY_MS)
    );

    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.reconnect = false;
        ssh.reconnect_delay_ms = 0;
    }
    assert!(!ssh_reconnect_enabled(&profile));
    assert_eq!(
        ssh_reconnect_delay(&profile),
        Duration::from_millis(portmate_core::MIN_SSH_RECONNECT_DELAY_MS)
    );

    let ssh = match profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh,
        _ => panic!("expected SSH profile"),
    };
    profile.connection = ConnectionConfig::Tmux(SshConnection {
        reconnect: true,
        ..ssh
    });
    profile.kind = SessionKind::Tmux;
    assert!(ssh_reconnect_enabled(&profile));
}

#[test]
fn ssh_establishment_comparison_ignores_only_host_key_last_seen() {
    let mut attempt = test_ssh_profile();
    let now = Utc::now();
    let key = TrustedHostKey {
        id: "host-key-generation".to_string(),
        profile_id: Some(attempt.id.clone()),
        alias: "bench-device".to_string(),
        host: "192.0.2.10".to_string(),
        port: 22,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:original".to_string(),
        public_key_base64: "YWJj".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: now,
        last_seen: now,
    };
    if let ConnectionConfig::Ssh(ssh) = &mut attempt.connection {
        ssh.trusted_host_keys.push(key);
    }

    let mut observed = attempt.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut observed.connection {
        ssh.trusted_host_keys[0].last_seen = now + chrono::Duration::seconds(1);
    }
    assert!(ssh_establishment_profile_matches(&attempt, &observed));

    if let ConnectionConfig::Ssh(ssh) = &mut observed.connection {
        ssh.trusted_host_keys[0].fingerprint_sha256 = "SHA256:changed".to_string();
    }
    assert!(!ssh_establishment_profile_matches(&attempt, &observed));
}

#[test]
fn ssh_channel_exit_status_preserves_remote_disconnect_diagnostic() {
    assert_eq!(
        ssh_channel_disconnect_reason(&SshBackendMessage::ExitStatus(7)).as_deref(),
        Some("SSH remote process exited with status 7")
    );
    assert_eq!(ssh_channel_disconnect_reason(&SshBackendMessage::Eof), None);
}

#[test]
fn ssh_reconnect_profile_reloads_latest_store_snapshot() {
    let mut profile = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut profile.connection {
        ssh.endpoint.host = "old.example".to_string();
        ssh.username = "old-user".to_string();
        ssh.password_secret_ref = Some("keychain:old-password".to_string());
        ssh.passphrase_secret_ref = Some("keychain:old-passphrase".to_string());
    }
    let state = test_app_state(profile.clone(), PathBuf::from("reconnect-test.sqlite3"));
    assert!(ssh_reconnect_attempt_matches_profile(&profile, &profile));
    assert_eq!(
        ssh_reconnect_profile_state(&state.store.lock().unwrap(), "ssh-session-1", &profile,),
        SshReconnectProfileState::Current
    );
    let mut terminal_updated = profile.clone();
    terminal_updated.terminal.rows += 1;
    assert!(!ssh_reconnect_attempt_matches_profile(
        &profile,
        &terminal_updated
    ));
    let mut updated = profile.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut updated.connection {
        ssh.endpoint.host = "new.example".to_string();
        ssh.username = "new-user".to_string();
        ssh.reconnect_delay_ms = 2_500;
        ssh.keepalive_enabled = true;
        ssh.keepalive_interval_seconds = 75;
        ssh.keepalive_max_missed = 7;
        ssh.proxy = ProxyConfig {
            enabled: true,
            kind: ProxyKind::HttpConnect,
            host: "proxy.example".to_string(),
            port: 3128,
            ..ProxyConfig::default()
        };
        ssh.password_secret_ref = Some("keychain:new-password".to_string());
        ssh.passphrase_secret_ref = Some("keychain:new-passphrase".to_string());
    }
    state.store.lock().unwrap().upsert_profile(updated);

    assert_eq!(
        ssh_reconnect_profile_state(&state.store.lock().unwrap(), "ssh-session-1", &profile,),
        SshReconnectProfileState::Changed
    );

    let latest = latest_ssh_reconnect_profile(&state, "ssh-session-1")
        .unwrap()
        .unwrap();
    assert!(!ssh_reconnect_attempt_matches_profile(&profile, &latest));
    let latest = match latest.connection {
        ConnectionConfig::Ssh(ssh) => ssh,
        _ => panic!("expected SSH profile"),
    };
    assert_eq!(latest.endpoint.host, "new.example");
    assert_eq!(latest.username, "new-user");
    assert_eq!(latest.reconnect_delay_ms, 2_500);
    assert!(latest.keepalive_enabled);
    assert_eq!(latest.keepalive_interval_seconds, 75);
    assert_eq!(latest.keepalive_max_missed, 7);
    assert!(latest.proxy.enabled);
    assert_eq!(latest.proxy.kind, ProxyKind::HttpConnect);
    assert_eq!(latest.proxy.host, "proxy.example");
    assert_eq!(latest.proxy.port, 3128);
    assert_eq!(
        latest.password_secret_ref.as_deref(),
        Some("keychain:new-password")
    );
    assert_eq!(
        latest.passphrase_secret_ref.as_deref(),
        Some("keychain:new-passphrase")
    );

    let mut disabled = state
        .store
        .lock()
        .unwrap()
        .profile("ssh-session-1")
        .unwrap();
    if let ConnectionConfig::Ssh(ssh) = &mut disabled.connection {
        ssh.reconnect = false;
    }
    state.store.lock().unwrap().upsert_profile(disabled);
    assert_eq!(
        ssh_reconnect_profile_state(&state.store.lock().unwrap(), "ssh-session-1", &profile,),
        SshReconnectProfileState::Disabled
    );
    assert!(latest_ssh_reconnect_profile(&state, "ssh-session-1")
        .unwrap()
        .is_none());

    let mut non_ssh = test_shell_profile();
    non_ssh.id = "ssh-session-1".to_string();
    state.store.lock().unwrap().upsert_profile(non_ssh);
    assert_eq!(
        ssh_reconnect_profile_state(&state.store.lock().unwrap(), "ssh-session-1", &profile,),
        SshReconnectProfileState::Disabled
    );
    assert!(latest_ssh_reconnect_profile(&state, "ssh-session-1")
        .unwrap()
        .is_none());
}

#[test]
fn ssh_client_config_applies_profile_keepalive_settings() {
    let mut profile = test_ssh_profile();
    let ssh = match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh,
        _ => panic!("expected SSH profile"),
    };
    ssh.keepalive_enabled = true;
    ssh.keepalive_interval_seconds = 75;
    ssh.keepalive_max_missed = 7;

    let config = ssh_client_config(ssh);
    assert_eq!(config.keepalive_interval, Some(Duration::from_secs(75)));
    assert_eq!(config.keepalive_max, 7);
    assert!(config.nodelay);

    ssh.keepalive_enabled = false;
    let config = ssh_client_config(ssh);
    assert_eq!(config.keepalive_interval, None);
    assert_eq!(config.keepalive_max, 7);

    ssh.keepalive_enabled = true;
    ssh.keepalive_max_missed = 0;
    let config = ssh_client_config(ssh);
    assert_eq!(config.keepalive_interval, Some(Duration::from_secs(75)));
    assert_eq!(config.keepalive_max, 0);
}

#[test]
fn ask_every_time_only_accepts_a_one_time_key_id() {
    let mut policy = portmate_core::HostKeyPolicy::profile_alias("bench-device");
    assert!(trusted_host_key_allowed(&policy, "persistent-key", &[]));

    policy.mode = HostKeyMode::AskEveryTime;
    assert!(!trusted_host_key_allowed(
        &policy,
        "persistent-key",
        &["one-time-key".to_string()]
    ));
    assert!(trusted_host_key_allowed(
        &policy,
        "one-time-key",
        &["one-time-key".to_string()]
    ));
}

#[test]
fn ssh_handler_lock_poisoning_returns_a_protocol_error() {
    let state = Mutex::new(());
    let _ = std::panic::catch_unwind(|| {
        let _guard = state.lock().unwrap();
        panic!("poison SSH handler state for the regression check");
    });

    let error = lock_ssh_handler_state(&state, "host key observation").unwrap_err();
    assert!(matches!(error, russh::Error::IO(_)));
    assert!(error
        .to_string()
        .contains("PortMate SSH host key observation lock is poisoned"));
}

#[test]
fn jump_endpoint_details_validate_each_hop() {
    let jump = portmate_core::JumpHop {
        host: " bastion-2 ".to_string(),
        port: 2222,
        username: " deploy ".to_string(),
        password_secret_ref: None,
        passphrase_secret_ref: None,
        identity_ref: Some("jump-key".to_string()),
        host_key_policy: None,
    };
    assert_eq!(
        jump_endpoint_details(&jump, 1).unwrap(),
        ("bastion-2".to_string(), 2222, "deploy".to_string())
    );

    let mut invalid = jump.clone();
    invalid.host = " ".to_string();
    assert!(jump_endpoint_details(&invalid, 0)
        .unwrap_err()
        .contains("第 1 跳 主机不能为空"));

    invalid = jump.clone();
    invalid.port = 0;
    assert!(jump_endpoint_details(&invalid, 2)
        .unwrap_err()
        .contains("第 3 跳 端口必须"));

    invalid = jump;
    invalid.username = " ".to_string();
    assert!(jump_endpoint_details(&invalid, 3)
        .unwrap_err()
        .contains("第 4 跳 用户名不能为空"));
}

#[test]
fn jump_ssh_connection_uses_independent_credentials_and_policy() {
    let mut profile = test_ssh_profile();
    let ssh = match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh,
        _ => panic!("expected SSH profile"),
    };
    ssh.password_secret_ref = Some("keychain:target-password".to_string());
    ssh.passphrase_secret_ref = Some("keychain:target-passphrase".to_string());
    ssh.identity_refs.push(portmate_core::IdentityRef {
        id: "target-key".to_string(),
        label: "target".to_string(),
        source: IdentitySource::SystemFile,
        fingerprint_sha256: None,
        path: Some("~/.ssh/id_target".to_string()),
        secret_ref: None,
    });
    ssh.identity_refs.push(portmate_core::IdentityRef {
        id: "jump-key".to_string(),
        label: "jump".to_string(),
        source: IdentitySource::SystemFile,
        fingerprint_sha256: None,
        path: Some("~/.ssh/id_jump".to_string()),
        secret_ref: None,
    });

    let jump_policy = portmate_core::HostKeyPolicy {
        mode: HostKeyMode::AskEveryTime,
        alias: Some(" bastion-a ".to_string()),
        trust_scope: HostKeyScope::User,
        allow_rotation: true,
        check_ip: true,
    };
    let jump = portmate_core::JumpHop {
        host: " bastion.example ".to_string(),
        port: 2222,
        username: " jumpuser ".to_string(),
        password_secret_ref: Some(" keychain:jump-password ".to_string()),
        passphrase_secret_ref: Some(" keychain:jump-passphrase ".to_string()),
        identity_ref: Some("jump-key".to_string()),
        host_key_policy: Some(jump_policy),
    };

    let policy = jump_host_key_policy(ssh, &jump);
    assert_eq!(policy.mode, HostKeyMode::AskEveryTime);
    assert_eq!(policy.alias.as_deref(), Some("bastion-a"));
    assert_eq!(policy.trust_scope, HostKeyScope::User);
    assert!(policy.allow_rotation);
    assert!(policy.check_ip);

    let jump_ssh = jump_ssh_connection(ssh, &jump, policy.clone());
    assert_eq!(jump_ssh.endpoint.host, "bastion.example");
    assert_eq!(jump_ssh.username, "jumpuser");
    assert_eq!(
        jump_ssh.password_secret_ref.as_deref(),
        Some("keychain:jump-password")
    );
    assert_eq!(
        jump_ssh.passphrase_secret_ref.as_deref(),
        Some("keychain:jump-passphrase")
    );
    assert_eq!(jump_ssh.host_key_policy, policy);
    assert_eq!(jump_ssh.tcp_keepalive_enabled, ssh.tcp_keepalive_enabled);
    assert_eq!(jump_ssh.identity_refs.len(), 1);
    assert_eq!(jump_ssh.identity_refs[0].id, "jump-key");
}

#[test]
fn jump_ssh_connection_falls_back_to_parent_credentials_and_policy() {
    let mut profile = test_ssh_profile();
    let ssh = match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) => ssh,
        _ => panic!("expected SSH profile"),
    };
    ssh.password_secret_ref = Some("keychain:target-password".to_string());
    ssh.passphrase_secret_ref = Some("keychain:target-passphrase".to_string());
    ssh.host_key_policy = portmate_core::HostKeyPolicy {
        mode: HostKeyMode::TrustOnFirstUse,
        alias: Some("target-alias".to_string()),
        trust_scope: HostKeyScope::Project,
        allow_rotation: true,
        check_ip: true,
    };

    let jump = portmate_core::JumpHop {
        host: "bastion.example".to_string(),
        port: 22,
        username: "jumpuser".to_string(),
        password_secret_ref: None,
        passphrase_secret_ref: None,
        identity_ref: None,
        host_key_policy: None,
    };

    let policy = jump_host_key_policy(ssh, &jump);
    assert_eq!(policy.mode, HostKeyMode::TrustOnFirstUse);
    assert_eq!(policy.alias.as_deref(), Some("jump:bastion.example:22"));
    assert_eq!(policy.trust_scope, HostKeyScope::Profile);
    assert!(policy.allow_rotation);
    assert!(policy.check_ip);

    let jump_ssh = jump_ssh_connection(ssh, &jump, policy);
    assert_eq!(
        jump_ssh.password_secret_ref.as_deref(),
        Some("keychain:target-password")
    );
    assert_eq!(
        jump_ssh.passphrase_secret_ref.as_deref(),
        Some("keychain:target-passphrase")
    );
}

#[test]
fn jump_runtime_credentials_do_not_override_independent_secret_refs() {
    assert_eq!(
        jump_runtime_credential(Some("target-password"), Some("keychain:jump-password")),
        None
    );
    assert_eq!(
        jump_runtime_credential(Some("target-passphrase"), Some(" keychain:jump-key ")),
        None
    );
    assert_eq!(
        jump_runtime_credential(Some("shared-password"), None).as_deref(),
        Some("shared-password")
    );
    assert_eq!(
        jump_runtime_credential(Some("shared-password"), Some(" ")).as_deref(),
        Some("shared-password")
    );
    assert_eq!(jump_runtime_credential(Some(""), None), None);
}

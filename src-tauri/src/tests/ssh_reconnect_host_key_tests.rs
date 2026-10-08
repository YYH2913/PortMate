use super::*;

fn reconnect_ssh_mut(profile: &mut SessionProfile) -> &mut SshConnection {
    match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh,
        _ => panic!("expected SSH-backed profile"),
    }
}

fn reconnect_key_observation() -> HostKeyObservation {
    HostKeyObservation {
        host: "192.0.2.10".into(),
        port: 22,
        alias: Some("bench-device".into()),
        algorithm: "ssh-ed25519".into(),
        public_key_base64: "YWJj".into(),
    }
}

#[test]
fn reconnect_host_key_bypass_requires_enabled_automatic_reconnect() {
    let mut profile = test_ssh_profile();
    let ssh = reconnect_ssh_mut(&mut profile);
    assert_eq!(
        SshHostKeyVerification::for_establishment(ssh, true),
        SshHostKeyVerification::Standard
    );
    ssh.reconnect_ignore_host_key_changes = true;
    assert_eq!(
        SshHostKeyVerification::for_establishment(ssh, false),
        SshHostKeyVerification::Standard
    );
    assert_eq!(
        SshHostKeyVerification::for_establishment(ssh, true),
        SshHostKeyVerification::ReconnectIgnoreChanges
    );
    ssh.reconnect = false;
    assert_eq!(
        SshHostKeyVerification::for_establishment(ssh, true),
        SshHostKeyVerification::Standard
    );
}

#[test]
fn reconnect_host_key_bypass_preserves_endpoint_scope_and_key_validation() {
    let profile = test_ssh_profile();
    let ConnectionConfig::Ssh(ssh) = &profile.connection else {
        unreachable!()
    };
    let mut policy = ssh.host_key_policy.clone();
    let original = reconnect_key_observation();
    let mut keys = HostKeyStore::new();
    keys.apply_decision(
        &profile.id,
        &policy,
        &original,
        HostKeyDecision::AppendToProfile,
    )
    .unwrap();
    let changed = HostKeyObservation {
        public_key_base64: "eHl6".into(),
        ..original.clone()
    };
    let bypass = SshHostKeyVerification::ReconnectIgnoreChanges;
    for mode in [
        HostKeyMode::Strict,
        HostKeyMode::TrustOnFirstUse,
        HostKeyMode::AskEveryTime,
    ] {
        policy.mode = mode;
        assert!(verify_ssh_host_key_observation(
            &profile.id,
            &policy,
            &keys,
            &[],
            &changed,
            SshHostKeyVerification::Standard
        )
        .is_err());
        assert!(verify_ssh_host_key_observation(
            &profile.id,
            &policy,
            &keys,
            &[],
            &changed,
            bypass
        )
        .is_ok());
        let changed_algorithm = HostKeyObservation {
            algorithm: "ssh-rsa".into(),
            ..changed.clone()
        };
        assert!(verify_ssh_host_key_observation(
            &profile.id,
            &policy,
            &keys,
            &[],
            &changed_algorithm,
            bypass
        )
        .is_ok());
        let certificate = HostKeyObservation {
            algorithm: "ssh-ed25519-cert-v01@openssh.com".into(),
            ..changed.clone()
        };
        assert!(verify_ssh_host_key_observation(
            &profile.id,
            &policy,
            &keys,
            &[],
            &certificate,
            bypass,
        )
        .is_err());
    }
    policy.mode = HostKeyMode::Strict;
    for unknown in [
        HostKeyObservation {
            alias: Some("unseen-device".into()),
            ..changed.clone()
        },
        HostKeyObservation {
            port: 2222,
            ..changed.clone()
        },
    ] {
        assert!(verify_ssh_host_key_observation(
            &profile.id,
            &policy,
            &keys,
            &[],
            &unknown,
            bypass
        )
        .is_err());
    }
    assert!(verify_ssh_host_key_observation(
        "other-profile",
        &policy,
        &keys,
        &[],
        &changed,
        bypass
    )
    .is_err());
    policy.check_ip = true;
    let moved = HostKeyObservation {
        host: "192.0.2.20".into(),
        ..changed.clone()
    };
    assert!(
        verify_ssh_host_key_observation(&profile.id, &policy, &keys, &[], &moved, bypass).is_err()
    );
    let malformed = HostKeyObservation {
        public_key_base64: "%%%".into(),
        ..changed
    };
    assert!(
        verify_ssh_host_key_observation(&profile.id, &policy, &keys, &[], &malformed, bypass)
            .is_err()
    );
    policy.mode = HostKeyMode::AskEveryTime;
    assert!(
        verify_ssh_host_key_observation(&profile.id, &policy, &keys, &[], &original, bypass)
            .is_err()
    );
    let confirmed_ids = vec![keys.keys[0].id.clone()];
    assert!(verify_ssh_host_key_observation(
        &profile.id,
        &policy,
        &keys,
        &confirmed_ids,
        &original,
        bypass
    )
    .is_ok());
}

#[test]
fn reconnect_host_key_bypass_records_changes_without_mutating_any_trust_source() {
    for jump in [false, true] {
        for trust_source in ["store", "profile", "one-time"] {
            let root = canonical_test_tempdir();
            let mut profile = test_ssh_profile();
            reconnect_ssh_mut(&mut profile).reconnect_ignore_host_key_changes = true;
            let state = test_app_state(profile.clone(), root.path().join("store.sqlite3"));
            let mut policy = reconnect_ssh_mut(&mut profile).host_key_policy.clone();
            // A TOFU policy with rotation enabled must not persist the ignored key either.
            policy.mode = HostKeyMode::TrustOnFirstUse;
            policy.allow_rotation = true;
            reconnect_ssh_mut(&mut profile).host_key_policy = policy.clone();
            state.store.lock().unwrap().upsert_profile(profile.clone());
            let original = reconnect_key_observation();
            let mut keys = HostKeyStore::new();
            let key = keys
                .apply_decision(
                    &profile.id,
                    &policy,
                    &original,
                    HostKeyDecision::AppendToProfile,
                )
                .unwrap()
                .unwrap();
            let mut one_time = Vec::new();
            {
                let mut store = state.store.lock().unwrap();
                match trust_source {
                    "store" => {
                        store.host_keys.keys.push(key.clone());
                        mirror_persistent_host_keys(&mut store, &[key]).unwrap();
                    }
                    "profile" => {
                        reconnect_ssh_mut(&mut profile).trusted_host_keys.push(key);
                        store.upsert_profile(profile);
                    }
                    "one-time" => one_time.push(key),
                    _ => unreachable!(),
                }
            }
            let expected = state
                .store
                .lock()
                .unwrap()
                .profile("ssh-session-1")
                .unwrap();
            let before = state.store.lock().unwrap().clone();
            let changed = HostKeyObservation {
                algorithm: if jump { "ssh-rsa" } else { "ssh-ed25519" }.into(),
                public_key_base64: "eHl6".into(),
                ..original
            };
            let observation = Arc::new(Mutex::new(Some(changed.clone())));
            let guard = HostKeyPersistenceGuard {
                profile_id: &expected.id,
                expected_profile: Some(&expected),
                host_key_verification: SshHostKeyVerification::ReconnectIgnoreChanges,
            };
            if jump {
                persist_observed_host_key_with_policy(
                    &state.store,
                    &state.store_path,
                    guard,
                    &policy,
                    &observation,
                    &one_time,
                    "Jump Host #1",
                )
                .unwrap();
            } else {
                persist_observed_host_key(
                    &state.store,
                    &state.store_path,
                    guard,
                    &observation,
                    &one_time,
                )
                .unwrap();
            }
            let after = state.store.lock().unwrap();
            assert_eq!(after.host_keys.keys, before.host_keys.keys);
            assert_eq!(after.profiles, before.profiles);
            let event = after.events.last().unwrap().text.as_deref().unwrap();
            assert!(event.contains("自动重连临时放行"));
            assert!(event.contains(&changed.fingerprint_sha256().unwrap()));
            drop(after);
            let loaded = load_store(&state.store_path).unwrap();
            assert_eq!(loaded.host_keys.keys, before.host_keys.keys);
            assert_eq!(loaded.profiles, before.profiles);
        }
    }
}

#[test]
fn reconnect_host_key_bypass_rejects_settings_changed_during_establishment() {
    for jump in [false, true] {
        let root = canonical_test_tempdir();
        let mut expected = test_ssh_profile();
        reconnect_ssh_mut(&mut expected).reconnect_ignore_host_key_changes = true;
        let state = test_app_state(expected.clone(), root.path().join("store.sqlite3"));
        let policy = reconnect_ssh_mut(&mut expected).host_key_policy.clone();
        let original = reconnect_key_observation();
        state
            .store
            .lock()
            .unwrap()
            .host_keys
            .apply_decision(
                &expected.id,
                &policy,
                &original,
                HostKeyDecision::AppendToProfile,
            )
            .unwrap();
        let mut updated = expected.clone();
        reconnect_ssh_mut(&mut updated).reconnect_ignore_host_key_changes = false;
        assert!(!ssh_reconnect_attempt_matches_profile(&expected, &updated));
        state.store.lock().unwrap().upsert_profile(updated);
        let before = state.store.lock().unwrap().clone();
        let observed = Arc::new(Mutex::new(Some(HostKeyObservation {
            public_key_base64: "eHl6".into(),
            ..original
        })));
        let guard = HostKeyPersistenceGuard {
            profile_id: &expected.id,
            expected_profile: Some(&expected),
            host_key_verification: SshHostKeyVerification::ReconnectIgnoreChanges,
        };
        let result = if jump {
            persist_observed_host_key_with_policy(
                &state.store,
                &state.store_path,
                guard,
                &policy,
                &observed,
                &[],
                "Jump Host #1",
            )
        } else {
            persist_observed_host_key(&state.store, &state.store_path, guard, &observed, &[])
        };
        assert!(result.unwrap_err().contains("profile changed"));
        let after = state.store.lock().unwrap();
        assert_eq!(after.host_keys.keys, before.host_keys.keys);
        assert_eq!(after.events, before.events);
        assert!(!state.store_path.exists());
    }
}

#[cfg(unix)]
#[test]
fn explicit_none_authentication_connects_without_password_on_both_backends() {
    let root = canonical_test_tempdir();
    let key = root.path().join("none-host-key");
    generate_ed25519_test_key(&key);
    tauri::async_runtime::block_on(async {
        let (port, counters, server) = spawn_mixed_auth_test_server(&key, "user", "unused").await;
        counters.allow_none_auth.store(true, Ordering::SeqCst);
        for libssh in [false, true] {
            if libssh && !cfg!(target_os = "linux") { continue; }
            let mut profile = test_ssh_profile();
            let ssh = reconnect_ssh_mut(&mut profile);
            ssh.endpoint.host = "127.0.0.1".into();
            ssh.endpoint.port = port;
            ssh.username = "user".into();
            ssh.reconnect = false;
            ssh.password_secret_ref = Some("stronghold:unused-none-password".into());
            ssh.passphrase_secret_ref = Some("stronghold:unused-none-passphrase".into());
            ssh.host_key_policy.mode = HostKeyMode::TrustOnFirstUse;
            ssh.identity_policy.auth_order = if libssh { vec![AuthMethod::GssapiWithMic, AuthMethod::None] } else { vec![AuthMethod::None] };
            let path = root.path().join(if libssh { "none-libssh.sqlite3" } else { "none-russh.sqlite3" });
            let state = test_app_state(profile.clone(), path);
            let opened = establish_ssh_runtime_with_timeout(&state, &profile, None, None, SSH_CONNECT_TIMEOUT, None).await.unwrap();
            assert_eq!(opened.auth_method, AuthMethod::None);
            assert_eq!(opened.runtime.handle.lock().await.is_libssh(), libssh);
            assert!(!state.store.lock().unwrap().host_keys.keys.is_empty());
            disconnect_ssh_runtime(opened.runtime, opened.read_half, opened.reader_finished, "none auth test complete").await;
        }
        server.abort();
    });
}

#[cfg(unix)]
#[test]
fn reconnect_host_key_bypass_establishes_real_ssh_and_tmux_runtimes_only_when_opted_in() {
    let root = canonical_test_tempdir();
    let host_key = root.path().join("new-host-key");
    let old_key = root.path().join("old-host-key");
    generate_ed25519_test_key(&host_key);
    generate_ed25519_test_key(&old_key);
    tauri::async_runtime::block_on(async {
        let (port, counters, task) =
            spawn_mixed_auth_test_server(&host_key, "user", "unused").await;
        counters.allow_none_auth.store(true, Ordering::SeqCst);
        let mut backends = vec![SshBackendKind::Russh];
        if cfg!(target_os = "linux") {
            backends.push(SshBackendKind::Libssh);
        }
        for backend in backends {
            for tmux in [false, true] {
                let mut profile = test_ssh_profile();
                let ssh = reconnect_ssh_mut(&mut profile);
                ssh.endpoint.host = "127.0.0.1".into();
                ssh.endpoint.port = port;
                ssh.username = "user".into();
                ssh.identity_policy.auth_order = if backend == SshBackendKind::Libssh {
                    vec![AuthMethod::GssapiWithMic, AuthMethod::None]
                } else {
                    vec![AuthMethod::None]
                };
                ssh.agent_policy.enabled = false;
                let old_observation = HostKeyObservation {
                    host: ssh.endpoint.host.clone(),
                    port,
                    alias: ssh.host_key_policy.alias.clone(),
                    algorithm: "ssh-ed25519".into(),
                    public_key_base64: load_secret_key(&old_key, None)
                        .unwrap()
                        .public_key()
                        .public_key_base64(),
                };
                if tmux {
                    let ConnectionConfig::Ssh(ssh) = profile.connection else {
                        unreachable!()
                    };
                    profile.connection = ConnectionConfig::Tmux(ssh);
                    profile.kind = SessionKind::Tmux;
                }
                let state = test_app_state(
                    profile.clone(),
                    root.path().join(format!("{backend:?}-{tmux}.sqlite3")),
                );
                let policy = reconnect_ssh_mut(&mut profile).host_key_policy.clone();
                {
                    let mut store = state.store.lock().unwrap();
                    apply_persistent_host_key_decision_with_policy(
                        &mut store,
                        &profile.id,
                        &policy,
                        &old_observation,
                        HostKeyDecision::AppendToProfile,
                    )
                    .unwrap();
                    profile = store.profile(&profile.id).unwrap();
                }
                let error = establish_ssh_reconnect_runtime(&state, &profile)
                    .await
                    .err()
                    .unwrap();
                assert!(error.contains("host key"), "{backend:?}: {error}");
                reconnect_ssh_mut(&mut profile).reconnect_ignore_host_key_changes = true;
                state.store.lock().unwrap().upsert_profile(profile.clone());
                let error = establish_ssh_runtime(&state, &profile, None, None)
                    .await
                    .err()
                    .unwrap();
                assert!(error.contains("host key"), "manual {backend:?}: {error}");
                let before = state.store.lock().unwrap().clone();
                let runtime = establish_ssh_reconnect_runtime(&state, &profile)
                    .await
                    .unwrap();
                assert_eq!(runtime.runtime.backend, backend);
                assert_eq!(runtime.auth_method, AuthMethod::None);
                {
                    let after = state.store.lock().unwrap();
                    assert_eq!(after.host_keys.keys, before.host_keys.keys);
                    assert_eq!(after.profiles, before.profiles);
                    assert!(after
                        .events
                        .last()
                        .unwrap()
                        .text
                        .as_deref()
                        .unwrap()
                        .contains("自动重连临时放行"));
                }
                disconnect_ssh_runtime(
                    runtime.runtime,
                    runtime.read_half,
                    runtime.reader_finished,
                    "reconnect host-key test",
                )
                .await;
            }
        }
        task.abort();
    });
}

#[cfg(unix)]
#[test]
fn reconnect_host_key_bypass_covers_every_hop_in_real_jump_chains() {
    let root = canonical_test_tempdir();
    let host_key = root.path().join("new-host-key");
    let old_key = root.path().join("old-host-key");
    generate_ed25519_test_key(&host_key);
    generate_ed25519_test_key(&old_key);
    tauri::async_runtime::block_on(async {
        let (target_port, target_counters, target_task) =
            spawn_mixed_auth_test_server(&host_key, "user", "unused").await;
        let (first_port, first_counters, first_task) =
            spawn_mixed_auth_test_server(&host_key, "user", "unused").await;
        let (second_port, second_counters, second_task) =
            spawn_mixed_auth_test_server(&host_key, "user", "unused").await;
        for counters in [target_counters, first_counters, second_counters] {
            counters.allow_none_auth.store(true, Ordering::SeqCst);
        }
        let mut backends = vec![SshBackendKind::Russh];
        if cfg!(target_os = "linux") {
            backends.push(SshBackendKind::Libssh);
        }
        for backend in backends {
            let mut profile = test_ssh_profile();
            let ssh = reconnect_ssh_mut(&mut profile);
            ssh.endpoint.host = "127.0.0.1".into();
            ssh.endpoint.port = target_port;
            ssh.username = "user".into();
            ssh.identity_policy.auth_order = if backend == SshBackendKind::Libssh {
                vec![AuthMethod::GssapiWithMic, AuthMethod::None]
            } else {
                vec![AuthMethod::None]
            };
            ssh.agent_policy.enabled = false;
            ssh.jumps = [first_port, second_port]
                .map(|port| portmate_core::JumpHop {
                    host: "127.0.0.1".into(),
                    port,
                    username: "user".into(),
                    password_secret_ref: None,
                    passphrase_secret_ref: None,
                    identity_ref: None,
                    host_key_policy: None,
                })
                .to_vec();
            let old_base64 = load_secret_key(&old_key, None)
                .unwrap()
                .public_key()
                .public_key_base64();
            let mut policies = vec![(target_port, ssh.host_key_policy.clone())];
            policies.extend(
                ssh.jumps
                    .iter()
                    .map(|jump| (jump.port, jump_host_key_policy(ssh, jump))),
            );
            let state = test_app_state(
                profile.clone(),
                root.path().join(format!("jump-{backend:?}.sqlite3")),
            );
            {
                let mut store = state.store.lock().unwrap();
                for (port, policy) in policies {
                    let original = HostKeyObservation {
                        host: "127.0.0.1".into(),
                        port,
                        alias: policy.alias.clone(),
                        algorithm: "ssh-ed25519".into(),
                        public_key_base64: old_base64.clone(),
                    };
                    apply_persistent_host_key_decision_with_policy(
                        &mut store,
                        &profile.id,
                        &policy,
                        &original,
                        HostKeyDecision::AppendToProfile,
                    )
                    .unwrap();
                }
                profile = store.profile(&profile.id).unwrap();
            }
            let error = establish_ssh_reconnect_runtime(&state, &profile)
                .await
                .err()
                .unwrap();
            assert!(
                error.contains("Jump Host") && error.contains("host key"),
                "{error}"
            );
            reconnect_ssh_mut(&mut profile).reconnect_ignore_host_key_changes = true;
            state.store.lock().unwrap().upsert_profile(profile.clone());
            let error = establish_ssh_runtime(&state, &profile, None, None)
                .await
                .err()
                .unwrap();
            assert!(
                error.contains("Jump Host") && error.contains("host key"),
                "{error}"
            );
            let scan = scan_ssh_host_key_inner(&state, profile.clone(), None, None)
                .await
                .unwrap();
            assert!(matches!(
                scan.evaluation,
                HostKeyEvaluation::Mismatch { .. }
            ));
            assert_eq!(scan.observation.port, first_port);
            let before = state.store.lock().unwrap().clone();
            let runtime = establish_ssh_reconnect_runtime(&state, &profile)
                .await
                .unwrap();
            assert_eq!(runtime.runtime.backend, backend);
            assert_eq!(runtime.runtime.jump_handles.len(), 2);
            {
                let after = state.store.lock().unwrap();
                assert_eq!(after.host_keys.keys, before.host_keys.keys);
                assert_eq!(after.profiles, before.profiles);
                let ignored = after
                    .events
                    .iter()
                    .filter(|event| {
                        event
                            .text
                            .as_deref()
                            .is_some_and(|text| text.contains("自动重连临时放行"))
                    })
                    .count();
                assert_eq!(ignored, 3);
            }
            disconnect_ssh_runtime(
                runtime.runtime,
                runtime.read_half,
                runtime.reader_finished,
                "jump reconnect host-key test",
            )
            .await;
        }
        for task in [target_task, first_task, second_task] {
            task.abort();
        }
    });
}

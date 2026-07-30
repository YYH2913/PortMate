use super::*;

#[cfg(target_os = "linux")]
#[test]
fn external_ssh_gssapi_runtime_matrix_case() {
    let _runtime_guard = shared_runtime_test_guard();
    let Ok(case) = std::env::var("PORTMATE_COMPAT_GSSAPI_CASE") else {
        eprintln!("skipping external GSSAPI compatibility test: matrix environment is not set");
        return;
    };
    let host = std::env::var("PORTMATE_COMPAT_GSSAPI_HOST").unwrap();
    let port = std::env::var("PORTMATE_COMPAT_GSSAPI_PORT")
        .unwrap()
        .parse::<u16>()
        .unwrap();
    let verify_pty_resize =
        std::env::var("PORTMATE_COMPAT_GSSAPI_VERIFY_PTY_RESIZE").as_deref() != Ok("0");
    let local_root = std::env::temp_dir().join(format!(
        "portmate-external-gssapi-{}-{}",
        case,
        Uuid::new_v4()
    ));
    fs::create_dir_all(&local_root).unwrap();

    tauri::async_runtime::block_on(async {
        let mut profile = test_ssh_profile();
        let ConnectionConfig::Ssh(ssh) = &mut profile.connection else {
            panic!("expected SSH profile");
        };
        ssh.endpoint.host = host;
        ssh.endpoint.port = port;
        ssh.username = "portmate".to_string();
        ssh.reconnect = false;
        ssh.host_key_policy.mode = if case == "host-key-reject" {
            HostKeyMode::Strict
        } else {
            HostKeyMode::TrustOnFirstUse
        };
        let mixed_auth = matches!(
            case.as_str(),
            "gssapi-preferred"
                | "corrupt-ticket-password-fallback"
                | "password-fallback"
                | "server-disabled-password-fallback"
        );
        ssh.identity_policy.auth_order = if mixed_auth {
            vec![AuthMethod::GssapiWithMic, AuthMethod::Password]
        } else {
            vec![AuthMethod::GssapiWithMic]
        };
        ssh.identity_refs.clear();
        ssh.agent_policy.enabled = false;
        ssh.agent_policy.forwarding = false;
        ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;

        let state = test_app_state(profile.clone(), local_root.join("portmate-store.sqlite3"));
        let password = match case.as_str() {
            "gssapi-preferred" => Some("deliberately-wrong-password".to_string()),
            "corrupt-ticket-password-fallback"
            | "password-fallback"
            | "server-disabled-password-fallback" => Some("portmate".to_string()),
            _ => None,
        };
        let opened = open_ssh_session(&state, profile.clone(), password, None).await;

        match case.as_str() {
            "host-key-reject" => {
                let error = opened.unwrap_err();
                assert!(error.contains("SSH host key 未受信任"), "{error}");
            }
            "corrupt-ticket" | "no-ticket" => {
                let error = opened.unwrap_err();
                assert!(error.contains("GSSAPI authentication"), "{error}");
            }
            "server-disabled" => {
                let error = opened.unwrap_err();
                assert!(
                    error.contains("did not advertise gssapi-with-mic"),
                    "{error}"
                );
            }
            "success"
            | "gssapi-preferred"
            | "corrupt-ticket-password-fallback"
            | "password-fallback"
            | "server-disabled-password-fallback"
            | "sftp-rejected"
            | "sftp-operation-denied" => {
                let connected =
                    opened.unwrap_or_else(|error| panic!("GSSAPI open failed: {error}"));
                assert_eq!(connected.runtime.status, SessionStatus::Connected);
                let expected_auth = if matches!(
                    case.as_str(),
                    "corrupt-ticket-password-fallback"
                        | "password-fallback"
                        | "server-disabled-password-fallback"
                ) {
                    AuthMethod::Password
                } else {
                    AuthMethod::GssapiWithMic
                };
                let recorded_auth =
                    state
                        .store
                        .lock()
                        .unwrap()
                        .profile(&profile.id)
                        .and_then(|profile| match &profile.connection {
                            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
                                ssh.identity_policy.last_successful
                            }
                            _ => None,
                        });
                assert_eq!(recorded_auth, Some(expected_auth));
                assert!(state
                    .store
                    .lock()
                    .unwrap()
                    .host_keys
                    .keys
                    .iter()
                    .any(|key| {
                        key.profile_id.as_deref() == Some(profile.id.as_str())
                            && key.host == "localhost"
                            && key.port == port
                    }));

                let health = ssh_health::check_ssh_health_inner(&state, &profile.id, true)
                    .await
                    .unwrap();
                assert!(health.transport_round_trip_ms.is_some());
                assert!(health.channel_round_trip_ms.is_some());
                if matches!(case.as_str(), "sftp-rejected" | "sftp-operation-denied") {
                    assert_eq!(health.status, ssh_health::SshHealthStatus::Degraded);
                    assert!(health.sftp_round_trip_ms.is_none());
                    assert!(health.sftp_error.is_some());
                } else {
                    assert_eq!(health.status, ssh_health::SshHealthStatus::Healthy);
                    assert!(health.sftp_round_trip_ms.is_some());
                    assert!(health.sftp_error.is_none());
                }

                resize_session_inner(&state, profile.id.clone(), 101, 37)
                    .await
                    .unwrap();
                let shell_probe = if verify_pty_resize {
                    "printf '__PORTMATE_GSSAPI_SIZE__'; stty size; printf '__PORTMATE_GSSAPI_DONE__\\n'\n"
                } else {
                    "printf '__PORTMATE_GSSAPI_SHELL____PORTMATE_GSSAPI_DONE__\\n'\n"
                };
                send_text_inner(
                    state.session_io(),
                    profile.id.clone(),
                    shell_probe.to_string(),
                )
                .await
                .unwrap();
                tokio::time::timeout(Duration::from_secs(5), async {
                    loop {
                        let ready =
                            state
                                .store
                                .lock()
                                .unwrap()
                                .screen(&profile.id)
                                .is_some_and(|screen| {
                                    screen.contains("__PORTMATE_GSSAPI_DONE__")
                                        && if verify_pty_resize {
                                            screen.contains("__PORTMATE_GSSAPI_SIZE__")
                                                && screen.contains("37 101")
                                        } else {
                                            screen.contains("__PORTMATE_GSSAPI_SHELL__")
                                        }
                                });
                        if ready {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .expect("GSSAPI shell probe timed out");

                close_session_inner(&state, profile.id.clone())
                    .await
                    .unwrap();
                assert!(!state.ssh.lock().unwrap().contains_key(&profile.id));
            }
            other => panic!("unsupported GSSAPI compatibility case: {other}"),
        }
    });

    fs::remove_dir_all(local_root).unwrap();
}

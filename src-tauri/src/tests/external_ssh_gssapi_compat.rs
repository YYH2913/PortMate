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
        ssh.identity_policy.auth_order = vec![AuthMethod::GssapiWithMic];
        ssh.identity_refs.clear();
        ssh.agent_policy.enabled = false;
        ssh.agent_policy.forwarding = false;
        ssh.agent_policy.offer_mode = portmate_core::AgentOfferMode::Disabled;

        let state = test_app_state(profile.clone(), local_root.join("portmate-store.sqlite3"));
        let opened = open_ssh_session(&state, profile.clone(), None, None).await;

        match case.as_str() {
            "host-key-reject" => {
                let error = opened.unwrap_err();
                assert!(error.contains("SSH host key 未受信任"), "{error}");
            }
            "no-ticket" => {
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
            "success" => {
                let connected =
                    opened.unwrap_or_else(|error| panic!("GSSAPI open failed: {error}"));
                assert_eq!(connected.runtime.status, SessionStatus::Connected);
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

                let health = ssh_health::check_ssh_health_inner(&state, &profile.id, false)
                    .await
                    .unwrap();
                assert_eq!(health.status, ssh_health::SshHealthStatus::Healthy);
                assert!(health.transport_round_trip_ms.is_some());
                assert!(health.channel_round_trip_ms.is_some());
                assert!(health.sftp_round_trip_ms.is_none());

                resize_session_inner(&state, profile.id.clone(), 101, 37)
                    .await
                    .unwrap();
                send_text_inner(
                    state.session_io(),
                    profile.id.clone(),
                    "printf '__PORTMATE_GSSAPI_SIZE__'; stty size; printf '__PORTMATE_GSSAPI_DONE__\\n'\n"
                        .to_string(),
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
                                    screen.contains("__PORTMATE_GSSAPI_SIZE__")
                                        && screen.contains("37 101")
                                        && screen.contains("__PORTMATE_GSSAPI_DONE__")
                                });
                        if ready {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                })
                .await
                .expect("GSSAPI PTY resize output timed out");

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

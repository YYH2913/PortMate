use super::*;

#[test]
fn export_bundle_includes_diagnostics_and_redacts_text() {
    let mut store = test_store();
    let ConnectionConfig::Shell(shell) = &mut store.profiles[0].connection else {
        unreachable!("test store should use a shell profile");
    };
    shell.args = vec!["--password".to_string(), "opaque-shell-secret".to_string()];
    shell.cwd = Some("/home/operator/private-shell-cwd".to_string());
    store
        .record_stream_event_with_bytes_ref(
            "test-session",
            EventDirection::Inbound,
            EventStream::Stdout,
            "password=hunter2",
            Some("test.raw:0:16".to_string()),
        )
        .unwrap();
    store.record_transfer(TransferTask {
        id: "transfer-1".to_string(),
        session_id: "test-session".to_string(),
        protocol: TransferProtocol::Sftp,
        source: "a".to_string(),
        destination: "b".to_string(),
        bytes_total: 16,
        bytes_done: 16,
        status: TransferStatus::Completed,
        message: Some("completed".to_string()),
        started_at: None,
        finished_at: None,
        average_bytes_per_second: None,
    });
    store
        .send_text("client", "test-session", "token=abc123")
        .unwrap();
    store
        .record_command_history(
            "deploy --password command-history-secret".to_string(),
            100,
            30,
            Utc::now().timestamp_millis(),
        )
        .unwrap();

    let plain = store.export_session_bundle("test-session");
    let bundle = store.export_session_bundle_redacted("test-session");
    let plain_rendered = serde_json::to_string(&plain).unwrap();
    let rendered = serde_json::to_string(&bundle).unwrap();

    assert!(!plain_rendered.contains("command-history-secret"));
    assert!(!rendered.contains("command-history-secret"));
    assert!(!rendered.contains("test.raw:0:16"));
    assert!(rendered.contains("transfer-1"));
    assert!(rendered.contains("send_text"));
    assert!(!rendered.contains("hunter2"));
    assert!(!rendered.contains("abc123"));
    assert!(!rendered.contains("opaque-shell-secret"));
    assert!(!rendered.contains("/home/operator/private-shell-cwd"));
    assert!(bundle["summary"]["profile"]["connection"]["args"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(bundle["logShards"].as_array().unwrap().is_empty());
    assert!(bundle["events"]
        .as_array()
        .unwrap()
        .iter()
        .all(|event| event["bytesRef"].is_null()));
}

#[test]
fn redacted_bundle_removes_ssh_and_tmux_credentials_and_local_paths() {
    for (kind, connection) in [
        (
            SessionKind::Ssh,
            ConnectionConfig::Ssh(sensitive_ssh_connection()),
        ),
        (
            SessionKind::Tmux,
            ConnectionConfig::Tmux(sensitive_ssh_connection()),
        ),
    ] {
        let mut store = test_store();
        store.profiles[0].kind = kind;
        store.profiles[0].connection = connection;
        store.profiles[0].logging.path_template =
            "/home/operator/private-logs/{session}.raw".to_string();
        store.profiles[0].transfer.default_local_dir =
            Some("/home/operator/private-downloads".to_string());
        store.profiles[0].triggers = vec![TriggerSpec {
            id: "sensitive-trigger".to_string(),
            label: "password=trigger-label-secret".to_string(),
            matcher: TriggerMatcher::Contains {
                text: "token=trigger-match-secret".to_string(),
                case_sensitive: false,
            },
            actions: vec![
                TriggerAction::SendText {
                    text: "opaque-send-secret".to_string(),
                },
                TriggerAction::LocalCommand {
                    command: "/home/operator/private-scripts/deploy".to_string(),
                },
                TriggerAction::CustomLink {
                    url_template: "https://internal.invalid/?token=trigger-url-secret".to_string(),
                },
            ],
            enabled: true,
        }];
        store.runtimes[0].active_transport = kind;
        store.runtimes[0].cwd = Some("/home/operator/runtime-cwd".to_string());
        store.runtimes[0].last_disconnect_reason = Some("password=disconnect-secret".to_string());
        store
            .record_event(
                "test-session",
                EventDirection::Inbound,
                EventStream::Stdout,
                Some("password=event-secret".to_string()),
                Some("v2:/home/operator/private-logs/raw:0:12:digest".to_string()),
                BTreeMap::from([(
                    "diagnostic".to_string(),
                    "token=annotation-secret".to_string(),
                )]),
            )
            .unwrap();
        store.record_timeline_mark(TimelineMark {
            id: "timeline-sensitive".to_string(),
            session_id: "test-session".to_string(),
            ts: Utc::now(),
            label: "password=timeline-secret".to_string(),
            details: Some("token=timeline-details-secret".to_string()),
        });
        store.record_transfer(TransferTask {
            id: "transfer-sensitive".to_string(),
            session_id: "test-session".to_string(),
            protocol: TransferProtocol::Sftp,
            source: "/home/operator/source-secret.txt".to_string(),
            destination: "/srv/private/destination-secret.txt".to_string(),
            bytes_total: 12,
            bytes_done: 12,
            status: TransferStatus::Completed,
            message: Some("token=transfer-message-secret".to_string()),
            started_at: None,
            finished_at: None,
            average_bytes_per_second: None,
        });
        store.record_audit(AuditRecord {
            id: "audit-sensitive".to_string(),
            ts: Utc::now(),
            actor: "desktop-user".to_string(),
            action: "export-test".to_string(),
            session_id: Some("test-session".to_string()),
            decision: "recorded".to_string(),
            details: BTreeMap::from([(
                "diagnostic".to_string(),
                "password=audit-secret".to_string(),
            )]),
        });
        store.record_sysmon_snapshot(SysmonSnapshot {
            session_id: "test-session".to_string(),
            ts: Utc::now(),
            uptime_seconds: 123,
            cpu_percent: 12.5,
            memory_percent: 34.5,
            rx_kbps: 56.5,
            tx_kbps: 78.5,
            load_average: [0.5, 1.0, 1.5],
            memory_total_bytes: 1024,
            memory_available_bytes: 512,
            processes: vec![SysmonProcess {
                pid: 4242,
                name: "password=sysmon-process-secret".to_string(),
                cpu_percent: 9.5,
                memory_percent: 8.5,
                rss_bytes: 256,
            }],
            disks: vec![SysmonDisk {
                filesystem: "/dev/mapper/private-filesystem".to_string(),
                mount_point: "/srv/private-mount".to_string(),
                total_bytes: 4096,
                available_bytes: 2048,
                used_percent: 50.0,
            }],
            network_interfaces: vec![SysmonNetworkInterface {
                name: "customer-private-interface".to_string(),
                addresses: vec!["10.0.0.25/24".to_string()],
                rx_bytes: 100,
                tx_bytes: 200,
                rx_kbps: 3.5,
                tx_kbps: 4.5,
            }],
        });

        let plain = store.export_session_bundle("test-session");
        let redacted = store.export_session_bundle_redacted("test-session");
        let plain_json = serde_json::to_string(&plain).unwrap();
        let redacted_json = serde_json::to_string(&redacted).unwrap();
        let sensitive_values = [
            "keyring:target-password-ref",
            "stronghold:target-passphrase-ref",
            "keyring:proxy-password-ref",
            "/home/operator/.ssh/private-key",
            "stronghold:identity-secret-ref",
            "keyring:jump-password-ref",
            "stronghold:jump-passphrase-ref",
            "/home/operator/private-logs/{session}.raw",
            "/home/operator/private-downloads",
            "/home/operator/runtime-cwd",
            "disconnect-secret",
            "event-secret",
            "annotation-secret",
            "timeline-secret",
            "timeline-details-secret",
            "/home/operator/source-secret.txt",
            "/srv/private/destination-secret.txt",
            "transfer-message-secret",
            "audit-secret",
            "trigger-label-secret",
            "trigger-match-secret",
            "opaque-send-secret",
            "/home/operator/private-scripts/deploy",
            "trigger-url-secret",
            "v2:/home/operator/private-logs/raw:0:12:digest",
            "sysmon-process-secret",
            "/dev/mapper/private-filesystem",
            "/srv/private-mount",
            "customer-private-interface",
            "10.0.0.25/24",
        ];

        for sensitive in sensitive_values {
            assert!(
                plain_json.contains(sensitive),
                "plain {kind:?} bundle should retain {sensitive}"
            );
            assert!(
                !redacted_json.contains(sensitive),
                "redacted {kind:?} bundle leaked {sensitive}"
            );
        }
        assert!(redacted["logShards"].as_array().unwrap().is_empty());
        assert!(redacted["events"]
            .as_array()
            .unwrap()
            .iter()
            .all(|event| event["bytesRef"].is_null()));
        assert_eq!(
            redacted["summary"]["profile"]["connection"]["identityRefs"][0]["fingerprintSha256"],
            "SHA256:diagnostic-fingerprint"
        );
        assert_eq!(redacted["transfers"][0]["protocol"], "sftp");
        assert_eq!(redacted["transfers"][0]["status"], "completed");
        assert_eq!(redacted["sysmon"]["processes"][0]["pid"], 4242);
        assert_eq!(
            redacted["sysmon"]["processes"][0]["name"],
            "<redacted-process>"
        );
        assert_eq!(
            redacted["sysmon"]["disks"][0]["filesystem"],
            "<redacted-filesystem>"
        );
        assert_eq!(
            redacted["sysmon"]["disks"][0]["mountPoint"],
            "<redacted-mount-point>"
        );
        assert_eq!(
            redacted["sysmon"]["networkInterfaces"][0]["name"],
            "<redacted-interface>"
        );
        assert_eq!(
            redacted["sysmon"]["networkInterfaces"][0]["addresses"][0],
            "<redacted-address>"
        );
        assert_eq!(
            redacted["summary"]["profile"]["triggers"][0]["actions"][0]["text"],
            "<redacted>"
        );
    }
}

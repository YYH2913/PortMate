#[test]
fn remote_tunnel_listener_probe_parses_linux_bsd_macos_and_unsupported_outputs() {
    let proc_output = "__PORTMATE_PROC__\n  0: 0100007F:0016 00000000:0000 0A 00000000:00000000\n  1: 0100007F:2710 00000000:0000 01 00000000:00000000\n";
    assert_eq!(
        parse_remote_listener_probe(proc_output, 22),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(proc_output, 10_000),
        RemoteListenerProbe::Missing
    );

    let ss_output =
        "__PORTMATE_SS__\nLISTEN 0 128 127.0.0.1:10022 0.0.0.0:*\nLISTEN 0 128 [::]:2200 [::]:*\n";
    assert_eq!(
        parse_remote_listener_probe(ss_output, 10_022),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(ss_output, 2_200),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(ss_output, 22),
        RemoteListenerProbe::Missing
    );

    let sockstat_output = "__PORTMATE_SOCKSTAT__\nUSER COMMAND PID FD PROTO LOCAL ADDRESS FOREIGN ADDRESS\nroot sshd 431 7 tcp4 127.0.0.1:10022 *:*\nroot sshd 431 8 tcp6 [::]:2200 [::]:*\n";
    assert_eq!(
        parse_remote_listener_probe(sockstat_output, 10_022),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(sockstat_output, 22),
        RemoteListenerProbe::Missing
    );

    let sockstat_service_output = "__PORTMATE_SOCKSTAT__\nUSER COMMAND PID FD PROTO LOCAL ADDRESS FOREIGN ADDRESS\nroot sshd 431 7 tcp4 127.0.0.1:ssh *:*\n";
    assert_eq!(
        parse_remote_listener_probe(sockstat_service_output, 22),
        RemoteListenerProbe::Missing
    );

    let lsof_output = "__PORTMATE_LSOF__\nCOMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\nsshd 912 root 5u IPv4 0x1 0t0 TCP *:2200 (LISTEN)\n";
    assert_eq!(
        parse_remote_listener_probe(lsof_output, 2_200),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe(lsof_output, 22),
        RemoteListenerProbe::Missing
    );

    let bsd_netstat_output = "__PORTMATE_NETSTAT__\ntcp4 0 0 127.0.0.1.10022 *.* LISTEN\n";
    assert_eq!(
        parse_remote_listener_probe(bsd_netstat_output, 10_022),
        RemoteListenerProbe::Listening
    );
    assert_eq!(
        parse_remote_listener_probe("__PORTMATE_UNSUPPORTED__\n", 22),
        RemoteListenerProbe::Unsupported
    );
    assert_eq!(
        parse_remote_listener_probe("unexpected output", 22),
        RemoteListenerProbe::Unsupported
    );
}

#[test]
fn remote_tunnel_probe_only_marks_successful_cross_platform_tools() {
    assert!(REMOTE_TUNNEL_PROBE_COMMAND
        .contains("cat /proc/net/tcp /proc/net/tcp6 2>/dev/null || true"));
    assert!(REMOTE_TUNNEL_PROBE_COMMAND
        .contains("command -v sockstat >/dev/null 2>&1 && probe=$(sockstat -46ln 2>/dev/null)"));
    assert!(REMOTE_TUNNEL_PROBE_COMMAND.contains(
        "command -v lsof >/dev/null 2>&1 && probe=$(lsof -nP -iTCP -sTCP:LISTEN 2>/dev/null)"
    ));
    assert!(REMOTE_TUNNEL_PROBE_COMMAND
        .contains("command -v netstat >/dev/null 2>&1 && probe=$(netstat -ltn 2>/dev/null)"));
    assert!(
        REMOTE_TUNNEL_PROBE_COMMAND.find("sockstat").unwrap()
            < REMOTE_TUNNEL_PROBE_COMMAND.find("netstat").unwrap()
    );
    assert!(
        REMOTE_TUNNEL_PROBE_COMMAND.find("lsof").unwrap()
            < REMOTE_TUNNEL_PROBE_COMMAND.find("netstat").unwrap()
    );
}

#[test]
fn remote_tunnel_health_recovery_preserves_non_health_errors() {
    let metrics = TunnelMetrics::default();
    metrics.record_error("remote forward health check failed: listener missing");
    assert!(metrics.clear_error_with_prefix(REMOTE_TUNNEL_HEALTH_ERROR_PREFIX));
    assert!(metrics
        .snapshot(TunnelSpec {
            id: "remote".to_string(),
            label: "remote".to_string(),
            mode: TunnelMode::Remote,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_022,
            target_host: "127.0.0.1".to_string(),
            target_port: 22,
            enabled: true,
        })
        .last_error
        .is_none());

    metrics.record_error("remote tunnel target connect failed");
    assert!(!metrics.clear_error_with_prefix(REMOTE_TUNNEL_HEALTH_ERROR_PREFIX));
    assert_eq!(
        metrics
            .snapshot(TunnelSpec {
                id: "remote".to_string(),
                label: "remote".to_string(),
                mode: TunnelMode::Remote,
                bind_host: "127.0.0.1".to_string(),
                bind_port: 10_022,
                target_host: "127.0.0.1".to_string(),
                target_port: 22,
                enabled: true,
            })
            .last_error
            .as_deref(),
        Some("remote tunnel target connect failed")
    );
}

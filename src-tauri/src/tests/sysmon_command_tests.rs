#[tokio::test]
async fn local_sysmon_command_capture_enforces_exit_timeout_and_stream_bounds() {
    assert_eq!(
        exec_local_sysmon_command("sh", &["-c", "printf portmate"], Duration::from_secs(1))
            .await
            .unwrap(),
        "portmate"
    );

    let exit_error = exec_local_sysmon_command(
        "sh",
        &["-c", "printf partial; printf denied >&2; exit 7"],
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();
    assert!(exit_error.contains("7"));
    assert!(exit_error.contains("denied"));

    let timeout_error =
        exec_local_sysmon_command("sh", &["-c", "sleep 1"], Duration::from_millis(20))
            .await
            .unwrap_err();
    assert!(timeout_error.contains("超时"));

    let overflow_error = read_bounded_local_sysmon_output(&b"12345"[..], 4, "stdout")
        .await
        .unwrap_err();
    assert!(overflow_error.contains("4"));
    assert!(overflow_error.contains("stdout"));
}

#[cfg(unix)]
#[tokio::test]
async fn local_sysmon_timeout_stops_descendants_that_inherit_output() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("child-survived-timeout");
    let command = format!(
        "(sleep 0.2; : > {}) & wait",
        shell_quote(marker.to_str().unwrap())
    );

    let error = exec_local_sysmon_command("sh", &["-c", &command], Duration::from_millis(20))
        .await
        .unwrap_err();
    assert!(error.contains("超时"), "{error}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !marker.exists(),
        "Sysmon timeout left a child process running after its parent was killed"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn local_sysmon_deadline_includes_descendant_output_drain() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("child-held-stdout-after-parent-exit");
    let command = format!(
        "(sleep 0.2; : > {}) & exit 0",
        shell_quote(marker.to_str().unwrap())
    );

    let error = exec_local_sysmon_command("sh", &["-c", &command], Duration::from_millis(20))
        .await
        .unwrap_err();
    assert!(error.contains("超时"), "{error}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !marker.exists(),
        "Sysmon deadline did not terminate a descendant that kept stdout open"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn local_sysmon_collects_live_linux_resource_details() {
    let snapshot = collect_local_sysmon("local-session").await.unwrap();

    assert_eq!(snapshot.session_id, "local-session");
    assert!(snapshot.uptime_seconds > 0);
    assert!((0.0..=100.0).contains(&snapshot.cpu_percent));
    assert!(snapshot.memory_total_bytes > 0);
    assert!(snapshot.memory_available_bytes <= snapshot.memory_total_bytes);
    assert!(snapshot.load_average.iter().all(|value| value.is_finite()));
    assert!(!snapshot.processes.is_empty());
    assert!(!snapshot.disks.is_empty());
    assert!(!snapshot.network_interfaces.is_empty());
    assert!(snapshot
        .network_interfaces
        .iter()
        .any(|interface| !interface.addresses.is_empty()));
}

#[cfg(target_os = "linux")]
#[test]
fn local_linux_sysmon_native_addresses_do_not_require_cli_tools() {
    let addresses = read_linux_network_addresses_from_getifaddrs();
    assert!(addresses
        .values()
        .flatten()
        .any(|address| address == "127.0.0.1/8" || address == "::1/128"));
    assert_eq!(linux_sysmon_prefix_length(&[255, 255, 255, 0]), Some(24));
    assert_eq!(linux_sysmon_prefix_length(&[255, 0, 255, 0]), None);
}

#[cfg(target_os = "linux")]
#[test]
fn local_linux_kernel_address_files_are_bounded() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fib_trie");
    fs::write(&path, b"safe").unwrap();
    assert_eq!(
        read_bounded_local_linux_proc_file(&path, 4).as_deref(),
        Some("safe")
    );

    fs::write(&path, b"oversized").unwrap();
    assert!(read_bounded_local_linux_proc_file(&path, 4).is_none());
}

#[cfg(target_os = "linux")]
#[test]
fn local_linux_sysmon_command_candidates_include_system_sbin_paths() {
    assert_eq!(
        linux_sysmon_command_candidates("ip"),
        vec!["ip", "/usr/sbin/ip", "/usr/bin/ip", "/sbin/ip", "/bin/ip",]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn local_linux_address_command_capture_bounds_output_and_runtime() {
    assert_eq!(
        exec_bounded_sync_sysmon_command(
            "sh",
            &["-c", "printf portmate"],
            Duration::from_secs(1),
            32,
        )
        .as_deref(),
        Some("portmate")
    );
    assert!(exec_bounded_sync_sysmon_command(
        "sh",
        &["-c", "printf 12345"],
        Duration::from_secs(1),
        4,
    )
    .is_none());
    let started = Instant::now();
    assert!(exec_bounded_sync_sysmon_command(
        "sh",
        &["-c", "sleep 1"],
        Duration::from_millis(20),
        32,
    )
    .is_none());
    assert!(started.elapsed() < Duration::from_millis(500));
}

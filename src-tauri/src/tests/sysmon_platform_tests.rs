#[test]
fn remote_sysmon_output_parses_summary_and_structured_details() {
    let output = "123.4 1.0\n\
        __PORTMATE_MEMINFO__\nMemTotal: 1000 kB\nMemAvailable: 250 kB\n\
        __PORTMATE_STAT1__\ncpu 100 0 100 800 0 0 0 0 0 0\n\
        __PORTMATE_NET1__\nInter-| Receive | Transmit\n face | bytes | bytes\neth0: 1024 1 0 0 0 0 0 0 2048 1 0 0 0 0 0 0\n\
        __PORTMATE_STAT2__\ncpu 150 0 150 900 0 0 0 0 0 0\n\
        __PORTMATE_NET2__\nInter-| Receive | Transmit\n face | bytes | bytes\neth0: 3072 1 0 0 0 0 0 0 4096 1 0 0 0 0 0 0\n\
        __PORTMATE_LOADAVG__\n1.25 0.50 0.25 1/10 1\n\
        __PORTMATE_PROCESSES__\n2 12.5 1.5 2048 worker\n3 8.0 4.0 4096 database worker\n\
        __PORTMATE_DISKS__\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/root 1000 750 250 75% /\n";
    let snapshot = parse_remote_sysmon_output("remote-session", output).unwrap();

    assert_eq!(snapshot.session_id, "remote-session");
    assert_eq!(snapshot.uptime_seconds, 123);
    assert_eq!(snapshot.cpu_percent, 50.0);
    assert_eq!(snapshot.memory_percent, 75.0);
    assert_eq!((snapshot.rx_kbps, snapshot.tx_kbps), (10.0, 10.0));
    assert_eq!(snapshot.load_average, [1.25, 0.5, 0.25]);
    assert_eq!(snapshot.processes.len(), 2);
    assert_eq!(snapshot.processes[1].name, "database worker");
    assert_eq!(snapshot.disks.len(), 1);
    assert_eq!(snapshot.network_interfaces.len(), 1);
}

#[test]
fn remote_macos_sysmon_output_parses_bounded_system_details() {
    let output = "__PORTMATE_BOOT__\n{ sec = 1700000000, usec = 0 } Tue Nov 14 22:13:20 2023\n\
        __PORTMATE_CPU__\nCPU usage: 12.50% user, 7.50% sys, 80.00% idle\n\
        __PORTMATE_MEMORY__\n4096000000\nMach Virtual Memory Statistics: (page size of 4096 bytes)\nPages free: 100000.\nPages active: 500000.\nPages inactive: 140000.\nPages speculative: 10000.\n\
        __PORTMATE_NET1__\nName Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll\nen0 1500 <Link#4> aa:bb:cc:dd:ee:ff 10 0 1024 20 0 2048 0\nen0 1500 192.0.2 192.0.2.10 10 0 1024 20 0 2048 0\nlo0 16384 <Link#1> 00:00:00:00:00:00 5 0 500 5 0 500 0\n\
        __PORTMATE_NET2__\nName Mtu Network Address Ipkts Ierrs Ibytes Opkts Oerrs Obytes Coll\nen0 1500 <Link#4> aa:bb:cc:dd:ee:ff 12 0 3072 22 0 4096 0\nen0 1500 192.0.2 192.0.2.10 12 0 3072 22 0 4096 0\nlo0 16384 <Link#1> 00:00:00:00:00:00 5 0 500 5 0 500 0\n\
        __PORTMATE_LOADAVG__\n{ 1.25 0.50 0.25 }\n\
        __PORTMATE_PROCESSES__\n42 25.0 2.5 2048 WindowServer\n84 10.0 1.0 1024 helper process\n\
        __PORTMATE_DISKS__\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/disk3s1s1 1000 750 250 75% /\n";
    let sampled_at = "2023-11-14T22:15:23Z".parse::<DateTime<Utc>>().unwrap();
    let snapshot = parse_remote_macos_sysmon_output_at("mac-session", output, sampled_at).unwrap();

    assert_eq!(snapshot.session_id, "mac-session");
    assert_eq!(snapshot.uptime_seconds, 123);
    assert_eq!(snapshot.cpu_percent, 20.0);
    assert_eq!(snapshot.memory_total_bytes, 4_096_000_000);
    assert_eq!(snapshot.memory_available_bytes, 1_024_000_000);
    assert_eq!(snapshot.memory_percent, 75.0);
    assert_eq!(snapshot.load_average, [1.25, 0.5, 0.25]);
    assert_eq!((snapshot.rx_kbps, snapshot.tx_kbps), (10.0, 10.0));
    assert_eq!(snapshot.network_interfaces.len(), 2);
    assert_eq!(snapshot.network_interfaces[0].name, "en0");
    assert_eq!(snapshot.processes.len(), 2);
    assert_eq!(snapshot.processes[1].name, "helper process");
    assert_eq!(snapshot.disks.len(), 1);
    assert_eq!(snapshot.disks[0].mount_point, "/");
}

#[test]
fn remote_freebsd_sysmon_output_parses_bounded_system_details() {
    let output = "__PORTMATE_BOOT__\n{ sec = 1700000000, usec = 0 } Tue Nov 14 22:13:20 2023\n\
        __PORTMATE_STAT1__\n100 10 50 5 835\n\
        __PORTMATE_NET1__\nName Mtu Network Address Ipkts Ierrs Idrop Ibytes Opkts Oerrs Obytes Coll\nem0 1500 <Link#1> aa:bb:cc:dd:ee:ff 10 0 0 1024 20 0 2048 0\nem0 1500 192.0.2.0 192.0.2.10 10 0 0 1024 20 0 2048 0\nlo0 16384 <Link#2> 00:00:00:00:00:00 5 0 0 500 5 0 500 0\n\
        __PORTMATE_STAT2__\n120 10 70 5 895\n\
        __PORTMATE_NET2__\nName Mtu Network Address Ipkts Ierrs Idrop Ibytes Opkts Oerrs Obytes Coll\nem0 1500 <Link#1> aa:bb:cc:dd:ee:ff 12 0 0 3072 22 0 4096 0\nem0 1500 192.0.2.0 192.0.2.10 12 0 0 3072 22 0 4096 0\nlo0 16384 <Link#2> 00:00:00:00:00:00 5 0 0 500 5 0 500 0\n\
        __PORTMATE_MEMORY__\ntotal 4096000000\npage_size 4096\nfree 100000\ninactive 100000\ncache 50000\n\
        __PORTMATE_LOADAVG__\n{ 0.75 0.50 0.25 }\n\
        __PORTMATE_PROCESSES__\n42 25.0 2.5 2048 bhyve\n84 10.0 1.0 1024 service worker\n\
        __PORTMATE_DISKS__\nFilesystem 1024-blocks Used Available Capacity Mounted on\n/dev/ada0p2 1000 750 250 75% /\n";
    let sampled_at = "2023-11-14T22:15:23Z".parse::<DateTime<Utc>>().unwrap();
    let snapshot =
        parse_remote_freebsd_sysmon_output_at("freebsd-session", output, sampled_at).unwrap();

    assert_eq!(snapshot.session_id, "freebsd-session");
    assert_eq!(snapshot.uptime_seconds, 123);
    assert!((snapshot.cpu_percent - 40.0).abs() < 0.001);
    assert_eq!(snapshot.memory_total_bytes, 4_096_000_000);
    assert_eq!(snapshot.memory_available_bytes, 1_024_000_000);
    assert_eq!(snapshot.memory_percent, 75.0);
    assert_eq!(snapshot.load_average, [0.75, 0.5, 0.25]);
    assert_eq!((snapshot.rx_kbps, snapshot.tx_kbps), (10.0, 10.0));
    assert_eq!(snapshot.network_interfaces.len(), 2);
    assert_eq!(snapshot.network_interfaces[0].name, "em0");
    assert_eq!(snapshot.processes.len(), 2);
    assert_eq!(snapshot.processes[1].name, "service worker");
    assert_eq!(snapshot.disks.len(), 1);
    assert_eq!(snapshot.disks[0].mount_point, "/");
}

#[test]
fn windows_powershell_command_uses_exact_utf16le_encoded_script() {
    let script = "[Console]::Out.WriteLine('PortMate ✓')";
    let command = windows_powershell_command(script);
    let encoded = command
        .strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")
        .unwrap();
    let bytes = BASE64_STANDARD.decode(encoded).unwrap();
    assert_eq!(bytes.len() % 2, 0);
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    assert_eq!(String::from_utf16(&units).unwrap(), script);
    assert!(!REMOTE_WINDOWS_SYSMON_SCRIPT
        .to_ascii_lowercase()
        .contains("commandline"));
    assert!(REMOTE_WINDOWS_SYSMON_SCRIPT.contains("Select-Object -First 8"));
    assert!(REMOTE_WINDOWS_SYSMON_SCRIPT.contains(
        "$addresses = @($_.IPAddress | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) })"
    ));
    assert!(!REMOTE_WINDOWS_SYSMON_SCRIPT.contains(
        "IPAddress | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } | Select-Object -First 8"
    ));
    assert!(REMOTE_WINDOWS_SYSMON_SCRIPT.contains("Select-Object -First 16"));
    assert!(REMOTE_WINDOWS_SYSMON_SCRIPT.contains("Select-Object -First 128"));
    assert!(REMOTE_WINDOWS_SYSMON_SCRIPT.contains("$matchedIpAddressNames"));
    assert!(REMOTE_WINDOWS_SYSMON_SCRIPT.contains("$addressOnlyNetworkInterfaces"));
    assert!(REMOTE_WINDOWS_SYSMON_SCRIPT.contains("addresses = @($_.Value)"));
    assert_eq!(
        remote_sysmon_platform_label("\r\nLinux\r\n").as_deref(),
        Some("Linux")
    );
    assert_eq!(
        remote_sysmon_platform_label("sh : The term 'sh' is not recognized"),
        None
    );
}

#[test]
fn remote_windows_sysmon_json_is_bounded_and_sanitized() {
    let processes = (0..10)
        .map(|index| {
            serde_json::json!({
                "pid": index + 1,
                "name": format!(" process-{index}\u{0007} "),
                "cpuPercent": 150.0 - index as f32,
                "rssBytes": 100 + index,
            })
        })
        .collect::<Vec<_>>();
    let disks = (0..18)
        .map(|index| {
            serde_json::json!({
                "filesystem": "NTFS",
                "mountPoint": format!("D{index}:"),
                "totalBytes": 1000,
                "availableBytes": 2000,
            })
        })
        .collect::<Vec<_>>();
    let mut network_interfaces = (0..34)
        .map(|index| {
            serde_json::json!({
                "name": format!(" nic-{index}-{}\n", "x".repeat(80)),
                "rxBytes": index * 1000,
                "txBytes": index * 500,
                "rxKbps": index as f32,
                "txKbps": index as f32 / 2.0,
            })
        })
        .collect::<Vec<_>>();
    network_interfaces.push(serde_json::json!({
        "name": format!(" NIC-33-{} ", "X".repeat(80)),
        "rxBytes": 999_000,
        "txBytes": 999_000,
        "rxKbps": 999.0,
        "txKbps": 999.0,
    }));
    network_interfaces.push(serde_json::json!({
        "name": format!(" nic-0-{} ", "x".repeat(80)),
        "addresses": ["198.51.100.9", "2001:db8::9", "198.51.100.9"],
        "rxBytes": 0,
        "txBytes": 0,
        "rxKbps": 0.0,
        "txKbps": 0.0,
    }));
    let payload = serde_json::json!({
        "uptimeSeconds": 123,
        "cpuPercent": 150.0,
        "memoryTotalBytes": 1000,
        "memoryAvailableBytes": 1200,
        "processes": processes,
        "disks": disks,
        "networkInterfaces": network_interfaces,
    });
    let output = format!("ignored preface\r\n{REMOTE_WINDOWS_SYSMON_JSON_MARKER}\r\n{payload}\r\n");
    let sampled_at = "2026-07-14T12:34:56Z".parse::<DateTime<Utc>>().unwrap();
    let snapshot =
        parse_remote_windows_sysmon_output_at("windows-session", &output, sampled_at).unwrap();

    assert_eq!(snapshot.session_id, "windows-session");
    assert_eq!(snapshot.ts, sampled_at);
    assert_eq!(snapshot.uptime_seconds, 123);
    assert_eq!(snapshot.cpu_percent, 100.0);
    assert_eq!(snapshot.memory_available_bytes, 1000);
    assert_eq!(snapshot.memory_percent, 0.0);
    assert_eq!(snapshot.load_average, [0.0; 3]);
    assert_eq!(snapshot.processes.len(), MAX_SYSMON_PROCESSES);
    assert!(snapshot.processes.iter().all(|process| {
        process.cpu_percent == 100.0
            && process.memory_percent <= 100.0
            && !process.name.chars().any(char::is_control)
    }));
    assert_eq!(snapshot.disks.len(), MAX_SYSMON_DISKS);
    assert!(snapshot
        .disks
        .iter()
        .all(|disk| disk.available_bytes == disk.total_bytes && disk.used_percent == 0.0));
    assert_eq!(
        snapshot.network_interfaces.len(),
        MAX_SYSMON_NETWORK_INTERFACES
    );
    assert!(snapshot.network_interfaces.iter().all(|interface| {
        interface.name.chars().count() <= 64 && !interface.name.chars().any(char::is_control)
    }));
    assert_eq!(
        snapshot
            .network_interfaces
            .iter()
            .map(|interface| interface.name.to_ascii_lowercase())
            .collect::<HashSet<_>>()
            .len(),
        snapshot.network_interfaces.len()
    );
    let nic_zero = snapshot
        .network_interfaces
        .iter()
        .find(|interface| interface.name.starts_with("nic-0-"))
        .unwrap();
    assert_eq!(nic_zero.addresses, vec!["198.51.100.9", "2001:db8::9"]);
    assert_eq!((snapshot.rx_kbps, snapshot.tx_kbps), (558.0, 279.0));
    assert!(parse_remote_windows_sysmon_output("windows-session", "{}").is_err());
}

#[test]
fn remote_windows_sysmon_keeps_usable_addresses_before_interface_address_limit() {
    let link_local = (0..MAX_SYSMON_NETWORK_ADDRESSES_PER_INTERFACE)
        .map(|index| format!("fe80::{index}"))
        .collect::<Vec<_>>();
    let payload = serde_json::json!({
        "uptimeSeconds": 1,
        "cpuPercent": 1.0,
        "memoryTotalBytes": 1024,
        "memoryAvailableBytes": 512,
        "networkInterfaces": [{
            "name": "Ethernet",
            "addresses": [
                link_local[0], link_local[1], link_local[2], link_local[3],
                link_local[4], link_local[5], link_local[6], link_local[7],
                "192.0.2.42", "2001:db8::42"
            ],
            "rxBytes": 0,
            "txBytes": 0,
            "rxKbps": 0.0,
            "txKbps": 0.0,
        }],
    });
    let output = format!("{REMOTE_WINDOWS_SYSMON_JSON_MARKER}\n{payload}");
    let snapshot = parse_remote_windows_sysmon_output("windows-session", &output).unwrap();

    assert_eq!(snapshot.network_interfaces.len(), 1);
    assert_eq!(
        &snapshot.network_interfaces[0].addresses[..2],
        ["192.0.2.42".to_string(), "2001:db8::42".to_string()]
    );
    assert_eq!(
        snapshot.network_interfaces[0].addresses.len(),
        MAX_SYSMON_NETWORK_ADDRESSES_PER_INTERFACE
    );
    assert!(!snapshot.network_interfaces[0]
        .addresses
        .contains(&"fe80::6".to_string()));
    assert!(!snapshot.network_interfaces[0]
        .addresses
        .contains(&"fe80::7".to_string()));
}


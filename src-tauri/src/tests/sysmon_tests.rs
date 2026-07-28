use super::*;

#[test]
fn sysmon_detail_parsers_bound_sort_and_measure_samples() {
    let processes = parse_sysmon_processes(
        "1 0.1 0.2 100 init\n\
         2 12.5 1.5 2048 worker\n\
         3 8.0 4.0 4096 database worker\n\
         4 7.0 1.0 100 p4\n\
         5 6.0 1.0 100 p5\n\
         6 5.0 1.0 100 p6\n\
         7 4.0 1.0 100 p7\n\
         8 3.0 1.0 100 p8\n\
         9 2.0 1.0 100 p9\n\
         10 NaN 1.0 100 invalid\n",
    );
    assert_eq!(processes.len(), MAX_SYSMON_PROCESSES);
    assert_eq!(
        (processes[0].pid, processes[0].name.as_str()),
        (2, "worker")
    );
    assert_eq!(processes[0].rss_bytes, 2 * 1024 * 1024);
    assert_eq!(processes[1].name, "database worker");
    assert!(!processes.iter().any(|process| process.pid == 10));

    assert_eq!(
        parse_cpu_times("cpu 18446744073709551615 1 1 1\n"),
        Some((1, u64::MAX))
    );

    let disks = parse_sysmon_disks(
        "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
         /dev/root 1000 750 250 75% /\n\
         /dev/data 2000 500 1500 25% /srv/data\n\
         /dev/duplicate 2000 1000 1000 50% /srv/data\n",
    );
    assert_eq!(disks.len(), 2);
    assert_eq!(disks[0].mount_point, "/");
    assert_eq!(disks[0].total_bytes, 1_024_000);
    assert_eq!(disks[0].available_bytes, 256_000);
    assert_eq!(disks[0].used_percent, 75.0);

    let before = parse_network_interfaces(
        "Inter-| Receive | Transmit\n face | bytes | bytes\n\
         eth0: 1024 1 0 0 0 0 0 0 2048 1 0 0 0 0 0 0\n",
    );
    let after = parse_network_interfaces(
        "Inter-| Receive | Transmit\n face | bytes | bytes\n\
         eth0: 3072 1 0 0 0 0 0 0 4096 1 0 0 0 0 0 0\n\
         lo: 500 1 0 0 0 0 0 0 500 1 0 0 0 0 0 0\n",
    );
    let addresses = parse_linux_network_addresses(
        "2: eth0    inet 192.0.2.10/24 brd 192.0.2.255 scope global eth0\n\
         2: eth0    inet6 fe80::1/64 scope link\n\
         lo        Link encap:Local Loopback\n\
                   inet addr:127.0.0.1  Mask:255.0.0.0\n",
    );
    let interfaces = network_interface_rates(before, after, addresses, 2.0);
    assert_eq!(interfaces.len(), 2);
    assert_eq!(interfaces[0].name, "eth0");
    assert_eq!(interfaces[0].addresses, vec!["192.0.2.10/24", "fe80::1/64"]);
    assert_eq!((interfaces[0].rx_kbps, interfaces[0].tx_kbps), (1.0, 1.0));
    assert_eq!(interfaces[1].addresses, vec!["127.0.0.1"]);
    assert_eq!((interfaces[1].rx_kbps, interfaces[1].tx_kbps), (0.0, 0.0));
    assert_eq!(aggregate_network_rates(&interfaces), (1.0, 1.0));

    let veth_addresses = parse_linux_network_addresses(
        r#"2: eth0@if7    inet 198.51.100.5/24 brd 198.51.100.255 scope global eth0
2: eth0@if7    inet6 fe80::2/64 scope link"#,
    );
    assert_eq!(
        veth_addresses.get("eth0").cloned().unwrap_or_default(),
        vec!["198.51.100.5/24", "fe80::2/64"]
    );
    assert!(!veth_addresses.contains_key("eth0@if7"));

    let ifconfig_addresses = parse_linux_network_addresses(
        r#"eth0: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>  mtu 1500
    inet 203.0.113.7  netmask 255.255.255.0  broadcast 203.0.113.255
    inet6 fe80::3  prefixlen 64  scopeid 0x20<link>
    ether 02:42:ac:11:00:02  txqueuelen 0  (Ethernet)
lo: flags=73<UP,LOOPBACK,RUNNING>  mtu 65536
    inet 127.0.0.1  netmask 255.0.0.0"#,
    );
    assert_eq!(
        ifconfig_addresses.get("eth0").cloned().unwrap_or_default(),
        vec!["203.0.113.7", "fe80::3"]
    );
    assert_eq!(
        ifconfig_addresses.get("lo").cloned().unwrap_or_default(),
        vec!["127.0.0.1"]
    );
    assert!(!ifconfig_addresses.contains_key("ether"));

    let compact_ifconfig_addresses = parse_linux_network_addresses(
        r#"br-lan    Link encap:Ethernet  HWaddr 02:11:22:33:44:55
br-lan    inet 192.168.1.1  Bcast:192.168.1.255  Mask:255.255.255.0
br-lan    inet6 fe80::211:22ff:fe33:4455/64 Scope:Link
eth0      inet addr:10.0.0.2  Bcast:10.0.0.255  Mask:255.255.255.0"#,
    );
    assert_eq!(
        compact_ifconfig_addresses
            .get("br-lan")
            .cloned()
            .unwrap_or_default(),
        vec!["192.168.1.1", "fe80::211:22ff:fe33:4455/64"]
    );
    assert_eq!(
        compact_ifconfig_addresses
            .get("eth0")
            .cloned()
            .unwrap_or_default(),
        vec!["10.0.0.2"]
    );

    let busybox_ifconfig_addresses = parse_linux_network_addresses(
        r#"br-lan    Link encap:Ethernet  HWaddr 02:11:22:33:44:55
      inet addr:192.168.3.1  Bcast:192.168.3.255  Mask:255.255.255.0
      inet6 addr:fe80::211:22ff:fe33:4455/64 Scope:Link
"#,
    );
    assert_eq!(
        busybox_ifconfig_addresses
            .get("br-lan")
            .cloned()
            .unwrap_or_default(),
        vec!["192.168.3.1", "fe80::211:22ff:fe33:4455/64"]
    );

    let openwrt_addresses = parse_openwrt_network_interface_dump(
        r#"{
  "interface": [
{
  "interface": "lan",
  "device": "br-lan",
  "l3_device": "br-lan",
  "ipv4-address": [{"address": "192.168.8.1", "mask": 24}],
  "ipv6-address": [{"address": "fd12:3456::1", "mask": 64}]
},
{
  "interface": "wan",
  "l3_device": "pppoe-wan",
  "ipv4-address": [{"address": "198.51.100.42", "mask": 32}]
}
  ]
}"#,
    );
    assert_eq!(
        openwrt_addresses.get("br-lan").cloned().unwrap_or_default(),
        vec!["192.168.8.1/24", "fd12:3456::1/64"]
    );
    assert_eq!(
        openwrt_addresses
            .get("pppoe-wan")
            .cloned()
            .unwrap_or_default(),
        vec!["198.51.100.42/32"]
    );
    assert!(parse_openwrt_network_interface_dump("{not-json").is_empty());

    let before_rows = (0..=MAX_SYSMON_NETWORK_INTERFACES)
        .map(|index| format!("veth-{index}: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n"))
        .chain(std::iter::once(
            "br-lan: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n".to_string(),
        ))
        .collect::<String>();
    let after_rows = (0..=MAX_SYSMON_NETWORK_INTERFACES)
        .map(|index| {
            let bytes = (index as u64 + 1) * 1024;
            format!("veth-{index}: {bytes} 0 0 0 0 0 0 0 {bytes} 0 0 0 0 0 0 0\n")
        })
        .chain(std::iter::once(
            "br-lan: 1 0 0 0 0 0 0 0 1 0 0 0 0 0 0 0\n".to_string(),
        ))
        .collect::<String>();
    let output = format!(
        "1.0 0.0\n__PORTMATE_MEMINFO__\nMemTotal: 1 kB\nMemAvailable: 1 kB\n__PORTMATE_STAT1__\ncpu 1 0 0 1\n__PORTMATE_NET1__\nInter-| Receive | Transmit\n face | bytes | bytes\n{before_rows}__PORTMATE_STAT2__\ncpu 2 0 0 2\n__PORTMATE_NET2__\nInter-| Receive | Transmit\n face | bytes | bytes\n{after_rows}__PORTMATE_ADDRS__\n__PORTMATE_LOADAVG__\n0 0 0\n__PORTMATE_PROCESSES__\n__PORTMATE_DISKS__\n"
    );
    let mut snapshot = parse_remote_sysmon_output("openwrt-session", &output).unwrap();
    assert_eq!(
        snapshot.network_interfaces.len(),
        MAX_SYSMON_NETWORK_INTERFACES
    );
    assert!(!snapshot
        .network_interfaces
        .iter()
        .any(|interface| interface.name == "br-lan"));
    assert!(remote_linux_sysmon_needs_network_address_fallback(
        &snapshot.network_interfaces
    ));
    let link_local_only = [SysmonNetworkInterface {
        name: "br-lan".to_string(),
        addresses: vec!["fe80::211:22ff:fe33:4455/64".to_string()],
        rx_bytes: 0,
        tx_bytes: 0,
        rx_kbps: 0.0,
        tx_kbps: 0.0,
    }];
    assert!(remote_linux_sysmon_needs_network_address_fallback(
        &link_local_only
    ));
    let lan_address = [SysmonNetworkInterface {
        addresses: vec!["192.168.8.1/24".to_string()],
        ..link_local_only[0].clone()
    }];
    assert!(!remote_linux_sysmon_needs_network_address_fallback(
        &lan_address
    ));
    merge_remote_linux_sysmon_network_addresses(
        &mut snapshot.network_interfaces,
        &mut snapshot.rx_kbps,
        &mut snapshot.tx_kbps,
        &output,
        openwrt_addresses,
    );
    assert_eq!(snapshot.network_interfaces[0].name, "br-lan");
    assert_eq!(
        snapshot.network_interfaces[0].addresses,
        vec!["192.168.8.1/24", "fd12:3456::1/64"]
    );
    assert!(!remote_linux_sysmon_needs_network_address_fallback(
        &snapshot.network_interfaces
    ));

    let fallback_addresses = first_nonempty_linux_network_addresses([
        "ip: unsupported output".to_string(),
        "br-lan    inet 192.168.2.1  Bcast:192.168.2.255  Mask:255.255.255.0".to_string(),
    ]);
    assert_eq!(
        fallback_addresses
            .get("br-lan")
            .cloned()
            .unwrap_or_default(),
        vec!["192.168.2.1"]
    );
    let source_reads = std::cell::Cell::new(0);
    let lazy_fallback_addresses = first_nonempty_linux_network_addresses((0..3).map(|index| {
        source_reads.set(source_reads.get() + 1);
        match index {
            0 => "ip: unsupported output".to_string(),
            1 => "eth0 inet 198.51.100.42/24 scope global".to_string(),
            _ => panic!("address lookup continued after a valid source"),
        }
    }));
    assert_eq!(source_reads.get(), 2);
    assert_eq!(
        lazy_fallback_addresses
            .get("eth0")
            .cloned()
            .unwrap_or_default(),
        vec!["198.51.100.42/24"]
    );
    let hostname_source_reads = std::cell::Cell::new(0);
    let hostname_output = first_nonempty_linux_hostname_address_output((0..3).map(|index| {
        hostname_source_reads.set(hostname_source_reads.get() + 1);
        match index {
            0 => "127.0.0.1 ::1".to_string(),
            1 => "127.0.0.1 198.51.100.42 2001:db8::42".to_string(),
            _ => panic!("hostname lookup continued after a usable address"),
        }
    }));
    assert_eq!(hostname_source_reads.get(), 2);
    assert_eq!(
        parse_linux_hostname_network_addresses(&hostname_output),
        vec!["198.51.100.42", "2001:db8::42"]
    );
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("ip -o addr show 2>/dev/null | head -n 64"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("ifconfig -a 2>/dev/null"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("busybox ip -o addr show 2>/dev/null"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("busybox ip addr show 2>/dev/null"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("busybox ifconfig -a 2>/dev/null"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("__PORTMATE_KERNEL_IPV6__"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("/proc/net/if_inet6"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("__PORTMATE_KERNEL_IPV4__"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("/proc/net/fib_trie"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("__PORTMATE_KERNEL_ROUTE__"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("/proc/net/route"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("__PORTMATE_HOSTNAME_ADDRS__"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("hostname -I 2>/dev/null"));
    assert!(REMOTE_OPENWRT_SYSMON_NETWORK_COMMAND
        .contains("ubus call network.interface dump 2>/dev/null"));
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("head -n 384"));

    assert_eq!(
        parse_load_average("1.25 0.50 0.25 1/10 1"),
        Some([1.25, 0.5, 0.25])
    );
    assert_eq!(
        parse_memory_usage("MemTotal: 1000 kB\n\nMemAvailable: 250 kB\n"),
        Some((1_024_000, 256_000, 75.0))
    );
}

#[test]
fn sysmon_network_interfaces_keep_addressed_interfaces_within_the_display_limit() {
    let before = (0..=MAX_SYSMON_NETWORK_INTERFACES)
        .map(|index| (format!("veth-{index}"), (0, 0)))
        .chain(std::iter::once(("uplink".to_string(), (0, 0))))
        .collect::<BTreeMap<_, _>>();
    let after = (0..=MAX_SYSMON_NETWORK_INTERFACES)
        .map(|index| {
            (
                format!("veth-{index}"),
                ((index as u64 + 1) * 1024, (index as u64 + 1) * 1024),
            )
        })
        .chain(std::iter::once(("uplink".to_string(), (1, 1))))
        .collect::<BTreeMap<_, _>>();
    let addresses = BTreeMap::from([("uplink".to_string(), vec!["198.51.100.42/24".to_string()])]);

    let interfaces = network_interface_rates(before, after, addresses, 1.0);

    assert_eq!(interfaces.len(), MAX_SYSMON_NETWORK_INTERFACES);
    assert_eq!(interfaces[0].name, "uplink");
    assert_eq!(interfaces[0].addresses, vec!["198.51.100.42/24"]);
    assert!(REMOTE_LINUX_SYSMON_COMMAND.contains("head -n 258 /proc/net/dev"));
}

#[test]
fn sysmon_network_address_limit_keeps_usable_addresses_before_link_local_entries() {
    let link_local = (0..MAX_SYSMON_NETWORK_ADDRESSES_PER_INTERFACE)
        .map(|index| format!("2: eth0 inet6 fe80::{index}/64 scope link\n"))
        .collect::<String>();
    let addresses = parse_linux_network_addresses(&format!(
        "{link_local}2: eth0 inet 192.0.2.42/24 scope global eth0\n2: eth0 inet6 2001:db8::42/64 scope global\n"
    ));
    let eth0 = addresses.get("eth0").cloned().unwrap_or_default();

    assert_eq!(eth0.len(), MAX_SYSMON_NETWORK_ADDRESSES_PER_INTERFACE);
    assert_eq!(
        &eth0[..2],
        ["192.0.2.42/24".to_string(), "2001:db8::42/64".to_string()]
    );
    assert!(!eth0.contains(&"fe80::6/64".to_string()));
    assert!(!eth0.contains(&"fe80::7/64".to_string()));
}

#[test]
fn remote_linux_sysmon_keeps_addressed_interface_without_proc_net_counter() {
    let output = r#"1.0 0.0
__PORTMATE_MEMINFO__
MemTotal: 1024 kB
MemAvailable: 512 kB
__PORTMATE_STAT1__
cpu 1 0 0 1
__PORTMATE_NET1__
Inter-| Receive | Transmit
 face | bytes | bytes
__PORTMATE_STAT2__
cpu 2 0 0 2
__PORTMATE_NET2__
Inter-| Receive | Transmit
 face | bytes | bytes
__PORTMATE_ADDRS__
7: br-lan    inet 192.168.8.1/24 brd 192.168.8.255 scope global br-lan
__PORTMATE_LOADAVG__
0.00 0.00 0.00
__PORTMATE_PROCESSES__
__PORTMATE_DISKS__
"#;

    let snapshot = parse_remote_sysmon_output("remote-session", output).unwrap();

    assert_eq!(snapshot.network_interfaces.len(), 1);
    assert_eq!(snapshot.network_interfaces[0].name, "br-lan");
    assert_eq!(
        snapshot.network_interfaces[0].addresses,
        vec!["192.168.8.1/24"]
    );
    assert_eq!(snapshot.network_interfaces[0].rx_bytes, 0);
    assert_eq!(snapshot.network_interfaces[0].tx_bytes, 0);
    assert_eq!(snapshot.rx_kbps, 0.0);
    assert_eq!(snapshot.tx_kbps, 0.0);
}

#[test]
fn openwrt_sysmon_fallback_keeps_ubus_addresses_without_proc_net_interfaces() {
    let output = "1.0 0.0\n\
__PORTMATE_MEMINFO__\n\
MemTotal: 1024 kB\n\
MemAvailable: 512 kB\n\
__PORTMATE_STAT1__\n\
cpu 1 0 0 1\n\
__PORTMATE_NET1__\n\
Inter-| Receive | Transmit\n\
 face | bytes | bytes\n\
__PORTMATE_STAT2__\n\
cpu 2 0 0 2\n\
__PORTMATE_NET2__\n\
Inter-| Receive | Transmit\n\
 face | bytes | bytes\n\
__PORTMATE_ADDRS__\n\
__PORTMATE_LOADAVG__\n\
0.00 0.00 0.00\n\
__PORTMATE_PROCESSES__\n\
__PORTMATE_DISKS__\n";
    let mut snapshot = parse_remote_sysmon_output("openwrt-session", output).unwrap();
    assert!(snapshot.network_interfaces.is_empty());
    assert!(remote_linux_sysmon_needs_network_address_fallback(
        &snapshot.network_interfaces
    ));

    merge_remote_linux_sysmon_network_addresses(
        &mut snapshot.network_interfaces,
        &mut snapshot.rx_kbps,
        &mut snapshot.tx_kbps,
        output,
        BTreeMap::from([(
            "br-lan".to_string(),
            vec!["192.168.8.1/24".to_string(), "fd12:3456::1/64".to_string()],
        )]),
    );

    assert_eq!(snapshot.network_interfaces.len(), 1);
    assert_eq!(snapshot.network_interfaces[0].name, "br-lan");
    assert_eq!(
        snapshot.network_interfaces[0].addresses,
        vec!["192.168.8.1/24", "fd12:3456::1/64"]
    );
    assert_eq!(snapshot.network_interfaces[0].rx_bytes, 0);
    assert_eq!(snapshot.network_interfaces[0].tx_bytes, 0);
    assert_eq!(snapshot.rx_kbps, 0.0);
    assert_eq!(snapshot.tx_kbps, 0.0);
}

#[test]
fn remote_linux_sysmon_uses_kernel_address_tables_after_sparse_tool_output() {
    let output = r#"1.0 0.0
__PORTMATE_MEMINFO__
MemTotal: 1024 kB
MemAvailable: 512 kB
__PORTMATE_STAT1__
cpu 1 0 0 1
__PORTMATE_NET1__
Inter-| Receive | Transmit
 face | bytes | bytes
__PORTMATE_STAT2__
cpu 2 0 0 2
__PORTMATE_NET2__
Inter-| Receive | Transmit
 face | bytes | bytes
__PORTMATE_ADDRS__
2: wan0    inet6 fe80::1/64 scope link
__PORTMATE_KERNEL_IPV6__
00000000000000000000000000000001 01 80 10 80       lo
20010db8000000000000000000000042 03 40 00 80       wan0
__PORTMATE_KERNEL_IPV4__
Main:
  +-- 0.0.0.0/0 3 0 4
 |-- 0.0.0.0
    /0 universe UNICAST
 +-- 192.0.2.0/24 2 0 1
    |-- 192.0.2.0
       /24 link UNICAST
    |-- 192.0.2.42
       /32 host LOCAL
    |-- 192.0.2.255
       /32 link BROADCAST
__PORTMATE_KERNEL_ROUTE__
Iface	Destination	Gateway 	Flags	RefCnt	Use	Metric	Mask	MTU	Window	IRTT
wan0	00000000	00000000	0003	0	0	0	00000000	0	0	0
__PORTMATE_HOSTNAME_ADDRS__
127.0.0.1 192.0.2.99 2001:db8::99 fe80::99
__PORTMATE_LOADAVG__
0.00 0.00 0.00
__PORTMATE_PROCESSES__
__PORTMATE_DISKS__
"#;

    let mut snapshot = parse_remote_sysmon_output("remote-session", output).unwrap();
    assert!(remote_linux_sysmon_needs_network_address_fallback(
        &snapshot.network_interfaces
    ));

    let kernel_addresses = parse_remote_linux_kernel_network_addresses(output);
    assert_eq!(
        kernel_addresses.get("wan0").cloned().unwrap_or_default(),
        vec![
            "192.0.2.42",
            "192.0.2.99",
            "2001:db8::42/64",
            "2001:db8::99",
        ]
    );

    merge_remote_linux_sysmon_network_addresses(
        &mut snapshot.network_interfaces,
        &mut snapshot.rx_kbps,
        &mut snapshot.tx_kbps,
        output,
        kernel_addresses,
    );

    assert!(snapshot.network_interfaces.iter().any(|interface| {
        interface.name == "wan0"
            && interface.addresses
                == [
                    "192.0.2.42",
                    "192.0.2.99",
                    "2001:db8::42/64",
                    "2001:db8::99",
                    "fe80::1/64",
                ]
    }));
    assert!(!remote_linux_sysmon_needs_network_address_fallback(
        &snapshot.network_interfaces
    ));
}

#[test]
fn remote_linux_kernel_addresses_fill_a_missing_default_interface() {
    let output = r#"__PORTMATE_KERNEL_IPV6__
__PORTMATE_KERNEL_IPV4__
__PORTMATE_KERNEL_ROUTE__
Iface	Destination	Gateway 	Flags	RefCnt	Use	Metric	Mask	MTU	Window	IRTT
eth0	00000000	00000000	0003	0	0	0	00000000	0	0	0
__PORTMATE_HOSTNAME_ADDRS__
192.0.2.77
__PORTMATE_LOADAVG__
"#;
    let addresses = parse_remote_linux_kernel_network_addresses(output);
    assert_eq!(
        addresses.get("eth0").cloned().unwrap_or_default(),
        vec!["192.0.2.77"]
    );

    let docker = SysmonNetworkInterface {
        name: "docker0".to_string(),
        addresses: vec!["172.17.0.1/16".to_string()],
        rx_bytes: 0,
        tx_bytes: 0,
        rx_kbps: 0.0,
        tx_kbps: 0.0,
    };
    let missing_default = SysmonNetworkInterface {
        name: "eth0".to_string(),
        addresses: Vec::new(),
        ..docker.clone()
    };
    assert!(!remote_linux_sysmon_needs_network_address_fallback(&[
        docker.clone(),
        missing_default.clone(),
    ]));
    assert!(remote_linux_kernel_addresses_need_merge(
        &[docker.clone(), missing_default],
        &addresses
    ));

    let addressed_default = SysmonNetworkInterface {
        name: "eth0".to_string(),
        addresses: vec!["192.0.2.77".to_string()],
        ..docker.clone()
    };
    assert!(!remote_linux_kernel_addresses_need_merge(
        &[docker.clone(), addressed_default],
        &addresses
    ));
    let ipv6_only_default = SysmonNetworkInterface {
        name: "eth0".to_string(),
        addresses: vec!["2001:db8::77/64".to_string()],
        ..docker.clone()
    };
    assert!(remote_linux_kernel_addresses_need_merge(
        &[docker.clone(), ipv6_only_default],
        &addresses
    ));
    let prefixed_default = SysmonNetworkInterface {
        name: "eth0".to_string(),
        addresses: vec!["192.0.2.77/24".to_string()],
        ..docker.clone()
    };
    assert!(!remote_linux_kernel_addresses_need_merge(
        &[docker.clone(), prefixed_default],
        &addresses
    ));
    assert!(remote_linux_kernel_addresses_need_merge(
        &[docker],
        &BTreeMap::from([("kernel".to_string(), vec!["192.0.2.88".to_string()])])
    ));
    assert_eq!(
        parse_linux_default_route_interface(
            "Iface\tDestination\tGateway \tFlags\neth0\t00000000\t00000000\t0001\n"
        ),
        Some("eth0".to_string())
    );
    assert_eq!(
        parse_linux_default_route_interface(
            "Iface\tDestination\tGateway \tFlags\nlo\t00000000\t00000000\t0001\n"
        ),
        None
    );
}

#[test]
fn linux_kernel_address_fallback_fills_partial_native_enumeration() {
    let ipv4 = r#"Main:
  +-- 0.0.0.0/0 3 0 4
 |-- 0.0.0.0
    /0 universe UNICAST
 +-- 192.0.2.0/24 2 0 1
    |-- 192.0.2.0
       /24 link UNICAST
    |-- 192.0.2.77
       /32 host LOCAL
    |-- 192.0.2.255
       /32 link BROADCAST
"#;
    let route = "Iface\tDestination\tGateway \tFlags\neth0\t00000000\t00000000\t0003\n";
    let mut native = BTreeMap::from([
        ("lo".to_string(), vec!["127.0.0.1/8".to_string()]),
        ("eth0".to_string(), vec!["198.51.100.12/24".to_string()]),
    ]);

    let fallback = collect_linux_kernel_network_addresses("", ipv4, route, "");
    merge_linux_network_address_maps(&mut native, fallback.clone());
    merge_linux_network_address_maps(&mut native, fallback);

    assert_eq!(
        native.get("eth0").cloned().unwrap_or_default(),
        vec!["198.51.100.12/24", "192.0.2.77"]
    );
}

#[test]
fn sysmon_commit_failure_rolls_back_snapshot_and_success_event() {
    let root =
        std::env::temp_dir().join(format!("portmate-sysmon-commit-failure-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let blocked_parent = root.join("not-a-directory");
    fs::write(&blocked_parent, b"blocked").unwrap();
    let profile = test_shell_profile();
    let state = test_app_state(
        profile.clone(),
        blocked_parent.join("portmate-store.sqlite3"),
    );

    let error =
        commit_sysmon_snapshot(&state, &profile.id, test_sysmon_snapshot(&profile.id)).unwrap_err();

    assert!(error.contains("无法判定 Store 提交是否生效"), "{error}");
    let store = state.store.lock().unwrap();
    assert!(store.sysmon.is_empty());
    assert!(store.events.is_empty());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn sysmon_commit_rejects_a_profile_deleted_during_collection() {
    let profile = test_shell_profile();
    let state = test_app_state(
        profile.clone(),
        PathBuf::from("sysmon-deleted-profile-test.sqlite3"),
    );
    {
        let mut store = state.store.lock().unwrap();
        store.profiles.clear();
        store.runtimes.clear();
    }

    let error =
        commit_sysmon_snapshot(&state, &profile.id, test_sysmon_snapshot(&profile.id)).unwrap_err();

    assert!(error.contains("unknown session"));
    let store = state.store.lock().unwrap();
    assert!(store.sysmon.is_empty());
    assert!(store.events.is_empty());
}

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

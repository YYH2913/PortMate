use super::*;

pub(super) fn remote_linux_sysmon_needs_network_address_fallback(
    interfaces: &[SysmonNetworkInterface],
) -> bool {
    interfaces.is_empty()
        || !interfaces.iter().any(|interface| {
            interface.name != "lo"
                && interface
                    .addresses
                    .iter()
                    .any(|address| is_usable_sysmon_network_address(address))
        })
}

pub(super) fn parse_remote_linux_kernel_network_addresses(
    output: &str,
) -> BTreeMap<String, Vec<String>> {
    let ipv6 = section_between(
        output,
        "__PORTMATE_KERNEL_IPV6__",
        "__PORTMATE_KERNEL_IPV4__",
    );
    let ipv4_end = if output.contains("__PORTMATE_KERNEL_ROUTE__") {
        "__PORTMATE_KERNEL_ROUTE__"
    } else if output.contains("__PORTMATE_HOSTNAME_ADDRS__") {
        "__PORTMATE_HOSTNAME_ADDRS__"
    } else {
        "__PORTMATE_LOADAVG__"
    };
    let ipv4 = section_between(output, "__PORTMATE_KERNEL_IPV4__", ipv4_end);
    let route = section_between(
        output,
        "__PORTMATE_KERNEL_ROUTE__",
        "__PORTMATE_HOSTNAME_ADDRS__",
    );
    let hostname = section_between(
        output,
        "__PORTMATE_HOSTNAME_ADDRS__",
        "__PORTMATE_LOADAVG__",
    );
    collect_linux_kernel_network_addresses(ipv6, ipv4, route, hostname)
}

pub(super) fn collect_linux_kernel_network_addresses(
    ipv6: &str,
    ipv4: &str,
    route: &str,
    hostname: &str,
) -> BTreeMap<String, Vec<String>> {
    let mut addresses = parse_linux_if_inet6_addresses(ipv6);
    let mut kernel_addresses = parse_linux_fib_trie_local_addresses(ipv4);
    kernel_addresses.extend(parse_linux_hostname_network_addresses(hostname));
    let kernel_addresses = normalize_sysmon_addresses(kernel_addresses);
    if !kernel_addresses.is_empty() {
        if let Some(interface) = parse_linux_default_route_interface(route) {
            let entry = addresses.entry(interface).or_default();
            let mut combined = std::mem::take(entry);
            combined.extend(kernel_addresses);
            *entry = normalize_sysmon_addresses(combined);
        } else {
            addresses.insert("kernel".to_string(), kernel_addresses);
        }
    }
    addresses
}

pub(super) fn parse_linux_default_route_interface(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 4 || !fields[1].eq_ignore_ascii_case("00000000") {
            continue;
        }
        let Ok(flags) = u32::from_str_radix(fields[3], 16) else {
            continue;
        };
        if flags & 0x1 == 0 {
            continue;
        }
        let interface = normalize_linux_sysmon_interface_name(fields[0]);
        if !interface.is_empty() && interface != "lo" {
            return Some(interface);
        }
    }
    None
}

pub(super) fn parse_linux_hostname_network_addresses(raw: &str) -> Vec<String> {
    let mut addresses = Vec::new();
    for value in raw.split_whitespace() {
        let Some(address) = normalize_sysmon_address(value) else {
            continue;
        };
        if !is_usable_sysmon_network_address(&address) {
            continue;
        }
        push_sysmon_network_address(&mut addresses, address);
    }
    addresses
}

pub(super) fn parse_linux_if_inet6_addresses(raw: &str) -> BTreeMap<String, Vec<String>> {
    let mut addresses = BTreeMap::<String, Vec<String>>::new();
    for line in raw.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 6 || fields[0].len() != 32 {
            continue;
        }
        let Some(prefix) = u8::from_str_radix(fields[2], 16)
            .ok()
            .filter(|prefix| *prefix <= 128)
        else {
            continue;
        };
        let Some(value) = u128::from_str_radix(fields[0], 16).ok() else {
            continue;
        };
        let name = normalize_linux_sysmon_interface_name(fields[5]);
        if name.is_empty() {
            continue;
        }
        let address = format!(
            "{}/{}",
            std::net::Ipv6Addr::from(value.to_be_bytes()),
            prefix
        );
        if !is_usable_sysmon_network_address(&address) {
            continue;
        }
        push_sysmon_network_address(addresses.entry(name).or_default(), address);
    }
    addresses
}

pub(super) fn parse_linux_fib_trie_local_addresses(raw: &str) -> Vec<String> {
    let lines = raw.lines().collect::<Vec<_>>();
    let mut addresses = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let candidate = line
            .trim()
            .strip_prefix("|--")
            .or_else(|| line.trim().strip_prefix("+--"))
            .map(str::trim)
            .and_then(|value| value.split_whitespace().next());
        let Some(candidate) = candidate else {
            continue;
        };
        if candidate.parse::<std::net::Ipv4Addr>().is_err()
            || !lines.iter().skip(index + 1).take(2).any(|next| {
                let next = next.trim();
                next.starts_with('/') && next.contains("host LOCAL")
            })
            || !is_usable_sysmon_network_address(candidate)
        {
            continue;
        }
        push_sysmon_network_address(&mut addresses, candidate.to_string());
    }
    addresses
}

pub(super) fn remote_linux_kernel_addresses_need_merge(
    interfaces: &[SysmonNetworkInterface],
    addresses: &BTreeMap<String, Vec<String>>,
) -> bool {
    addresses.iter().any(|(name, candidates)| {
        if name == "lo" {
            return false;
        }
        candidates.iter().any(|candidate| {
            is_usable_sysmon_network_address(candidate)
                && !interfaces.iter().any(|interface| {
                    (name == "kernel" || interface.name == *name)
                        && interface
                            .addresses
                            .iter()
                            .any(|existing| same_sysmon_network_address(existing, candidate))
                })
        })
    })
}

pub(super) fn merge_remote_linux_sysmon_network_addresses(
    interfaces: &mut Vec<SysmonNetworkInterface>,
    rx_kbps: &mut f32,
    tx_kbps: &mut f32,
    output: &str,
    addresses: BTreeMap<String, Vec<String>>,
) {
    if addresses.is_empty() {
        return;
    }
    let address_output = section_between(output, "__PORTMATE_ADDRS__", "__PORTMATE_LOADAVG__");
    let mut all_addresses = parse_linux_network_addresses(address_output);
    for (name, extra_addresses) in addresses {
        let entry = all_addresses.entry(name).or_default();
        let mut combined = std::mem::take(entry);
        combined.extend(extra_addresses);
        *entry = normalize_sysmon_addresses(combined);
    }
    let net1 = section_between(output, "__PORTMATE_NET1__", "__PORTMATE_STAT2__");
    let net2 = section_between(output, "__PORTMATE_NET2__", "__PORTMATE_ADDRS__");
    let mut refreshed = network_interface_rates(
        parse_network_interfaces(net1),
        parse_network_interfaces(net2),
        all_addresses.clone(),
        REMOTE_SYSMON_SAMPLE_SECONDS,
    );
    for (name, addresses) in all_addresses {
        if refreshed.iter().any(|interface| interface.name == name) {
            continue;
        }
        refreshed.push(SysmonNetworkInterface {
            name,
            addresses,
            rx_bytes: 0,
            tx_bytes: 0,
            rx_kbps: 0.0,
            tx_kbps: 0.0,
        });
    }
    sort_sysmon_network_interfaces(&mut refreshed);
    refreshed.truncate(MAX_SYSMON_NETWORK_INTERFACES);
    (*rx_kbps, *tx_kbps) = aggregate_network_rates(&refreshed);
    *interfaces = refreshed;
}

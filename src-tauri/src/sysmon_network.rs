use super::*;

pub(super) const MAX_SYSMON_NETWORK_INTERFACES: usize = 32;
pub(super) const MAX_SYSMON_NETWORK_ADDRESSES_PER_INTERFACE: usize = 8;
pub(super) const REMOTE_SYSMON_SAMPLE_SECONDS: f32 = 0.2;

pub(super) fn parse_network_interfaces(raw: &str) -> BTreeMap<String, (u64, u64)> {
    let mut interfaces = BTreeMap::new();
    for line in raw.lines().skip(2) {
        let Some((name, values)) = line.split_once(':') else {
            continue;
        };
        let parts = values.split_whitespace().collect::<Vec<_>>();
        if parts.len() >= 16 {
            let name = bounded_sysmon_label(name.trim(), 64);
            if name.is_empty() {
                continue;
            }
            let rx = parts[0].parse::<u64>().unwrap_or_default();
            let tx = parts[8].parse::<u64>().unwrap_or_default();
            interfaces.insert(name, (rx, tx));
        }
    }
    interfaces
}

pub(super) fn parse_linux_network_addresses(raw: &str) -> BTreeMap<String, Vec<String>> {
    let mut addresses = BTreeMap::<String, Vec<String>>::new();
    let mut current_interface: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let fields = trimmed.split_whitespace().collect::<Vec<_>>();
        if let Some(name) = linux_sysmon_interface_header_name(trimmed, &fields) {
            current_interface = Some(name);
        }

        let name = linux_sysmon_address_line_name(&fields).or_else(|| current_interface.clone());
        let address = fields
            .iter()
            .enumerate()
            .find_map(|(index, field)| {
                if !(*field == "inet" || *field == "inet6") || index + 1 >= fields.len() {
                    return None;
                }
                let next = fields[index + 1];
                if next == "addr:" {
                    fields.get(index + 2).copied()
                } else {
                    Some(next)
                }
            })
            .or_else(|| {
                fields
                    .iter()
                    .position(|field| *field == "addr")
                    .and_then(|index| fields.get(index + 1).copied())
            })
            .or_else(|| {
                fields.iter().find_map(|field| {
                    field
                        .strip_prefix("addr:")
                        .filter(|value| !value.is_empty())
                })
            });
        let (Some(name), Some(address)) = (name, address) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let Some(address) = normalize_sysmon_address(address) else {
            continue;
        };
        push_sysmon_network_address(addresses.entry(name).or_default(), address);
    }
    addresses
}

pub(super) fn parse_openwrt_network_interface_dump(raw: &str) -> BTreeMap<String, Vec<String>> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(raw) else {
        return BTreeMap::new();
    };
    let Some(interfaces) = root.get("interface").and_then(serde_json::Value::as_array) else {
        return BTreeMap::new();
    };

    let mut addresses = BTreeMap::<String, Vec<String>>::new();
    for interface in interfaces {
        let Some(name) = ["l3_device", "device", "interface"]
            .into_iter()
            .filter_map(|field| interface.get(field).and_then(serde_json::Value::as_str))
            .map(normalize_linux_sysmon_interface_name)
            .find(|name| !name.is_empty())
        else {
            continue;
        };

        let entry = addresses.entry(name).or_default();
        for (field, max_prefix) in [("ipv4-address", 32_u8), ("ipv6-address", 128_u8)] {
            let Some(items) = interface.get(field).and_then(serde_json::Value::as_array) else {
                continue;
            };
            for item in items {
                let Some(address) = openwrt_sysmon_interface_address(item, max_prefix) else {
                    continue;
                };
                push_sysmon_network_address(entry, address);
            }
        }
    }
    addresses
}

pub(super) fn openwrt_sysmon_interface_address(
    value: &serde_json::Value,
    max_prefix: u8,
) -> Option<String> {
    let raw_address = value
        .as_str()
        .or_else(|| value.get("address").and_then(serde_json::Value::as_str))?;
    let address = normalize_sysmon_address(raw_address)?;
    if address.contains(':') != (max_prefix == 128) {
        return None;
    }
    if address.contains('/') {
        return Some(address);
    }
    let prefix = value
        .get("mask")
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
        .filter(|prefix| *prefix <= max_prefix);
    match prefix {
        Some(prefix) => Some(format!("{address}/{prefix}")),
        None => Some(address),
    }
}

pub(super) fn linux_sysmon_interface_header_name(trimmed: &str, fields: &[&str]) -> Option<String> {
    if let Some((index, rest)) = trimmed.split_once(": ") {
        if index.chars().all(|character| character.is_ascii_digit()) {
            let name = rest.split_whitespace().next()?;
            let name = normalize_linux_sysmon_interface_name(name);
            return (!name.is_empty() && !matches!(name.as_str(), "inet" | "inet6" | "link"))
                .then_some(name);
        }
    }

    let first = fields.first()?;
    let rest = fields.get(1).copied().unwrap_or_default();
    let name = normalize_linux_sysmon_interface_name(first);
    (!name.is_empty()
        && (rest.starts_with("flags=") || rest == "Link" || rest == "inet" || rest == "inet6"))
        .then_some(name)
}

pub(super) fn linux_sysmon_address_line_name(fields: &[&str]) -> Option<String> {
    if fields.len() >= 4
        && fields[0]
            .trim_end_matches(':')
            .chars()
            .all(|character| character.is_ascii_digit())
        && (fields[2] == "inet" || fields[2] == "inet6")
    {
        let name = normalize_linux_sysmon_interface_name(fields[1]);
        return (!name.is_empty()).then_some(name);
    }

    let first = fields.first().copied()?;
    if first == "inet" || first == "inet6" {
        return None;
    }
    let has_address = fields
        .iter()
        .any(|field| *field == "inet" || *field == "inet6");
    if !has_address {
        return None;
    }
    let name = normalize_linux_sysmon_interface_name(first);
    (!name.is_empty() && !matches!(name.as_str(), "inet" | "inet6" | "link")).then_some(name)
}

pub(super) fn normalize_linux_sysmon_interface_name(value: &str) -> String {
    let name = value
        .trim()
        .trim_end_matches(':')
        .split('@')
        .next()
        .unwrap_or("");
    bounded_sysmon_label(name, 64)
}

pub(super) fn parse_bsd_network_interfaces(raw: &str) -> BTreeMap<String, (u64, u64)> {
    let lines = raw.lines().collect::<Vec<_>>();
    let Some((header_index, header)) = lines.iter().enumerate().find_map(|(index, line)| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        (fields.contains(&"Name") && fields.contains(&"Ibytes") && fields.contains(&"Obytes"))
            .then_some((index, fields))
    }) else {
        return BTreeMap::new();
    };
    let Some(name_index) = header.iter().position(|field| *field == "Name") else {
        return BTreeMap::new();
    };
    let Some(rx_index) = header.iter().position(|field| *field == "Ibytes") else {
        return BTreeMap::new();
    };
    let Some(tx_index) = header.iter().position(|field| *field == "Obytes") else {
        return BTreeMap::new();
    };
    let minimum_fields = name_index.max(rx_index).max(tx_index) + 1;
    let mut interfaces = BTreeMap::new();
    for line in lines.into_iter().skip(header_index + 1) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < minimum_fields {
            continue;
        }
        let name = bounded_sysmon_label(fields[name_index].trim_end_matches('*'), 64);
        let Some(rx_bytes) = fields[rx_index].parse::<u64>().ok() else {
            continue;
        };
        let Some(tx_bytes) = fields[tx_index].parse::<u64>().ok() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        interfaces
            .entry(name)
            .and_modify(|(rx, tx): &mut (u64, u64)| {
                *rx = (*rx).max(rx_bytes);
                *tx = (*tx).max(tx_bytes);
            })
            .or_insert((rx_bytes, tx_bytes));
    }
    interfaces
}

pub(super) fn parse_bsd_network_addresses(raw: &str) -> BTreeMap<String, Vec<String>> {
    let lines = raw.lines().collect::<Vec<_>>();
    let Some((header_index, header)) = lines.iter().enumerate().find_map(|(index, line)| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        (fields.contains(&"Name") && fields.contains(&"Address")).then_some((index, fields))
    }) else {
        return BTreeMap::new();
    };
    let Some(name_index) = header.iter().position(|field| *field == "Name") else {
        return BTreeMap::new();
    };
    let Some(address_index) = header.iter().position(|field| *field == "Address") else {
        return BTreeMap::new();
    };
    let minimum_fields = name_index.max(address_index) + 1;
    let mut addresses = BTreeMap::<String, Vec<String>>::new();
    for line in lines.into_iter().skip(header_index + 1) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < minimum_fields {
            continue;
        }
        let name = bounded_sysmon_label(fields[name_index].trim_end_matches('*'), 64);
        let Some(address) = normalize_sysmon_address(fields[address_index]) else {
            continue;
        };
        push_sysmon_network_address(addresses.entry(name).or_default(), address);
    }
    addresses
}

pub(super) fn sort_sysmon_network_interfaces(interfaces: &mut [SysmonNetworkInterface]) {
    interfaces.sort_by(|left, right| {
        left.addresses
            .is_empty()
            .cmp(&right.addresses.is_empty())
            .then_with(|| {
                let left_rate = left.rx_kbps + left.tx_kbps;
                let right_rate = right.rx_kbps + right.tx_kbps;
                right_rate.total_cmp(&left_rate)
            })
            .then_with(|| left.name.cmp(&right.name))
    });
}

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

pub(super) fn is_usable_sysmon_network_address(value: &str) -> bool {
    let address = value.split_once('/').map_or(value, |(address, _)| address);
    match address.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(address)) => {
            !address.is_loopback() && !address.is_unspecified() && !address.is_link_local()
        }
        Ok(std::net::IpAddr::V6(address)) => {
            !address.is_loopback() && !address.is_unspecified() && !address.is_unicast_link_local()
        }
        Err(_) => false,
    }
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

pub(super) fn same_sysmon_network_address(existing: &str, candidate: &str) -> bool {
    sysmon_network_address_host(existing) == sysmon_network_address_host(candidate)
}

pub(super) fn sysmon_network_address_host(value: &str) -> &str {
    value.split_once('/').map_or(value, |(address, _)| address)
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

pub(super) fn network_interface_rates(
    before: BTreeMap<String, (u64, u64)>,
    after: BTreeMap<String, (u64, u64)>,
    addresses: BTreeMap<String, Vec<String>>,
    seconds: f32,
) -> Vec<SysmonNetworkInterface> {
    let seconds = if seconds.is_finite() && seconds > 0.0 {
        seconds
    } else {
        1.0
    };
    let mut interfaces = after
        .into_iter()
        .map(|(name, (rx_bytes, tx_bytes))| {
            let (rx_before, tx_before) = before.get(&name).copied().unwrap_or((rx_bytes, tx_bytes));
            let interface_addresses = addresses.get(&name).cloned().unwrap_or_default();
            SysmonNetworkInterface {
                name,
                addresses: interface_addresses,
                rx_bytes,
                tx_bytes,
                rx_kbps: rx_bytes.saturating_sub(rx_before) as f32 / 1024.0 / seconds,
                tx_kbps: tx_bytes.saturating_sub(tx_before) as f32 / 1024.0 / seconds,
            }
        })
        .collect::<Vec<_>>();
    for (name, addresses) in addresses {
        if addresses.is_empty() || interfaces.iter().any(|interface| interface.name == name) {
            continue;
        }
        interfaces.push(SysmonNetworkInterface {
            name,
            addresses,
            rx_bytes: 0,
            tx_bytes: 0,
            rx_kbps: 0.0,
            tx_kbps: 0.0,
        });
    }
    sort_sysmon_network_interfaces(&mut interfaces);
    interfaces.truncate(MAX_SYSMON_NETWORK_INTERFACES);
    interfaces
}

pub(super) fn normalize_sysmon_addresses(addresses: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for address in addresses {
        let Some(address) = normalize_sysmon_address(&address) else {
            continue;
        };
        push_sysmon_network_address(&mut normalized, address);
    }
    normalized
}

pub(super) fn push_sysmon_network_address(addresses: &mut Vec<String>, candidate: String) {
    if addresses
        .iter()
        .any(|existing| same_sysmon_network_address(existing, &candidate))
    {
        return;
    }
    addresses.push(candidate);
    addresses.sort_by_key(|address| sysmon_network_address_priority(address));
    addresses.truncate(MAX_SYSMON_NETWORK_ADDRESSES_PER_INTERFACE);
}

pub(super) fn sysmon_network_address_priority(value: &str) -> u8 {
    let address = sysmon_network_address_host(value);
    match address.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(address)) => {
            if !address.is_loopback() && !address.is_unspecified() && !address.is_link_local() {
                0
            } else if address.is_link_local() {
                2
            } else {
                4
            }
        }
        Ok(std::net::IpAddr::V6(address)) => {
            if !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_unicast_link_local()
            {
                1
            } else if address.is_unicast_link_local() {
                3
            } else {
                5
            }
        }
        Err(_) => 6,
    }
}

pub(super) fn normalize_sysmon_address(value: &str) -> Option<String> {
    let value = value
        .trim()
        .strip_prefix("addr:")
        .unwrap_or(value.trim())
        .trim_matches(|character| matches!(character, ',' | ';' | '(' | ')' | '[' | ']'));
    if value.is_empty()
        || value.starts_with('<')
        || value.contains("::")
            && value
                .chars()
                .all(|character| character == ':' || character == '0')
    {
        return None;
    }
    let address = value.split('%').next().unwrap_or(value);
    let address = bounded_sysmon_label(address, 96);
    if is_mac_like_address(&address) {
        return None;
    }
    if address.is_empty()
        || !address
            .chars()
            .all(|character| character.is_ascii_hexdigit() || matches!(character, '.' | ':' | '/'))
    {
        return None;
    }
    Some(address)
}

pub(super) fn is_mac_like_address(value: &str) -> bool {
    let parts = value.split(':').collect::<Vec<_>>();
    parts.len() == 6
        && parts.iter().all(|part| {
            part.len() == 2 && part.chars().all(|character| character.is_ascii_hexdigit())
        })
}

pub(super) fn aggregate_network_rates(interfaces: &[SysmonNetworkInterface]) -> (f32, f32) {
    interfaces.iter().fold((0.0, 0.0), |(rx, tx), interface| {
        (rx + interface.rx_kbps, tx + interface.tx_kbps)
    })
}

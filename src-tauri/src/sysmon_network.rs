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

pub(super) fn same_sysmon_network_address(existing: &str, candidate: &str) -> bool {
    sysmon_network_address_host(existing) == sysmon_network_address_host(candidate)
}

pub(super) fn sysmon_network_address_host(value: &str) -> &str {
    value.split_once('/').map_or(value, |(address, _)| address)
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

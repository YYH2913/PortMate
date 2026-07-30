use super::*;

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

use crate::models::{TunnelEgress, TunnelMode, TunnelRouteRule, TunnelSpec};
use ipnet::IpNet;
use std::collections::HashSet;
use std::net::IpAddr;

pub const MAX_TUNNELS_PER_PROFILE: usize = 64;
pub const MAX_TUNNEL_ID_CHARACTERS: usize = 128;
pub const MAX_TUNNEL_LABEL_CHARACTERS: usize = 128;
pub const MAX_TUNNEL_HOST_CHARACTERS: usize = 255;
pub const MAX_TUNNEL_ROUTE_RULES: usize = 64;

pub fn validate_tunnels(tunnels: &[TunnelSpec]) -> Result<(), String> {
    if tunnels.len() > MAX_TUNNELS_PER_PROFILE {
        return Err(format!("tunnel count exceeds {MAX_TUNNELS_PER_PROFILE}"));
    }
    let mut ids = HashSet::with_capacity(tunnels.len());
    for (index, tunnel) in tunnels.iter().enumerate() {
        validate_tunnel(tunnel).map_err(|error| format!("tunnel {}: {error}", index + 1))?;
        if !ids.insert(tunnel.id.as_str()) {
            return Err(format!("tunnel {}: duplicate id", index + 1));
        }
    }
    Ok(())
}

pub fn normalize_tunnels(tunnels: Vec<TunnelSpec>) -> Vec<TunnelSpec> {
    let mut normalized = Vec::with_capacity(tunnels.len().min(MAX_TUNNELS_PER_PROFILE));
    let mut ids = HashSet::with_capacity(normalized.capacity());
    for enabled in [true, false] {
        for tunnel in tunnels.iter().filter(|tunnel| tunnel.enabled == enabled) {
            if normalized.len() >= MAX_TUNNELS_PER_PROFILE {
                return normalized;
            }
            let tunnel = normalize_tunnel(tunnel.clone());
            if validate_tunnel(&tunnel).is_ok() && ids.insert(tunnel.id.clone()) {
                normalized.push(tunnel);
            }
        }
    }
    normalized
}

fn normalize_tunnel(mut tunnel: TunnelSpec) -> TunnelSpec {
    tunnel.id = tunnel.id.trim().to_string();
    tunnel.label = tunnel.label.trim().to_string();
    tunnel.bind_host = tunnel.bind_host.trim().to_string();
    tunnel.target_host = tunnel.target_host.trim().to_string();
    if tunnel.mode == TunnelMode::Dynamic {
        tunnel.target_host.clear();
        tunnel.target_port = 0;
        tunnel.route_rules = normalize_tunnel_route_rules(std::mem::take(&mut tunnel.route_rules));
    } else {
        tunnel.route_rules.clear();
    }
    tunnel
}

fn validate_tunnel(tunnel: &TunnelSpec) -> Result<(), String> {
    validate_text("id", &tunnel.id, MAX_TUNNEL_ID_CHARACTERS, false, false)?;
    validate_text(
        "label",
        &tunnel.label,
        MAX_TUNNEL_LABEL_CHARACTERS,
        false,
        false,
    )?;
    validate_host(
        "bind host",
        &tunnel.bind_host,
        tunnel.mode == TunnelMode::Remote,
    )?;
    if tunnel.egress == TunnelEgress::PortmateHost && tunnel.mode == TunnelMode::Remote {
        return Err("PortMate host egress does not support remote SSH forwarding".to_string());
    }
    match tunnel.mode {
        TunnelMode::Dynamic => {
            if !tunnel.target_host.is_empty() || tunnel.target_port != 0 {
                return Err("dynamic tunnel must not have a target".to_string());
            }
            validate_tunnel_route_rules(&tunnel.route_rules)?;
            if tunnel.egress == TunnelEgress::PortmateHost && tunnel.route_rules.is_empty() {
                return Err(
                    "PortMate host SOCKS5 proxies require at least one route rule".to_string(),
                );
            }
        }
        TunnelMode::Local | TunnelMode::Remote => {
            if !tunnel.route_rules.is_empty() {
                return Err("route rules are only supported by dynamic tunnels".to_string());
            }
            validate_host("target host", &tunnel.target_host, false)?;
            if tunnel.target_port == 0 {
                return Err("target port must be between 1 and 65535".to_string());
            }
        }
    }
    Ok(())
}

pub fn normalize_tunnel_route_rules(rules: Vec<TunnelRouteRule>) -> Vec<TunnelRouteRule> {
    rules
        .into_iter()
        .map(|mut rule| {
            // Keep unsafe input intact so validation can reject it instead of
            // silently trimming a leading or trailing control character.
            if !rule.host.chars().any(char::is_control) {
                rule.host = normalized_route_host(&rule.host);
            }
            rule
        })
        .collect()
}

fn normalized_route_host(value: &str) -> String {
    let value = value.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Ok(network) = value.parse::<IpNet>() {
        network.trunc().to_string()
    } else if let Ok(address) = value.parse::<IpAddr>() {
        address.to_string()
    } else {
        value
    }
}

pub fn validate_tunnel_route_rules(rules: &[TunnelRouteRule]) -> Result<(), String> {
    if rules.len() > MAX_TUNNEL_ROUTE_RULES {
        return Err(format!("route rule count exceeds {MAX_TUNNEL_ROUTE_RULES}"));
    }
    let mut seen = HashSet::with_capacity(rules.len());
    for (index, rule) in rules.iter().enumerate() {
        validate_route_rule(rule).map_err(|error| format!("route rule {}: {error}", index + 1))?;
        if !seen.insert((rule.host.as_str(), rule.port)) {
            return Err(format!("route rule {}: duplicate rule", index + 1));
        }
    }
    Ok(())
}

fn validate_route_rule(rule: &TunnelRouteRule) -> Result<(), String> {
    validate_host("host", &rule.host, false)?;
    if normalized_route_host(&rule.host) != rule.host {
        return Err(
            "host must be normalized without surrounding whitespace or a trailing dot".to_string(),
        );
    }
    if rule.port == Some(0) {
        return Err("port must be between 1 and 65535".to_string());
    }
    if rule.host.starts_with("*.") {
        let suffix = &rule.host[2..];
        if !valid_dns_name(suffix) {
            return Err("wildcard host must be a valid *.example.com suffix".to_string());
        }
        return Ok(());
    }
    if rule.host.contains('/') {
        let network = rule
            .host
            .parse::<IpNet>()
            .map_err(|_| "CIDR host must be a valid IPv4 or IPv6 network".to_string())?;
        if network.trunc().to_string() != rule.host {
            return Err("CIDR host must use its canonical network address".to_string());
        }
        return Ok(());
    }
    if rule.host.parse::<IpAddr>().is_ok() || valid_dns_name(&rule.host) {
        return Ok(());
    }
    Err("host must be a domain, wildcard domain, IP address, or CIDR".to_string())
}

fn valid_dns_name(value: &str) -> bool {
    if value.is_empty() || value.len() > 253 || value.starts_with('.') || value.ends_with('.') {
        return false;
    }
    value.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

pub fn tunnel_route_allowed(
    rules: &[TunnelRouteRule],
    target_host: &str,
    target_port: u16,
) -> bool {
    if rules.is_empty() {
        return true;
    }
    let target_host = target_host
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let target_ip = target_host.parse::<IpAddr>().ok();
    rules.iter().any(|rule| {
        if rule.port.is_some_and(|port| port != target_port) {
            return false;
        }
        if let Ok(network) = rule.host.parse::<IpNet>() {
            return target_ip.is_some_and(|address| network.contains(&address));
        }
        if let Some(suffix) = rule.host.strip_prefix("*.") {
            return target_host.len() > suffix.len()
                && target_host.ends_with(suffix)
                && target_host.as_bytes()[target_host.len() - suffix.len() - 1] == b'.';
        }
        rule.host == target_host
    })
}

fn validate_host(label: &str, value: &str, allow_empty: bool) -> Result<(), String> {
    validate_text(label, value, MAX_TUNNEL_HOST_CHARACTERS, allow_empty, true)
}

fn validate_text(
    label: &str,
    value: &str,
    max_characters: usize,
    allow_empty: bool,
    reject_whitespace: bool,
) -> Result<(), String> {
    if value.trim() != value {
        return Err(format!("{label} must not have surrounding whitespace"));
    }
    if !allow_empty && value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    let mut count = 0_usize;
    for character in value.chars() {
        count = count.saturating_add(1);
        if count > max_characters {
            return Err(format!(
                "{label} exceeds {max_characters} Unicode characters"
            ));
        }
        if character.is_control() {
            return Err(format!("{label} must not contain control characters"));
        }
        if reject_whitespace && character.is_whitespace() {
            return Err(format!("{label} must not contain whitespace"));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../test/rust/portmate-core/unit/tunnels.rs"]
mod tests;

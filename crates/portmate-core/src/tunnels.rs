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
mod tests {
    use super::*;

    fn test_tunnel(id: impl Into<String>, mode: TunnelMode, enabled: bool) -> TunnelSpec {
        TunnelSpec {
            id: id.into(),
            label: "Tunnel".to_string(),
            egress: TunnelEgress::Ssh,
            mode,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_022,
            target_host: if mode == TunnelMode::Dynamic {
                String::new()
            } else {
                "device.internal".to_string()
            },
            target_port: if mode == TunnelMode::Dynamic { 0 } else { 22 },
            route_rules: Vec::new(),
            enabled,
        }
    }

    #[test]
    fn validates_counts_ids_fields_and_mode_shape() {
        let local = test_tunnel("local", TunnelMode::Local, true);
        validate_tunnels(std::slice::from_ref(&local)).unwrap();

        assert!(
            validate_tunnels(&vec![local.clone(); MAX_TUNNELS_PER_PROFILE + 1])
                .unwrap_err()
                .contains("count exceeds")
        );
        assert!(validate_tunnels(&[local.clone(), local.clone()])
            .unwrap_err()
            .contains("duplicate id"));

        let mut invalid_host = local.clone();
        invalid_host.target_host = "bad host".to_string();
        assert!(validate_tunnels(&[invalid_host])
            .unwrap_err()
            .contains("must not contain whitespace"));

        let mut invalid_label = local.clone();
        invalid_label.label = "x".repeat(MAX_TUNNEL_LABEL_CHARACTERS + 1);
        assert!(validate_tunnels(&[invalid_label])
            .unwrap_err()
            .contains("label exceeds"));

        let mut invalid_dynamic = test_tunnel("dynamic", TunnelMode::Dynamic, true);
        invalid_dynamic.target_host = "ignored.invalid".to_string();
        invalid_dynamic.target_port = 443;
        assert!(validate_tunnels(&[invalid_dynamic])
            .unwrap_err()
            .contains("must not have a target"));
    }

    #[test]
    fn loaded_normalization_prefers_enabled_valid_unique_tunnels() {
        let mut invalid = test_tunnel("invalid", TunnelMode::Local, true);
        invalid.bind_host = "bad\nhost".to_string();
        let tunnels = std::iter::once(test_tunnel("duplicate", TunnelMode::Local, false))
            .chain(std::iter::once(invalid))
            .chain(
                (0..MAX_TUNNELS_PER_PROFILE)
                    .map(|index| test_tunnel(format!("enabled-{index}"), TunnelMode::Local, true)),
            )
            .chain(std::iter::once(test_tunnel(
                "duplicate",
                TunnelMode::Remote,
                true,
            )))
            .collect();

        let normalized = normalize_tunnels(tunnels);
        assert_eq!(normalized.len(), MAX_TUNNELS_PER_PROFILE);
        assert!(normalized.iter().all(|tunnel| tunnel.enabled));
        assert!(normalized.iter().all(|tunnel| tunnel.id != "invalid"));
        assert_eq!(
            normalized
                .iter()
                .map(|tunnel| tunnel.id.as_str())
                .collect::<HashSet<_>>()
                .len(),
            normalized.len()
        );
    }

    #[test]
    fn normalizes_and_matches_dynamic_route_rules() {
        let mut dynamic = test_tunnel("dynamic", TunnelMode::Dynamic, true);
        dynamic.route_rules = vec![
            TunnelRouteRule {
                host: " *.Example.COM. ".to_string(),
                port: Some(443),
            },
            TunnelRouteRule {
                host: "10.9.8.7/8".to_string(),
                port: None,
            },
            TunnelRouteRule {
                host: "2001:0DB8::1/32".to_string(),
                port: Some(22),
            },
        ];
        let dynamic = normalize_tunnels(vec![dynamic]).remove(0);
        assert_eq!(dynamic.route_rules[0].host, "*.example.com");
        assert_eq!(dynamic.route_rules[1].host, "10.0.0.0/8");
        assert_eq!(dynamic.route_rules[2].host, "2001:db8::/32");
        validate_tunnels(std::slice::from_ref(&dynamic)).unwrap();

        assert!(tunnel_route_allowed(
            &dynamic.route_rules,
            "api.example.com",
            443
        ));
        assert!(!tunnel_route_allowed(
            &dynamic.route_rules,
            "example.com",
            443
        ));
        assert!(!tunnel_route_allowed(
            &dynamic.route_rules,
            "api.example.com",
            80
        ));
        assert!(tunnel_route_allowed(
            &dynamic.route_rules,
            "10.20.30.40",
            8080
        ));
        assert!(tunnel_route_allowed(
            &dynamic.route_rules,
            "2001:db8::9",
            22
        ));
        assert!(!tunnel_route_allowed(
            &dynamic.route_rules,
            "2001:db8::9",
            23
        ));
        assert!(!tunnel_route_allowed(
            &dynamic.route_rules,
            "192.168.1.1",
            443
        ));
        assert!(tunnel_route_allowed(&[], "anything.invalid", 1));
    }

    #[test]
    fn rejects_invalid_or_wrong_mode_route_rules() {
        let mut local = test_tunnel("local", TunnelMode::Local, true);
        local.route_rules = vec![TunnelRouteRule {
            host: "example.com".to_string(),
            port: None,
        }];
        assert!(validate_tunnels(&[local])
            .unwrap_err()
            .contains("only supported by dynamic"));

        for host in [
            "*",
            "*.bad_domain",
            "10.0.0.1/999",
            "bad..host",
            "\nexample.com",
        ] {
            let mut dynamic = test_tunnel("dynamic", TunnelMode::Dynamic, true);
            dynamic.route_rules = vec![TunnelRouteRule {
                host: host.to_string(),
                port: None,
            }];
            assert!(normalize_tunnels(vec![dynamic]).is_empty(), "{host}");
        }

        let duplicate = normalize_tunnel_route_rules(vec![
            TunnelRouteRule {
                host: "Example.COM.".to_string(),
                port: Some(443),
            },
            TunnelRouteRule {
                host: "example.com".to_string(),
                port: Some(443),
            },
        ]);
        assert!(validate_tunnel_route_rules(&duplicate)
            .unwrap_err()
            .contains("duplicate rule"));

        let too_many = vec![
            TunnelRouteRule {
                host: "example.com".to_string(),
                port: None,
            };
            MAX_TUNNEL_ROUTE_RULES + 1
        ];
        assert!(validate_tunnel_route_rules(&too_many)
            .unwrap_err()
            .contains("count exceeds"));
    }
}

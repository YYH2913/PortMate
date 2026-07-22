use crate::models::{TunnelMode, TunnelSpec};
use std::collections::HashSet;

pub const MAX_TUNNELS_PER_PROFILE: usize = 64;
pub const MAX_TUNNEL_ID_CHARACTERS: usize = 128;
pub const MAX_TUNNEL_LABEL_CHARACTERS: usize = 128;
pub const MAX_TUNNEL_HOST_CHARACTERS: usize = 255;

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
    match tunnel.mode {
        TunnelMode::Dynamic => {
            if !tunnel.target_host.is_empty() || tunnel.target_port != 0 {
                return Err("dynamic tunnel must not have a target".to_string());
            }
        }
        TunnelMode::Local | TunnelMode::Remote => {
            validate_host("target host", &tunnel.target_host, false)?;
            if tunnel.target_port == 0 {
                return Err("target port must be between 1 and 65535".to_string());
            }
        }
    }
    Ok(())
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
            mode,
            bind_host: "127.0.0.1".to_string(),
            bind_port: 10_022,
            target_host: if mode == TunnelMode::Dynamic {
                String::new()
            } else {
                "device.internal".to_string()
            },
            target_port: if mode == TunnelMode::Dynamic { 0 } else { 22 },
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
}

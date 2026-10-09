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

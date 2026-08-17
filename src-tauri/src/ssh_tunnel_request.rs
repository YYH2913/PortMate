use super::*;

pub(super) fn normalize_tunnel_request(
    mut request: CreateTunnelRequest,
) -> Result<CreateTunnelRequest, String> {
    for (label, value) in [
        ("session id", request.session_id.as_str()),
        ("bind host", request.bind_host.as_str()),
        ("target host", request.target_host.as_str()),
    ] {
        if value.chars().any(char::is_control) {
            return Err(format!(
                "tunnel {label} must not contain control characters"
            ));
        }
    }
    if request
        .label
        .as_deref()
        .is_some_and(|label| label.chars().any(char::is_control))
    {
        return Err("tunnel label must not contain control characters".to_string());
    }
    request.session_id = request.session_id.trim().to_string();
    request.bind_host = request.bind_host.trim().to_string();
    request.target_host = request.target_host.trim().to_string();
    if request.route_rules.len() > MAX_TUNNEL_ROUTE_RULES {
        return Err(format!(
            "tunnel route rule count exceeds {MAX_TUNNEL_ROUTE_RULES}"
        ));
    }
    for (index, rule) in request.route_rules.iter().enumerate() {
        if rule.host.chars().any(char::is_control) {
            return Err(format!(
                "tunnel route rule {} host must not contain control characters",
                index + 1
            ));
        }
    }
    request.route_rules = normalize_tunnel_route_rules(request.route_rules);
    request.label = request
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned);

    if request.mode == TunnelMode::Dynamic {
        request.target_host.clear();
        request.target_port = 0;
        validate_tunnel_route_rules(&request.route_rules)?;
    } else if !request.route_rules.is_empty() {
        return Err("tunnel route rules are only supported by dynamic mode".to_string());
    }
    match request.egress {
        TunnelEgress::Ssh => {
            if request.allow_remote_bind {
                return Err(
                    "allowRemoteBind is only valid for PortMate host egress".to_string(),
                );
            }
        }
        TunnelEgress::PortmateHost => {
            if request.mode == TunnelMode::Remote {
                return Err(
                    "PortMate host egress supports local TCP and dynamic SOCKS5 modes only"
                        .to_string(),
                );
            }
            if request.mode == TunnelMode::Dynamic && request.route_rules.is_empty() {
                return Err(
                    "PortMate host SOCKS5 proxies require at least one route rule".to_string(),
                );
            }
            if !request.allow_remote_bind && !is_loopback_tunnel_bind_host(&request.bind_host) {
                return Err(
                    "PortMate host egress requires a loopback bind host unless allowRemoteBind is true"
                        .to_string(),
                );
            }
        }
    }
    validate_tunnel_request_text(
        "session id",
        &request.session_id,
        MAX_SESSION_PROFILE_ID_CHARACTERS,
        false,
        false,
    )?;
    validate_tunnel_request_text(
        "bind host",
        &request.bind_host,
        MAX_TUNNEL_HOST_CHARACTERS,
        request.mode == TunnelMode::Remote,
        true,
    )?;
    if let Some(label) = request.label.as_deref() {
        validate_tunnel_request_text("label", label, MAX_TUNNEL_LABEL_CHARACTERS, false, false)?;
    }
    if request.mode != TunnelMode::Dynamic {
        if request.target_host.is_empty() || request.target_port == 0 {
            return Err("local and remote tunnels require a target host and port".to_string());
        }
        validate_tunnel_request_text(
            "target host",
            &request.target_host,
            MAX_TUNNEL_HOST_CHARACTERS,
            false,
            true,
        )?;
    }
    Ok(request)
}

pub(super) fn normalize_host_route_request(
    request: CreateHostRouteRequest,
) -> Result<CreateHostRouteRequest, String> {
    let normalized = normalize_tunnel_request(CreateTunnelRequest {
        session_id: "portmate-host".to_string(),
        egress: TunnelEgress::PortmateHost,
        mode: request.mode,
        bind_host: request.bind_host,
        bind_port: request.bind_port,
        target_host: request.target_host,
        target_port: request.target_port,
        route_rules: request.route_rules,
        allow_remote_bind: request.allow_remote_bind,
        label: request.label,
    })?;
    Ok(CreateHostRouteRequest {
        mode: normalized.mode,
        bind_host: normalized.bind_host,
        bind_port: normalized.bind_port,
        target_host: normalized.target_host,
        target_port: normalized.target_port,
        route_rules: normalized.route_rules,
        allow_remote_bind: normalized.allow_remote_bind,
        label: normalized.label,
    })
}

fn is_loopback_tunnel_bind_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

pub(super) fn validate_tunnel_request_text(
    label: &str,
    value: &str,
    max_characters: usize,
    allow_empty: bool,
    reject_whitespace: bool,
) -> Result<(), String> {
    if !allow_empty && value.is_empty() {
        return Err(format!("tunnel {label} must not be empty"));
    }
    let mut count = 0_usize;
    for character in value.chars() {
        count = count.saturating_add(1);
        if count > max_characters {
            return Err(format!(
                "tunnel {label} exceeds {max_characters} Unicode characters"
            ));
        }
        if character.is_control() {
            return Err(format!(
                "tunnel {label} must not contain control characters"
            ));
        }
        if reject_whitespace && character.is_whitespace() {
            return Err(format!("tunnel {label} must not contain whitespace"));
        }
    }
    Ok(())
}

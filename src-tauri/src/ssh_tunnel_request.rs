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
    request.label = request
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(ToOwned::to_owned);

    if request.mode == TunnelMode::Dynamic {
        request.target_host.clear();
        request.target_port = 0;
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

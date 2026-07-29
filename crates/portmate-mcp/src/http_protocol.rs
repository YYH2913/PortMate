use crate::http_request::HttpRequest;
use anyhow::{anyhow, Result};
use serde_json::Value;

pub(crate) const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
pub(crate) const MCP_PROTOCOL_VERSIONS: [&str; 3] =
    ["2024-11-05", "2025-03-26", MCP_PROTOCOL_VERSION];

pub(crate) fn negotiated_mcp_protocol_version(params: &Value) -> &str {
    params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .filter(|version| MCP_PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(MCP_PROTOCOL_VERSION)
}

pub(crate) fn http_protocol_version(request: &HttpRequest) -> &str {
    request
        .headers
        .get("mcp-protocol-version")
        .map(String::as_str)
        .filter(|version| MCP_PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(MCP_PROTOCOL_VERSION)
}

pub(crate) fn validate_mcp_protocol_version(request: &HttpRequest) -> Result<()> {
    let Some(version) = request.headers.get("mcp-protocol-version") else {
        return Ok(());
    };
    if MCP_PROTOCOL_VERSIONS.contains(&version.as_str()) {
        Ok(())
    } else {
        Err(anyhow!(
            "unsupported MCP-Protocol-Version `{version}`; supported versions: {}",
            MCP_PROTOCOL_VERSIONS.join(", ")
        ))
    }
}

pub(crate) fn has_json_http_content_type(request: &HttpRequest) -> bool {
    request.headers.get("content-type").is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("application/json")
    })
}

pub(crate) fn accepts_json_http_response(request: &HttpRequest) -> bool {
    accepts_http_media_type(request, true, |media_type| {
        matches!(media_type, "*/*" | "application/*" | "application/json")
    })
}

pub(crate) fn accepts_sse_http_response(request: &HttpRequest) -> bool {
    accepts_http_media_type(request, false, |media_type| {
        media_type == "text/event-stream"
    })
}

pub(crate) fn is_sse_stream_request(request: &HttpRequest) -> bool {
    request.method == "GET" && request.path == "/mcp" && accepts_sse_http_response(request)
}

fn accepts_http_media_type(
    request: &HttpRequest,
    default_when_missing: bool,
    matches_media_type: impl Fn(&str) -> bool,
) -> bool {
    let Some(accept) = request.headers.get("accept") else {
        return default_when_missing;
    };
    accept.split(',').any(|item| {
        let mut parts = item.split(';');
        let media_type = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
        let mut quality = 1.0_f32;
        for parameter in parts {
            let Some((name, value)) = parameter.trim().split_once('=') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("q") {
                quality = value.trim().parse::<f32>().unwrap_or(0.0);
            }
        }
        (0.0..=1.0).contains(&quality) && quality > 0.0 && matches_media_type(&media_type)
    })
}

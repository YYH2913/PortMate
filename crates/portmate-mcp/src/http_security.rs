use crate::http_request::HttpRequest;
use crate::keyring_store::{read_secret_from_keyring, write_secret_to_keyring};
use anyhow::{anyhow, Result};
use std::net::SocketAddr;
use uuid::Uuid;

pub(crate) const HTTP_TOKEN_REF: &str = "keychain:mcp-http-token";

#[derive(Debug, Clone)]
pub(crate) struct HttpSecurityConfig {
    token: String,
    allowed_origins: Vec<String>,
}

impl HttpSecurityConfig {
    pub(crate) fn from_environment(addr: SocketAddr) -> Result<Self> {
        let token = std::env::var("PORTMATE_MCP_HTTP_TOKEN")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_string())
            .or_else(|| read_or_create_http_token().ok())
            .ok_or_else(|| anyhow!("failed to load or create MCP HTTP token"))?;
        let mut allowed_origins = std::env::var("PORTMATE_MCP_HTTP_ORIGINS")
            .or_else(|_| std::env::var("PORTMATE_MCP_HTTP_ORIGIN"))
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if allowed_origins.is_empty() {
            allowed_origins.push(format!("http://127.0.0.1:{}", addr.port()));
            allowed_origins.push(format!("http://localhost:{}", addr.port()));
        }
        Ok(Self::new(token, allowed_origins))
    }

    pub(crate) fn new(token: String, allowed_origins: Vec<String>) -> Self {
        Self {
            token,
            allowed_origins,
        }
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }
}

pub(crate) fn validate_origin(origin: Option<&str>, config: &HttpSecurityConfig) -> Result<()> {
    let Some(origin) = origin.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if config
        .allowed_origins
        .iter()
        .any(|allowed| allowed == "*" || allowed == origin)
    {
        Ok(())
    } else {
        Err(anyhow!("Origin `{origin}` is not allowed"))
    }
}

pub(crate) fn authorized_http_request(request: &HttpRequest, token: &str) -> bool {
    if let Some(value) = request.headers.get("authorization") {
        let mut parts = value.split_whitespace();
        if parts
            .next()
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("bearer"))
        {
            if let (Some(candidate), None) = (parts.next(), parts.next()) {
                return constant_time_str_eq(candidate, token);
            }
        }
    }
    request
        .headers
        .get("x-portmate-mcp-token")
        .is_some_and(|candidate| constant_time_str_eq(candidate.trim(), token))
}

fn read_or_create_http_token() -> Result<String> {
    match read_secret_from_keyring(HTTP_TOKEN_REF) {
        Ok(token) if !token.trim().is_empty() => Ok(token),
        _ => {
            let token = Uuid::new_v4().to_string();
            write_secret_to_keyring(HTTP_TOKEN_REF, &token)?;
            Ok(token)
        }
    }
}

fn constant_time_str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

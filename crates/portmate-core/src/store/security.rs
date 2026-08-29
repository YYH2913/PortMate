use super::SessionStore;
use crate::host_keys::{HostKeyEvaluation, HostKeyObservation};
use crate::models::{
    AuthMethod, ConnectionConfig, HostKeyDecision, McpScope, SshConnection, TrustedHostKey,
    DEFAULT_MCP_HTTP_CLIENT_ID,
};
use chrono::Utc;

impl SessionStore {
    pub fn record_auth_success(
        &mut self,
        session_id: &str,
        method: AuthMethod,
    ) -> Result<(), String> {
        let profile = self
            .profiles
            .iter_mut()
            .find(|profile| profile.id == session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;

        match &mut profile.connection {
            ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
                ssh.identity_policy.last_successful = ssh
                    .identity_policy
                    .record_success
                    .then_some(method)
                    .filter(|method| ssh.identity_policy.auth_order.contains(method));
                Ok(())
            }
            _ => Err(format!("profile is not SSH-backed: {session_id}")),
        }
    }

    pub fn evaluate_host_key(
        &self,
        profile_id: &str,
        observation: &HostKeyObservation,
    ) -> Result<HostKeyEvaluation, String> {
        let policy = self
            .ssh_profile(profile_id)
            .map(|ssh| &ssh.host_key_policy)
            .ok_or_else(|| format!("profile is not SSH-backed: {profile_id}"))?;
        self.host_keys
            .evaluate(profile_id, policy, observation)
            .map_err(|error| error.to_string())
    }

    pub fn apply_host_key_decision(
        &mut self,
        profile_id: &str,
        observation: &HostKeyObservation,
        decision: HostKeyDecision,
    ) -> Result<Option<TrustedHostKey>, String> {
        let policy = self
            .ssh_profile(profile_id)
            .map(|ssh| ssh.host_key_policy.clone())
            .ok_or_else(|| format!("profile is not SSH-backed: {profile_id}"))?;
        self.host_keys
            .apply_decision(profile_id, &policy, observation, decision)
            .map_err(|error| error.to_string())
    }

    pub fn mcp_can(&self, client_id: &str, scope: McpScope, session_id: Option<&str>) -> bool {
        let now = Utc::now();
        self.grants
            .iter()
            .filter(|grant| grant.client_id == client_id)
            .any(|grant| grant.allows(scope, session_id, now))
    }

    pub fn mcp_can_read(&self, client_id: &str, scope: McpScope, session_id: Option<&str>) -> bool {
        let client_id = client_id.trim();
        !client_id.is_empty()
            && client_id.len() <= 128
            && !client_id.chars().any(char::is_control)
            && self.mcp_can(client_id, scope, session_id)
    }

    /// Resolve the client identity used by the HTTP bridge without widening a
    /// grant. A matching explicit or stored identity wins; a single active
    /// grant is adopted only for the legacy default/empty identity. With
    /// multiple unmatched grants we retain the configured identity so the
    /// request fails closed instead of guessing an authorization boundary.
    pub fn mcp_resolved_client_id(&self, configured: Option<&str>) -> String {
        let configured = configured.map(str::trim).filter(|value| !value.is_empty());
        let stored = self.mcp_http_settings.client_id.trim();
        let now = Utc::now();
        let active = self
            .grants
            .iter()
            .filter(|grant| {
                grant.revoked_at.is_none()
                    && !grant.expires_at.is_some_and(|expires| expires <= now)
            })
            .map(|grant| grant.client_id.as_str())
            .collect::<Vec<_>>();

        if let Some(configured) = configured
            .filter(|candidate| *candidate != DEFAULT_MCP_HTTP_CLIENT_ID)
            .filter(|candidate| active.contains(candidate))
        {
            return configured.to_string();
        }
        if !stored.is_empty() && active.contains(&stored) {
            return stored.to_string();
        }
        let legacy_default = (stored.is_empty() || stored == DEFAULT_MCP_HTTP_CLIENT_ID)
            && configured.is_none_or(|candidate| candidate == DEFAULT_MCP_HTTP_CLIENT_ID);
        if active.len() == 1 && legacy_default {
            return active[0].to_string();
        }
        // Multiple active grants are ambiguous. Preserve a non-default
        // operator choice in that case so the bridge fails closed instead of
        // guessing or combining authorization boundaries.
        if let Some(configured) =
            configured.filter(|candidate| *candidate != DEFAULT_MCP_HTTP_CLIENT_ID)
        {
            return configured.to_string();
        }
        if let Some(configured) = configured {
            return configured.to_string();
        }
        if !stored.is_empty() {
            return stored.to_string();
        }
        DEFAULT_MCP_HTTP_CLIENT_ID.to_string()
    }

    fn ssh_profile(&self, profile_id: &str) -> Option<&SshConnection> {
        self.profiles
            .iter()
            .find(|profile| profile.id == profile_id)
            .and_then(|profile| match &profile.connection {
                ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => Some(ssh),
                _ => None,
            })
    }
}

use super::SessionStore;
use crate::host_keys::{HostKeyEvaluation, HostKeyObservation};
use crate::models::{ConnectionConfig, HostKeyDecision, McpScope, SshConnection, TrustedHostKey};
use chrono::Utc;

impl SessionStore {
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
            && (self.grants.is_empty() || self.mcp_can(client_id, scope, session_id))
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

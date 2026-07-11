use crate::models::{HostKeyDecision, HostKeyPolicy, HostKeyScope, TrustedHostKey};
use base64::{engine::general_purpose, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyObservation {
    pub host: String,
    pub port: u16,
    pub alias: Option<String>,
    pub algorithm: String,
    pub public_key_base64: String,
}

impl HostKeyObservation {
    pub fn fingerprint_sha256(&self) -> Result<String, HostKeyError> {
        compute_ssh_sha256_fingerprint(&self.public_key_base64)
    }

    pub fn target_alias<'a>(&'a self, policy: &'a HostKeyPolicy) -> &'a str {
        self.alias
            .as_deref()
            .or(policy.alias.as_deref())
            .unwrap_or(self.host.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum HostKeyEvaluation {
    Trusted {
        matched_key_id: String,
        fingerprint_sha256: String,
    },
    Unknown {
        alias: String,
        host: String,
        port: u16,
        algorithm: String,
        fingerprint_sha256: String,
        reason: String,
    },
    Mismatch {
        alias: String,
        host: String,
        port: u16,
        algorithm: String,
        expected: Vec<TrustedHostKey>,
        observed_fingerprint_sha256: String,
        reason: String,
    },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HostKeyError {
    #[error("host key is not valid base64")]
    InvalidBase64,
    #[error("host key decision rejected the observed key")]
    Rejected,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostKeyStore {
    pub keys: Vec<TrustedHostKey>,
}

impl HostKeyStore {
    pub fn new() -> Self {
        Self { keys: Vec::new() }
    }

    pub fn evaluate(
        &self,
        profile_id: &str,
        policy: &HostKeyPolicy,
        observation: &HostKeyObservation,
    ) -> Result<HostKeyEvaluation, HostKeyError> {
        let fingerprint = observation.fingerprint_sha256()?;
        let alias = observation.target_alias(policy).to_string();
        let candidates = self.candidates(profile_id, policy, &alias, observation);

        if let Some(trusted) = candidates.iter().find(|key| {
            key.algorithm == observation.algorithm && key.fingerprint_sha256 == fingerprint
        }) {
            return Ok(HostKeyEvaluation::Trusted {
                matched_key_id: trusted.id.clone(),
                fingerprint_sha256: fingerprint,
            });
        }

        let same_algorithm: Vec<TrustedHostKey> = candidates
            .into_iter()
            .filter(|key| key.algorithm == observation.algorithm)
            .cloned()
            .collect();

        if !same_algorithm.is_empty() {
            if policy.allow_rotation {
                return Ok(HostKeyEvaluation::Unknown {
                    alias,
                    host: observation.host.clone(),
                    port: observation.port,
                    algorithm: observation.algorithm.clone(),
                    fingerprint_sha256: fingerprint,
                    reason: "alias key rotated; policy allows re-trusting it like a first meeting"
                        .to_string(),
                });
            }
            return Ok(HostKeyEvaluation::Mismatch {
                alias,
                host: observation.host.clone(),
                port: observation.port,
                algorithm: observation.algorithm.clone(),
                expected: same_algorithm,
                observed_fingerprint_sha256: fingerprint,
                reason: "same alias and algorithm already has a different host key".to_string(),
            });
        }

        Ok(HostKeyEvaluation::Unknown {
            alias,
            host: observation.host.clone(),
            port: observation.port,
            algorithm: observation.algorithm.clone(),
            fingerprint_sha256: fingerprint,
            reason: "no trusted key exists for this alias and algorithm".to_string(),
        })
    }

    pub fn apply_decision(
        &mut self,
        profile_id: &str,
        policy: &HostKeyPolicy,
        observation: &HostKeyObservation,
        decision: HostKeyDecision,
    ) -> Result<Option<TrustedHostKey>, HostKeyError> {
        match decision {
            HostKeyDecision::Reject => Err(HostKeyError::Rejected),
            HostKeyDecision::TrustOnce => Ok(None),
            HostKeyDecision::AppendToProfile | HostKeyDecision::AppendToProject => {
                let scope = if decision == HostKeyDecision::AppendToProject {
                    HostKeyScope::Project
                } else {
                    HostKeyScope::Profile
                };
                let key = self.make_trusted_key(profile_id, policy, observation, scope)?;
                self.keys.push(key.clone());
                Ok(Some(key))
            }
            HostKeyDecision::ReplaceForProfile => {
                let alias = observation.target_alias(policy).to_string();
                self.keys.retain(|key| {
                    !(key.profile_id.as_deref() == Some(profile_id)
                        && key.alias == alias
                        && key.port == observation.port
                        && key.algorithm == observation.algorithm)
                });
                let key =
                    self.make_trusted_key(profile_id, policy, observation, HostKeyScope::Profile)?;
                self.keys.push(key.clone());
                Ok(Some(key))
            }
        }
    }

    pub fn import_known_hosts(&mut self, profile_id: &str, contents: &str) -> Vec<KnownHostsLine> {
        contents
            .lines()
            .filter_map(KnownHostsLine::parse)
            .inspect(|line| {
                if let Ok(fingerprint) = compute_ssh_sha256_fingerprint(&line.public_key_base64) {
                    for host in &line.hosts {
                        let (alias, port) = split_known_host(host);
                        self.keys.push(TrustedHostKey {
                            id: Uuid::new_v4().to_string(),
                            profile_id: Some(profile_id.to_string()),
                            alias,
                            host: host.clone(),
                            port,
                            algorithm: line.algorithm.clone(),
                            fingerprint_sha256: fingerprint.clone(),
                            public_key_base64: line.public_key_base64.clone(),
                            scope: HostKeyScope::Profile,
                            label: Some("imported known_hosts".to_string()),
                            first_seen: Utc::now(),
                            last_seen: Utc::now(),
                        });
                    }
                }
            })
            .collect()
    }

    pub fn export_known_hosts(&self) -> String {
        self.keys
            .iter()
            .map(|key| format!("{} {} {}", key.alias, key.algorithm, key.public_key_base64))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn candidates<'a>(
        &'a self,
        profile_id: &str,
        policy: &HostKeyPolicy,
        alias: &str,
        observation: &HostKeyObservation,
    ) -> Vec<&'a TrustedHostKey> {
        self.keys
            .iter()
            .filter(|key| key.alias == alias && key.port == observation.port)
            .filter(|key| !policy.check_ip || key.host == observation.host)
            .filter(|key| match key.scope {
                HostKeyScope::Profile => key.profile_id.as_deref() == Some(profile_id),
                HostKeyScope::Project => matches!(
                    policy.trust_scope,
                    HostKeyScope::Project | HostKeyScope::Profile
                ),
                HostKeyScope::User => true,
            })
            .collect()
    }

    fn make_trusted_key(
        &self,
        profile_id: &str,
        policy: &HostKeyPolicy,
        observation: &HostKeyObservation,
        scope: HostKeyScope,
    ) -> Result<TrustedHostKey, HostKeyError> {
        Ok(TrustedHostKey {
            id: Uuid::new_v4().to_string(),
            profile_id: Some(profile_id.to_string()),
            alias: observation.target_alias(policy).to_string(),
            host: observation.host.clone(),
            port: observation.port,
            algorithm: observation.algorithm.clone(),
            fingerprint_sha256: observation.fingerprint_sha256()?,
            public_key_base64: observation.public_key_base64.clone(),
            scope,
            label: None,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownHostsLine {
    pub hosts: Vec<String>,
    pub algorithm: String,
    pub public_key_base64: String,
}

impl KnownHostsLine {
    pub fn parse(line: &str) -> Option<Self> {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('|') {
            return None;
        }
        let mut parts = trimmed.split_whitespace();
        let hosts = parts
            .next()?
            .split(',')
            .map(str::to_string)
            .collect::<Vec<_>>();
        let algorithm = parts.next()?.to_string();
        let public_key_base64 = parts.next()?.to_string();
        Some(Self {
            hosts,
            algorithm,
            public_key_base64,
        })
    }
}

pub fn compute_ssh_sha256_fingerprint(public_key_base64: &str) -> Result<String, HostKeyError> {
    let key_blob = general_purpose::STANDARD
        .decode(public_key_base64)
        .map_err(|_| HostKeyError::InvalidBase64)?;
    let digest = Sha256::digest(key_blob);
    let encoded = general_purpose::STANDARD_NO_PAD.encode(digest);
    Ok(format!("SHA256:{encoded}"))
}

fn split_known_host(host: &str) -> (String, u16) {
    if let Some(rest) = host.strip_prefix('[') {
        if let Some((name, port)) = rest.split_once("]:") {
            return (name.to_string(), port.parse().unwrap_or(22));
        }
    }
    (host.to_string(), 22)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::HostKeyPolicy;

    fn obs(alias: &str, key: &str) -> HostKeyObservation {
        HostKeyObservation {
            host: "192.168.1.10".to_string(),
            port: 22,
            alias: Some(alias.to_string()),
            algorithm: "ssh-ed25519".to_string(),
            public_key_base64: key.to_string(),
        }
    }

    #[test]
    fn same_ip_different_aliases_do_not_conflict() {
        let key_a = general_purpose::STANDARD.encode(b"device-a-key");
        let key_b = general_purpose::STANDARD.encode(b"device-b-key");
        let mut store = HostKeyStore::new();
        let policy_a = HostKeyPolicy::profile_alias("lab-a");
        let policy_b = HostKeyPolicy::profile_alias("lab-b");

        store
            .apply_decision(
                "lab-a",
                &policy_a,
                &obs("lab-a", &key_a),
                HostKeyDecision::AppendToProfile,
            )
            .unwrap();

        let evaluation = store
            .evaluate("lab-b", &policy_b, &obs("lab-b", &key_b))
            .unwrap();
        assert!(matches!(evaluation, HostKeyEvaluation::Unknown { .. }));
    }

    #[test]
    fn same_alias_same_algorithm_changed_key_is_mismatch() {
        let key_a = general_purpose::STANDARD.encode(b"device-a-key");
        let key_b = general_purpose::STANDARD.encode(b"device-b-key");
        let mut store = HostKeyStore::new();
        let policy = HostKeyPolicy::profile_alias("bench-slot-1");

        store
            .apply_decision(
                "profile",
                &policy,
                &obs("bench-slot-1", &key_a),
                HostKeyDecision::AppendToProfile,
            )
            .unwrap();

        let evaluation = store
            .evaluate("profile", &policy, &obs("bench-slot-1", &key_b))
            .unwrap();
        assert!(matches!(evaluation, HostKeyEvaluation::Mismatch { .. }));
    }

    #[test]
    fn allow_rotation_treats_mismatch_as_unknown_instead_of_blocking() {
        let key_a = general_purpose::STANDARD.encode(b"device-a-key");
        let key_b = general_purpose::STANDARD.encode(b"device-b-key");
        let mut store = HostKeyStore::new();
        let mut policy = HostKeyPolicy::profile_alias("bench-slot-1");
        policy.allow_rotation = true;

        store
            .apply_decision(
                "profile",
                &policy,
                &obs("bench-slot-1", &key_a),
                HostKeyDecision::AppendToProfile,
            )
            .unwrap();

        let evaluation = store
            .evaluate("profile", &policy, &obs("bench-slot-1", &key_b))
            .unwrap();
        assert!(matches!(evaluation, HostKeyEvaluation::Unknown { .. }));
    }

    #[test]
    fn check_ip_treats_same_alias_different_host_as_unknown() {
        let key_a = general_purpose::STANDARD.encode(b"device-a-key");
        let mut store = HostKeyStore::new();
        let mut policy = HostKeyPolicy::profile_alias("bench-slot-1");
        policy.check_ip = true;

        store
            .apply_decision(
                "profile",
                &policy,
                &obs("bench-slot-1", &key_a),
                HostKeyDecision::AppendToProfile,
            )
            .unwrap();

        let mut moved_obs = obs("bench-slot-1", &key_a);
        moved_obs.host = "192.168.1.99".to_string();
        let evaluation = store.evaluate("profile", &policy, &moved_obs).unwrap();
        assert!(matches!(evaluation, HostKeyEvaluation::Unknown { .. }));
    }

    #[test]
    fn multiple_algorithms_can_be_added_to_one_alias() {
        let key_a = general_purpose::STANDARD.encode(b"device-a-ed25519");
        let key_b = general_purpose::STANDARD.encode(b"device-a-rsa");
        let mut store = HostKeyStore::new();
        let policy = HostKeyPolicy::profile_alias("bench-slot-1");
        let mut rsa_obs = obs("bench-slot-1", &key_b);
        rsa_obs.algorithm = "rsa-sha2-512".to_string();

        store
            .apply_decision(
                "profile",
                &policy,
                &obs("bench-slot-1", &key_a),
                HostKeyDecision::AppendToProfile,
            )
            .unwrap();

        let evaluation = store.evaluate("profile", &policy, &rsa_obs).unwrap();
        assert!(matches!(evaluation, HostKeyEvaluation::Unknown { .. }));
        store
            .apply_decision(
                "profile",
                &policy,
                &rsa_obs,
                HostKeyDecision::AppendToProfile,
            )
            .unwrap();
        assert_eq!(store.keys.len(), 2);
    }
}

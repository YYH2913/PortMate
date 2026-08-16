use super::*;

fn snapshot(profile: &SessionProfile) -> String {
    ssh_health::ssh_health_profile_snapshot(profile).unwrap()
}

#[test]
fn ssh_health_profile_snapshot_accepts_the_connected_profile() {
    let profile = test_ssh_profile();
    assert!(ssh_health::validate_ssh_health_profile_snapshot(
        &profile.id,
        &snapshot(&profile),
        &profile,
    )
    .is_ok());
}

#[test]
fn ssh_health_profile_snapshot_ignores_host_key_mirrors_and_recorded_auth_success() {
    let connected = test_ssh_profile();
    let mut current = connected.clone();
    let ConnectionConfig::Ssh(ssh) = &mut current.connection else {
        unreachable!("test Profile must remain SSH-backed");
    };
    ssh.identity_policy.last_successful = Some(AuthMethod::PublicKey);
    let now = Utc::now();
    ssh.trusted_host_keys.push(TrustedHostKey {
        id: "host-key-1".to_string(),
        profile_id: Some(current.id.clone()),
        alias: current.id.clone(),
        host: ssh.endpoint.host.clone(),
        port: ssh.endpoint.port,
        algorithm: "ssh-ed25519".to_string(),
        fingerprint_sha256: "SHA256:test".to_string(),
        public_key_base64: "YWJj".to_string(),
        scope: HostKeyScope::Profile,
        label: None,
        first_seen: now,
        last_seen: now,
    });

    assert_eq!(snapshot(&connected), snapshot(&current));
}

#[test]
fn ssh_health_profile_snapshot_rejects_runtime_affecting_changes() {
    let connected = test_ssh_profile();
    let connected_snapshot = snapshot(&connected);
    let mut changed_profiles = Vec::new();

    let mut endpoint = connected.clone();
    let ConnectionConfig::Ssh(ssh) = &mut endpoint.connection else {
        unreachable!("test Profile must remain SSH-backed");
    };
    ssh.endpoint.host = "replacement.example".to_string();
    changed_profiles.push(endpoint);

    let mut proxy = connected.clone();
    let ConnectionConfig::Ssh(ssh) = &mut proxy.connection else {
        unreachable!("test Profile must remain SSH-backed");
    };
    ssh.proxy.enabled = true;
    changed_profiles.push(proxy);

    let mut jump = connected.clone();
    let ConnectionConfig::Ssh(ssh) = &mut jump.connection else {
        unreachable!("test Profile must remain SSH-backed");
    };
    ssh.jumps.push(portmate_core::JumpHop {
        host: "jump.example".to_string(),
        port: 22,
        username: "ops".to_string(),
        password_secret_ref: None,
        passphrase_secret_ref: None,
        identity_ref: None,
        host_key_policy: None,
    });
    changed_profiles.push(jump);

    let mut authentication = connected.clone();
    let ConnectionConfig::Ssh(ssh) = &mut authentication.connection else {
        unreachable!("test Profile must remain SSH-backed");
    };
    ssh.identity_policy.identities_only = false;
    changed_profiles.push(authentication);

    let mut terminal = connected.clone();
    terminal.terminal.rows += 1;
    changed_profiles.push(terminal);

    for changed in changed_profiles {
        let error = ssh_health::validate_ssh_health_profile_snapshot(
            &connected.id,
            &connected_snapshot,
            &changed,
        )
        .unwrap_err();
        assert!(error.contains("配置已更改"), "{error}");
    }
}

#[test]
fn ssh_health_profile_snapshot_rejects_a_different_profile_id() {
    let connected = test_ssh_profile();
    let mut different = connected.clone();
    different.id = "different-session".to_string();
    let error = ssh_health::validate_ssh_health_profile_snapshot(
        &connected.id,
        &snapshot(&connected),
        &different,
    )
    .unwrap_err();
    assert!(error.contains("Profile 与会话不匹配"), "{error}");
}

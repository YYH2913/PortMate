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

#[test]
fn known_hosts_rejects_invalid_or_zero_ports_instead_of_using_port_22() {
    let key = general_purpose::STANDARD.encode(b"known-host-key");
    let mut store = HostKeyStore::new();
    let imported = store.import_known_hosts(
        "profile",
        &format!(
            "[router.example]:not-a-port ssh-ed25519 {key}\n\
             [router.example]:0 ssh-ed25519 {key}\n\
             [router.example]:2222 ssh-ed25519 {key}\n"
        ),
    );

    assert_eq!(imported.len(), 3);
    assert_eq!(store.keys.len(), 1);
    assert_eq!(store.keys[0].port, 2222);
}

#[test]
fn known_hosts_nonstandard_port_round_trips_and_matches_check_ip() {
    let key = general_purpose::STANDARD.encode(b"router-key");
    let contents = format!("[router.example]:2222 ssh-ed25519 {key}");
    let mut store = HostKeyStore::new();

    let imported = store.import_known_hosts("profile", &contents);

    assert_eq!(imported.len(), 1);
    assert_eq!(store.keys[0].alias, "router.example");
    assert_eq!(store.keys[0].host, "router.example");
    assert_eq!(store.keys[0].port, 2222);
    assert_eq!(store.export_known_hosts(), contents);

    let mut policy = HostKeyPolicy::profile_alias("router.example");
    policy.check_ip = true;
    let observation = HostKeyObservation {
        host: "router.example".to_string(),
        port: 2222,
        alias: Some("router.example".to_string()),
        algorithm: "ssh-ed25519".to_string(),
        public_key_base64: key,
    };
    assert!(matches!(
        store.evaluate("profile", &policy, &observation).unwrap(),
        HostKeyEvaluation::Trusted { .. }
    ));
}

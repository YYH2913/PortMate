use super::*;

#[test]
fn ssh_health_errors_are_bounded_on_character_boundaries() {
    let error = "故".repeat(MAX_SSH_HEALTH_ERROR_CHARACTERS + 1);
    let bounded = bounded_ssh_health_error(&error);
    assert_eq!(bounded.chars().count(), MAX_SSH_HEALTH_ERROR_CHARACTERS + 3);
    assert!(bounded.ends_with("..."));
}

#[test]
fn ssh_health_status_serializes_as_stable_kebab_case() {
    assert_eq!(
        serde_json::to_string(&SshHealthStatus::Unresponsive).unwrap(),
        "\"unresponsive\""
    );
}

#[test]
fn ssh_health_backend_serializes_as_stable_kebab_case() {
    assert_eq!(
        serde_json::to_string(&SshBackendKind::Libssh).unwrap(),
        "\"libssh\""
    );
}

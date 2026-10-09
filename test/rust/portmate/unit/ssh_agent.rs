use super::*;

#[test]
fn agent_operations_drop_unresponsive_futures_at_their_deadline() {
    tauri::async_runtime::block_on(async {
        let started = Instant::now();
        let result = run_agent_operation("test ssh-agent", async {
            std::future::pending::<Result<(), String>>().await
        })
        .await;
        assert!(result.unwrap_err().contains("超时"));
        assert!(started.elapsed() < Duration::from_secs(6));
    });
}

#[test]
fn agent_identity_path_matches_exact_comment_bytes() {
    let key = russh::keys::PublicKey::from_openssh(
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAhLsO2cjvuaNmiRlw4TJjIL+yzlPke9KgoXSfTiaqzQ",
    )
    .unwrap();
    let identity = AgentIdentity::PublicKey {
        key,
        comment: "accepted-comment ".to_string(),
    };
    let exact = AgentIdentityFilter {
        label: "agent key".to_string(),
        fingerprint_sha256: None,
        path: Some("accepted-comment ".to_string()),
    };
    let lossy = AgentIdentityFilter {
        path: Some("accepted-comment".to_string()),
        ..exact.clone()
    };

    assert!(agent_identity_matches(&identity, &[exact]));
    assert!(!agent_identity_matches(&identity, &[lossy]));
}

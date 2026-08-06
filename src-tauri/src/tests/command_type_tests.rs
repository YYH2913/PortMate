use super::*;

#[test]
fn remaining_command_types_keep_stable_json_contracts() {
    let write = serde_json::to_value(SecretWriteRequest {
        secret_ref: Some("keychain:existing".to_string()),
        secret: "replacement".to_string(),
        storage: Some(SecretStorage::Portable),
    })
    .unwrap();
    assert_eq!(write["secretRef"], "keychain:existing");
    assert_eq!(write["storage"], "portable");

    let proxy: ProxyPasswordUpdate = serde_json::from_value(serde_json::json!({
        "action": "set",
        "password": "proxy-secret"
    }))
    .unwrap();
    assert!(matches!(
        proxy,
        ProxyPasswordUpdate::Set { storage: None, .. }
    ));

    let deleted = serde_json::to_value(DeleteSessionProfileResponse {
        deleted_profile_id: "ssh-1".to_string(),
        sessions: Vec::new(),
        one_keys: Vec::new(),
        host_keys: HostKeyStore::default(),
        grants: Vec::new(),
    })
    .unwrap();
    assert_eq!(deleted["deletedProfileId"], "ssh-1");
    assert!(deleted["oneKeys"].as_array().unwrap().is_empty());

    let config = serde_json::to_value(McpHttpConfig {
        settings: McpHttpSettings {
            listen_host: "127.0.0.1".to_string(),
            port: 43123,
            allowed_origins: vec!["http://127.0.0.1".to_string()],
            client_id: "test-client".to_string(),
            trusted: false,
            allow_remote: false,
        },
        remote_access: false,
        endpoint: "http://127.0.0.1:43123/mcp".to_string(),
        token_ref: "keychain:mcp-http".to_string(),
        token_available: true,
        default_origin: "http://127.0.0.1".to_string(),
        executable: "/opt/portmate-mcp".to_string(),
        store_path: "/tmp/portmate.sqlite3".to_string(),
        start_command: "portmate-mcp --http".to_string(),
    })
    .unwrap();
    assert_eq!(config["tokenRef"], "keychain:mcp-http");
    assert_eq!(config["tokenAvailable"], true);
    assert_eq!(config["defaultOrigin"], "http://127.0.0.1");
    assert_eq!(config["listenHost"], "127.0.0.1");
    assert_eq!(config["clientId"], "test-client");
    assert_eq!(config["remoteAccess"], false);
    assert_eq!(config["startCommand"], "portmate-mcp --http");

    let runtime = serde_json::to_value(McpHttpRuntimeStatus {
        phase: McpHttpRuntimePhase::Running,
        endpoint: Some("http://127.0.0.1:43123/mcp".to_string()),
        pid: Some(4242),
        started_at: Some(Utc::now()),
        message: None,
    })
    .unwrap();
    assert_eq!(runtime["phase"], "running");
    assert_eq!(runtime["pid"], 4242);
    assert_eq!(runtime["endpoint"], "http://127.0.0.1:43123/mcp");
    assert!(runtime["startedAt"].is_string());
}

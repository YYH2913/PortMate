use super::*;
use crate::{HostScriptConfig, HostScriptLanguage, HostScriptParameter, HostScriptParameterKind};

fn config() -> HostScriptConfig {
    HostScriptConfig {
        language: HostScriptLanguage::Python,
        interpreter: String::new(),
        working_directory: String::new(),
        timeout_seconds: 30,
        allowed_client_ids: vec!["a".into()],
        parameters: vec![HostScriptParameter {
            name: "message".into(),
            description: "text".into(),
            kind: HostScriptParameterKind::String,
            required: true,
        }],
    }
}
#[test]
fn host_parameter_validation_is_strict_and_bounded() {
    let host = config();
    validate_host_script_config(&host).unwrap();
    for bad in [
        serde_json::json!({}),
        serde_json::json!([]),
        serde_json::json!({"message":3}),
        serde_json::json!({"message":"x","extra":true}),
        serde_json::json!({"message":"\0"}),
        serde_json::json!({"message":"x".repeat(17000)}),
    ] {
        assert!(validate_host_script_parameters(&host, &bad).is_err());
    }
    validate_host_script_parameters(&host, &serde_json::json!({"message":"$(whoami); ' 中文"}))
        .unwrap();
    let mut invalid = host.clone();
    invalid.timeout_seconds = 0;
    assert!(validate_host_script_config(&invalid).is_err());
    invalid = host.clone();
    invalid.interpreter = "python -c injected".into();
    assert!(validate_host_script_config(&invalid).is_err());
    invalid = host.clone();
    invalid.parameters.push(HostScriptParameter {
        name: "MESSAGE".into(),
        ..host.parameters[0].clone()
    });
    assert!(validate_host_script_config(&invalid).is_err());
}
#[test]
fn host_tool_schema_and_client_scope_survive_normalization() {
    let now = chrono::Utc::now();
    let script = CustomScript {
        id: Uuid::new_v4().to_string(),
        name: "test".into(),
        description: "test skill".into(),
        content: "secret source".into(),
        host: config(),
        mcp_enabled: true,
        created_at: now,
        updated_at: now,
    };
    assert!(script.allows_host_client("a"));
    assert!(!script.allows_host_client("b"));
    let tool = host_script_tool_definition(&script).unwrap();
    assert_eq!(tool.input_schema["properties"]["message"]["type"], "string");
    assert_eq!(
        tool.input_schema["required"],
        serde_json::json!(["message"])
    );
    assert_eq!(tool.input_schema["additionalProperties"], false);
    assert!(!serde_json::to_string(&tool)
        .unwrap()
        .contains("secret source"));
    assert_eq!(
        normalize_loaded_custom_scripts(vec![script.clone()]),
        vec![script]
    );
}

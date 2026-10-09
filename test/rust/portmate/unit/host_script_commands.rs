use super::*;

#[test]
fn valid_large_parameters_are_not_rejected_by_a_total_environment_budget() {
    let root = tempfile::tempdir().unwrap();
    let mut script: CustomScript = serde_json::from_value(serde_json::json!({
        "id": Uuid::new_v4(), "name": "large input", "description": "", "content": "",
        "createdAt": Utc::now(), "updatedAt": Utc::now(), "mcpEnabled": false,
        "host": {"language": "python", "timeoutSeconds": 5,
            "parameters": [{"name": "value", "description": "", "kind": "string", "required": true}]}
    })).unwrap();
    script.host.working_directory = root.path().to_str().unwrap().into();
    script.host.interpreter = std::env::current_exe().unwrap().to_str().unwrap().into();
    let parameters = serde_json::json!({"value": "x".repeat(16_360)});
    validate_host_script_parameters(&script.host, &parameters).unwrap();
    let (command, _files) = prepare_command(&script, &parameters).unwrap();
    let environment = command.as_std().get_envs().collect::<BTreeMap<_, _>>();
    assert_eq!(
        environment[std::ffi::OsStr::new("PORTMATE_PARAM_value")]
            .unwrap()
            .len(),
        16_360
    );
    assert_eq!(
        environment[std::ffi::OsStr::new("PORTMATE_INPUT_JSON")]
            .unwrap()
            .len(),
        16_372
    );
}

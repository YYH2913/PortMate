use crate::{CustomScript, SessionEvent};
use std::collections::HashSet;
use uuid::Uuid;

pub const MAX_CUSTOM_SCRIPTS: usize = 128;
pub const MAX_CUSTOM_SCRIPT_NAME_CHARACTERS: usize = 128;
pub const MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS: usize = 1_024;
pub const MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS: usize = 65_536;
pub const MAX_CUSTOM_SCRIPT_CONTENT_BYTES: usize = 256 * 1024;
pub const CUSTOM_SCRIPT_EVENT_TEXT: &str = "<custom-script>";

pub fn validate_host_script_config(host: &crate::HostScriptConfig) -> Result<(), String> {
    if !(1..=60).contains(&host.timeout_seconds) {
        return Err("host script timeout must be 1–60 seconds".into());
    }
    for path in [&host.interpreter, &host.working_directory] {
        if path.len() > 4096 || path.chars().any(char::is_control) {
            return Err("invalid host script path".into());
        }
        // Accept both platform path syntaxes so synced stores remain readable.
        if !path.is_empty()
            && !path.starts_with('/')
            && !(path.as_bytes().get(1) == Some(&b':')
                && path
                    .as_bytes()
                    .get(2)
                    .is_some_and(|c| matches!(c, b'\\' | b'/')))
            && !path.starts_with("\\\\")
        {
            return Err("host script paths must be absolute".into());
        }
    }
    if host.allowed_client_ids.len() > 128 || host.parameters.len() > 32 {
        return Err("host script allows at most 128 clients and 32 parameters".into());
    }
    let mut clients = HashSet::new();
    for id in &host.allowed_client_ids {
        if id.is_empty()
            || id.len() > 128
            || id.trim() != id
            || id.chars().any(char::is_control)
            || !clients.insert(id)
        {
            return Err("invalid or duplicate host script client ID".into());
        }
    }
    let mut names = HashSet::new();
    for parameter in &host.parameters {
        if parameter.name.is_empty()
            || parameter.name.len() > 64
            || !parameter
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || parameter.name.as_bytes()[0].is_ascii_digit()
            || !names.insert(parameter.name.to_ascii_lowercase())
            || parameter.description.len() > 1024
            || parameter.description.chars().any(char::is_control)
        {
            return Err("invalid or duplicate host script parameter".into());
        }
    }
    Ok(())
}

pub fn validate_host_script_parameters(
    host: &crate::HostScriptConfig,
    value: &serde_json::Value,
) -> Result<(), String> {
    use crate::HostScriptParameterKind::*;
    let object = value
        .as_object()
        .ok_or("host script parameters must be a JSON object")?;
    if serde_json::to_vec(value).map_err(|e| e.to_string())?.len() > 16 * 1024 {
        return Err("host script parameters exceed 16 KiB".into());
    }
    for key in object.keys() {
        if !host.parameters.iter().any(|p| p.name == *key) {
            return Err(format!("unknown host script parameter: {key}"));
        }
    }
    for parameter in &host.parameters {
        let Some(value) = object.get(&parameter.name) else {
            if parameter.required {
                return Err(format!("missing parameter: {}", parameter.name));
            }
            continue;
        };
        let valid = match parameter.kind {
            String => value.is_string(),
            Number => value.is_number(),
            Integer => value.is_i64() || value.is_u64(),
            Boolean => value.is_boolean(),
        };
        if !valid {
            return Err(format!("invalid parameter type: {}", parameter.name));
        }
        if value.as_str().is_some_and(|s| s.contains('\0')) {
            return Err(format!("parameter cannot contain NUL: {}", parameter.name));
        }
    }
    Ok(())
}

pub fn host_script_tool_definition(script: &CustomScript) -> Option<crate::McpToolDefinition> {
    let host = &script.host;
    let properties = host
        .parameters
        .iter()
        .map(|p| {
            (
                p.name.clone(),
                serde_json::json!({
                    "type": p.kind, "description": p.description,
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    Some(crate::McpToolDefinition {
        name: script.host_tool_name(),
        title: script.name.clone(),
        description: format!(
            "Runs on the PortMate HOST, not a terminal session. {}",
            script.description
        ),
        input_schema: serde_json::json!({"type":"object", "properties":properties,
            "required":host.parameters.iter().filter(|p| p.required).map(|p| &p.name).collect::<Vec<_>>(),
            "additionalProperties":false}),
        read_only: false,
    })
}

pub fn normalize_custom_script_content(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}

pub fn redact_custom_script_event_bodies(events: &mut [SessionEvent]) -> usize {
    let mut redacted = 0;
    for event in events {
        if event.annotations.contains_key("customScriptId")
            && event
                .text
                .as_deref()
                .is_some_and(|text| text != CUSTOM_SCRIPT_EVENT_TEXT)
        {
            event.text = Some(CUSTOM_SCRIPT_EVENT_TEXT.to_string());
            redacted += 1;
        }
    }
    redacted
}

pub fn validate_custom_script(script: &CustomScript) -> Result<(), String> {
    if Uuid::parse_str(&script.id).is_err() {
        return Err("custom script ID must be a UUID".to_string());
    }
    if script.name.is_empty()
        || script.name.chars().count() > MAX_CUSTOM_SCRIPT_NAME_CHARACTERS
        || script.name.chars().any(char::is_control)
    {
        return Err(format!(
            "custom script name must be printable and contain 1 to {MAX_CUSTOM_SCRIPT_NAME_CHARACTERS} characters"
        ));
    }
    if script.description.chars().count() > MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS
        || script.description.chars().any(char::is_control)
    {
        return Err(format!(
            "custom script description must be printable and contain at most {MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS} characters"
        ));
    }
    if script.content.trim().is_empty()
        || script.content.contains('\0')
        || script.content.chars().count() > MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS
        || script.content.len() > MAX_CUSTOM_SCRIPT_CONTENT_BYTES
    {
        return Err(format!(
            "custom script content must be non-empty, contain no NUL, and stay within {MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS} characters/{MAX_CUSTOM_SCRIPT_CONTENT_BYTES} bytes"
        ));
    }
    validate_host_script_config(&script.host)?;
    if script.mcp_enabled && script.host.allowed_client_ids.is_empty() {
        return Err("select at least one MCP client for a published host script".into());
    }
    if script.created_at > script.updated_at {
        return Err("custom script timestamps are inconsistent".to_string());
    }
    Ok(())
}

pub fn normalize_loaded_custom_scripts(scripts: Vec<CustomScript>) -> Vec<CustomScript> {
    let mut normalized = Vec::with_capacity(scripts.len().min(MAX_CUSTOM_SCRIPTS));
    let mut seen_ids = HashSet::new();
    for mut script in scripts {
        if normalized.len() >= MAX_CUSTOM_SCRIPTS {
            break;
        }
        script.name = script.name.trim().to_string();
        script.description = script.description.trim().to_string();
        script.content = normalize_custom_script_content(&script.content);
        if !seen_ids.insert(script.id.clone()) || validate_custom_script(&script).is_err() {
            continue;
        }
        normalized.push(script);
    }
    normalized
}

#[cfg(test)]
mod host_tests {
    use super::*;
    use crate::{
        HostScriptConfig, HostScriptLanguage, HostScriptParameter, HostScriptParameterKind,
    };

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
}

use crate::{CustomScript, SessionEvent};
use std::collections::HashSet;
use uuid::Uuid;

pub const MAX_CUSTOM_SCRIPTS: usize = 128;
pub const MAX_CUSTOM_SCRIPT_NAME_CHARACTERS: usize = 128;
pub const MAX_CUSTOM_SCRIPT_DESCRIPTION_CHARACTERS: usize = 1_024;
pub const MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS: usize = 65_536;
pub const MAX_CUSTOM_SCRIPT_CONTENT_BYTES: usize = 256 * 1024;
pub const MAX_CUSTOM_SCRIPT_SESSIONS: usize = 1_024;
pub const CUSTOM_SCRIPT_EVENT_TEXT: &str = "<custom-script>";

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
    if script.allowed_session_ids.len() > MAX_CUSTOM_SCRIPT_SESSIONS {
        return Err(format!(
            "custom script session limit exceeded ({MAX_CUSTOM_SCRIPT_SESSIONS})"
        ));
    }
    if !script.allow_all_sessions && script.allowed_session_ids.is_empty() && script.mcp_enabled {
        return Err("an MCP-enabled custom script must target at least one session".to_string());
    }
    let mut seen = HashSet::with_capacity(script.allowed_session_ids.len());
    for session_id in &script.allowed_session_ids {
        if session_id.is_empty()
            || session_id.len() > 128
            || session_id.chars().any(char::is_control)
        {
            return Err("custom script contains an invalid session ID".to_string());
        }
        if !seen.insert(session_id) {
            return Err("custom script contains duplicate session IDs".to_string());
        }
    }
    if script.created_at > script.updated_at {
        return Err("custom script timestamps are inconsistent".to_string());
    }
    Ok(())
}

pub fn normalize_loaded_custom_scripts(
    scripts: Vec<CustomScript>,
    known_session_ids: &HashSet<String>,
) -> Vec<CustomScript> {
    let mut normalized = Vec::with_capacity(scripts.len().min(MAX_CUSTOM_SCRIPTS));
    let mut seen_ids = HashSet::new();
    for mut script in scripts {
        if normalized.len() >= MAX_CUSTOM_SCRIPTS {
            break;
        }
        script.name = script.name.trim().to_string();
        script.description = script.description.trim().to_string();
        script.content = normalize_custom_script_content(&script.content);
        let was_scoped = !script.allow_all_sessions;
        let mut seen_sessions = HashSet::new();
        script.allowed_session_ids.retain(|session_id| {
            known_session_ids.contains(session_id) && seen_sessions.insert(session_id.clone())
        });
        if script.allow_all_sessions {
            script.allowed_session_ids.clear();
        } else if was_scoped && script.allowed_session_ids.is_empty() {
            script.mcp_enabled = false;
        }
        if !seen_ids.insert(script.id.clone()) || validate_custom_script(&script).is_err() {
            continue;
        }
        normalized.push(script);
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn script() -> CustomScript {
        let now = Utc::now();
        CustomScript {
            id: Uuid::new_v4().to_string(),
            name: "Inspect service".to_string(),
            description: "Reads service state".to_string(),
            content: "systemctl status portmate".to_string(),
            allow_all_sessions: false,
            allowed_session_ids: vec!["session-a".to_string()],
            mcp_enabled: true,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn validation_rejects_unscoped_and_oversized_scripts() {
        let mut invalid = script();
        invalid.allowed_session_ids.clear();
        assert!(validate_custom_script(&invalid).is_err());

        invalid.allow_all_sessions = true;
        invalid.content = "x".repeat(MAX_CUSTOM_SCRIPT_CONTENT_CHARACTERS + 1);
        assert!(validate_custom_script(&invalid).is_err());
    }

    #[test]
    fn loaded_scripts_never_expand_a_lost_session_scope() {
        let normalized = normalize_loaded_custom_scripts(vec![script()], &HashSet::new());
        assert_eq!(normalized.len(), 1);
        assert!(!normalized[0].allow_all_sessions);
        assert!(normalized[0].allowed_session_ids.is_empty());
        assert!(!normalized[0].mcp_enabled);
    }

    #[test]
    fn loaded_scripts_normalize_newlines_and_deduplicate_sessions() {
        let mut loaded = script();
        loaded.content = "line 1\r\nline 2\r".to_string();
        loaded.allowed_session_ids.push("session-a".to_string());
        let known = HashSet::from(["session-a".to_string()]);
        let normalized = normalize_loaded_custom_scripts(vec![loaded], &known);
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].content, "line 1\nline 2\n");
        assert_eq!(normalized[0].allowed_session_ids, ["session-a"]);
    }

    #[test]
    fn loaded_custom_script_events_drop_persisted_bodies() {
        let mut events = vec![crate::SessionEvent {
            id: Uuid::new_v4().to_string(),
            session_id: "session-a".to_string(),
            pane_id: "session-a:main".to_string(),
            ts: Utc::now(),
            direction: crate::EventDirection::Outbound,
            stream: crate::EventStream::Stdout,
            bytes_ref: Some("raw:0:27".to_string()),
            text: Some("private-script-body-marker".to_string()),
            annotations: std::collections::BTreeMap::from([(
                "customScriptId".to_string(),
                Uuid::new_v4().to_string(),
            )]),
        }];

        assert_eq!(redact_custom_script_event_bodies(&mut events), 1);
        assert_eq!(events[0].text.as_deref(), Some(CUSTOM_SCRIPT_EVENT_TEXT));
        assert_eq!(redact_custom_script_event_bodies(&mut events), 0);
        assert!(events[0].bytes_ref.is_some());
    }
}

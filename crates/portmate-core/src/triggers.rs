use crate::models::{TriggerAction, TriggerMatcher, TriggerSpec};
use regex::RegexBuilder;
use std::collections::HashSet;

pub const MAX_TRIGGERS_PER_PROFILE: usize = 64;
pub const MAX_TRIGGER_ACTIONS: usize = 16;
pub const MAX_TRIGGER_ID_CHARACTERS: usize = 128;
pub const MAX_TRIGGER_LABEL_CHARACTERS: usize = 128;
pub const MAX_TRIGGER_MATCHER_CHARACTERS: usize = 1_024;
pub const MAX_TRIGGER_ACTION_VALUE_CHARACTERS: usize = 4_096;
const MAX_TRIGGER_REGEX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerMatch {
    pub trigger_id: String,
    pub label: String,
    pub actions: Vec<TriggerAction>,
}

pub fn evaluate_triggers(triggers: &[TriggerSpec], line: &str) -> Vec<TriggerMatch> {
    triggers
        .iter()
        .take(MAX_TRIGGERS_PER_PROFILE)
        .filter(|trigger| {
            trigger.enabled
                && validate_trigger_metadata(trigger).is_ok()
                && matcher_hits(&trigger.matcher, line)
                && validate_trigger_actions(trigger).is_ok()
        })
        .map(|trigger| TriggerMatch {
            trigger_id: trigger.id.clone(),
            label: trigger.label.clone(),
            actions: trigger.actions.clone(),
        })
        .collect()
}

pub fn validate_triggers(triggers: &[TriggerSpec]) -> Result<(), String> {
    if triggers.len() > MAX_TRIGGERS_PER_PROFILE {
        return Err(format!("trigger count exceeds {MAX_TRIGGERS_PER_PROFILE}"));
    }
    let mut ids = HashSet::with_capacity(triggers.len());
    for (index, trigger) in triggers.iter().enumerate() {
        validate_trigger(trigger).map_err(|error| format!("trigger {}: {error}", index + 1))?;
        if !ids.insert(trigger.id.as_str()) {
            return Err(format!("trigger {}: duplicate id", index + 1));
        }
    }
    Ok(())
}

pub fn normalize_triggers(triggers: Vec<TriggerSpec>) -> Vec<TriggerSpec> {
    let mut normalized = Vec::with_capacity(triggers.len().min(MAX_TRIGGERS_PER_PROFILE));
    let mut ids = HashSet::with_capacity(normalized.capacity());
    for trigger in triggers.into_iter().take(MAX_TRIGGERS_PER_PROFILE) {
        if validate_trigger(&trigger).is_ok() && ids.insert(trigger.id.clone()) {
            normalized.push(trigger);
        }
    }
    normalized
}

fn validate_trigger(trigger: &TriggerSpec) -> Result<(), String> {
    validate_trigger_metadata(trigger)?;
    validate_matcher(&trigger.matcher).and_then(|()| validate_trigger_actions(trigger))
}

fn validate_trigger_metadata(trigger: &TriggerSpec) -> Result<(), String> {
    validate_text("id", &trigger.id, MAX_TRIGGER_ID_CHARACTERS, false)?;
    if trigger.id.trim() != trigger.id {
        return Err("id must not have surrounding whitespace".to_string());
    }
    if trigger.id.chars().any(char::is_control) {
        return Err("id must not contain control characters".to_string());
    }
    validate_text("label", &trigger.label, MAX_TRIGGER_LABEL_CHARACTERS, true)?;
    if trigger.label.chars().any(char::is_control) {
        return Err("label must not contain control characters".to_string());
    }
    Ok(())
}

fn validate_trigger_actions(trigger: &TriggerSpec) -> Result<(), String> {
    if trigger.actions.len() > MAX_TRIGGER_ACTIONS {
        return Err(format!("action count exceeds {MAX_TRIGGER_ACTIONS}"));
    }
    for (index, action) in trigger.actions.iter().enumerate() {
        validate_action(action).map_err(|error| format!("action {}: {error}", index + 1))?;
    }
    Ok(())
}

fn validate_matcher(matcher: &TriggerMatcher) -> Result<(), String> {
    validate_matcher_shape(matcher)?;
    if let TriggerMatcher::Regex { pattern } = matcher {
        build_trigger_regex(pattern).map(|_| ())?;
    }
    Ok(())
}

fn validate_matcher_shape(matcher: &TriggerMatcher) -> Result<(), String> {
    match matcher {
        TriggerMatcher::Contains { text, .. } => validate_text(
            "contains matcher",
            text,
            MAX_TRIGGER_MATCHER_CHARACTERS,
            false,
        ),
        TriggerMatcher::Regex { pattern } => validate_text(
            "regex matcher",
            pattern,
            MAX_TRIGGER_MATCHER_CHARACTERS,
            false,
        ),
    }
}

fn validate_action(action: &TriggerAction) -> Result<(), String> {
    match action {
        TriggerAction::Highlight { color } => {
            if color.len() != 7
                || !color.starts_with('#')
                || !color[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err("highlight color must use #RRGGBB".to_string());
            }
            Ok(())
        }
        TriggerAction::Sound { name } => {
            if matches!(name.as_str(), "bell" | "chime" | "alert") {
                Ok(())
            } else {
                Err("sound must be bell, chime, or alert".to_string())
            }
        }
        TriggerAction::LocalCommand { command } => {
            validate_text(
                "local command",
                command,
                MAX_TRIGGER_ACTION_VALUE_CHARACTERS,
                false,
            )?;
            if command.trim().is_empty() {
                Err("local command must not be blank".to_string())
            } else {
                Ok(())
            }
        }
        TriggerAction::SendText { text } => {
            validate_text("send text", text, MAX_TRIGGER_ACTION_VALUE_CHARACTERS, true)
        }
        TriggerAction::Notification { message } => validate_text(
            "notification",
            message,
            MAX_TRIGGER_ACTION_VALUE_CHARACTERS,
            true,
        ),
        TriggerAction::TimelineMark { label } => validate_text(
            "timeline label",
            label,
            MAX_TRIGGER_ACTION_VALUE_CHARACTERS,
            true,
        ),
        TriggerAction::CustomLink { url_template } => validate_text(
            "custom link",
            url_template,
            MAX_TRIGGER_ACTION_VALUE_CHARACTERS,
            true,
        ),
    }
}

fn validate_text(
    label: &str,
    value: &str,
    max_characters: usize,
    allow_empty: bool,
) -> Result<(), String> {
    if !allow_empty && value.is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if value.contains('\0') {
        return Err(format!("{label} must not contain NUL"));
    }
    if value.chars().count() > max_characters {
        return Err(format!("{label} exceeds {max_characters} characters"));
    }
    Ok(())
}

fn build_trigger_regex(pattern: &str) -> Result<regex::Regex, String> {
    RegexBuilder::new(pattern)
        .size_limit(MAX_TRIGGER_REGEX_BYTES)
        .build()
        .map_err(|error| format!("invalid regex matcher: {error}"))
}

fn matcher_hits(matcher: &TriggerMatcher, line: &str) -> bool {
    if validate_matcher_shape(matcher).is_err() {
        return false;
    }
    match matcher {
        TriggerMatcher::Contains {
            text,
            case_sensitive,
        } => {
            if *case_sensitive {
                line.contains(text)
            } else {
                line.to_lowercase().contains(&text.to_lowercase())
            }
        }
        TriggerMatcher::Regex { pattern } => build_trigger_regex(pattern)
            .map(|regex| regex.is_match(line))
            .unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_contains_and_regex() {
        let triggers = vec![
            TriggerSpec {
                id: "panic".to_string(),
                label: "Kernel panic".to_string(),
                matcher: TriggerMatcher::Contains {
                    text: "kernel panic".to_string(),
                    case_sensitive: false,
                },
                actions: vec![TriggerAction::TimelineMark {
                    label: "panic".to_string(),
                }],
                enabled: true,
            },
            TriggerSpec {
                id: "ip".to_string(),
                label: "IP".to_string(),
                matcher: TriggerMatcher::Regex {
                    pattern: r"\b\d{1,3}(\.\d{1,3}){3}\b".to_string(),
                },
                actions: vec![TriggerAction::Highlight {
                    color: "#58a6ff".to_string(),
                }],
                enabled: true,
            },
        ];
        assert_eq!(
            evaluate_triggers(&triggers, "KERNEL PANIC at 10.0.0.1").len(),
            2
        );
    }

    fn test_trigger(id: impl Into<String>) -> TriggerSpec {
        TriggerSpec {
            id: id.into(),
            label: "Test".to_string(),
            matcher: TriggerMatcher::Contains {
                text: "match".to_string(),
                case_sensitive: true,
            },
            actions: vec![TriggerAction::TimelineMark {
                label: "mark".to_string(),
            }],
            enabled: true,
        }
    }

    #[test]
    fn validates_trigger_counts_identifiers_regexes_and_actions() {
        let trigger = test_trigger("one");
        validate_triggers(std::slice::from_ref(&trigger)).unwrap();

        let duplicate = vec![trigger.clone(), trigger.clone()];
        assert_eq!(
            validate_triggers(&duplicate).unwrap_err(),
            "trigger 2: duplicate id"
        );

        let mut invalid_regex = trigger.clone();
        invalid_regex.matcher = TriggerMatcher::Regex {
            pattern: "(".to_string(),
        };
        assert!(validate_triggers(&[invalid_regex])
            .unwrap_err()
            .contains("invalid regex matcher"));

        let mut blank_command = trigger;
        blank_command.actions = vec![TriggerAction::LocalCommand {
            command: "  ".to_string(),
        }];
        assert!(validate_triggers(&[blank_command])
            .unwrap_err()
            .contains("local command must not be blank"));
    }

    #[test]
    fn runtime_evaluation_and_loaded_normalization_fail_closed_at_limits() {
        let triggers = (0..=MAX_TRIGGERS_PER_PROFILE)
            .map(|index| test_trigger(format!("trigger-{index}")))
            .collect::<Vec<_>>();
        assert!(validate_triggers(&triggers).is_err());
        assert_eq!(
            normalize_triggers(triggers.clone()).len(),
            MAX_TRIGGERS_PER_PROFILE
        );
        assert_eq!(
            evaluate_triggers(&triggers, "match").len(),
            MAX_TRIGGERS_PER_PROFILE
        );

        let mut oversized = test_trigger("oversized");
        oversized.actions = vec![TriggerAction::SendText {
            text: "x".repeat(MAX_TRIGGER_ACTION_VALUE_CHARACTERS + 1),
        }];
        assert!(normalize_triggers(vec![oversized.clone()]).is_empty());
        assert!(evaluate_triggers(&[oversized], "match").is_empty());

        let mut oversized_matcher = test_trigger("oversized-matcher");
        oversized_matcher.matcher = TriggerMatcher::Contains {
            text: "x".repeat(MAX_TRIGGER_MATCHER_CHARACTERS + 1),
            case_sensitive: false,
        };
        assert!(evaluate_triggers(&[oversized_matcher], "match").is_empty());
    }
}

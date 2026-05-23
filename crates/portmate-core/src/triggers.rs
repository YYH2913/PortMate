use crate::models::{TriggerAction, TriggerMatcher, TriggerSpec};
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerMatch {
    pub trigger_id: String,
    pub label: String,
    pub actions: Vec<TriggerAction>,
}

pub fn evaluate_triggers(triggers: &[TriggerSpec], line: &str) -> Vec<TriggerMatch> {
    triggers
        .iter()
        .filter(|trigger| trigger.enabled && matcher_hits(&trigger.matcher, line))
        .map(|trigger| TriggerMatch {
            trigger_id: trigger.id.clone(),
            label: trigger.label.clone(),
            actions: trigger.actions.clone(),
        })
        .collect()
}

fn matcher_hits(matcher: &TriggerMatcher, line: &str) -> bool {
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
        TriggerMatcher::Regex { pattern } => Regex::new(pattern)
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
}

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

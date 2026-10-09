use super::*;

#[test]
fn redacts_common_secret_shapes() {
    let text = "password=hunter2 token: abc123 normal";
    let redacted = redact_secrets(text);
    assert!(!redacted.contains("hunter2"));
    assert!(!redacted.contains("abc123"));
    assert!(!redacted.contains("normal"));
}

#[test]
fn redacts_json_credentials_and_complete_bearer_tokens() {
    let text = r#"{"token":"abc123","password":"hunter2"} Authorization: Bearer abc+/DEF_123=-"#;

    let redacted = redact_secrets(text);

    assert_eq!(
        redacted,
        r#"{"token":"<redacted>","password":"<redacted>"} Authorization: Bearer <redacted>"#
    );
    assert!(!redacted.contains("abc123"));
    assert!(!redacted.contains("hunter2"));
    assert!(!redacted.contains("DEF_123"));
}

#[test]
fn redacts_credentials_with_spaces_without_leaving_a_suffix() {
    let text = r#"password=multi word secret token:alpha beta password="quoted value with spaces""#;
    let redacted = redact_secrets(text);
    assert!(!redacted.contains("multi word"));
    assert!(!redacted.contains("alpha beta"));
    assert_eq!(
        redacted,
        r#"password=<redacted> token:<redacted> password="<redacted>""#
    );
}

#[test]
fn redacts_comma_containing_quoted_and_unquoted_values_without_suffixes() {
    for (text, expected) in [
        (
            "password=\"a,b\" token=ok",
            "password=\"<redacted>\" token=<redacted>",
        ),
        (
            "password='a,b' token=ok",
            "password='<redacted>' token=<redacted>",
        ),
        (
            "password=a,b token=ok",
            "password=<redacted> token=<redacted>",
        ),
        (
            "password=foo, token=ok",
            "password=<redacted>, token=<redacted>",
        ),
    ] {
        assert_eq!(redact_secrets(text), expected);
    }
}

#[test]
fn transfer_redaction_removes_both_paths_without_mutating_the_source() {
    let transfer = TransferTask {
        id: "transfer-1".to_string(),
        session_id: "session-1".to_string(),
        protocol: crate::models::TransferProtocol::Sftp,
        source: "/home/operator/private-source".to_string(),
        destination: "remote:/srv/private-target".to_string(),
        bytes_total: 12,
        bytes_done: 6,
        status: crate::models::TransferStatus::Running,
        message: Some("token=transfer-secret".to_string()),
        started_at: None,
        finished_at: None,
        average_bytes_per_second: None,
    };

    let redacted = redact_transfer_task(transfer.clone());

    assert_eq!(redacted.source, "<redacted-path>");
    assert_eq!(redacted.destination, "<redacted-path>");
    assert_eq!(redacted.message.as_deref(), Some("running"));
    assert_eq!(transfer.source, "/home/operator/private-source");
    assert_eq!(transfer.destination, "remote:/srv/private-target");
}

#[test]
fn event_redaction_never_returns_a_custom_script_body() {
    let event = SessionEvent {
        id: "event-1".to_string(),
        session_id: "session-1".to_string(),
        pane_id: "session-1:main".to_string(),
        ts: chrono::Utc::now(),
        direction: crate::models::EventDirection::Outbound,
        stream: crate::models::EventStream::Stdout,
        bytes_ref: Some("raw:0:27".to_string()),
        text: Some("private-script-body-marker".to_string()),
        annotations: std::collections::BTreeMap::from([(
            "customScriptId".to_string(),
            "69c06a07-dc48-4d4e-9498-6f42b6deab21".to_string(),
        )]),
    };

    let redacted = redact_session_event(event);
    assert_eq!(
        redacted.text.as_deref(),
        Some(crate::CUSTOM_SCRIPT_EVENT_TEXT)
    );
    assert!(redacted.bytes_ref.is_none());
}

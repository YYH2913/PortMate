use crate::models::{
    AuditRecord, ConnectionConfig, SessionEvent, SessionSummary, SysmonSnapshot, TimelineMark,
    TransferTask, TriggerAction, TriggerMatcher,
};
use regex::Regex;
use std::sync::OnceLock;

fn secret_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        vec![
            Regex::new(r"(?i)(bearer\s+)([a-z0-9._~+/=-]+)").unwrap(),
            Regex::new(
                r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
            )
            .unwrap(),
        ]
    })
}

fn redact_key_value_secrets(input: &str) -> String {
    static KEY_PREFIX: OnceLock<Regex> = OnceLock::new();
    let key_prefix = KEY_PREFIX.get_or_init(|| {
        Regex::new(
            r#"(?i)["']?(?:password|passwd|pwd|token|api[_-]?key|secret)["']?\s*[:=]\s*"#,
        )
        .unwrap()
    });
    let matches = key_prefix.find_iter(input).collect::<Vec<_>>();
    let mut output = input.to_string();
    for (index, matched) in matches.iter().enumerate().rev() {
        let value_start = matched.end();
        let next_key = matches.get(index + 1).is_some();
        let mut value_limit = matches
            .get(index + 1)
            .map(|next| next.start())
            .unwrap_or(input.len());
        if let Some(offset) = input[value_start..value_limit]
            .find(['\r', '\n'])
        {
            value_limit = value_start + offset;
        }
        if value_start >= value_limit {
            continue;
        }
        let mut content_start = value_start;
        while content_start < value_limit {
            let character = input[content_start..].chars().next().unwrap();
            if !character.is_whitespace() {
                break;
            }
            content_start += character.len_utf8();
        }
        let value = &input[content_start..value_limit];
        if let Some(quote) = value.chars().next().filter(|quote| *quote == '"' || *quote == '\'') {
            let mut escaped = false;
            let mut close = None;
            for (offset, character) in value[quote.len_utf8()..].char_indices() {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == quote {
                    close = Some(offset + quote.len_utf8());
                    break;
                }
            }
            if let Some(close) = close {
                let content_end = content_start + close;
                let replacement_start = content_start + quote.len_utf8();
                output.replace_range(replacement_start..content_end, "<redacted>");
                continue;
            }
        }
        let value_end = if next_key {
            let trimmed_end = input[value_start..value_limit].trim_end().len() + value_start;
            let delimiter_start = input[value_start..trimmed_end]
                .char_indices()
                .rev()
                .find_map(|(offset, character)| {
                    matches!(character, ',' | ';').then_some(value_start + offset)
                })
                .filter(|delimiter| input[*delimiter + 1..trimmed_end].trim().is_empty())
                .unwrap_or(trimmed_end);
            delimiter_start
        } else {
            value_limit
        };
        if value_end > value_start {
            output.replace_range(value_start..value_end, "<redacted>");
        }
    }
    output
}

pub fn redact_secrets(input: &str) -> String {
    secret_patterns()
        .iter()
        .fold(redact_key_value_secrets(input), |mut acc, pattern| {
            loop {
                let next = pattern
                    .replace_all(&acc, |caps: &regex::Captures| match caps.len() {
                        3 => format!("{}<redacted>", &caps[1]),
                    _ => "<redacted-secret>".to_string(),
                    })
                    .to_string();
                if next == acc {
                    break;
                }
                acc = next;
            }
            acc
        })
}

pub fn redact_session_summary(mut summary: SessionSummary) -> SessionSummary {
    summary.last_line = summary.last_line.map(|text| redact_secrets(&text));
    summary.runtime.cwd = None;
    summary.runtime.last_disconnect_reason = summary
        .runtime
        .last_disconnect_reason
        .map(|reason| redact_secrets(&reason));
    summary.profile.logging.path_template = "<redacted-path-template>".to_string();
    summary.profile.transfer.default_local_dir = None;
    for trigger in &mut summary.profile.triggers {
        trigger.label = redact_secrets(&trigger.label);
        match &mut trigger.matcher {
            TriggerMatcher::Contains { text, .. } => *text = redact_secrets(text),
            TriggerMatcher::Regex { pattern } => *pattern = redact_secrets(pattern),
        }
        for action in &mut trigger.actions {
            match action {
                TriggerAction::SendText { text } => *text = "<redacted>".to_string(),
                TriggerAction::LocalCommand { command } => *command = "<redacted>".to_string(),
                TriggerAction::CustomLink { url_template } => {
                    *url_template = "<redacted-url-template>".to_string();
                }
                TriggerAction::Notification { message } => {
                    *message = redact_secrets(message);
                }
                TriggerAction::TimelineMark { label } => *label = redact_secrets(label),
                TriggerAction::Highlight { .. } | TriggerAction::Sound { .. } => {}
            }
        }
    }
    match &mut summary.profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            ssh.password_secret_ref = None;
            ssh.passphrase_secret_ref = None;
            ssh.proxy.password_secret_ref = None;
            for identity in &mut ssh.identity_refs {
                identity.path = None;
                identity.secret_ref = None;
            }
            for jump in &mut ssh.jumps {
                jump.password_secret_ref = None;
                jump.passphrase_secret_ref = None;
            }
        }
        ConnectionConfig::Telnet(tcp) | ConnectionConfig::Tcp(tcp) => {
            tcp.proxy.password_secret_ref = None;
        }
        ConnectionConfig::Shell(shell) => {
            shell.cwd = None;
            shell.args.clear();
        }
        ConnectionConfig::Serial(_) => {}
    }
    summary
}

pub fn redact_session_event(mut event: SessionEvent) -> SessionEvent {
    crate::redact_custom_script_event_bodies(std::slice::from_mut(&mut event));
    event.text = event.text.take().map(|text| redact_secrets(&text));
    event.bytes_ref = None;
    for value in event.annotations.values_mut() {
        *value = redact_secrets(value);
    }
    event
}

pub fn redact_session_events(events: Vec<SessionEvent>) -> Vec<SessionEvent> {
    events.into_iter().map(redact_session_event).collect()
}

pub fn redact_timeline_marks(mut marks: Vec<TimelineMark>) -> Vec<TimelineMark> {
    for mark in &mut marks {
        mark.label = redact_secrets(&mark.label);
        mark.details = mark.details.take().map(|details| redact_secrets(&details));
    }
    marks
}

pub fn redact_transfer_task(mut transfer: TransferTask) -> TransferTask {
    transfer.source = "<redacted-path>".to_string();
    transfer.destination = "<redacted-path>".to_string();
    transfer.message = transfer.message.as_ref().map(|_| {
        match transfer.status {
            crate::models::TransferStatus::Queued => "queued",
            crate::models::TransferStatus::Running => "running",
            crate::models::TransferStatus::Completed => "completed",
            crate::models::TransferStatus::Failed => "failed",
            crate::models::TransferStatus::Cancelled => "cancelled",
        }
        .to_string()
    });
    transfer
}

pub fn redact_audit_records(mut records: Vec<AuditRecord>) -> Vec<AuditRecord> {
    for record in &mut records {
        for value in record.details.values_mut() {
            *value = redact_secrets(value);
        }
    }
    records
}

pub fn redact_sysmon_snapshot(mut snapshot: SysmonSnapshot) -> SysmonSnapshot {
    for process in &mut snapshot.processes {
        process.name = "<redacted-process>".to_string();
    }
    for disk in &mut snapshot.disks {
        disk.filesystem = "<redacted-filesystem>".to_string();
        disk.mount_point = "<redacted-mount-point>".to_string();
    }
    for interface in &mut snapshot.network_interfaces {
        interface.name = "<redacted-interface>".to_string();
        interface.addresses = interface
            .addresses
            .iter()
            .map(|_| "<redacted-address>".to_string())
            .collect();
    }
    snapshot
}

#[cfg(test)]
mod tests {
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
        let text =
            r#"{"token":"abc123","password":"hunter2"} Authorization: Bearer abc+/DEF_123=-"#;

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
}

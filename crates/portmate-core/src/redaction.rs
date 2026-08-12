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
            Regex::new(
                r#"(?i)(["']?(?:password|passwd|pwd|token|api[_-]?key|secret)["']?\s*[:=]\s*["']?)([^\s"']+)"#,
            )
            .unwrap(),
            Regex::new(r"(?i)(bearer\s+)([a-z0-9._~+/=-]+)").unwrap(),
            Regex::new(
                r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
            )
            .unwrap(),
        ]
    })
}

pub fn redact_secrets(input: &str) -> String {
    secret_patterns()
        .iter()
        .fold(input.to_string(), |acc, pattern| {
            pattern
                .replace_all(&acc, |caps: &regex::Captures| {
                    if caps.len() > 2 {
                        format!("{}<redacted>", &caps[1])
                    } else {
                        "<redacted-secret>".to_string()
                    }
                })
                .to_string()
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
        assert!(redacted.contains("normal"));
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
}

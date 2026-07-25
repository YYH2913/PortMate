use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use portmate_core::{EventDirection, EventStream, OneKeyCredential, SessionStore};
use regex::Regex;

use super::OneKeyField;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum DetectedOneKeyPrompt {
    Username,
    Password { username_hint: Option<String> },
}

pub(super) struct OneKeyPromptValidation {
    pub(super) one_key_id: String,
    pub(super) one_key_updated_at: DateTime<Utc>,
    pub(super) field: OneKeyField,
    pub(super) prompt_event_id: String,
}

pub(super) fn validate_one_key_prompt_completion(
    store: &SessionStore,
    one_key: &OneKeyCredential,
    session_id: &str,
    field: OneKeyField,
    prompt_event_id: &str,
) -> Result<(), String> {
    let prompt = one_key_prompt_at_event(store, session_id, prompt_event_id)?;
    match (field, prompt) {
        (OneKeyField::Username, DetectedOneKeyPrompt::Username) => Ok(()),
        (OneKeyField::Password, DetectedOneKeyPrompt::Password { username_hint }) => {
            if username_hint
                .as_deref()
                .is_some_and(|username| username != one_key.username)
            {
                return Err("OneKey 用户名与终端密码提示不匹配".to_string());
            }
            Ok(())
        }
        (OneKeyField::Passphrase, _) => Err("终端提示补全不支持发送私钥口令".to_string()),
        _ => Err("OneKey 字段与当前终端提示不匹配".to_string()),
    }
}

fn one_key_prompt_at_event(
    store: &SessionStore,
    session_id: &str,
    prompt_event_id: &str,
) -> Result<DetectedOneKeyPrompt, String> {
    let mut raw = String::new();
    let mut found = false;
    for event in store
        .events
        .iter()
        .filter(|event| event.session_id == session_id)
    {
        if found {
            if matches!(
                event.direction,
                EventDirection::Inbound | EventDirection::Outbound
            ) {
                return Err("终端提示已变化，请等待新的 OneKey 补全提示".to_string());
            }
            continue;
        }
        if event.direction == EventDirection::Outbound {
            raw.clear();
        } else if event.direction == EventDirection::Inbound
            && matches!(event.stream, EventStream::Stdout | EventStream::Stderr)
        {
            if let Some(text) = &event.text {
                raw.push_str(text);
                raw = raw
                    .chars()
                    .rev()
                    .take(MAX_ONE_KEY_PROMPT_BUFFER_CHARACTERS)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
            }
        }
        if event.id == prompt_event_id {
            if event.direction != EventDirection::Inbound
                || !matches!(event.stream, EventStream::Stdout | EventStream::Stderr)
                || event.text.as_deref().is_none_or(str::is_empty)
            {
                return Err("promptEventId 不是有效的终端入站提示事件".to_string());
            }
            found = true;
        }
    }
    if !found {
        return Err("终端提示已不存在，请等待新的 OneKey 补全提示".to_string());
    }
    detect_one_key_terminal_prompt(&raw)
        .ok_or_else(|| "promptEventId 不再匹配 OneKey 用户名或密码提示".to_string())
}

const MAX_ONE_KEY_PROMPT_BUFFER_CHARACTERS: usize = 1024;

pub(super) fn detect_one_key_terminal_prompt(raw: &str) -> Option<DetectedOneKeyPrompt> {
    static PASSWORD_CHANGE: OnceLock<Regex> = OnceLock::new();
    static PASSWORD_FOR: OnceLock<Regex> = OnceLock::new();
    static OPENSSH_PASSWORD: OnceLock<Regex> = OnceLock::new();
    static PASSWORD: OnceLock<Regex> = OnceLock::new();
    static USERNAME: OnceLock<Regex> = OnceLock::new();

    let display = sanitize_terminal_prompt_text(raw)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let line = display.rsplit('\n').next()?.trim_end();
    if line.is_empty() {
        return None;
    }
    let password_change = PASSWORD_CHANGE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(?:new|retype|repeat|confirm)\s+(?:new\s+)?password(?:\s+for\s+\S+)?\s*:\s*$",
        )
        .expect("valid OneKey regex")
    });
    if password_change.is_match(line) {
        return None;
    }
    let password_for = PASSWORD_FOR.get_or_init(|| {
        Regex::new(r"(?i)\bpassword\s+for\s+([^\s:]+)\s*:\s*$").expect("valid OneKey regex")
    });
    if let Some(captures) = password_for.captures(line) {
        return Some(DetectedOneKeyPrompt::Password {
            username_hint: captures.get(1).map(|value| value.as_str().to_string()),
        });
    }
    let openssh_password = OPENSSH_PASSWORD.get_or_init(|| {
        Regex::new(r"(?i)(?:^|\s)([^\s@]+)@\S+(?:'s)?\s+password\s*:\s*$")
            .expect("valid OneKey regex")
    });
    if let Some(captures) = openssh_password.captures(line) {
        return Some(DetectedOneKeyPrompt::Password {
            username_hint: captures.get(1).map(|value| value.as_str().to_string()),
        });
    }
    let password =
        PASSWORD.get_or_init(|| Regex::new(r"(?i)\bpassword\s*:\s*$").expect("valid OneKey regex"));
    if password.is_match(line) {
        return Some(DetectedOneKeyPrompt::Password {
            username_hint: None,
        });
    }
    let username = USERNAME.get_or_init(|| {
        Regex::new(r"(?i)\b(?:username|login)\s*:\s*$").expect("valid OneKey regex")
    });
    username
        .is_match(line)
        .then_some(DetectedOneKeyPrompt::Username)
}

fn sanitize_terminal_prompt_text(raw: &str) -> String {
    let mut output = Vec::new();
    let mut characters = raw.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\u{1b}' {
            match characters.next() {
                Some('[') => {
                    for value in characters.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&value) {
                            break;
                        }
                    }
                }
                Some(']') | Some('P') | Some('^') | Some('_') => {
                    let mut escaped = false;
                    for value in characters.by_ref() {
                        if value == '\u{7}' || (escaped && value == '\\') {
                            break;
                        }
                        escaped = value == '\u{1b}';
                    }
                }
                Some(_) | None => {}
            }
            continue;
        }
        if matches!(character, '\u{8}' | '\u{7f}') {
            if !matches!(output.last(), Some('\n' | '\r')) {
                output.pop();
            }
            continue;
        }
        if !character.is_control() || matches!(character, '\n' | '\r' | '\t') {
            output.push(character);
        }
    }
    output.into_iter().collect()
}

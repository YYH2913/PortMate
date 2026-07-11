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
}

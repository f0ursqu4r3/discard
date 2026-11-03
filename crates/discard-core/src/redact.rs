//! Secret redaction for logs and for error text shown to the user.

use regex::Regex;
use std::sync::LazyLock;

const MASK: &str = "[REDACTED]";

/// Each rule keeps the first capture group (if any) and masks the rest.
static RULES: LazyLock<Vec<(Regex, String)>> = LazyLock::new(|| {
    let rule = |re: &str, rep: &str| (Regex::new(re).expect("valid redaction regex"), rep.to_string());
    vec![
        rule(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._\-+/=~]{6,}", &format!("$1 {MASK}")),
        rule(r"\bsk-[A-Za-z0-9_\-]{16,}", MASK),
        rule(r"\bgh[pousr]_[A-Za-z0-9]{20,}", MASK),
        rule(r"\bxox[baprs]-[A-Za-z0-9\-]{10,}", MASK),
        rule(
            r#"(?i)("?\b(?:api[_-]?key|token|secret|password|passwd|authorization)\b"?\s*[:=]\s*)("[^"]*"|'[^']*'|[^\s,;}]+)"#,
            &format!("${{1}}{MASK}"),
        ),
    ]
});

/// Replace likely secrets with `[REDACTED]`. Conservative: it prefers a false positive to a leak.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for (re, rep) in RULES.iter() {
        out = re.replace_all(&out, rep.as_str()).into_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_bearer_tokens() {
        assert_eq!(redact("Authorization failed for Bearer abc.DEF-123_x"), "Authorization failed for Bearer [REDACTED]");
    }

    #[test]
    fn redacts_api_keys_by_shape() {
        let out = redact("key sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAA here");
        assert!(!out.contains("AAAA"), "{out}");
        assert!(out.contains("[REDACTED]"));
    }

    #[test]
    fn redacts_vendor_tokens_by_shape() {
        for t in ["ghp_abcdefghijklmnopqrstuvwxyz0123", "xoxb-1234567890-abcdefghij"] {
            assert!(!redact(&format!("t={t} end")).contains(t), "{t}");
        }
    }

    #[test]
    fn redacts_key_value_pairs() {
        assert_eq!(redact("password=hunter2 user=bob"), "password=[REDACTED] user=bob");
        assert_eq!(redact("api_key: abc123"), "api_key: [REDACTED]");
        assert_eq!(redact("TOKEN = 'abc def'"), "TOKEN = [REDACTED]");
    }

    #[test]
    fn redacts_json_style_pairs() {
        assert_eq!(redact(r#"{"token":"abc","n":1}"#), r#"{"token":[REDACTED],"n":1}"#);
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for t in ["No conversation found with session ID: abc", "used 5 tokens", "tokenizer ready", "ls -la /tmp"] {
            assert_eq!(redact(t), t);
        }
    }

    #[test]
    fn redacts_every_occurrence() {
        let out = redact("secret=a secret=b");
        assert_eq!(out, "secret=[REDACTED] secret=[REDACTED]");
    }
}

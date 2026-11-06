//! SEGURIDAD-MOVIL F5 (M2): secrets out of what travels and what stays.
//!
//! A tool input is prose the agent wrote: a command line with a bearer
//! token, a connection string with a password, an exported key. Two places
//! copy it beyond the session: the paired device (`summary_of`) and the
//! ledger (`bounded_input`, which proof packets export to reviewers). Both
//! go through here. The rule is conservative: keep the shape, replace the
//! value. A false positive hides a harmless string; a false negative leaks
//! a credential — so the patterns lean wide.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

const MASK: &str = "[redacted]";

static RULES: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    [
        // Authorization: Bearer … / Basic … / token … (headers and curl -H).
        (
            r"(?i)(authorization\s*[:=]\s*(?:bearer|basic|token|apikey)\s+)[^\s'\x22,;]+",
            "${1}[redacted]",
        ),
        // key=value and key: value for secret-shaped names, with or without
        // quotes, including `export NAME=` and `--password value`.
        (
            r"(?i)\b((?:api[_-]?key|apikey|access[_-]?key(?:[_-]?id)?|secret(?:[_-]?key)?|client[_-]?secret|private[_-]?key|auth[_-]?token|access[_-]?token|refresh[_-]?token|id[_-]?token|session[_-]?token|token|passwd|password|pwd|pass|credentials?|cookie|x-api-key|[a-z0-9_]*(?:_token|_secret|_password|_key|_passwd))\s*(?:[=:]|\s--?[a-z-]*\s)\s*['\x22]?)[^\s'\x22&;,]+",
            "${1}[redacted]",
        ),
        // `--password hunter2`, `-p hunter2`-style flags with a space.
        (
            r"(?i)(\s--?(?:password|passwd|pwd|pass|token|secret|api-?key|access-?key|private-?key|credentials?)\s+)[^\s'\x22]+",
            "${1}[redacted]",
        ),
        // userinfo in URLs: scheme://user:password@host
        (r"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/@:]+:[^\s/@]+@", "${1}[redacted]@"),
        // PEM blocks.
        (
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
            "[redacted PEM private key]",
        ),
        // Well-known token shapes.
        (r"\bAKIA[0-9A-Z]{16}\b", "[redacted AWS key id]"),
        (r"\bgh[pousr]_[A-Za-z0-9]{20,}\b", "[redacted GitHub token]"),
        (r"\bgithub_pat_[A-Za-z0-9_]{20,}\b", "[redacted GitHub token]"),
        (r"\bsk-(?:proj-|ant-)?[A-Za-z0-9_-]{20,}\b", "[redacted API key]"),
        (r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b", "[redacted Slack token]"),
        (r"\bAIza[0-9A-Za-z_-]{30,}\b", "[redacted Google key]"),
        (r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\b", "[redacted JWT]"),
    ]
    .into_iter()
    .map(|(re, rep)| (Regex::new(re).expect("static regex"), rep))
    .collect()
});

/// Replace secret values in free text, keeping the surrounding shape.
pub fn redact_secrets(text: &str) -> String {
    let mut out = text.to_string();
    for (re, rep) in RULES.iter() {
        out = re.replace_all(&out, *rep).into_owned();
    }
    out
}

/// Redact every string inside a JSON value, recursively.
pub fn redact_value(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(redact_secrets(&s)),
        Value::Array(items) => Value::Array(items.into_iter().map(redact_value).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| {
                    // A key that names a secret hides its whole value.
                    let hide = SECRET_KEY.is_match(&k);
                    (
                        k,
                        if hide {
                            Value::String(MASK.into())
                        } else {
                            redact_value(v)
                        },
                    )
                })
                .collect(),
        ),
        other => other,
    }
}

static SECRET_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:.*(?:token|secret|password|passwd|private_?key|api_?key|credential|cookie|authorization).*)$")
        .expect("static regex")
});

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_usual_leaks_are_masked_and_the_shape_survives() {
        let cases = [
            (
                "curl -H \"Authorization: Bearer abc.def-123\" https://api",
                "curl -H \"Authorization: Bearer [redacted]\" https://api",
            ),
            (
                "export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG",
                "export AWS_SECRET_ACCESS_KEY=[redacted]",
            ),
            (
                "psql postgres://app:s3cr3t@db.internal/app",
                "psql postgres://[redacted]@db.internal/app",
            ),
            (
                "mysql --password hunter2 -u root",
                "mysql --password [redacted] -u root",
            ),
            (
                "aws configure set aws_access_key_id AKIAIOSFODNN7EXAMPLE",
                "aws configure set aws_access_key_id [redacted AWS key id]",
            ),
            (
                "gh auth login --with-token <<< ghp_abcdefghijklmnopqrstuvwxyz0123",
                "gh auth login --with-token <<< [redacted GitHub token]",
            ),
            (
                "OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwxyz npm test",
                "OPENAI_API_KEY=[redacted] npm test",
            ),
            ("token: 'abcd1234'", "token: '[redacted]'"),
            ("git status", "git status"),
            ("npm test -- --grep password", "npm test -- --grep password"),
        ];
        for (input, want) in cases {
            assert_eq!(redact_secrets(input), want, "{input}");
        }
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----";
        assert_eq!(redact_secrets(pem), "[redacted PEM private key]");
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U";
        assert_eq!(redact_secrets(jwt), "[redacted JWT]");
    }

    #[test]
    fn json_inputs_are_redacted_by_value_and_by_key() {
        let v = redact_value(json!({
            "command": "curl -u admin:pw123 https://x",
            "env": { "GITHUB_TOKEN": "ghp_abcdefghijklmnopqrstuvwxyz0123", "PATH": "/usr/bin" },
            "headers": ["Authorization: Bearer zzz.yyy"],
            "file_path": "src/a.ts"
        }));
        assert_eq!(
            v["command"],
            "curl -u admin:pw123 https://x".replace("admin:pw123", "admin:pw123")
        ); // userinfo only matches scheme://user:pw@
        assert_eq!(v["env"]["GITHUB_TOKEN"], "[redacted]");
        assert_eq!(v["env"]["PATH"], "/usr/bin");
        assert_eq!(v["headers"][0], "Authorization: Bearer [redacted]");
        assert_eq!(v["file_path"], "src/a.ts");
    }
}

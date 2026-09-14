//! Cross-platform validation and sanitization for corpus entry names (skills,
//! memories) that become file or directory names.
//!
//! Enforced on EVERY platform, not just Windows: an entry authored on Linux
//! must survive a Windows checkout of the same project. Windows rejects
//! `< > : " | ? *` and control chars in file names ("The filename, directory
//! name, or volume label syntax is incorrect", os error 123), trailing dots
//! and spaces, and device names (`con`, `nul`, `com1`…) regardless of
//! extension. Model-proposed names are the usual source of trouble — e.g. a
//! curator merge titled "deploy: fixy".

/// Longest name component we accept. Generous for slugs while keeping deep
/// project paths clear of Windows' 260-char default path limit.
pub const MAX_COMPONENT_LEN: usize = 80;

/// Why `name` can't become a single file/dir name on every platform we ship
/// to, or `None` when it is safe. Callers wrap the reason in their own error.
pub fn component_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("empty name".into());
    }
    if name.len() > MAX_COMPONENT_LEN {
        return Some(format!("longer than {MAX_COMPONENT_LEN} bytes"));
    }
    if let Some(c) = name.chars().find(|c| {
        matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '/' | '\\') || (*c as u32) < 0x20
    }) {
        return Some(format!(
            "contains `{}`, which Windows forbids in file names",
            c.escape_default()
        ));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("ends with a dot or space, which Windows strips/forbids".into());
    }
    // Device names are reserved with ANY extension: `con.md` is still `con`.
    let stem = name.split('.').next().unwrap_or(name);
    if is_reserved_stem(stem) {
        return Some(format!("`{stem}` is a reserved Windows device name"));
    }
    None
}

/// `CON`/`PRN`/`AUX`/`NUL`/`COM1-9`/`LPT1-9`, case-insensitive.
fn is_reserved_stem(stem: &str) -> bool {
    let lower = stem.to_ascii_lowercase();
    matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || (lower.len() == 4
            && (lower.starts_with("com") || lower.starts_with("lpt"))
            && lower.as_bytes()[3].is_ascii_digit()
            && lower.as_bytes()[3] != b'0')
}

/// Collapse an arbitrary (model-proposed) title into a slug that
/// `component_problem` accepts: lowercase, alnum/`-`/`_` only, length-capped,
/// and nudged off reserved device names. May return an empty string when the
/// input has no salvageable characters — callers must treat that as invalid.
pub fn sanitize_slug(name: &str) -> String {
    let mut s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    s.truncate(60); // ASCII-only by construction, so no char-boundary risk
    let s = s.trim_matches('-').to_string();
    if is_reserved_stem(&s) {
        format!("{s}-entry")
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_ordinary_entry_names() {
        for ok in ["run-tests", "deploy_fixy", "windows-argv-limit.md", "a"] {
            assert_eq!(component_problem(ok), None, "{ok}");
        }
    }

    #[test]
    fn rejects_windows_invalid_names() {
        // The real-world case: a colon in a model-proposed name → os error 123.
        for bad in [
            "deploy: fixy",
            "what?.md",
            "a*b",
            "quote\"name",
            "pipe|name",
            "trailing.",
            "trailing ",
            "con",
            "CON.md",
            "com7",
            "lpt9.md",
            "",
        ] {
            assert!(component_problem(bad).is_some(), "{bad:?}");
        }
        assert!(component_problem(&"x".repeat(MAX_COMPONENT_LEN + 1)).is_some());
    }

    #[test]
    fn non_reserved_lookalikes_pass() {
        for ok in [
            "console",
            "com",
            "com10",
            "lpt0",
            "nul-pointer",
            "config.md",
        ] {
            assert_eq!(component_problem(ok), None, "{ok}");
        }
    }

    #[test]
    fn sanitize_produces_names_the_validator_accepts() {
        assert_eq!(sanitize_slug("deploy: fixy"), "deploy--fixy");
        assert_eq!(sanitize_slug("  My Cool Skill!  "), "my-cool-skill");
        assert_eq!(sanitize_slug("con"), "con-entry");
        assert_eq!(sanitize_slug("///"), "");
        let long = sanitize_slug(&"x".repeat(200));
        assert_eq!(long.len(), 60);
        for input in ["deploy: fixy", "CON", "a?b*c", &"y".repeat(300)] {
            let s = sanitize_slug(input);
            assert_eq!(component_problem(&s), None, "{input} -> {s}");
        }
    }
}

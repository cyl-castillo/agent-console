//! Small text helpers shared by the services that carry agent words into
//! the ledger or a device: one capping rule, one place.

/// Trim, then keep at most `max` chars; an ellipsis marks the cut. Char
/// based on purpose — byte slicing would panic on multi-byte text.
pub fn cap_chars(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out: String = t.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::cap_chars;

    #[test]
    fn caps_on_char_boundaries_and_trims() {
        assert_eq!(cap_chars("  hola  ", 10), "hola");
        assert_eq!(cap_chars("áéíóú", 3), "áéí…");
        assert_eq!(cap_chars("abc", 3), "abc");
        assert_eq!(cap_chars("", 3), "");
    }
}

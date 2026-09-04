//! Provenance stamps for generated corpus entries (skills, memories).
//!
//! Model generations turn over underneath the corpus: a skill written under one
//! generation quietly ages under the next, and nothing recorded which pipeline
//! wrote it or when. This module stamps `generated-at` / `generated-by` into an
//! entry's frontmatter at the few points where generated content is written, so
//! the curator can later reason about age and origin with evidence instead of
//! guessing. Entries written before this existed simply lack the keys — which
//! is itself a signal ("provenance unknown").

use std::time::{SystemTime, UNIX_EPOCH};

const AT_KEY: &str = "generated-at:";
const BY_KEY: &str = "generated-by:";

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Civil UTC date ("2026-09-04") from epoch ms. Hand-rolled (Howard Hinnant's
/// civil-from-days), matching the scheduler's no-chrono time math; a stamp is
/// day-granular on purpose — it feeds "how old is this entry", not ordering.
fn utc_date(ms: i64) -> String {
    let z = ms.div_euclid(86_400_000) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Return `content` with `generated-at` (today, UTC) and `generated-by` set in
/// its frontmatter. Existing stamp lines are replaced, never duplicated — a
/// curator rewrite whose model echoed the old stamp back must end up with one
/// current stamp, not two. Content without a frontmatter block gains a minimal
/// one; content whose block never closes is returned untouched (stamping must
/// not make malformed frontmatter worse).
pub fn stamp(content: &str, by: &str) -> String {
    stamp_at(content, by, now_ms())
}

fn stamp_at(content: &str, by: &str, ms: i64) -> String {
    let block = format!("{AT_KEY} {}\n{BY_KEY} {by}", utc_date(ms));
    let Some(rest) = content.strip_prefix("---") else {
        return format!("---\n{block}\n---\n\n{content}");
    };
    let Some(end) = rest.find("\n---") else {
        return content.to_string();
    };
    let kept: Vec<&str> = rest[..end]
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with(AT_KEY) && !t.starts_with(BY_KEY)
        })
        .collect();
    format!("---{}\n{block}{}", kept.join("\n"), &rest[end..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_civil_utc() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(86_400_000 - 1), "1970-01-01");
        assert_eq!(utc_date(86_400_000), "1970-01-02");
        // 2026-09-04 00:00:00 UTC
        assert_eq!(utc_date(1_788_480_000_000), "2026-09-04");
    }

    #[test]
    fn stamps_inside_existing_frontmatter() {
        let out = stamp_at("---\nname: x\ndescription: y\n---\n\nbody\n", "advisor", 0);
        assert_eq!(
            out,
            "---\nname: x\ndescription: y\ngenerated-at: 1970-01-01\ngenerated-by: advisor\n---\n\nbody\n"
        );
    }

    #[test]
    fn replaces_an_existing_stamp_instead_of_duplicating() {
        let once = stamp_at("---\nname: x\n---\nbody", "coach", 0);
        let twice = stamp_at(&once, "curator", 86_400_000);
        assert_eq!(twice.matches(AT_KEY).count(), 1);
        assert!(twice.contains("generated-at: 1970-01-02"));
        assert!(twice.contains("generated-by: curator"));
        assert!(!twice.contains("generated-by: coach"));
    }

    #[test]
    fn content_without_frontmatter_gains_a_minimal_block() {
        let out = stamp_at("# just a body\n", "coach", 0);
        assert!(out.starts_with("---\ngenerated-at: 1970-01-01\ngenerated-by: coach\n---\n\n"));
        assert!(out.ends_with("# just a body\n"));
    }

    #[test]
    fn unclosed_frontmatter_is_left_untouched() {
        let broken = "---\nname: x\nno closing delimiter";
        assert_eq!(stamp_at(broken, "coach", 0), broken);
    }
}

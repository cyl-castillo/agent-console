//! The user's work profile (E3 of the knowledge flywheel).
//!
//! One global, user-editable markdown document describing HOW this user
//! works — conventions they enforce, cadence they follow, corrections they
//! keep repeating. It is injected once per terminal session (the flywheel's
//! "system prompt"), maintained by the user directly and, via reflect
//! suggestions, by the app — always suggest-only, the file is the user's.
//!
//! Global on purpose: the profile describes the person, not a repo.
//! Per-project knowledge already has a home (project memories).

use std::fs;
use std::path::PathBuf;

use crate::error::{AppError, AppResult};

/// Injection excerpt cap. The profile rides on every session's first prompt,
/// so it must stay a note, not a document — beyond this only the head ships.
const PROFILE_INJECT_CAP: usize = 800;

/// Starter content shown (not written) when no profile exists yet. Comment
/// lines double as instructions and are stripped from injection, so an
/// untouched template injects nothing.
pub const PROFILE_TEMPLATE: &str = "\
<!-- Your work profile: how you like to work, in your own words. -->
<!-- Injected once per session so agents start knowing your style. -->
<!-- Short imperative lines work best. Delete these comments as you fill it. -->

## Conventions
<!-- e.g. \"Commit messages in English, imperative mood.\" -->

## Cadence
<!-- e.g. \"Plan first, then phase-by-phase; never merge without my GUI check.\" -->

## Recurring corrections
<!-- e.g. \"Don't add comments that narrate the diff.\" -->
";

fn profile_path() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console");
    fs::create_dir_all(&dir)?;
    Ok(dir.join("work-profile.md"))
}

/// Raw profile for the editor: the file if present, the template otherwise
/// (without writing it — an untouched template on disk would be noise).
pub fn get() -> String {
    profile_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_else(|| PROFILE_TEMPLATE.to_string())
}

pub fn set(content: &str) -> AppResult<()> {
    let path = profile_path()?;
    let tmp = path.with_extension("md.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Append one line to a section (reflect's apply path, wired in E3 phase 2).
/// Creates the section at the end when missing. The heading is matched
/// exactly ("## Conventions").
pub fn append_line(section: &str, line: &str) -> AppResult<()> {
    let current = {
        let existing = profile_path().ok().and_then(|p| fs::read_to_string(p).ok());
        existing.unwrap_or_else(|| PROFILE_TEMPLATE.to_string())
    };
    let line = line.trim();
    if line.is_empty() {
        return Err(AppError::Other("empty profile line".into()));
    }
    let mut out = String::new();
    let mut inserted = false;
    for l in current.lines() {
        out.push_str(l);
        out.push('\n');
        if !inserted && l.trim() == section.trim() {
            out.push_str(line);
            out.push('\n');
            inserted = true;
        }
    }
    if !inserted {
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
        out.push_str(section.trim());
        out.push('\n');
        out.push_str(line);
        out.push('\n');
    }
    set(&out)
}

/// What injection actually ships: meaningful lines only (comments and blanks
/// stripped), capped. None when the profile is absent or still template-only —
/// an empty profile must inject nothing.
pub fn injectable_excerpt() -> Option<String> {
    let raw = profile_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())?;
    let meaningful: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("<!--"))
        .collect();
    // Headings alone carry no instruction — require at least one body line.
    if !meaningful.iter().any(|l| !l.starts_with('#')) {
        return None;
    }
    let joined = meaningful.join("\n");
    let capped: String = joined.chars().take(PROFILE_INJECT_CAP).collect();
    Some(capped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolate(suffix: &str) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "ac-profile-{suffix}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
    }

    #[test]
    fn absent_and_template_profiles_inject_nothing() {
        let _env = crate::test_support::lock_env();
        isolate("empty");
        assert_eq!(injectable_excerpt(), None, "no file → nothing");
        assert_eq!(get(), PROFILE_TEMPLATE, "editor still shows the template");
        set(PROFILE_TEMPLATE).unwrap();
        assert_eq!(
            injectable_excerpt(),
            None,
            "untouched template (comments + headings only) → nothing"
        );
    }

    #[test]
    fn real_content_injects_stripped_and_capped() {
        let _env = crate::test_support::lock_env();
        isolate("content");
        set("<!-- note -->\n## Conventions\nCommit in English.\n\nPlan first.\n").unwrap();
        let e = injectable_excerpt().expect("has content");
        assert!(e.contains("Commit in English."));
        assert!(e.contains("Plan first."));
        assert!(!e.contains("<!--"), "comments never ship");

        let long = format!("## Conventions\n{}", "x".repeat(5 * PROFILE_INJECT_CAP));
        set(&long).unwrap();
        let e = injectable_excerpt().expect("still injects the head");
        assert!(e.chars().count() <= PROFILE_INJECT_CAP);
    }

    #[test]
    fn append_line_targets_section_or_creates_it() {
        let _env = crate::test_support::lock_env();
        isolate("append");
        set("## Conventions\nExisting rule.\n\n## Cadence\nShip small.\n").unwrap();
        append_line("## Conventions", "New rule.").unwrap();
        let got = get();
        let conventions_idx = got.find("## Conventions").unwrap();
        let cadence_idx = got.find("## Cadence").unwrap();
        let new_idx = got.find("New rule.").unwrap();
        assert!(conventions_idx < new_idx && new_idx < cadence_idx, "{got}");

        append_line("## Recurring corrections", "Stop doing X.").unwrap();
        let got = get();
        assert!(
            got.contains("## Recurring corrections\nStop doing X."),
            "{got}"
        );
        assert!(
            append_line("## Cadence", "   ").is_err(),
            "blank line rejected"
        );
    }
}

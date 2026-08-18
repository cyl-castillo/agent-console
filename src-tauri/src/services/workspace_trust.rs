//! Workspace trust: do the CLIs consider the active project directory trusted?
//!
//! Both engines gate hook execution on workspace trust, and the failure is
//! silent: in an untrusted directory `claude`/`codex` simply never run our
//! bridge scripts, so snapshots, the activity stream and the approvals bridge
//! stay empty while the console still reports "integration active". Upstream
//! keeps tightening this (Claude 2.1.3/2.1.51/2.1.218; 2.1.225 added a trust
//! prompt to `claude agents`; 2.1.232 stopped nested git repos inheriting the
//! parent's trust — Codex 0.147 now also requires explicit trust for
//! unfamiliar local projects), so more directories start out untrusted than
//! before. We read each CLI's own trust store and let the GUI say so.
//!
//! Trust stores (both keyed by absolute directory):
//! - Claude: `~/.claude.json` → `projects["<dir>"].hasTrustDialogAccepted`
//! - Codex: `~/.codex/config.toml` → `[projects."<dir>"] trust_level = "trusted"`
//!
//! We never *write* to either store: accepting trust is a decision the user
//! makes in the CLI's own prompt.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What we can say about one engine's trust for a directory.
///
/// `Unknown` is load-bearing: a missing or unparseable config means the user
/// may simply never have run that CLI, and claiming "untrusted" there would
/// be a false alarm. Only a readable store that lacks (or denies) the
/// directory yields `Untrusted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustState {
    Trusted,
    Untrusted,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceTrust {
    /// The directory these verdicts are about (absolute, as given).
    pub dir: PathBuf,
    pub claude: TrustState,
    pub codex: TrustState,
}

/// Trust verdicts for `dir`, reading both CLIs' stores under `home`.
pub fn trust_for_in(home: &Path, dir: &Path) -> WorkspaceTrust {
    let scope = Scope::of(dir);
    WorkspaceTrust {
        dir: dir.to_path_buf(),
        claude: claude_trust(&home.join(".claude.json"), &scope),
        codex: codex_trust(&home.join(".codex").join("config.toml"), &scope),
    }
}

/// Trust verdicts for `dir` using the real home directory.
pub fn trust_for(dir: &Path) -> WorkspaceTrust {
    match dirs::home_dir() {
        Some(home) => trust_for_in(&home, dir),
        None => WorkspaceTrust {
            dir: dir.to_path_buf(),
            claude: TrustState::Unknown,
            codex: TrustState::Unknown,
        },
    }
}

/// Which recorded directories can vouch for the directory we're asking about.
///
/// Both CLIs record the directory the session was launched from, but grant
/// trust to the whole repository — so an entry for `<repo>/src-tauri` also
/// covers `<repo>` and `<repo>/src`. Matching the exact path only would report
/// "untrusted" for a repo the user trusted from a subdirectory, which is the
/// one mistake this feature cannot afford.
struct Scope {
    dir: PathBuf,
    /// Nearest enclosing git repository root, if any.
    repo_root: Option<PathBuf>,
}

/// `.git` is a directory in a checkout and a file in a worktree; both count.
fn is_repo_root(dir: &Path) -> bool {
    dir.join(".git").exists()
}

impl Scope {
    fn of(dir: &Path) -> Self {
        let repo_root = dir
            .ancestors()
            .find(|a| is_repo_root(a))
            .map(Path::to_path_buf);
        Scope {
            dir: dir.to_path_buf(),
            repo_root,
        }
    }

    /// Does a trust entry recorded for `entry` cover our directory?
    fn covered_by(&self, entry: &Path) -> bool {
        if entry == self.dir {
            return true;
        }
        let Some(root) = &self.repo_root else {
            // Outside a repository, trust is just that one directory.
            return false;
        };
        // Ancestors within the repository, and the root itself.
        if self.dir.starts_with(entry) && entry.starts_with(root) {
            return true;
        }
        // A sibling/child entry inside the same repository — unless it sits in
        // a nested repository, which since Claude 2.1.232 carries its own
        // trust and cannot vouch for the parent.
        entry.starts_with(root)
            && !entry
                .ancestors()
                .take_while(|a| *a != root.as_path())
                .any(is_repo_root)
    }
}

/// `~/.claude.json` → `projects["<dir>"].hasTrustDialogAccepted`.
fn claude_trust(config: &Path, scope: &Scope) -> TrustState {
    let Ok(txt) = std::fs::read_to_string(config) else {
        return TrustState::Unknown;
    };
    let Ok(json) = serde_json::from_str::<Value>(&txt) else {
        return TrustState::Unknown;
    };
    let Some(projects) = json.get("projects").and_then(Value::as_object) else {
        return TrustState::Unknown;
    };
    for (entry, value) in projects {
        let accepted = value
            .get("hasTrustDialogAccepted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if accepted && scope.covered_by(Path::new(entry)) {
            return TrustState::Trusted;
        }
    }
    TrustState::Untrusted
}

/// `~/.codex/config.toml` → `[projects."<dir>"] trust_level = "trusted"`.
///
/// Hand-rolled rather than pulling in a TOML parser for one lookup: we track
/// the current table header and only read `trust_level` while inside a
/// `[projects."..."]` table, so keys of the same name elsewhere can't match.
fn codex_trust(config: &Path, scope: &Scope) -> TrustState {
    let Ok(txt) = std::fs::read_to_string(config) else {
        return TrustState::Unknown;
    };
    let mut in_wanted_table = false;
    for line in txt.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_wanted_table = codex_project_path(header)
                .map(|p| scope.covered_by(Path::new(p)))
                .unwrap_or(false);
            continue;
        }
        if !in_wanted_table {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "trust_level" && value.trim().trim_matches('"') == "trusted" {
                return TrustState::Trusted;
            }
        }
    }
    TrustState::Untrusted
}

/// `projects."/some/dir"` → `/some/dir`. Anything else (another table, an
/// unquoted key) yields None.
fn codex_project_path(header: &str) -> Option<&str> {
    header
        .trim()
        .strip_prefix("projects.")?
        .trim()
        .strip_prefix('"')?
        .strip_suffix('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ac-trust-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_claude(home: &Path, body: &str) {
        fs::write(home.join(".claude.json"), body).unwrap();
    }

    fn write_codex(home: &Path, body: &str) {
        let dir = home.join(".codex");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), body).unwrap();
    }

    #[test]
    fn missing_configs_are_unknown_not_untrusted() {
        let home = temp_home("missing");
        let t = trust_for_in(&home, Path::new("/some/project"));
        assert_eq!(t.claude, TrustState::Unknown);
        assert_eq!(t.codex, TrustState::Unknown);
    }

    #[test]
    fn unparseable_claude_config_is_unknown() {
        let home = temp_home("garbage");
        write_claude(&home, "{ not json");
        let t = trust_for_in(&home, Path::new("/some/project"));
        assert_eq!(t.claude, TrustState::Unknown);
    }

    #[test]
    fn claude_reads_the_trust_dialog_flag() {
        let home = temp_home("claude");
        write_claude(
            &home,
            r#"{"projects":{"/a/yes":{"hasTrustDialogAccepted":true},
                            "/a/no":{"hasTrustDialogAccepted":false}}}"#,
        );
        let cfg = home.join(".claude.json");
        let at = |p: &str| Scope::of(Path::new(p));
        assert_eq!(claude_trust(&cfg, &at("/a/yes")), TrustState::Trusted);
        assert_eq!(claude_trust(&cfg, &at("/a/no")), TrustState::Untrusted);
        // A directory the store has never seen is untrusted, not unknown:
        // the store itself is readable, so its silence is an answer.
        assert_eq!(claude_trust(&cfg, &at("/a/never")), TrustState::Untrusted);
    }

    #[test]
    fn codex_reads_the_project_trust_level() {
        let home = temp_home("codex");
        write_codex(
            &home,
            r#"
[projects."/a/yes"]
trust_level = "trusted"

[projects."/a/no"]
trust_level = "untrusted"

[notice]
trust_level = "trusted"
"#,
        );
        let cfg = home.join(".codex").join("config.toml");
        let at = |p: &str| Scope::of(Path::new(p));
        assert_eq!(codex_trust(&cfg, &at("/a/yes")), TrustState::Trusted);
        assert_eq!(codex_trust(&cfg, &at("/a/no")), TrustState::Untrusted);
        // `trust_level` under an unrelated table must not leak into the verdict.
        assert_eq!(codex_trust(&cfg, &at("/notice")), TrustState::Untrusted);
        assert_eq!(codex_trust(&cfg, &at("/a/never")), TrustState::Untrusted);
    }

    #[test]
    fn subdirectory_inherits_trust_up_to_the_repo_root() {
        let home = temp_home("repo");
        let repo = home.join("repo");
        let sub = repo.join("src").join("deep");
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        write_claude(
            &home,
            &format!(
                r#"{{"projects":{{"{}":{{"hasTrustDialogAccepted":true}}}}}}"#,
                repo.to_string_lossy()
            ),
        );
        assert_eq!(trust_for_in(&home, &sub).claude, TrustState::Trusted);
    }

    #[test]
    fn a_nested_repo_does_not_inherit_the_parent_repos_trust() {
        // Claude 2.1.232: each repository needs its own trust confirmation.
        let home = temp_home("nested");
        let outer = home.join("outer");
        let inner = outer.join("vendor").join("inner");
        fs::create_dir_all(&inner).unwrap();
        fs::create_dir_all(outer.join(".git")).unwrap();
        fs::create_dir_all(inner.join(".git")).unwrap();
        write_claude(
            &home,
            &format!(
                r#"{{"projects":{{"{}":{{"hasTrustDialogAccepted":true}}}}}}"#,
                outer.to_string_lossy()
            ),
        );
        assert_eq!(trust_for_in(&home, &inner).claude, TrustState::Untrusted);
    }

    #[test]
    fn an_entry_recorded_in_a_sibling_subdir_still_covers_the_repo() {
        // The real shape on this machine: the CLI recorded `<repo>/src-tauri`
        // (where the session was launched) while the console asks about
        // `<repo>`. Trust is repository-wide, so this must not read as untrusted.
        let home = temp_home("sibling");
        let repo = home.join("repo");
        fs::create_dir_all(repo.join("src-tauri")).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        write_claude(
            &home,
            &format!(
                r#"{{"projects":{{"{}":{{"hasTrustDialogAccepted":true}}}}}}"#,
                repo.join("src-tauri").to_string_lossy()
            ),
        );
        assert_eq!(trust_for_in(&home, &repo).claude, TrustState::Trusted);
    }

    #[test]
    fn an_entry_inside_a_nested_repo_cannot_vouch_for_the_parent() {
        let home = temp_home("vendor");
        let repo = home.join("repo");
        let vendored = repo.join("vendor").join("dep");
        fs::create_dir_all(&vendored).unwrap();
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(vendored.join(".git")).unwrap();
        write_claude(
            &home,
            &format!(
                r#"{{"projects":{{"{}":{{"hasTrustDialogAccepted":true}}}}}}"#,
                vendored.to_string_lossy()
            ),
        );
        assert_eq!(trust_for_in(&home, &repo).claude, TrustState::Untrusted);
    }

    #[test]
    fn outside_a_repo_only_the_directory_itself_counts() {
        let home = temp_home("norepo");
        let plain = home.join("scratch");
        fs::create_dir_all(plain.join("sub")).unwrap();
        write_claude(
            &home,
            &format!(
                r#"{{"projects":{{"{}":{{"hasTrustDialogAccepted":true}}}}}}"#,
                plain.to_string_lossy()
            ),
        );
        assert_eq!(trust_for_in(&home, &plain).claude, TrustState::Trusted);
        assert_eq!(
            trust_for_in(&home, &plain.join("sub")).claude,
            TrustState::Untrusted
        );
    }

    #[test]
    fn each_engine_is_judged_on_its_own_store() {
        let home = temp_home("mixed");
        // Claude's store is readable and silent about the dir; codex's is absent.
        // One verdict must not colour the other.
        write_claude(&home, r#"{"projects":{}}"#);
        let t = trust_for_in(&home, Path::new("/a/project"));
        assert_eq!(t.claude, TrustState::Untrusted);
        assert_eq!(t.codex, TrustState::Unknown);
    }
}

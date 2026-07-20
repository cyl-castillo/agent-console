use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    /// "skill" | "command" | "agent"
    pub kind: String,
    /// "project" | "user"
    pub source: String,
    pub path: PathBuf,
    pub description: Option<String>,
    pub allowed_tools: Vec<String>,
}

/// Walks both project-level (.claude/skills|commands|agents) and user-level
/// (~/.claude/skills|commands|agents) and returns every entity found.
pub fn list(project_root: Option<&Path>) -> AppResult<Vec<Skill>> {
    let mut out = Vec::new();

    if let Some(root) = project_root {
        let base = root.join(".claude");
        out.extend(scan(&base.join("skills"), "skill", "project"));
        out.extend(scan(&base.join("commands"), "command", "project"));
        out.extend(scan(&base.join("agents"), "agent", "project"));
    }
    if let Some(home) = dirs::home_dir() {
        let base = home.join(".claude");
        out.extend(scan(&base.join("skills"), "skill", "user"));
        out.extend(scan(&base.join("commands"), "command", "user"));
        out.extend(scan(&base.join("agents"), "agent", "user"));
    }
    // Sort: project before user, then alphabetic.
    out.sort_by(|a, b| {
        let s_cmp = a.source.cmp(&b.source);
        if s_cmp != std::cmp::Ordering::Equal {
            return s_cmp;
        }
        a.name.to_lowercase().cmp(&b.name.to_lowercase())
    });
    Ok(out)
}

fn scan(dir: &Path, kind: &str, source: &str) -> Vec<Skill> {
    if !dir.is_dir() {
        return Vec::new();
    }
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy().to_string();

        // Skills are a dir containing SKILL.md; commands/agents are .md files.
        let (display_name, md_path, root_path) = if path.is_dir() {
            let md = path.join("SKILL.md");
            if !md.exists() {
                continue;
            }
            (name.clone(), md, path.clone())
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or(name);
            (stem, path.clone(), path.clone())
        } else {
            continue;
        };

        let (description, allowed_tools) = parse_frontmatter(&md_path);
        out.push(Skill {
            name: display_name,
            kind: kind.to_string(),
            source: source.to_string(),
            path: root_path,
            description,
            allowed_tools,
        });
    }
    out
}

/// Tolerant frontmatter parser — looks for `description:` and `allowed-tools:`.
/// Does not require a full YAML implementation.
fn parse_frontmatter(path: &Path) -> (Option<String>, Vec<String>) {
    let Ok(content) = fs::read_to_string(path) else {
        return (None, Vec::new());
    };
    if !content.starts_with("---") {
        return (None, Vec::new());
    }
    let after = &content[3..];
    let Some(end) = after.find("\n---") else {
        return (None, Vec::new());
    };
    let fm = &after[..end];

    let mut description: Option<String> = None;
    let mut allowed_tools: Vec<String> = Vec::new();
    for line in fm.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("description:") {
            let v = rest
                .trim()
                .trim_matches(|c| c == '"' || c == '\'')
                .to_string();
            if !v.is_empty() {
                description = Some(v);
            }
        } else if let Some(rest) = l.strip_prefix("allowed-tools:") {
            let v = rest.trim().trim_matches(|c| c == '[' || c == ']');
            allowed_tools = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    (description, allowed_tools)
}

/// Read the raw SKILL.md (or .md) so the UI can preview it.
pub fn read_md(path: &Path) -> AppResult<String> {
    let target = if path.is_dir() {
        path.join("SKILL.md")
    } else {
        path.to_path_buf()
    };
    Ok(fs::read_to_string(&target)?)
}

/// Create or **overwrite** a project skill's SKILL.md. Unlike the Advisor's
/// `create_skill` (which refuses to clobber an existing skill), the curator needs
/// to rewrite entries in place (refactor) and to fold merged content onto a
/// surviving name — so this overwrites by design.
pub fn write(project_root: &Path, name: &str, content: &str) -> AppResult<PathBuf> {
    let dir = skill_dir(project_root, name)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    fs::write(&path, content)?;
    Ok(path)
}

/// Retire a project skill by **moving** it under `.claude/skills/_archived/`
/// rather than deleting it — curation is suggest-only and reversible, so an
/// archived skill can always be restored. Returns the new (archived) location.
/// `list`/`scan` skip the `_archived` dir naturally (it holds no SKILL.md at its
/// top level), so archived skills disappear from the active corpus.
pub fn archive(project_root: &Path, name: &str) -> AppResult<PathBuf> {
    let dir = skill_dir(project_root, name)?;
    if !dir.exists() {
        return Err(AppError::NotFound(format!("skill '{name}'")));
    }
    let archived = project_root
        .join(".claude")
        .join("skills")
        .join("_archived");
    fs::create_dir_all(&archived)?;
    let dest = archived.join(name);
    // Replace a prior archive of the same name so a re-archive can't fail.
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::rename(&dir, &dest)?;
    Ok(dest)
}

/// Resolve a project skill directory by name, defending against traversal and
/// reserving the leading-underscore namespace (e.g. `_archived`) and dotfiles.
fn skill_dir(project_root: &Path, name: &str) -> AppResult<PathBuf> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('_')
        || name.starts_with('.')
    {
        return Err(AppError::InvalidArgument(format!(
            "invalid skill name: {name}"
        )));
    }
    Ok(project_root.join(".claude").join("skills").join(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ac-skills-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn write_creates_then_overwrites() {
        let root = temp_root();
        let p = write(&root, "demo", "---\nname: demo\n---\n\nv1").unwrap();
        assert!(p.exists());
        assert!(read_md(&root.join(".claude/skills/demo"))
            .unwrap()
            .contains("v1"));
        // Overwrite in place (refactor path) — must not error like create_skill.
        write(&root, "demo", "---\nname: demo\n---\n\nv2").unwrap();
        assert!(read_md(&p).unwrap().contains("v2"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_moves_and_hides_from_list() {
        let root = temp_root();
        write(&root, "stale", "---\nname: stale\n---\n\nbody").unwrap();
        assert!(list(Some(&root)).unwrap().iter().any(|s| s.name == "stale"));

        let dest = archive(&root, "stale").unwrap();
        assert!(dest.exists(), "archived copy exists");
        assert!(
            !root.join(".claude/skills/stale").exists(),
            "original moved"
        );
        // Gone from the active corpus; `_archived` itself isn't listed as a skill.
        let names: Vec<String> = list(Some(&root))
            .unwrap()
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert!(
            !names.iter().any(|n| n == "stale" || n == "_archived"),
            "{names:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_unsafe_names() {
        let root = temp_root();
        assert!(write(&root, "../escape", "x").is_err());
        assert!(write(&root, "_archived", "x").is_err());
        assert!(archive(&root, "missing").is_err());
        let _ = fs::remove_dir_all(&root);
    }
}

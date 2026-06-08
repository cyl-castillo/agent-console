use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::AppResult;

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

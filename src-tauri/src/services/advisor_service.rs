use crate::services::proc;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRecommendation {
    pub name: String,
    pub description: String,
    pub why_it_fits: String,
    /// "project" | "user"
    pub scope: String,
    pub skill_md_content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResult {
    pub recommendations: Vec<SkillRecommendation>,
    pub raw_excerpt: String,
}

/// Run `claude -p` in plan mode with the project as cwd. Returns parsed
/// recommendations. The prompt instructs Claude to return strict JSON only.
pub fn analyze(project_root: &Path) -> AppResult<AnalysisResult> {
    if !project_root.is_dir() {
        return Err(AppError::NotADirectory(project_root.display().to_string()));
    }

    let context = gather_context(project_root)?;
    let prompt = build_prompt(&context);

    // Resolve `claude` via the shared resolver — a GUI launch doesn't inherit
    // the login-shell PATH, so the bare name would fail to spawn. stdio + the
    // Windows no-window flag are set inside claude_cli::command.
    let mut cmd = crate::services::claude_cli::command(&[
        "-p",
        &prompt,
        "--permission-mode",
        "plan",
        "--output-format",
        "text",
    ]);
    cmd.current_dir(project_root);
    let output = cmd
        .output()
        .map_err(|e| AppError::Other(format!("failed to spawn `claude`: {e}. Is it on PATH?")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Other(format!(
            "claude exited with status {}: {}",
            output.status, stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let recommendations = parse_recommendations(&stdout)?;
    Ok(AnalysisResult {
        recommendations,
        raw_excerpt: truncate(&stdout, 4000),
    })
}

/// Write a single recommendation to disk as a Claude skill.
/// scope: "project" -> <root>/.claude/skills/<name>/SKILL.md
/// scope: "user"    -> ~/.claude/skills/<name>/SKILL.md
pub fn create_skill(
    project_root: &Path,
    scope: &str,
    name: &str,
    skill_md_content: &str,
) -> AppResult<PathBuf> {
    let safe = sanitize_name(name);
    if safe.is_empty() {
        return Err(AppError::InvalidArgument("empty skill name".into()));
    }

    let base = match scope {
        "project" => project_root.join(".claude").join("skills"),
        "user" => dirs::home_dir()
            .ok_or_else(|| AppError::Other("no home dir".into()))?
            .join(".claude")
            .join("skills"),
        other => {
            return Err(AppError::InvalidArgument(format!(
                "scope must be 'project' or 'user', got {other}"
            )))
        }
    };

    let dir = base.join(&safe);
    if dir.exists() {
        return Err(AppError::Other(format!(
            "skill '{safe}' already exists at {}",
            dir.display()
        )));
    }
    fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    fs::write(&path, skill_md_content)?;
    Ok(path)
}

struct Context {
    tree_excerpt: String,
    manifests: String,
    existing_skills: String,
    git_log: String,
}

fn gather_context(root: &Path) -> AppResult<Context> {
    let tree_excerpt = tree_two_levels(root);
    let manifests = read_manifests(root);
    let existing_skills = list_existing_skills(root);
    let git_log = git_log_recent(root);
    Ok(Context {
        tree_excerpt,
        manifests,
        existing_skills,
        git_log,
    })
}

fn tree_two_levels(root: &Path) -> String {
    use walkdir::WalkDir;
    let mut lines = Vec::new();
    for entry in WalkDir::new(root)
        .min_depth(1)
        .max_depth(2)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                "node_modules" | "target" | "dist" | "build" | ".git" | ".next" | ".venv"
            )
        })
        .flatten()
        .take(200)
    {
        let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
        let suffix = if entry.file_type().is_dir() { "/" } else { "" };
        lines.push(format!("{}{}", rel.display(), suffix));
    }
    lines.join("\n")
}

fn read_manifests(root: &Path) -> String {
    let candidates = [
        "package.json",
        "Cargo.toml",
        "pom.xml",
        "build.gradle",
        "pyproject.toml",
        "requirements.txt",
        "go.mod",
        "Gemfile",
        "composer.json",
    ];
    let mut out = String::new();
    for name in candidates {
        let p = root.join(name);
        if let Ok(contents) = fs::read_to_string(&p) {
            out.push_str(&format!("--- {name} ---\n{}\n", truncate(&contents, 2000)));
        }
    }
    out
}

fn list_existing_skills(root: &Path) -> String {
    let dir = root.join(".claude").join("skills");
    if !dir.is_dir() {
        return "(none)".to_string();
    }
    let Ok(read) = fs::read_dir(&dir) else {
        return "(none)".to_string();
    };
    let names: Vec<String> = read
        .flatten()
        .filter_map(|e| {
            if e.path().is_dir() {
                Some(e.file_name().to_string_lossy().to_string())
            } else {
                None
            }
        })
        .collect();
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.join(", ")
    }
}

fn git_log_recent(root: &Path) -> String {
    let Ok(out) = proc::command("git")
        .args(["log", "--oneline", "-30"])
        .current_dir(root)
        .output()
    else {
        return "(no git)".to_string();
    };
    if !out.status.success() {
        return "(no git)".to_string();
    }
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn build_prompt(ctx: &Context) -> String {
    format!(
        r#"You are an expert at designing Claude Code skills for software projects.

I will give you a snapshot of a project. Propose 3 to 6 NEW skills that would
genuinely save time for whoever works on it. Skills already present should not
be re-proposed.

A Claude Code skill is a directory under .claude/skills/<name>/ with a
SKILL.md file. The SKILL.md begins with YAML frontmatter:

---
name: kebab-case-name
description: one-line description used to decide when to invoke the skill
---

Then a short body (under ~30 lines) describing when to use it and step-by-step
guidance. Keep each skill focused on ONE workflow.

PROJECT CONTEXT
===============

File tree (top 2 levels):
{tree}

Manifests:
{manifests}

Existing skills: {existing}

Recent git activity:
{git}

OUTPUT FORMAT (STRICT)
======================

Respond with ONLY a JSON object, no prose, no markdown code fences. Shape:

{{
  "recommendations": [
    {{
      "name": "kebab-case-name",
      "description": "one line",
      "whyItFits": "why this project benefits from this skill, 1-2 sentences",
      "scope": "project",
      "skillMdContent": "---\nname: kebab-case-name\ndescription: ...\n---\n\nBody..."
    }}
  ]
}}

scope is "project" if the skill is specific to this codebase, "user" if it's
generic enough to live in ~/.claude. Default to "project" when in doubt.
"#,
        tree = ctx.tree_excerpt,
        manifests = if ctx.manifests.is_empty() {
            "(none)"
        } else {
            &ctx.manifests
        },
        existing = ctx.existing_skills,
        git = ctx.git_log,
    )
}

#[derive(Deserialize)]
struct Wrapper {
    recommendations: Vec<SkillRecommendation>,
}

fn parse_recommendations(stdout: &str) -> AppResult<Vec<SkillRecommendation>> {
    let trimmed = stdout.trim();
    // Find the first '{' and the last '}'; tolerates leading/trailing chatter.
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    let json = match (start, end) {
        (Some(s), Some(e)) if e > s => &trimmed[s..=e],
        _ => {
            return Err(AppError::Other(format!(
                "claude output did not contain a JSON object. First 400 chars: {}",
                truncate(trimmed, 400)
            )))
        }
    };
    let wrapper: Wrapper = serde_json::from_str(json)
        .map_err(|e| AppError::Other(format!("failed to parse recommendations JSON: {e}")))?;
    Ok(wrapper.recommendations)
}

fn sanitize_name(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut cut = max;
        while !s.is_char_boundary(cut) && cut > 0 {
            cut -= 1;
        }
        format!("{}…", &s[..cut])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("ac-advisor-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parse_recommendations_tolerates_chatter_around_the_json() {
        let out = r#"Sure! Here are my recommendations:
{"recommendations": [{"name": "run-tests", "description": "d", "whyItFits": "w", "scope": "project", "skillMdContent": "---\nname: run-tests\n---\nBody"}]}
Hope that helps!"#;
        let recs = parse_recommendations(out).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].name, "run-tests");
        assert_eq!(recs[0].why_it_fits, "w");
    }

    #[test]
    fn parse_recommendations_rejects_output_without_json() {
        let err = parse_recommendations("I could not analyze this project.").unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
        let err = parse_recommendations("{\"recommendations\": [{\"broken\"").unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    #[test]
    fn sanitize_name_produces_safe_directory_names() {
        assert_eq!(sanitize_name("run-tests"), "run-tests");
        assert_eq!(sanitize_name("  My Cool Skill!  "), "my-cool-skill");
        assert_eq!(sanitize_name("../../etc/passwd"), "etc-passwd");
        assert_eq!(sanitize_name("---"), "");
        assert_eq!(sanitize_name("é!"), "");
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        // "ñ" is 2 bytes; cutting at byte 1 must back off, not panic.
        assert_eq!(truncate("ñx", 1), "…");
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "abc…");
    }

    #[test]
    fn create_skill_writes_project_scope_and_refuses_duplicates_and_junk() {
        let root = temp_root("create");
        let path = create_skill(&root, "project", "My Skill", "content").unwrap();
        assert!(path.ends_with(".claude/skills/my-skill/SKILL.md"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "content");

        // Same name again → refuse rather than clobber.
        let err = create_skill(&root, "project", "My Skill", "other").unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
        assert_eq!(fs::read_to_string(&path).unwrap(), "content");

        // A name that sanitizes to nothing, and an unknown scope.
        assert!(matches!(
            create_skill(&root, "project", "///", "c").unwrap_err(),
            AppError::InvalidArgument(_)
        ));
        assert!(matches!(
            create_skill(&root, "workspace", "ok-name", "c").unwrap_err(),
            AppError::InvalidArgument(_)
        ));

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn build_prompt_embeds_the_context_and_defaults_empty_manifests() {
        let ctx = Context {
            tree_excerpt: "src/\nsrc/main.rs".into(),
            manifests: String::new(),
            existing_skills: "release-flow".into(),
            git_log: "abc123 fix".into(),
        };
        let p = build_prompt(&ctx);
        assert!(p.contains("src/main.rs"));
        assert!(p.contains("Existing skills: release-flow"));
        assert!(p.contains("abc123 fix"));
        assert!(p.contains("Manifests:\n(none)"));
    }
}

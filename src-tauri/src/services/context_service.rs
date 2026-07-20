use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::project_manager::{self, WorkspaceContext};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileStat {
    pub path: String,
    pub exists: bool,
    pub size_bytes: u64,
    pub modified_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirStat {
    pub path: String,
    pub exists: bool,
    pub entry_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextStatus {
    pub project_claude_md: Option<FileStat>,
    pub global_claude_md: FileStat,
    pub memory_dir: DirStat,
}

pub fn status(project_root: Option<&Path>) -> AppResult<ContextStatus> {
    let project_claude_md = project_root.map(|root| file_stat(&root.join("CLAUDE.md")));
    let global = global_claude_md_path()?;
    let global_claude_md = file_stat(&global);
    let memory_dir = match project_root {
        Some(root) => dir_stat(&memory_dir_for(root)?),
        None => DirStat {
            path: String::new(),
            exists: false,
            entry_count: 0,
        },
    };
    Ok(ContextStatus {
        project_claude_md,
        global_claude_md,
        memory_dir,
    })
}

pub fn read_md(project_root: Option<&Path>, scope: &str) -> AppResult<String> {
    let path = resolve_md(project_root, scope)?;
    if !path.exists() {
        return Ok(String::new());
    }
    Ok(fs::read_to_string(&path)?)
}

/// Write CLAUDE.md, creating parent dirs if needed. The optional `expected_mtime_ms`
/// is used for conflict detection: when present and the file changed externally
/// since the caller read it, we reject with a recognizable error so the UI can
/// prompt the user.
pub fn write_md(
    project_root: Option<&Path>,
    scope: &str,
    content: &str,
    expected_mtime_ms: Option<i64>,
) -> AppResult<FileStat> {
    let path = resolve_md(project_root, scope)?;

    if let Some(expected) = expected_mtime_ms {
        if path.exists() {
            let stat = file_stat(&path);
            if stat.modified_ms != expected {
                return Err(AppError::Other(format!(
                    "context:conflict: file changed externally (expected {expected}, found {})",
                    stat.modified_ms,
                )));
            }
        }
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, content)?;
    Ok(file_stat(&path))
}

pub fn md_path_for(project_root: Option<&Path>, scope: &str) -> AppResult<PathBuf> {
    resolve_md(project_root, scope)
}

/// Build a starter CLAUDE.md from `workspace_context()`. Static template,
/// no LLM call — pure inspection of the project on disk.
pub fn generate_starter(project_root: &Path) -> AppResult<String> {
    let ctx = project_manager::workspace_context(project_root)?;
    Ok(render_starter(&ctx))
}

fn render_starter(ctx: &WorkspaceContext) -> String {
    let name = Path::new(&ctx.root)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    let mut md = String::new();
    md.push_str(&format!("# {name}\n\n"));
    md.push_str(
        "Project-level context for Claude Code. Loaded automatically into every session.\n\n",
    );

    md.push_str("## Stack\n\n");
    match (&ctx.language, &ctx.framework) {
        (Some(l), Some(f)) => md.push_str(&format!("- Language: {l}\n- Framework: {f}\n")),
        (Some(l), None) => md.push_str(&format!("- Language: {l}\n")),
        (None, Some(f)) => md.push_str(&format!("- Framework: {f}\n")),
        (None, None) => md.push_str("- _(no language/framework detected — fill in)_\n"),
    }
    md.push('\n');

    if !ctx.package_scripts.is_empty() {
        md.push_str("## Common commands\n\n");
        for s in &ctx.package_scripts {
            md.push_str(&format!("- `npm run {s}`\n"));
        }
        md.push('\n');
    }

    if !ctx.entry_points.is_empty() {
        md.push_str("## Entry points\n\n");
        for ep in &ctx.entry_points {
            md.push_str(&format!("- `{ep}`\n"));
        }
        md.push('\n');
    }

    md.push_str("## Conventions\n\n");
    md.push_str(
        "- _(describe the rules you want Claude to follow: tests, commit style, etc.)_\n\n",
    );

    md.push_str("## Avoid\n\n");
    md.push_str("- _(list things Claude should NOT do in this project)_\n");

    md
}

fn resolve_md(project_root: Option<&Path>, scope: &str) -> AppResult<PathBuf> {
    match scope {
        "project" => {
            let root = project_root.ok_or_else(|| {
                AppError::InvalidArgument("project scope requires an open project".into())
            })?;
            Ok(root.join("CLAUDE.md"))
        }
        "global" => global_claude_md_path(),
        other => Err(AppError::InvalidArgument(format!(
            "scope must be 'project' or 'global', got {other}"
        ))),
    }
}

fn global_claude_md_path() -> AppResult<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    Ok(home.join(".claude").join("CLAUDE.md"))
}

/// Encode the absolute project path into the slug Claude Code uses for
/// per-project state: replace each `/` (or `\` on Windows) with `-`. The
/// leading separator becomes a leading `-` too.
pub fn memory_dir_for(project_root: &Path) -> AppResult<PathBuf> {
    let abs = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.to_path_buf());
    let s = abs.to_string_lossy().replace(['/', '\\'], "-");
    let home = dirs::home_dir().ok_or_else(|| AppError::Other("no home dir".into()))?;
    Ok(home.join(".claude").join("projects").join(s).join("memory"))
}

fn file_stat(path: &Path) -> FileStat {
    if let Ok(meta) = fs::metadata(path) {
        let size_bytes = meta.len();
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        FileStat {
            path: path.to_string_lossy().to_string(),
            exists: true,
            size_bytes,
            modified_ms,
        }
    } else {
        FileStat {
            path: path.to_string_lossy().to_string(),
            exists: false,
            size_bytes: 0,
            modified_ms: 0,
        }
    }
}

fn dir_stat(path: &Path) -> DirStat {
    let exists = path.is_dir();
    let entry_count = if exists {
        fs::read_dir(path)
            .ok()
            .map(|r| {
                r.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".md"))
                    .count() as u32
            })
            .unwrap_or(0)
    } else {
        0
    };
    DirStat {
        path: path.to_string_lossy().to_string(),
        exists,
        entry_count,
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
            std::env::temp_dir().join(format!("ac-context-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_md_scopes() {
        let root = temp_root("resolve");
        assert_eq!(
            resolve_md(Some(&root), "project").unwrap(),
            root.join("CLAUDE.md")
        );
        assert!(matches!(
            resolve_md(None, "project").unwrap_err(),
            AppError::InvalidArgument(_)
        ));
        assert!(matches!(
            resolve_md(Some(&root), "workspace").unwrap_err(),
            AppError::InvalidArgument(_)
        ));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_md_missing_file_is_empty_not_error() {
        let root = temp_root("readmd");
        assert_eq!(read_md(Some(&root), "project").unwrap(), "");
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn write_md_detects_external_edits_via_mtime() {
        let root = temp_root("conflict");
        let stat = write_md(Some(&root), "project", "v1", None).unwrap();

        // Stale expectation → recognizable conflict error, file untouched.
        let err =
            write_md(Some(&root), "project", "v2", Some(stat.modified_ms - 5000)).unwrap_err();
        assert!(err.to_string().contains("context:conflict"), "{err}");
        assert_eq!(fs::read_to_string(root.join("CLAUDE.md")).unwrap(), "v1");

        // Matching expectation → write goes through.
        write_md(Some(&root), "project", "v2", Some(stat.modified_ms)).unwrap();
        assert_eq!(fs::read_to_string(root.join("CLAUDE.md")).unwrap(), "v2");

        // No expectation → last-writer-wins overwrite.
        write_md(Some(&root), "project", "v3", None).unwrap();
        assert_eq!(fs::read_to_string(root.join("CLAUDE.md")).unwrap(), "v3");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn render_starter_reflects_the_detected_stack() {
        let ctx = WorkspaceContext {
            root: PathBuf::from("/work/fixy-app"),
            language: Some("typescript".into()),
            framework: Some("vite".into()),
            file_count: 10,
            package_scripts: vec!["dev".into(), "test".into()],
            entry_points: vec!["src/main.tsx".into()],
            readme_preview: None,
        };
        let md = render_starter(&ctx);
        assert!(md.starts_with("# fixy-app\n"));
        assert!(md.contains("- Language: typescript"));
        assert!(md.contains("- Framework: vite"));
        assert!(md.contains("`npm run dev`"));
        assert!(md.contains("`src/main.tsx`"));

        // Nothing detected → explicit fill-in hint instead of empty sections.
        let bare = WorkspaceContext {
            root: PathBuf::from("/work/x"),
            language: None,
            framework: None,
            file_count: 0,
            package_scripts: vec![],
            entry_points: vec![],
            readme_preview: None,
        };
        let md = render_starter(&bare);
        assert!(md.contains("no language/framework detected"));
        assert!(!md.contains("## Common commands"));
    }

    #[test]
    fn dir_stat_counts_only_markdown_memories() {
        let root = temp_root("dirstat");
        fs::write(root.join("a.md"), "x").unwrap();
        fs::write(root.join("b.md"), "x").unwrap();
        fs::write(root.join("index.json"), "{}").unwrap();
        let stat = dir_stat(&root);
        assert!(stat.exists);
        assert_eq!(stat.entry_count, 2);

        let gone = dir_stat(&root.join("nope"));
        assert!(!gone.exists);
        assert_eq!(gone.entry_count, 0);
        fs::remove_dir_all(&root).ok();
    }

    /// One test fn — it mutates the process-global HOME so the slug lands in
    /// a sandbox instead of the developer's real ~/.claude.
    #[test]
    fn memory_dir_slug_encodes_the_project_path() {
        let _env = crate::test_support::lock_env();
        let fake_home = temp_root("home");
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", &fake_home);

        let root = temp_root("slug");
        let dir = memory_dir_for(&root).unwrap();

        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }

        let canon = root.canonicalize().unwrap();
        let expected_slug = canon.to_string_lossy().replace(['/', '\\'], "-");
        assert!(dir.starts_with(&fake_home));
        assert!(dir.ends_with(Path::new(&expected_slug).join("memory")));
        // The slug is a single path component — no separators survive.
        assert!(!expected_slug.contains('/'));

        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&fake_home).ok();
    }
}

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
    pub language: Option<String>,
    pub framework: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileNode {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// `None` means "not loaded yet". Empty vec means "loaded, no children".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileNode>>,
}

/// Folders we never descend into when scanning a repo.
const IGNORED_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", ".next", ".nuxt",
    ".venv", "venv", "__pycache__", ".idea", ".vscode", ".gradle",
];

pub fn open_project(path: &Path) -> AppResult<Project> {
    if !path.exists() {
        return Err(AppError::NotFound(path.display().to_string()));
    }
    if !path.is_dir() {
        return Err(AppError::NotADirectory(path.display().to_string()));
    }

    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    let (language, framework) = detect_stack(path);

    Ok(Project {
        root: path.to_path_buf(),
        name,
        language,
        framework,
    })
}

/// Read the repo tree, lazily — children are populated up to `depth` levels.
pub fn read_tree(root: &Path, depth: usize) -> AppResult<FileNode> {
    if !root.exists() {
        return Err(AppError::NotFound(root.display().to_string()));
    }
    build_node(root, depth)
}

fn build_node(path: &Path, depth: usize) -> AppResult<FileNode> {
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let is_dir = path.is_dir();
    build_node_known(path.to_path_buf(), name, is_dir, depth)
}

/// Build a node when the caller already knows `name` and `is_dir`, so we don't
/// re-`stat()` the path. On Windows every `stat`/`read_dir` is intercepted by
/// Defender, so the previous double-stat (one in the loop, one on recursion)
/// roughly doubled the cost of the project-open tree read.
fn build_node_known(path: PathBuf, name: String, is_dir: bool, depth: usize) -> AppResult<FileNode> {
    let children = if is_dir && depth > 0 {
        let mut entries: Vec<FileNode> = Vec::new();
        let read = std::fs::read_dir(&path)?;
        for entry in read.flatten() {
            let entry_path = entry.path();
            let entry_name = entry.file_name().to_string_lossy().to_string();
            // One stat per child (follows symlinks, as before); the result is
            // threaded into the recursive call so it isn't computed twice.
            let child_is_dir = entry_path.is_dir();

            // Skip ignored directories.
            if child_is_dir && IGNORED_DIRS.contains(&entry_name.as_str()) {
                continue;
            }

            entries.push(build_node_known(entry_path, entry_name, child_is_dir, depth - 1)?);
        }
        // Dirs first, then alphabetic.
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Some(entries)
    } else {
        // Either a file, or an unloaded (expandable) directory at depth 0.
        None
    };

    Ok(FileNode { name, path, is_dir, children })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceContext {
    pub root: PathBuf,
    pub language: Option<String>,
    pub framework: Option<String>,
    pub file_count: usize,
    pub package_scripts: Vec<String>,
    pub entry_points: Vec<String>,
    pub readme_preview: Option<String>,
}

pub fn workspace_context(root: &Path) -> AppResult<WorkspaceContext> {
    if !root.is_dir() {
        return Err(AppError::NotADirectory(root.display().to_string()));
    }
    let (language, framework) = detect_stack(root);

    // Quick file-count walk (capped, ignored dirs skipped).
    let mut file_count = 0usize;
    let walker = walkdir::WalkDir::new(root)
        .max_depth(8)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_dir() && IGNORED_DIRS.contains(&name.as_ref()))
        });
    for entry in walker.flatten().take(20_000) {
        if entry.file_type().is_file() { file_count += 1; }
    }

    let package_scripts = read_package_scripts(root);
    let entry_points = detect_entry_points(root);
    let readme_preview = read_readme_preview(root);

    Ok(WorkspaceContext {
        root: root.to_path_buf(),
        language,
        framework,
        file_count,
        package_scripts,
        entry_points,
        readme_preview,
    })
}

fn read_package_scripts(root: &Path) -> Vec<String> {
    let pkg = root.join("package.json");
    if !pkg.exists() { return Vec::new(); }
    let Ok(txt) = std::fs::read_to_string(&pkg) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else { return Vec::new() };
    let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) else { return Vec::new() };
    scripts.keys().cloned().collect()
}

fn detect_entry_points(root: &Path) -> Vec<String> {
    let candidates = [
        "src/main.rs", "src/lib.rs",
        "src/index.ts", "src/index.tsx", "src/main.ts", "src/main.tsx",
        "src/index.js", "src/main.js",
        "main.py", "app.py", "src/main.py",
        "main.go", "cmd/main.go",
        "src/main/java", // dir, indicates Java entry tree
    ];
    candidates.iter()
        .filter(|p| root.join(p).exists())
        .map(|p| (*p).to_string())
        .collect()
}

fn read_readme_preview(root: &Path) -> Option<String> {
    for name in ["README.md", "README.MD", "Readme.md", "readme.md", "README"] {
        let p = root.join(name);
        if !p.exists() { continue; }
        let txt = std::fs::read_to_string(&p).ok()?;
        let trimmed: String = txt
            .lines()
            .skip_while(|l| l.starts_with('#') || l.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(280)
            .collect();
        if !trimmed.is_empty() { return Some(trimmed); }
    }
    None
}

/// Best-effort detection based on marker files at the project root.
fn detect_stack(root: &Path) -> (Option<String>, Option<String>) {
    let has = |f: &str| root.join(f).exists();

    if has("package.json") {
        let framework = if has("next.config.js") || has("next.config.ts") || has("next.config.mjs") {
            Some("next".into())
        } else if has("vite.config.ts") || has("vite.config.js") {
            Some("vite".into())
        } else if has("svelte.config.js") {
            Some("svelte".into())
        } else if has("nuxt.config.ts") {
            Some("nuxt".into())
        } else {
            Some("node".into())
        };
        return (Some("javascript".into()), framework);
    }
    if has("pom.xml") {
        return (Some("java".into()), Some("maven".into()));
    }
    if has("build.gradle") || has("build.gradle.kts") {
        let lang = if has("build.gradle.kts") { "kotlin" } else { "java" };
        return (Some(lang.into()), Some("gradle".into()));
    }
    if has("Cargo.toml") {
        return (Some("rust".into()), Some("cargo".into()));
    }
    if has("requirements.txt") || has("pyproject.toml") || has("setup.py") {
        let framework = if has("pyproject.toml") {
            Some("poetry-or-pep621".into())
        } else {
            Some("pip".into())
        };
        return (Some("python".into()), framework);
    }
    if has("go.mod") {
        return (Some("go".into()), Some("gomod".into()));
    }
    (None, None)
}

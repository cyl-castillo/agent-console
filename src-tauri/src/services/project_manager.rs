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

    let children = if is_dir && depth > 0 {
        let mut entries: Vec<FileNode> = Vec::new();
        let read = std::fs::read_dir(path)?;
        for entry in read.flatten() {
            let entry_path = entry.path();
            let entry_name = entry.file_name().to_string_lossy().to_string();

            // Skip ignored directories.
            if entry_path.is_dir() && IGNORED_DIRS.contains(&entry_name.as_str()) {
                continue;
            }

            entries.push(build_node(&entry_path, depth - 1)?);
        }
        // Dirs first, then alphabetic.
        entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Some(entries)
    } else if is_dir {
        // Mark as expandable but unloaded.
        None
    } else {
        None
    };

    Ok(FileNode {
        name,
        path: path.to_path_buf(),
        is_dir,
        children,
    })
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

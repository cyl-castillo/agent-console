use std::path::Path;

use walkdir::WalkDir;

const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
    ".gradle",
    ".cache",
    ".turbo",
    ".parcel-cache",
    ".angular",
    "coverage",
];

pub fn index_files(root: &Path, limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(limit.min(4096));
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            if e.file_type().is_dir() && IGNORED_DIRS.iter().any(|d| *d == name.as_ref()) {
                return false;
            }
            true
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.is_empty() {
            continue;
        }
        out.push(s);
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_skips_churn_dirs_lists_files_only_and_respects_the_limit() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ac-palette-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(root.join("src/deep")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("README.md"), "x").unwrap();
        std::fs::write(root.join("src/main.rs"), "x").unwrap();
        std::fs::write(root.join("src/deep/util.rs"), "x").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "x").unwrap();
        std::fs::write(root.join(".git/config"), "x").unwrap();

        let mut files = index_files(&root, 1000);
        files.sort();
        // Relative, forward-slash paths; no dirs, nothing from churn dirs.
        assert_eq!(files, vec!["README.md", "src/deep/util.rs", "src/main.rs"]);

        assert_eq!(index_files(&root, 2).len(), 2);

        std::fs::remove_dir_all(&root).ok();
    }
}

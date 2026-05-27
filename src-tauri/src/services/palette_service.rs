use std::path::Path;

use walkdir::WalkDir;

const IGNORED_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", "build", ".next", ".nuxt",
    ".venv", "venv", "__pycache__", ".idea", ".vscode", ".gradle",
    ".cache", ".turbo", ".parcel-cache", ".angular", "coverage",
];

pub fn index_files(root: &Path, limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(limit.min(4096));
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 { return true; }
            let name = e.file_name().to_string_lossy();
            if e.file_type().is_dir() && IGNORED_DIRS.iter().any(|d| *d == name.as_ref()) {
                return false;
            }
            true
        });

    for entry in walker.flatten() {
        if !entry.file_type().is_file() { continue; }
        let Ok(rel) = entry.path().strip_prefix(root) else { continue; };
        let s = rel.to_string_lossy().replace('\\', "/");
        if s.is_empty() { continue; }
        out.push(s);
        if out.len() >= limit { break; }
    }
    out
}

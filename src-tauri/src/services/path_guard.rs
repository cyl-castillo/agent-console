//! Confine filesystem paths handed to IPC commands.
//!
//! Commands like `read_file_text` / `read_tree` / `skill_read` take a path
//! from the webview. The webview is ours — but it also renders agent output,
//! marketplace SKILL.md bodies and CLAUDE.md files, so a rendering bug (or a
//! future one) must not turn into "read any file on the machine". Every such
//! command resolves the path and checks it against the roots the feature is
//! actually about: the open project (and its worktrees) or the CLI config
//! dirs. Symlinks are resolved on both sides, so a link inside the repo that
//! points outside is rejected too.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

/// `path` canonicalized, if it resolves to somewhere under one of `roots`
/// (each root canonicalized as well; roots that don't exist are skipped).
/// A path that doesn't exist can't be confined — it yields `NotFound`, same
/// as the read that would have followed.
pub fn confine(path: &Path, roots: &[PathBuf]) -> AppResult<PathBuf> {
    let canon = path
        .canonicalize()
        .map_err(|_| AppError::NotFound(path.display().to_string()))?;
    let allowed = roots
        .iter()
        .filter_map(|r| r.canonicalize().ok())
        .any(|r| canon.starts_with(&r));
    if allowed {
        Ok(canon)
    } else {
        Err(AppError::InvalidArgument(format!(
            "path is outside the open project: {}",
            path.display()
        )))
    }
}

/// The CLI config dirs a user's skills/commands/agents live in.
pub fn cli_config_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".claude"));
        out.push(home.join(".codex"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ac-path-guard-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn accepts_files_under_a_root_and_rejects_the_rest() {
        let root = tmp("root");
        let other = tmp("other");
        fs::write(root.join("a.txt"), "x").unwrap();
        fs::write(other.join("b.txt"), "y").unwrap();

        let roots = vec![root.clone()];
        assert!(confine(&root.join("a.txt"), &roots).is_ok());
        assert!(confine(&root, &roots).is_ok(), "the root itself is inside");
        assert!(matches!(
            confine(&other.join("b.txt"), &roots),
            Err(AppError::InvalidArgument(_))
        ));
        // `..` escapes are resolved before the check, not string-compared.
        assert!(matches!(
            confine(
                &root
                    .join("..")
                    .join(other.file_name().unwrap())
                    .join("b.txt"),
                &roots
            ),
            Err(AppError::InvalidArgument(_))
        ));
        // Missing paths are NotFound, never "allowed by default".
        assert!(matches!(
            confine(&root.join("nope"), &roots),
            Err(AppError::NotFound(_))
        ));
        // A sibling whose name shares the prefix is NOT inside.
        let sibling = PathBuf::from(format!("{}2", root.display()));
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("c.txt"), "z").unwrap();
        assert!(confine(&sibling.join("c.txt"), &roots).is_err());
        let _ = fs::remove_dir_all(&sibling);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&other);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_pointing_outside_is_rejected() {
        let root = tmp("symroot");
        let other = tmp("symother");
        fs::write(other.join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(other.join("secret"), root.join("link")).unwrap();
        assert!(confine(&root.join("link"), &[root.clone()]).is_err());
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&other);
    }

    #[test]
    fn roots_that_do_not_exist_are_skipped_not_fatal() {
        let root = tmp("ghostroot");
        fs::write(root.join("a"), "").unwrap();
        let roots = vec![PathBuf::from("/definitely/not/here"), root.clone()];
        assert!(confine(&root.join("a"), &roots).is_ok());
        let _ = fs::remove_dir_all(&root);
    }
}

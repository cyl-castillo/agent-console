use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use tauri::{AppHandle, Emitter};

/// Debounced filesystem watcher that emits a single `git://changed` event
/// whenever something inside the project (or under `.git/`) is touched.
///
/// One project = one watcher. Starting a new one replaces the previous.
pub struct GitWatcher {
    inner: Arc<Mutex<Option<Active>>>,
}

struct Active {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    root: PathBuf,
}

impl Default for GitWatcher {
    fn default() -> Self { Self::new() }
}

impl GitWatcher {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(None)) }
    }

    /// Start (or restart) watching `root`. Cheap to call repeatedly; if the
    /// caller asks to watch the already-active root the call is a no-op.
    pub fn watch(&self, app: AppHandle, root: PathBuf) {
        {
            let guard = self.inner.lock().unwrap();
            if let Some(active) = guard.as_ref() {
                if active.root == root { return; }
            }
        }

        let app_handle = app.clone();
        let root_for_filter = root.clone();
        let debouncer = new_debouncer(
            Duration::from_millis(500),
            move |res: DebounceEventResult| {
                let Ok(events) = res else { return };
                if events.iter().any(|e| !ignored(&e.path, &root_for_filter)) {
                    let _ = app_handle.emit("git://changed", ());
                }
            },
        );

        let mut debouncer = match debouncer {
            Ok(d) => d,
            Err(e) => {
                eprintln!("git_watcher: failed to create debouncer: {e}");
                return;
            }
        };

        if let Err(e) = debouncer.watcher().watch(&root, RecursiveMode::Recursive) {
            eprintln!("git_watcher: failed to watch {}: {e}", root.display());
            return;
        }

        let mut guard = self.inner.lock().unwrap();
        *guard = Some(Active { _debouncer: debouncer, root });
    }

    pub fn stop(&self) {
        let mut guard = self.inner.lock().unwrap();
        *guard = None;
    }
}

/// Events we don't want to bubble up as "git changed":
/// - anything under `.git/objects/`, `.git/refs/`, `.git/logs/` — these
///   churn during background gc and on `git status` itself.
/// - typical heavy dirs that change on builds (node_modules, target, dist).
/// We do allow `.git/index` and `.git/HEAD` through, so a commit / checkout
/// is reflected immediately.
fn ignored(path: &Path, root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else { return true };
    let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    if parts.is_empty() { return true; }

    if parts[0] == ".git" {
        if parts.len() == 1 { return true; }
        match parts[1].as_str() {
            "HEAD" | "ORIG_HEAD" | "index" | "MERGE_HEAD" | "FETCH_HEAD" => false,
            _ => true,
        }
    } else {
        matches!(parts[0].as_str(),
            "node_modules" | "target" | "dist" | "build" | ".next" | ".venv" | ".idea" | ".vscode")
    }
}

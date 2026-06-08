use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use tauri::{AppHandle, Emitter};
#[cfg(target_os = "linux")]
use walkdir::WalkDir;

/// Directories we never register watches inside. These churn constantly
/// (builds, package managers, git internals) and recursively watching them
/// would register tens of thousands of inotify descriptors — slow to set up
/// and liable to exhaust `fs.inotify.max_user_watches` on Linux.
#[cfg(target_os = "linux")]
const UNWATCHED_DIRS: &[&str] = &[
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

/// Upper bound on how many directories we'll register watches for. A runaway
/// monorepo shouldn't be able to make startup crawl or blow the inotify limit;
/// past this we stop adding watches (working-tree edits in unwatched dirs just
/// won't auto-refresh until the next manual refresh).
#[cfg(target_os = "linux")]
const MAX_WATCH_DIRS: usize = 8_000;

/// Debounced filesystem watcher that emits a single `git://changed` event
/// whenever something inside the project (or under `.git/`) is touched.
///
/// One project = one watcher. Starting a new one replaces the previous.
pub struct GitWatcher {
    inner: Arc<Mutex<Option<Active>>>,
    /// Bumped on every watch() call. A background setup thread only installs
    /// its watcher if its generation is still the latest, so a fast-finishing
    /// stale thread can't clobber a newer project's watcher.
    generation: Arc<AtomicU64>,
}

struct Active {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    root: PathBuf,
}

impl Default for GitWatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl GitWatcher {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Start (or restart) watching `root`. Cheap to call repeatedly; if the
    /// caller asks to watch the already-active root the call is a no-op.
    ///
    /// Registration runs on a background thread: walking a large workspace and
    /// adding inotify watches can take seconds, and `open_project` must not
    /// block the UI on it. The watcher simply comes alive a moment after the
    /// project opens.
    pub fn watch(&self, app: AppHandle, root: PathBuf) {
        {
            let guard = self.inner.lock().unwrap();
            if let Some(active) = guard.as_ref() {
                if active.root == root {
                    return;
                }
            }
        }

        let inner = Arc::clone(&self.inner);
        let generation = Arc::clone(&self.generation);
        let my_gen = generation.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
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

            register_watches(debouncer.watcher(), &root);

            // A newer watch request may have superseded us while we walked;
            // only install if we're still the latest generation.
            if generation.load(Ordering::SeqCst) != my_gen {
                return;
            }
            let mut guard = inner.lock().unwrap();
            if generation.load(Ordering::SeqCst) != my_gen {
                return;
            }
            *guard = Some(Active {
                _debouncer: debouncer,
                root,
            });
        });
    }

    pub fn stop(&self) {
        // Bump the generation so any in-flight setup thread won't install.
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut guard = self.inner.lock().unwrap();
        *guard = None;
    }
}

/// Register filesystem watches for `root`, excluding heavy churn directories.
///
/// On Linux the backend is inotify, which needs one descriptor per watched
/// directory: a recursive watch on the whole tree would walk into
/// node_modules/target/.git and can exhaust `fs.inotify.max_user_watches`. So
/// we walk the tree ourselves, skip UNWATCHED_DIRS, and add a non-recursive
/// watch per surviving directory (plus a shallow watch on `.git` for
/// commit/checkout/stage signals). New runtime-created subdirs aren't
/// auto-watched — an acceptable gap for a git-status hint that can be
/// refreshed manually.
///
/// On macOS/Windows the backend (FSEvents / ReadDirectoryChangesW) handles
/// recursion natively and efficiently with a single watch, so thousands of
/// per-directory watches would be a regression — we keep one recursive watch.
#[cfg(target_os = "linux")]
fn register_watches(watcher: &mut dyn notify::Watcher, root: &Path) {
    let mut count = 0usize;
    let walker = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            let name = e.file_name().to_string_lossy();
            !(e.file_type().is_dir() && UNWATCHED_DIRS.iter().any(|d| *d == name.as_ref()))
        });
    for entry in walker.flatten() {
        if !entry.file_type().is_dir() {
            continue;
        }
        if watcher
            .watch(entry.path(), RecursiveMode::NonRecursive)
            .is_err()
        {
            continue;
        }
        count += 1;
        if count >= MAX_WATCH_DIRS {
            eprintln!(
                "git_watcher: hit {MAX_WATCH_DIRS} watch cap under {}",
                root.display()
            );
            break;
        }
    }
    let git_dir = root.join(".git");
    if git_dir.is_dir() {
        let _ = watcher.watch(&git_dir, RecursiveMode::NonRecursive);
    }
    if count == 0 {
        eprintln!(
            "git_watcher: watched no directories under {}",
            root.display()
        );
    }
}

#[cfg(not(target_os = "linux"))]
fn register_watches(watcher: &mut dyn notify::Watcher, root: &Path) {
    if let Err(e) = watcher.watch(root, RecursiveMode::Recursive) {
        eprintln!("git_watcher: failed to watch {}: {e}", root.display());
    }
}

/// Events we don't want to bubble up as "git changed":
/// - anything under `.git/objects/`, `.git/refs/`, `.git/logs/` — these
///   churn during background gc and on `git status` itself.
/// - typical heavy dirs that change on builds (node_modules, target, dist).
/// We do allow `.git/index` and `.git/HEAD` through, so a commit / checkout
/// is reflected immediately.
fn ignored(path: &Path, root: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(root) else {
        return true;
    };
    let parts: Vec<_> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if parts.is_empty() {
        return true;
    }

    if parts[0] == ".git" {
        if parts.len() == 1 {
            return true;
        }
        match parts[1].as_str() {
            "HEAD" | "ORIG_HEAD" | "index" | "MERGE_HEAD" | "FETCH_HEAD" => false,
            _ => true,
        }
    } else {
        matches!(
            parts[0].as_str(),
            "node_modules" | "target" | "dist" | "build" | ".next" | ".venv" | ".idea" | ".vscode"
        )
    }
}

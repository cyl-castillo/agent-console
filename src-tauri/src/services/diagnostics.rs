//! Observability: a log file the user can find, a panic that leaves a trace,
//! and a diagnostics bundle that turns "it doesn't work" into a report.
//!
//! Until now every backend failure went to stderr via `eprintln!`. On Windows
//! (GUI subsystem) stderr goes nowhere; on Linux/macOS it lives only in the
//! terminal the app happened to be launched from — which for a packaged app is
//! nobody's. Field reports (#72, #104, #138) and the two-month-dead memory
//! injection were debugged blind because of this.
//!
//! - `init_logging` installs `tracing` with a daily-rolling file under the
//!   platform data dir (`agent-console/logs/`, 7 files kept) plus stderr in
//!   debug builds. Level via `AGENT_CONSOLE_LOG` (EnvFilter syntax), default
//!   `info`.
//! - `install_panic_hook` writes the panic (message + location) through the
//!   same log before the default hook runs, so a crash is on disk.
//! - `render` turns a `DiagnosticsBundle` into the markdown a user pastes into
//!   an issue: build, environment, hooks state, store sizes and the log tail,
//!   with the home directory redacted.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

/// Daily files kept before the oldest is deleted.
const MAX_LOG_FILES: usize = 7;
/// Lines of the current log a bundle carries.
pub const LOG_TAIL_LINES: usize = 300;
/// Never read more than this from the end of a log for the tail.
const TAIL_READ_BYTES: u64 = 512 * 1024;
const LOG_PREFIX: &str = "agent-console";
const LOG_SUFFIX: &str = "log";

/// `<data_local>/agent-console/logs`, created on demand.
pub fn log_dir() -> Option<PathBuf> {
    let dir = dirs::data_local_dir()?.join("agent-console").join("logs");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Install the global subscriber. Returns the log directory when the file
/// layer is live; `None` means logging degraded to stderr only (no writable
/// data dir) — the app still runs.
pub fn init_logging() -> Option<PathBuf> {
    let filter = EnvFilter::try_from_env("AGENT_CONSOLE_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,agent_console_lib=info"));
    let dir = log_dir();
    let file_layer = dir.as_ref().and_then(|d| {
        let appender = tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(LOG_PREFIX)
            .filename_suffix(LOG_SUFFIX)
            .max_log_files(MAX_LOG_FILES)
            .build(d)
            .ok()?;
        Some(
            tracing_subscriber::fmt::layer()
                .with_writer(appender)
                .with_ansi(false)
                .with_target(true),
        )
    });
    let stderr_layer = (cfg!(debug_assertions) || file_layer.is_none())
        .then(|| tracing_subscriber::fmt::layer().with_writer(std::io::stderr));
    let installed = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer.is_some().then_some(()).and(file_layer))
        .with(stderr_layer)
        .try_init()
        .is_ok();
    if !installed {
        return None;
    }
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        commit = env!("AC_BUILD_COMMIT"),
        debug = cfg!(debug_assertions),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        "agent-console starting"
    );
    dir
}

/// Log the panic before the default hook (stderr + unwind) sees it. The
/// file appender writes synchronously, so the line is on disk even when the
/// process dies right after.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_string());
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown location".to_string());
        tracing::error!(target: "panic", %location, "{msg}");
        default(info);
    }));
}

/// Newest log file in `dir` by modification time.
pub fn current_log_file(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let name = p.file_name()?.to_string_lossy().to_string();
        if !name.starts_with(LOG_PREFIX) || !p.is_file() {
            continue;
        }
        let m = e.metadata().ok()?.modified().ok()?;
        if best.as_ref().is_none_or(|(t, _)| m > *t) {
            best = Some((m, p));
        }
    }
    best.map(|(_, p)| p)
}

/// Last `n` lines of `path`, reading at most `TAIL_READ_BYTES` from the end.
/// A file that can't be read yields an empty tail — the bundle still renders.
pub fn tail_lines(path: &Path, n: usize) -> Vec<String> {
    let Ok(mut f) = fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(TAIL_READ_BYTES);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut buf = Vec::new();
    if f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text.lines().collect();
    // A window cut mid-line starts with a fragment: drop it.
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    let skip = lines.len().saturating_sub(n);
    lines[skip..].iter().map(|s| s.to_string()).collect()
}

/// One immediate entry of a data directory with its size (files: own size,
/// directories: recursive sum).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DirEntryReport {
    pub name: String,
    pub bytes: u64,
    pub is_dir: bool,
}

/// Sizes of the immediate entries of `dir`, largest first. Missing dir ⇒ empty.
pub fn dir_report(dir: &Path) -> Vec<DirEntryReport> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<DirEntryReport> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_string_lossy().to_string();
            let is_dir = p.is_dir();
            let bytes = if is_dir {
                walkdir::WalkDir::new(&p)
                    .into_iter()
                    .flatten()
                    .filter(|e| e.file_type().is_file())
                    .filter_map(|e| e.metadata().ok())
                    .map(|m| m.len())
                    .sum()
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            };
            Some(DirEntryReport {
                name,
                bytes,
                is_dir,
            })
        })
        .collect();
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.name.cmp(&b.name)));
    out
}

/// Everything the bundle renders. Collected by the IPC command (it needs app
/// state); rendered here so the shape is testable without a running app.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsBundle {
    pub app_version: String,
    pub commit: String,
    pub build_time_secs: u64,
    pub debug: bool,
    pub snap: bool,
    pub os: String,
    pub arch: String,
    pub project_root: Option<String>,
    pub project_branch: Option<String>,
    pub hooks: serde_json::Value,
    pub preflight: serde_json::Value,
    pub inject_port_file: bool,
    pub data_dir: Option<String>,
    pub data_entries: Vec<DirEntryReport>,
    pub cache_dir: Option<String>,
    pub cache_entries: Vec<DirEntryReport>,
    pub log_file: Option<String>,
    pub log_tail: Vec<String>,
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Replace the user's home directory with `~` wherever it appears. The bundle
/// is meant to be pasted into a public issue; paths are useful, the username
/// in them is not.
pub fn redact_home(text: &str, home: Option<&Path>) -> String {
    let Some(home) = home.map(|h| h.to_string_lossy().to_string()) else {
        return text.to_string();
    };
    if home.is_empty() || home == "/" {
        return text.to_string();
    }
    let mut out = text.replace(&home, "~");
    // Windows paths inside JSON come escaped (`C:\\Users\\me`).
    let escaped = home.replace('\\', "\\\\");
    if escaped != home {
        out = out.replace(&escaped, "~");
    }
    out
}

/// Markdown for an issue or a support thread.
pub fn render(b: &DiagnosticsBundle) -> String {
    let mut s = String::new();
    s.push_str("## Agent Console diagnostics\n\n");
    s.push_str(&format!(
        "- Version: `v{}` · commit `{}`{}{}\n",
        b.app_version,
        b.commit,
        if b.debug { " · debug" } else { "" },
        if b.snap { " · snap" } else { "" }
    ));
    if b.build_time_secs > 0 {
        s.push_str(&format!("- Built: unix `{}`\n", b.build_time_secs));
    }
    s.push_str(&format!("- OS: `{} {}`\n", b.os, b.arch));
    match (&b.project_root, &b.project_branch) {
        (Some(r), Some(br)) => s.push_str(&format!("- Project: `{r}` @ `{br}`\n")),
        (Some(r), None) => s.push_str(&format!("- Project: `{r}`\n")),
        _ => s.push_str("- Project: none open\n"),
    }
    s.push_str(&format!(
        "- Inject endpoint port file: {}\n",
        if b.inject_port_file {
            "present"
        } else {
            "missing"
        }
    ));

    s.push_str("\n### Hooks\n\n```json\n");
    s.push_str(&serde_json::to_string_pretty(&b.hooks).unwrap_or_default());
    s.push_str("\n```\n");

    s.push_str("\n### Preflight\n\n```json\n");
    s.push_str(&serde_json::to_string_pretty(&b.preflight).unwrap_or_default());
    s.push_str("\n```\n");

    let dir_block =
        |s: &mut String, title: &str, dir: &Option<String>, entries: &[DirEntryReport]| {
            s.push_str(&format!("\n### {title}\n\n"));
            match dir {
                Some(d) => s.push_str(&format!("`{d}`\n\n")),
                None => s.push_str("(unavailable)\n\n"),
            }
            let total: u64 = entries.iter().map(|e| e.bytes).sum();
            for e in entries.iter().take(20) {
                s.push_str(&format!(
                    "- {}{} — {}\n",
                    e.name,
                    if e.is_dir { "/" } else { "" },
                    human(e.bytes)
                ));
            }
            if entries.len() > 20 {
                s.push_str(&format!("- … {} more\n", entries.len() - 20));
            }
            s.push_str(&format!("- **total — {}**\n", human(total)));
        };
    dir_block(&mut s, "Data dir", &b.data_dir, &b.data_entries);
    dir_block(&mut s, "Cache dir", &b.cache_dir, &b.cache_entries);

    s.push_str("\n### Log tail\n\n");
    match &b.log_file {
        Some(f) => s.push_str(&format!("`{f}` (last {} lines)\n\n", b.log_tail.len())),
        None => s.push_str("(no log file)\n\n"),
    }
    s.push_str("```\n");
    for l in &b.log_tail {
        s.push_str(l);
        s.push('\n');
    }
    s.push_str("```\n");
    redact_home(&s, dirs::home_dir().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ac-diag-{name}-{}-{}",
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
    fn tail_returns_the_last_n_whole_lines() {
        let d = tmp("tail");
        let p = d.join("x.log");
        let body: String = (1..=50).map(|i| format!("line {i}\n")).collect();
        fs::write(&p, body).unwrap();
        let t = tail_lines(&p, 3);
        assert_eq!(t, vec!["line 48", "line 49", "line 50"]);
        assert_eq!(tail_lines(&p, 500).len(), 50, "n larger than the file");
        assert!(tail_lines(&d.join("missing"), 3).is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn tail_drops_the_fragment_when_the_window_cuts_a_line() {
        let d = tmp("tailbig");
        let p = d.join("big.log");
        // > TAIL_READ_BYTES so the read starts mid-file.
        let line = "x".repeat(100);
        let body: String = (0..8000).map(|i| format!("{i}:{line}\n")).collect();
        fs::write(&p, body).unwrap();
        let t = tail_lines(&p, 2);
        assert_eq!(t.len(), 2);
        assert!(t[1].starts_with("7999:"));
        assert!(t[0].starts_with("7998:"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn dir_report_sums_directories_and_sorts_by_size() {
        let d = tmp("dir");
        fs::write(d.join("small.json"), "12").unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub").join("a"), "x".repeat(1000)).unwrap();
        fs::write(d.join("sub").join("b"), "y".repeat(1000)).unwrap();
        let r = dir_report(&d);
        assert_eq!(r[0].name, "sub");
        assert!(r[0].is_dir);
        assert_eq!(r[0].bytes, 2000);
        assert_eq!(r[1].name, "small.json");
        assert_eq!(r[1].bytes, 2);
        assert!(dir_report(&d.join("nope")).is_empty());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn current_log_file_picks_the_newest_matching_file() {
        let d = tmp("logs");
        fs::write(d.join("agent-console.2026-01-01.log"), "old").unwrap();
        fs::write(d.join("unrelated.txt"), "no").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(d.join("agent-console.2026-01-02.log"), "new").unwrap();
        let f = current_log_file(&d).unwrap();
        assert_eq!(f.file_name().unwrap(), "agent-console.2026-01-02.log");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn redact_home_replaces_plain_and_escaped_forms() {
        let home = Path::new("/home/carla");
        assert_eq!(
            redact_home("at /home/carla/x and /home/carla/y", Some(home)),
            "at ~/x and ~/y"
        );
        assert_eq!(redact_home("no home here", Some(home)), "no home here");
        assert_eq!(redact_home("/home/carla/x", None), "/home/carla/x");
        let win = Path::new("C:\\Users\\carla");
        assert_eq!(
            redact_home(
                "\"C:\\\\Users\\\\carla\\\\a\" C:\\Users\\carla\\b",
                Some(win)
            ),
            "\"~\\\\a\" ~\\b"
        );
        // Degenerate homes must not erase every slash in the text.
        assert_eq!(redact_home("/a/b", Some(Path::new("/"))), "/a/b");
    }

    #[test]
    fn render_carries_every_section_and_human_sizes() {
        let b = DiagnosticsBundle {
            app_version: "9.9.9".into(),
            commit: "abc1234".into(),
            build_time_secs: 1,
            debug: true,
            snap: false,
            os: "linux".into(),
            arch: "x86_64".into(),
            project_root: Some("/p/root".into()),
            project_branch: Some("main".into()),
            hooks: serde_json::json!({"installed": true}),
            preflight: serde_json::json!({"tools": []}),
            inject_port_file: true,
            data_dir: Some("/d".into()),
            data_entries: vec![DirEntryReport {
                name: "sessions.json".into(),
                bytes: 1_500_000,
                is_dir: false,
            }],
            cache_dir: None,
            cache_entries: vec![],
            log_file: Some("/d/logs/agent-console.log".into()),
            log_tail: vec!["l1".into(), "l2".into()],
        };
        let out = render(&b);
        assert!(out.contains("`v9.9.9` · commit `abc1234` · debug"));
        assert!(out.contains("- Project: `/p/root` @ `main`"));
        assert!(out.contains("\"installed\": true"));
        assert!(out.contains("- sessions.json — 1.4 MB"));
        assert!(out.contains("**total — 1.4 MB**"));
        assert!(out.contains("### Cache dir\n\n(unavailable)"));
        assert!(out.contains("(last 2 lines)"));
        assert!(out.contains("```\nl1\nl2\n```"));
        assert_eq!(human(0), "0 B");
        assert_eq!(human(1023), "1023 B");
        assert_eq!(human(1024), "1.0 KB");
    }
}

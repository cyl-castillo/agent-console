//! Spawning child processes WITHOUT a flashing console window on Windows.
//!
//! A GUI app that shells out to `git`/`gh` via `Command::new(...)` makes
//! Windows pop a console window for each child process — it flashes briefly
//! over whatever the user is doing (Teams, a browser, anything). When a
//! service polls git on a refresh, several of those windows blink in quick
//! succession. Routing every spawn through `command()` applies
//! `CREATE_NO_WINDOW` so the window never appears. On non-Windows platforms
//! this is a plain `Command::new`.
//!
//! `claude_cli::command()` keeps its own copy of this flag because it also
//! configures stdio; everything else should go through here.

use std::ffi::OsStr;
use std::process::Command;

/// How far up a process tree we are willing to walk. A CLI launched from a PTY
/// shell sits one or two levels down (`shell → claude`, or `shell → wrapper →
/// claude`); the bound is a cycle/runaway guard, not a real limit.
pub const MAX_ANCESTOR_DEPTH: usize = 16;

/// Parent pid of `pid`, or `None` when we can't tell (process gone, platform
/// without a cheap way to ask). Callers must treat `None` as "unknown", never
/// as "no parent" — an unknown chain means we decline to bind, not that we
/// bind to the wrong thing.
pub fn parent_of(pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        // /proc/<pid>/stat is one line: `pid (comm) state ppid ...`. `comm` can
        // contain spaces and parentheses, so split after the LAST ')'.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after = &stat[stat.rfind(')')? + 1..];
        after.split_whitespace().nth(1)?.parse().ok()
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        // macOS and friends: ask ps for just the ppid column.
        let out = command("ps")
            .args(["-o", "ppid=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    }
    // Windows has no equally cheap probe, and every caller degrades to "no
    // match" rather than guessing, so we simply never claim a parent there.
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

/// Walk up from `pid` and report whether `ancestor` is on the chain (a process
/// counts as its own ancestor). Stops at the depth bound, at pid 0/1, and on
/// any repeated pid, so a broken or racing process table can't loop forever.
pub fn descends_from(pid: u32, ancestor: u32, parent_of: &dyn Fn(u32) -> Option<u32>) -> bool {
    let mut current = pid;
    let mut seen = Vec::with_capacity(MAX_ANCESTOR_DEPTH);
    for _ in 0..MAX_ANCESTOR_DEPTH {
        if current == ancestor {
            return true;
        }
        if current <= 1 || seen.contains(&current) {
            return false;
        }
        seen.push(current);
        match parent_of(current) {
            Some(parent) => current = parent,
            None => return false,
        }
    }
    false
}

/// `Command::new(program)` that never flashes a console window on Windows.
pub fn command<S: AsRef<OsStr>>(program: S) -> Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

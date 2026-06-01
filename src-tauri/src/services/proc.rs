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

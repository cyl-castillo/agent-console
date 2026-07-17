use std::process::Command;

fn main() {
    // Build provenance, compiled into the binary: which commit and when.
    // Exists because we debugged against stale dev binaries twice in one week
    // with no way to tell from the app itself (MEJORAS-2026-07 R2.7).
    let commit = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let dirty = Command::new("git")
        .args(["status", "--porcelain"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false);
    println!(
        "cargo:rustc-env=AC_BUILD_COMMIT={commit}{}",
        if dirty { "+dirty" } else { "" }
    );
    // Seconds since epoch; the frontend formats it. Chrono-free on purpose.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=AC_BUILD_TIME={now}");
    // Re-run when HEAD moves so the commit doesn't go stale across checkouts.
    println!("cargo:rerun-if-changed=../.git/HEAD");

    tauri_build::build()
}

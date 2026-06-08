use crate::services::proc;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

const REPO: &str = "cyl-castillo/agent-console";
const FEEDBACK_LABEL: &str = "feedback";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackInput {
    pub title: String,
    pub description: String,
    /// "bug" | "feature" | "ux" | "other"
    pub category: String,
    /// "low" | "medium" | "high"
    pub severity: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackContext {
    pub app_version: String,
    pub os: String,
    pub project_name: Option<String>,
    pub branch: Option<String>,
}

pub fn dev_enabled() -> bool {
    matches!(std::env::var("AGENT_CONSOLE_DEV"), Ok(v) if !v.is_empty() && v != "0")
}

pub fn context(project_root: Option<&Path>, project_name: Option<&str>) -> FeedbackContext {
    FeedbackContext {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        os: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        project_name: project_name.map(|s| s.to_string()),
        branch: project_root.and_then(git_branch),
    }
}

fn git_branch(root: &Path) -> Option<String> {
    let out = proc::command("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

pub fn submit(input: FeedbackInput, ctx: &FeedbackContext) -> AppResult<String> {
    if !dev_enabled() {
        return Err(AppError::InvalidArgument(
            "feedback panel is disabled (set AGENT_CONSOLE_DEV=1)".into(),
        ));
    }
    let title = input.title.trim();
    let desc = input.description.trim();
    if title.is_empty() {
        return Err(AppError::InvalidArgument("title is required".into()));
    }
    if desc.is_empty() {
        return Err(AppError::InvalidArgument("description is required".into()));
    }
    let cat = sanitize_token(&input.category, &["bug", "feature", "ux", "other"]);
    let sev = sanitize_token(&input.severity, &["low", "medium", "high"]);
    let full_title = format!("[{cat}][{sev}] {title}");
    let body = format_body(desc, ctx, &cat, &sev);

    let out = proc::command("gh")
        .args(["issue", "create", "--repo", REPO, "--label", FEEDBACK_LABEL])
        .arg("--title")
        .arg(&full_title)
        .arg("--body")
        .arg(&body)
        .output()
        .map_err(|e| {
            AppError::Other(format!(
                "gh CLI not available ({e}). Install: https://cli.github.com/"
            ))
        })?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(AppError::Other(format!("gh issue create failed: {stderr}")));
    }
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let url = stdout
        .lines()
        .rev()
        .find(|l| l.contains("github.com"))
        .map(|s| s.to_string())
        .unwrap_or(stdout);
    Ok(url)
}

fn sanitize_token(v: &str, allowed: &[&str]) -> String {
    let v = v.trim().to_lowercase();
    if allowed.iter().any(|a| *a == v) {
        v
    } else {
        allowed[0].to_string()
    }
}

fn format_body(description: &str, ctx: &FeedbackContext, cat: &str, sev: &str) -> String {
    let mut s = String::new();
    s.push_str(description);
    s.push_str("\n\n---\n\n");
    s.push_str("**Submitted from Agent Console**\n\n");
    s.push_str(&format!("- Category: `{cat}`\n"));
    s.push_str(&format!("- Severity: `{sev}`\n"));
    s.push_str(&format!("- App version: `{}`\n", ctx.app_version));
    s.push_str(&format!("- OS: `{}`\n", ctx.os));
    if let Some(p) = &ctx.project_name {
        s.push_str(&format!("- Project: `{p}`\n"));
    }
    if let Some(b) = &ctx.branch {
        s.push_str(&format!("- Branch: `{b}`\n"));
    }
    s
}

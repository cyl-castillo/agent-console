use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// MCP servers are managed through the `claude mcp` CLI (no --json output, so
/// we parse the human-readable text of `mcp list` + `mcp get <name>`). This
/// keeps us in lockstep with Claude Code's own config across local/user/project
/// scopes without touching its internal config files.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub name: String,
    /// "local" | "user" | "project" | null
    pub scope: Option<String>,
    /// "stdio" | "http" | "sse" | null
    pub transport: Option<String>,
    pub command: Option<String>,
    pub args: Option<String>,
    pub url: Option<String>,
    pub env: Vec<String>,
    /// "connected" | "failed" | "pending" | "unknown"
    pub status: String,
    pub connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAddInput {
    pub name: String,
    /// "stdio" | "http" | "sse"
    pub transport: String,
    /// "local" | "user" | "project"
    pub scope: String,
    /// For stdio: the command line (command + args, space-separated).
    /// For http/sse: the server URL.
    pub command_or_url: String,
    /// `KEY=VALUE` entries (stdio).
    #[serde(default)]
    pub env: Vec<String>,
    /// `Name: Value` entries (http/sse).
    #[serde(default)]
    pub headers: Vec<String>,
}

fn claude_command(args: &[&str]) -> Command {
    // Delegates to the shared resolver so a GUI launch (no login-shell PATH)
    // still finds the `claude` binary. stdio + Windows no-window set there.
    crate::services::claude_cli::command(args)
}

fn run(args: &[&str], cwd: Option<&str>) -> AppResult<String> {
    let mut cmd = claude_command(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let output = cmd.output().map_err(|e| {
        AppError::Other(format!("failed to run `claude mcp`: {e}. Is claude on PATH?"))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let detail = if !stderr.trim().is_empty() { stderr } else { stdout };
        return Err(AppError::Other(detail.trim().to_string()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn name_re_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Names of configured servers, parsed from `claude mcp list`. Lines look like:
///   `memory: memory server - ✓ Connected`
fn list_names(cwd: Option<&str>) -> AppResult<Vec<String>> {
    let out = run(&["mcp", "list"], cwd)?;
    let mut names = Vec::new();
    for line in out.lines() {
        let line = line.trim_end();
        // Skip the "Checking MCP server health…" header and blank lines.
        if line.is_empty() || !line.contains(':') {
            continue;
        }
        if line.starts_with("Checking") || line.starts_with("No MCP") {
            continue;
        }
        if let Some((name, _rest)) = line.split_once(':') {
            let name = name.trim();
            if !name.is_empty() && !name.contains(' ') {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
}

/// Parse `claude mcp get <name>` detail text into an McpServer.
fn parse_get(name: &str, text: &str) -> McpServer {
    let mut scope = None;
    let mut transport = None;
    let mut command = None;
    let mut args = None;
    let mut url = None;
    let mut env = Vec::new();
    let mut status = "unknown".to_string();
    let mut connected = false;
    let mut in_env = false;

    for raw in text.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim();
        // Environment block: indented KEY=VALUE lines follow "Environment:".
        if in_env {
            if line.starts_with("    ") && trimmed.contains('=') && !trimmed.is_empty() {
                env.push(trimmed.to_string());
                continue;
            }
            in_env = false;
        }
        if let Some(v) = trimmed.strip_prefix("Scope:") {
            let v = v.trim();
            scope = Some(if v.contains("Local") {
                "local"
            } else if v.contains("User") {
                "user"
            } else if v.contains("Project") {
                "project"
            } else {
                "unknown"
            }
            .to_string());
        } else if let Some(v) = trimmed.strip_prefix("Status:") {
            if v.contains("Connected") {
                status = "connected".into();
                connected = true;
            } else if v.contains("Pending") {
                status = "pending".into();
            } else if v.contains("Failed") {
                status = "failed".into();
            }
        } else if let Some(v) = trimmed.strip_prefix("Type:") {
            transport = Some(v.trim().to_string());
        } else if let Some(v) = trimmed.strip_prefix("Command:") {
            command = Some(v.trim().to_string());
        } else if let Some(v) = trimmed.strip_prefix("Args:") {
            let v = v.trim();
            if !v.is_empty() {
                args = Some(v.to_string());
            }
        } else if let Some(v) = trimmed.strip_prefix("URL:") {
            url = Some(v.trim().to_string());
        } else if trimmed.starts_with("Environment:") {
            in_env = true;
        }
    }

    McpServer {
        name: name.to_string(),
        scope,
        transport,
        command,
        args,
        url,
        env,
        status,
        connected,
    }
}

/// List all configured MCP servers with details. `cwd` should be the project
/// root so project-scoped (.mcp.json) and local servers resolve correctly.
pub fn list_servers(cwd: Option<&str>) -> AppResult<Vec<McpServer>> {
    let names = list_names(cwd)?;
    let mut servers = Vec::with_capacity(names.len());
    for name in names {
        match run(&["mcp", "get", &name], cwd) {
            Ok(text) => servers.push(parse_get(&name, &text)),
            Err(_) => servers.push(McpServer {
                name,
                scope: None,
                transport: None,
                command: None,
                args: None,
                url: None,
                env: Vec::new(),
                status: "unknown".into(),
                connected: false,
            }),
        }
    }
    Ok(servers)
}

/// Add a server. Arg vectors are passed directly to the process (no shell), so
/// values can't be shell-injected; we still validate the name and scope.
pub fn add_server(input: &McpAddInput, cwd: Option<&str>) -> AppResult<String> {
    if !name_re_ok(&input.name) {
        return Err(AppError::InvalidArgument(format!("invalid server name: {}", input.name)));
    }
    let scope = match input.scope.as_str() {
        "local" | "user" | "project" => input.scope.as_str(),
        _ => "local",
    };
    let transport = match input.transport.as_str() {
        "stdio" | "http" | "sse" => input.transport.as_str(),
        _ => "stdio",
    };
    let target = input.command_or_url.trim();
    if target.is_empty() {
        return Err(AppError::InvalidArgument("command or URL is required".into()));
    }

    let mut args: Vec<String> = vec![
        "mcp".into(),
        "add".into(),
        "-s".into(),
        scope.into(),
        "-t".into(),
        transport.into(),
        input.name.clone(),
    ];

    if transport == "stdio" {
        // env flags go after the name and before `--`.
        for e in &input.env {
            let e = e.trim();
            if e.contains('=') {
                args.push("-e".into());
                args.push(e.to_string());
            }
        }
        // The command + its args follow `--` so subprocess flags aren't parsed.
        args.push("--".into());
        for tok in target.split_whitespace() {
            args.push(tok.to_string());
        }
    } else {
        // http/sse: url is the positional, headers are variadic at the end.
        args.push(target.to_string());
        for h in &input.headers {
            let h = h.trim();
            if h.contains(':') {
                args.push("-H".into());
                args.push(h.to_string());
            }
        }
    }

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = run(&args_ref, cwd)?;
    Ok(out.trim().to_string())
}

/// Remove a server from a given scope.
pub fn remove_server(name: &str, scope: &str, cwd: Option<&str>) -> AppResult<String> {
    if !name_re_ok(name) {
        return Err(AppError::InvalidArgument(format!("invalid server name: {name}")));
    }
    let scope = match scope {
        "local" | "user" | "project" => scope,
        _ => "local",
    };
    let out = run(&["mcp", "remove", name, "-s", scope], cwd)?;
    Ok(out.trim().to_string())
}

//! How risky is a tool request? One conservative classifier, shared by the
//! console (the tier on a DecisionCard, what a paired device or a sponsor may
//! approve) and by the hook bridge (the firewall guard, which runs before the
//! tool and outside the console). Pure: patterns over the request, no I/O.
//!
//! `src/lib/decisions.ts` mirrors it for the card the desktop draws.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

pub static DANGEROUS: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    [
        // Remote code execution in every spelling: `curl … | sh`, `| zsh`,
        // `sh <(curl …)`, `bash -c "$(curl …)"`, `source <(wget …)`.
        (
            r"(?i)\b(curl|wget)\b[^|#\n]*\|\s*(sudo\s+)?(ba|z|da|k|fi|c|tc)?sh\b",
            "downloads and executes remote code",
        ),
        (
            r"(?i)\b(ba|z|da|k)?sh\b[^\n]*\$\(\s*(curl|wget)\b",
            "downloads and executes remote code",
        ),
        (
            r"(?i)\b(source|\.|(ba|z|da|k)?sh)\s+<\(\s*(curl|wget)\b",
            "downloads and executes remote code",
        ),
        (
            r"(?i)>\s*/dev/(sda|hda|nvme|disk)\d*",
            "raw block device write",
        ),
        (r"(?i)\bdd\s+if=", "raw block device write"),
        (r"(?i)\bmkfs\b", "formats a filesystem"),
        // Deleting with any flag spelling, or through find/xargs.
        (
            r"(?i)\brm\b(\s+\S+)*\s+(-[a-z]*[rf]|--recursive|--force|--no-preserve-root)\b",
            "removes files recursively or by force",
        ),
        (
            r"(?i)\bfind\b[^\n]*\s-(exec|execdir|ok|okdir)\s+(sudo\s+)?rm\b",
            "removes matched files",
        ),
        (r"(?i)\bxargs\b[^\n]*\brm\b", "removes files through xargs"),
        // Rewriting remote history, with `-c k=v` / `-C dir` allowed between.
        (
            r"(?i)\bgit\b(\s+-[cC]\s*\S+)*\s+push\b[^\n]*(--force\b|--force-with-lease\b|\s-f\b|\s\+\S)",
            "force-push rewrites remote history",
        ),
        // Publishing and infrastructure.
        (r"(?i)\b(npm|pnpm|yarn|cargo|gem|twine)\s+publish\b", "publishes a package"),
        (
            r"(?i)\bterraform\s+(apply|destroy)\b",
            "changes infrastructure",
        ),
        (
            r"(?i)\bkubectl\s+(apply|delete|drain|replace)\b",
            "changes Kubernetes resources",
        ),
        (
            r"(?i)\b(aws|gcloud|az)\b[^\n]*\b(rm|delete|terminate|destroy)\b",
            "destroys cloud resources",
        ),
        // Writing where the next prompt or the next login runs code.
        (
            r"(?i)>>?\s*~?/?[^\s]*(\.ssh/|\.claude/|\.bashrc|\.zshrc|\.profile|\.bash_profile|\.zprofile|\.git/hooks/|/etc/)",
            "writes where code runs on the next prompt or login",
        ),
        (
            r"(?i)\b(tee|cp|mv|install)\b[^\n]*\s~?/?[^\s]*(\.ssh/|\.claude/|\.bashrc|\.zshrc|\.profile|\.git/hooks/|/etc/)",
            "writes where code runs on the next prompt or login",
        ),
    ]
    .into_iter()
    .map(|(re, why)| (Regex::new(re).expect("static regex"), why))
    .collect()
});

pub static CAUTION: LazyLock<Vec<(Regex, &str)>> = LazyLock::new(|| {
    [
        (r"(?i)\bgit\b(\s+-[cC]\s*\S+)*\s+push\b", "pushes to remote"),
        (
            r"(?i)\bgit\s+reset\s+--hard\b",
            "discards uncommitted changes",
        ),
        (r"(?i)\bgit\s+rebase\b", "rewrites commit history"),
        (r"(?i)\bgit\s+clean\s+-[a-z]*f", "removes untracked files"),
        (r"(?i)\bgit\s+restore\b", "discards working-tree changes"),
        (
            r"(?i)\bgit\s+checkout\s+(--\s|\.(\s|$))",
            "discards working-tree changes",
        ),
        (r"\bgit\s+branch\s+-[a-z]*D", "force-deletes a branch"),
        (r"(?i)\btruncate\b", "truncates a file"),
        (r"(?i)\bfind\b[^|]*\s-delete\b", "deletes matched files"),
        (r"(?i)\bsudo\b", "elevated privileges"),
        (r"(?i)\bchmod\b", "changes file permissions"),
        (
            r"(?i)\bapt(-get)?\s+(install|remove|purge)\b",
            "modifies system packages",
        ),
        (
            r"(?i)\bnpm\s+(install|uninstall)\s+-g\b",
            "modifies global packages",
        ),
        (r"(?i)\bpip\s+install\b", "installs Python package"),
        (r"(?i)\byarn\s+global\b", "modifies global packages"),
        (
            r"(?i)\bdocker\s+(rm|rmi|system\s+prune)\b",
            "destroys containers or images",
        ),
    ]
    .into_iter()
    .map(|(re, why)| (Regex::new(re).expect("static regex"), why))
    .collect()
});

pub static HIGH_REASON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)remote|delete|discards|destroys|removes|elevated|truncates|rewrites")
        .expect("static regex")
});

/// The read-only allowlist: one simple command whose head is here, followed
/// by whitespace or the end (`pwd=1 sh ./x.sh` is a shell with an assignment
/// in front, not `pwd`). Code runners (`node -e`, `python -c`) are not: they
/// are shells with another syntax. Neither are test, lint and build runners
/// (see `RUNNER`), nor `printenv`, which prints every token in the
/// environment. Mirrors `lib/decisions.ts`.
pub static READ_ONLY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(ls|pwd|cat|head|tail|wc|echo|which|type|date|whoami|stat|file|du|df|tree|rg|grep|egrep|fgrep|git\s+(status|log|diff|show|rev-parse|describe|blame|stash\s+list)|git\s+remote\s+-v|npm\s+(ls|outdated|view)|cargo\s+(fmt\s+--check|tree|metadata))(?:\s|$)",
    )
    .expect("static regex")
});

/// Tests, linters and builds execute what the repository says: a script in
/// `package.json`, a `Makefile` target, `conftest.py`, `build.rs`, an ESLint
/// config. The agent can edit all of those, so "run the tests" is "run a
/// program the agent may have just written" — never read-only, and so never
/// approvable from a device. Mirrors `RUNNER` in `lib/decisions.ts`.
pub static RUNNER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(npm\s+(test|t|run(-script)?)|pnpm\s+(test|run|exec)|yarn\s+(test|run)|npx|bunx?|cargo\s+(check|test|clippy|build|run|bench)|make|pytest|tox|go\s+(test|vet|build|run|generate))(?:\s|$)",
    )
    .expect("static regex")
});

/// Paths that hold credentials. Reading one hands a secret to the model and
/// to whatever it calls next, so a read of these is high whatever the tool.
/// Narrower than `SENSITIVE_PATH` on purpose: `.claude/` or `/etc/` are
/// dangerous to write, not to list.
pub static SECRET_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(^|/)(\.ssh|\.gnupg|\.aws|\.kube|\.docker|\.npmrc|\.netrc|\.git-credentials|\.env(\.[^/]+)?)(/|$)|(^|/)\.(claude|codex)/[^/]*(credentials|auth)[^/]*$|\.(pem|key|p12|pfx)$",
    )
    .expect("static regex")
});
/// `.env.example` and its kin are documentation, not secrets.
pub static ENV_TEMPLATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\.env\.(example|sample|template|dist)$").expect("static regex")
});

pub fn secret_path(path: &str) -> bool {
    SECRET_PATH.is_match(path) && !ENV_TEMPLATE.is_match(path)
}

/// The first argument of a shell command that names a credential path.
/// Words are split on whitespace and on the characters that glue a path to
/// something else (`--file=.env`, `<.env`, `a:b`), quotes dropped.
pub fn secret_arg(cmd: &str) -> Option<String> {
    cmd.split(|c: char| c.is_whitespace() || "=|;&<>()`,:".contains(c))
        .map(|w| w.trim_matches(|c| c == '"' || c == '\''))
        .find(|w| !w.is_empty() && secret_path(w))
        .map(str::to_string)
}

/// `find` unless it deletes, executes or writes; `git branch` only when it
/// lists (no flag that deletes, moves, copies or sets upstream, and no bare
/// name, which creates a branch). Spelled out as separate expressions because
/// the regex crate has no lookahead; `lib/decisions.ts` does the same.
pub static FIND_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*find(?:\s|$)").expect("static"));
pub static FIND_ACTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s-(delete|exec|execdir|ok|okdir|fprint0?|fprintf|fls)\b").expect("static")
});
pub static GIT_BRANCH_HEAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*git\s+branch(?:\s|$)").expect("static"));
pub static GIT_BRANCH_WRITES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\s(?:-[a-zA-Z]*[dDfFmMcCu][a-zA-Z]*|--(?:delete|force|move|copy|set-upstream-to(?:=\S*)?|unset-upstream|edit-description|track|no-track))(?:\s|$)",
    )
    .expect("static")
});
pub static GIT_BRANCH_LISTS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\s(?:--list|--contains|--no-contains|--merged|--no-merged|--points-at|--show-current|-[a-zA-Z]*l[a-zA-Z]*)(?:\s|=|$)",
    )
    .expect("static")
});
/// A non-option argument after the flags: a branch to create unless a
/// listing flag gives it another meaning.
pub static GIT_BRANCH_POSITIONAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*git\s+branch(?:\s+-\S+)*\s+[^-\s]").expect("static"));
/// Anything that chains, redirects, substitutes or backgrounds is not "one
/// read-only command", whatever its head says.
pub static COMPOUND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[;&>`\n]|\$\(|<<|<\(|\|\|").expect("static"));
pub static XARGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bxargs\b").expect("static"));
/// Options that turn a reader into a runner or a writer: `rg --pre <cmd>`
/// executes it per file, `git diff --output=<file>` writes one, `--ext-diff`
/// runs the configured driver, `tree -o` writes. (`env` and `less` left the
/// allowlist for the same reason: `env sh x.sh` is a shell.) Mirrors
/// `READER_ESCAPES` in `lib/decisions.ts`.
pub static READER_ESCAPES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*rg\b.*\s--(pre|pre-glob|hostname-bin)\b|^\s*git\b.*\s--(output|ext-diff|textconv)\b|^\s*git\b.*\s-O\S|^\s*tree\b.*\s-[a-zA-Z]*o\b|^\s*file\b.*\s-[a-zA-Z]*C",
    )
    .expect("static")
});

pub fn read_only_head(segment: &str) -> bool {
    if READER_ESCAPES.is_match(segment) {
        return false;
    }
    READ_ONLY.is_match(segment)
        || (FIND_HEAD.is_match(segment) && !FIND_ACTS.is_match(segment))
        || (GIT_BRANCH_HEAD.is_match(segment)
            && !GIT_BRANCH_WRITES.is_match(segment)
            && (!GIT_BRANCH_POSITIONAL.is_match(segment) || GIT_BRANCH_LISTS.is_match(segment)))
}

/// Read-only = no compound syntax, no xargs, and every pipe segment's head
/// in the allowlist (`git log | head` is fine; `ls | xargs rm` is not).
pub fn read_only_bash(cmd: &str) -> bool {
    if COMPOUND.is_match(cmd) || XARGS.is_match(cmd) {
        return false;
    }
    let mut segments = cmd.split('|').peekable();
    segments.peek().is_some() && segments.all(|seg| read_only_head(seg.trim()))
}

/// Paths where a write runs code later (hooks, shells, keys) or holds
/// secrets — high whatever the tool.
pub static SENSITIVE_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(^|/)(\.ssh|\.claude|\.codex|\.config/opencode|\.git/hooks|\.git/config|\.gnupg|\.aws|\.kube|\.docker|\.npmrc|\.netrc|\.bashrc|\.zshrc|\.profile|\.bash_profile|\.zprofile|\.env(\.[^/]+)?)(/|$)|^/etc/|\.(pem|key|p12|pfx)$",
    )
    .expect("static regex")
});

/// Tool servers whose operation name says it destroys, sends or publishes.
pub static MCP_DESTRUCTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(delete|remove|destroy|drop|purge|send|publish|deploy|merge|push|force)")
        .expect("static regex")
});

/// The engines name the shell differently (Claude `Bash`, Codex
/// `shell`/`local_shell`, OpenCode `bash`); the command may be an argv
/// array. One shape before classifying.
pub fn normalize_tool(tool: &str) -> &str {
    match tool {
        "Bash" | "bash" | "shell" | "local_shell" | "LocalShell" | "commandExecution"
        | "exec_command" => "Bash",
        "Edit" | "edit" => "Edit",
        "Write" | "write" => "Write",
        "MultiEdit" | "multiedit" => "MultiEdit",
        "NotebookEdit" => "NotebookEdit",
        "Read" | "read" => "Read",
        other => other,
    }
}

/// The command string of a shell request, whatever its shape.
pub fn command_of(input: &Value) -> String {
    match input.get("command").or_else(|| input.get("cmd")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// The tier the desktop computes for a request before it leaves the machine.
/// Mirrors `lib/decisions.ts`: for the shell the same patterns and allowlist;
/// for file tools a sensitive or out-of-project path is high; tool servers
/// are high when their operation's name says it destroys, sends or
/// publishes, medium otherwise. Only `low` is ever approvable from a device.
pub fn tier_for(
    tool: &str,
    input: &Value,
    cwd: Option<&str>,
) -> (&'static str, &'static str, String) {
    let tool = normalize_tool(tool);
    if tool == "Bash" {
        let cmd = command_of(input);
        if let Some((_, why)) = DANGEROUS.iter().find(|(re, _)| re.is_match(&cmd)) {
            return ("high", "Irreversible", why.to_string());
        }
        if let Some(path) = secret_arg(&cmd) {
            return ("high", "Reads or touches secrets", path);
        }
        if let Some((_, why)) = CAUTION.iter().find(|(re, _)| re.is_match(&cmd)) {
            return if HIGH_REASON.is_match(why) {
                ("high", "Irreversible or external", why.to_string())
            } else {
                ("medium", "Changes the machine", why.to_string())
            };
        }
        if read_only_bash(&cmd) {
            return (
                "low",
                "Read-only",
                "inspects without changing anything".into(),
            );
        }
        if RUNNER.is_match(&cmd) {
            return (
                "medium",
                "Runs project code",
                "tests, linters and builds execute what the repository says".into(),
            );
        }
        return (
            "medium",
            "Runs a command",
            "not in the read-only allowlist".into(),
        );
    }
    match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
            let path = ["file_path", "path", "notebook_path"]
                .iter()
                .find_map(|k| input.get(k).and_then(Value::as_str))
                .unwrap_or("");
            if SENSITIVE_PATH.is_match(path) {
                return (
                    "high",
                    "Touches secrets, hooks or a shell profile",
                    path.to_string(),
                );
            }
            if let Some(root) = cwd.filter(|c| !c.is_empty()) {
                let root = root.trim_end_matches('/');
                if path.starts_with('/') && path != root && !path.starts_with(&format!("{root}/")) {
                    return ("high", "Writes outside the project", path.to_string());
                }
            }
            (
                "medium",
                "Edits a file",
                "the diff is not on the device".into(),
            )
        }
        "Read" | "Grep" | "Glob" => {
            let path = ["file_path", "path"]
                .iter()
                .find_map(|k| input.get(k).and_then(Value::as_str))
                .unwrap_or("");
            if secret_path(path) {
                return ("high", "Reads secrets", path.to_string());
            }
            ("medium", "Uses a tool", format!("{tool} is not classified"))
        }
        "WebFetch" | "WebSearch" => ("medium", "Reaches a host", "outside the machine".into()),
        t if t.starts_with("mcp__") => {
            if MCP_DESTRUCTIVE.is_match(t) {
                (
                    "high",
                    "Tool server: destructive or external",
                    t.to_string(),
                )
            } else {
                ("medium", "Calls a tool server", t.to_string())
            }
        }
        _ => ("medium", "Uses a tool", format!("{tool} is not classified")),
    }
}

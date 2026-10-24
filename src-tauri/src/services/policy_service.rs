//! Approvals as policy, as code (COMPETENCIA-2026-10 F6): one versioned
//! file per project — `<root>/.agent-console/policy.json` — compiled into
//! what each engine actually enforces: Claude's `permissions` in
//! `.claude/settings.json`, Codex's `prefix_rule(...)` lines in
//! `.codex/rules/agent-console.rules`, OpenCode's `permission` object in
//! `opencode.json`. The file is the human's standing decision; its hash
//! goes to the ledger as `policy_set` and rides on every approval decided
//! while it is in force, so a proof packet says which policy was live.
//!
//! The grammar is deliberately small: an effect, a tool class, a pattern.
//! Prefix matching on commands is what all three engines share; paths are
//! globs for Claude and OpenCode (Codex has no per-path rule in
//! execpolicy, so edit/read rules are not compiled for it and the result
//! says so).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

pub const POLICY_DIR: &str = ".agent-console";
pub const POLICY_FILE: &str = "policy.json";
pub const POLICY_VERSION: u32 = 1;
const CODEX_RULES_FILE: &str = "agent-console.rules";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyRule {
    /// allow · ask · block
    pub effect: String,
    /// shell · edit · read · fetch · mcp
    pub tool: String,
    /// shell: a command prefix (`npm test`, `git *`); edit/read: a path glob
    /// (`src/**`); fetch: a domain (`docs.rs`); mcp: a server or tool name.
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub version: u32,
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            version: POLICY_VERSION,
            rules: Vec::new(),
        }
    }
}

/// What a policy compiles to, per engine — shown before applying and kept
/// in the `policy_set` line.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Compiled {
    pub claude: ClaudePermissions,
    /// `prefix_rule(...)` lines, in rule order.
    pub codex_rules: Vec<String>,
    /// The `permission` object for opencode.json.
    pub opencode_permission: Value,
    /// Rules an engine cannot express, with why.
    pub skipped: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudePermissions {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PolicyStatus {
    pub path: String,
    pub exists: bool,
    pub policy: Policy,
    /// sha256 of the canonical (compact, sorted) JSON of the policy.
    pub hash: String,
    pub compiled: Compiled,
    pub errors: Vec<String>,
}

pub fn policy_path(root: &Path) -> PathBuf {
    root.join(POLICY_DIR).join(POLICY_FILE)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// Canonical hash: serde_json keeps object keys sorted (BTreeMap), so the
/// compact serialisation is already canonical for our structs.
pub fn hash(policy: &Policy) -> String {
    sha256_hex(serde_json::to_string(policy).unwrap_or_default().as_bytes())
}

pub fn validate(policy: &Policy) -> Vec<String> {
    let mut errors = Vec::new();
    if policy.version != POLICY_VERSION {
        errors.push(format!("version must be {POLICY_VERSION}"));
    }
    for (i, r) in policy.rules.iter().enumerate() {
        let n = i + 1;
        if !matches!(r.effect.as_str(), "allow" | "ask" | "block") {
            errors.push(format!("rule {n}: effect must be allow, ask or block"));
        }
        if !matches!(r.tool.as_str(), "shell" | "edit" | "read" | "fetch" | "mcp") {
            errors.push(format!(
                "rule {n}: tool must be shell, edit, read, fetch or mcp"
            ));
        }
        if r.pattern.trim().is_empty() {
            errors.push(format!("rule {n}: pattern is empty"));
        }
        if r.pattern.chars().any(|c| c.is_control()) {
            errors.push(format!("rule {n}: pattern has control characters"));
        }
        if r.tool == "shell" && r.pattern.contains(['|', ';', '&', '`', '$', '>', '<']) {
            errors.push(format!(
                "rule {n}: a shell pattern is a command prefix, not a pipeline (no | ; & ` $ < >)"
            ));
        }
    }
    errors
}

fn claude_effect(effect: &str) -> &'static str {
    match effect {
        "allow" => "allow",
        "ask" => "ask",
        _ => "deny",
    }
}

fn codex_decision(effect: &str) -> &'static str {
    match effect {
        "allow" => "allow",
        "ask" => "prompt",
        _ => "forbidden",
    }
}

fn opencode_action(effect: &str) -> &'static str {
    match effect {
        "allow" => "allow",
        "ask" => "ask",
        _ => "deny",
    }
}

fn list_for<'a>(c: &'a mut ClaudePermissions, effect: &str) -> &'a mut Vec<String> {
    match effect {
        "allow" => &mut c.allow,
        "ask" => &mut c.ask,
        _ => &mut c.deny,
    }
}

fn starlark_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// A shell prefix with its wildcard stripped: `git *` → `git`, `npm test` →
/// `npm test` (exact tokens).
fn shell_prefix(pattern: &str) -> (String, bool) {
    let p = pattern.trim();
    if let Some(prefix) = p.strip_suffix(" *").or_else(|| p.strip_suffix(":*")) {
        (prefix.trim().to_string(), true)
    } else if p == "*" {
        (String::new(), true)
    } else {
        (p.to_string(), false)
    }
}

/// Compile the policy into each engine's own vocabulary. Pure.
pub fn compile(policy: &Policy) -> Compiled {
    let mut out = Compiled::default();
    let mut oc: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut oc_insert = |tool: &str, pattern: &str, action: &str| {
        let entry = oc.entry(tool.to_string()).or_insert_with(|| json!({}));
        if let Some(o) = entry.as_object_mut() {
            o.insert(pattern.to_string(), json!(action));
        }
    };
    for r in &policy.rules {
        let ce = claude_effect(&r.effect);
        match r.tool.as_str() {
            "shell" => {
                let (prefix, wildcard) = shell_prefix(&r.pattern);
                // Claude: `Bash(npm:*)` is the prefix form; exact otherwise.
                let claude_raw = if prefix.is_empty() {
                    "Bash".to_string()
                } else if wildcard {
                    format!("Bash({prefix}:*)")
                } else {
                    format!("Bash({prefix})")
                };
                list_for(&mut out.claude, ce).push(claude_raw.clone());
                // Codex: prefix_rule over the tokens (a bare `*` has no tokens).
                let tokens: Vec<String> = prefix.split_whitespace().map(String::from).collect();
                if tokens.is_empty() {
                    out.skipped.push(format!(
                        "codex: `{}` matches every command; execpolicy needs at least one token",
                        r.pattern
                    ));
                } else {
                    let items = tokens
                        .iter()
                        .map(|t| starlark_str(t))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.codex_rules.push(format!(
                        "prefix_rule(pattern=[{items}], decision={}, justification={})",
                        starlark_str(codex_decision(&r.effect)),
                        starlark_str(&format!("agent-console policy: {}", claude_raw))
                    ));
                }
                // OpenCode: glob on the command line.
                let oc_pat = if prefix.is_empty() {
                    "*".to_string()
                } else if wildcard {
                    format!("{prefix} *")
                } else {
                    prefix.clone()
                };
                oc_insert("bash", &oc_pat, opencode_action(&r.effect));
            }
            "edit" => {
                for tool in ["Edit", "Write", "MultiEdit"] {
                    list_for(&mut out.claude, ce).push(format!("{tool}({})", r.pattern));
                }
                oc_insert("edit", &r.pattern, opencode_action(&r.effect));
                out.skipped.push(format!(
                    "codex: edit `{}` — execpolicy has no per-path rule; use [permissions] in config.toml",
                    r.pattern
                ));
            }
            "read" => {
                list_for(&mut out.claude, ce).push(format!("Read({})", r.pattern));
                oc_insert("read", &r.pattern, opencode_action(&r.effect));
                out.skipped.push(format!(
                    "codex: read `{}` — execpolicy has no per-path rule",
                    r.pattern
                ));
            }
            "fetch" => {
                list_for(&mut out.claude, ce).push(format!("WebFetch(domain:{})", r.pattern));
                oc_insert(
                    "webfetch",
                    &format!("*{}*", r.pattern),
                    opencode_action(&r.effect),
                );
                out.skipped.push(format!(
                    "codex: fetch `{}` — network policy lives in [permissions.<name>.network]",
                    r.pattern
                ));
            }
            "mcp" => {
                // Claude names MCP tools `mcp__<server>__<tool>`; a server-wide
                // rule is `mcp__<server>`.
                let name = r.pattern.trim_start_matches("mcp__");
                list_for(&mut out.claude, ce).push(format!("mcp__{name}"));
                out.skipped.push(format!(
                    "codex: mcp `{}` — tool approval is per server in config.toml",
                    r.pattern
                ));
                out.skipped.push(format!(
                    "opencode: mcp `{}` — MCP tools follow the `mcp` permission, not a per-server rule",
                    r.pattern
                ));
            }
            _ => {}
        }
    }
    out.opencode_permission = Value::Object(oc);
    out
}

pub fn load(root: &Path) -> AppResult<Option<Policy>> {
    let path = policy_path(root);
    if !path.exists() {
        return Ok(None);
    }
    let txt = fs::read_to_string(&path)?;
    let policy: Policy = serde_json::from_str(&txt)
        .map_err(|e| AppError::InvalidArgument(format!("{}: {e}", path.display())))?;
    Ok(Some(policy))
}

pub fn status(root: &Path) -> AppResult<PolicyStatus> {
    let path = policy_path(root);
    let (policy, exists) = match load(root)? {
        Some(p) => (p, true),
        None => (Policy::default(), false),
    };
    let errors = validate(&policy);
    Ok(PolicyStatus {
        path: path.to_string_lossy().to_string(),
        hash: hash(&policy),
        compiled: compile(&policy),
        policy,
        exists,
        errors,
    })
}

/// Write the policy file (pretty JSON, atomic). Refuses an invalid policy.
pub fn save(root: &Path, policy: &Policy) -> AppResult<PolicyStatus> {
    let errors = validate(policy);
    if !errors.is_empty() {
        return Err(AppError::InvalidArgument(errors.join("; ")));
    }
    let path = policy_path(root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        format!(
            "{}\n",
            serde_json::to_string_pretty(policy).unwrap_or_default()
        ),
    )?;
    fs::rename(&tmp, &path)?;
    status(root)
}

fn write_atomic(path: &Path, content: &str) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("txt")
    ));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

const MANAGED_MARK: &str = "# agent-console policy — generated from .agent-console/policy.json; edit the policy, not this file";

/// Apply the compiled policy to the three engines' files under `root`.
/// Claude: the compiled rules are merged into `.claude/settings.json`
/// `permissions.{allow,ask,deny}` (other entries kept; a rule moving effect
/// is removed from the lists it left). Codex: `.codex/rules/agent-console.rules`
/// is rewritten whole (it is ours by name). OpenCode: `opencode.json`
/// `permission.<tool>` patterns are merged (other keys kept). Returns the
/// files written.
pub fn apply(root: &Path, compiled: &Compiled) -> AppResult<Vec<String>> {
    let mut written = Vec::new();
    // Claude
    let claude_path = root.join(".claude").join("settings.json");
    let mut settings: Value = if claude_path.exists() {
        serde_json::from_str(&fs::read_to_string(&claude_path)?).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };
    if !settings.is_object() {
        settings = json!({});
    }
    let compiled_all: Vec<&String> = compiled
        .claude
        .allow
        .iter()
        .chain(compiled.claude.ask.iter())
        .chain(compiled.claude.deny.iter())
        .collect();
    let perms = settings
        .as_object_mut()
        .unwrap()
        .entry("permissions")
        .or_insert_with(|| json!({}));
    if !perms.is_object() {
        *perms = json!({});
    }
    for (key, ours) in [
        ("allow", &compiled.claude.allow),
        ("ask", &compiled.claude.ask),
        ("deny", &compiled.claude.deny),
    ] {
        let existing = perms
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut next: Vec<Value> = existing
            .into_iter()
            // A compiled rule that now lives in another list leaves this one.
            .filter(|v| {
                v.as_str()
                    .map(|s| {
                        !compiled_all.iter().any(|c| c.as_str() == s) || ours.iter().any(|o| o == s)
                    })
                    .unwrap_or(true)
            })
            .collect();
        for raw in ours {
            if !next.iter().any(|v| v.as_str() == Some(raw)) {
                next.push(json!(raw));
            }
        }
        perms
            .as_object_mut()
            .unwrap()
            .insert(key.to_string(), Value::Array(next));
    }
    write_atomic(
        &claude_path,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&settings).unwrap_or_default()
        ),
    )?;
    written.push(claude_path.to_string_lossy().to_string());
    // Codex
    let codex_path = root.join(".codex").join("rules").join(CODEX_RULES_FILE);
    let mut body = format!("{MANAGED_MARK}\n");
    for line in &compiled.codex_rules {
        body.push_str(line);
        body.push('\n');
    }
    write_atomic(&codex_path, &body)?;
    written.push(codex_path.to_string_lossy().to_string());
    // OpenCode
    let oc_path = root.join("opencode.json");
    if root.join("opencode.jsonc").exists() {
        return Err(AppError::InvalidArgument(
            "the folder has an opencode.jsonc; move it to opencode.json (JSON) so the policy can be written"
                .into(),
        ));
    }
    let mut config: Value = if oc_path.exists() {
        serde_json::from_str(&fs::read_to_string(&oc_path)?).map_err(|e| {
            AppError::InvalidArgument(format!("opencode.json is not valid JSON: {e}"))
        })?
    } else {
        json!({ "$schema": "https://opencode.ai/config.json" })
    };
    if !config.is_object() {
        return Err(AppError::InvalidArgument(
            "opencode.json is not a JSON object".into(),
        ));
    }
    let perm = config
        .as_object_mut()
        .unwrap()
        .entry("permission")
        .or_insert_with(|| json!({}));
    if !perm.is_object() {
        *perm = json!({});
    }
    if let Some(ours) = compiled.opencode_permission.as_object() {
        for (tool, patterns) in ours {
            let slot = perm
                .as_object_mut()
                .unwrap()
                .entry(tool.clone())
                .or_insert_with(|| json!({}));
            // A plain "allow"/"ask"/"deny" string becomes the `*` of an object.
            if let Some(s) = slot.as_str().map(String::from) {
                *slot = json!({ "*": s });
            }
            if let (Some(dst), Some(src)) = (slot.as_object_mut(), patterns.as_object()) {
                for (k, v) in src {
                    dst.insert(k.clone(), v.clone());
                }
            }
        }
    }
    write_atomic(
        &oc_path,
        &format!(
            "{}\n",
            serde_json::to_string_pretty(&config).unwrap_or_default()
        ),
    )?;
    written.push(oc_path.to_string_lossy().to_string());
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(effect: &str, tool: &str, pattern: &str) -> PolicyRule {
        PolicyRule {
            effect: effect.into(),
            tool: tool.into(),
            pattern: pattern.into(),
            note: None,
        }
    }

    #[test]
    fn one_policy_compiles_to_the_three_engines_vocabularies() {
        let p = Policy {
            version: 1,
            rules: vec![
                rule("allow", "shell", "npm test"),
                rule("allow", "shell", "git *"),
                rule("block", "shell", "rm -rf *"),
                rule("ask", "edit", "src/**"),
                rule("block", "read", ".env"),
                rule("allow", "fetch", "docs.rs"),
                rule("ask", "mcp", "jira"),
            ],
        };
        assert!(validate(&p).is_empty());
        let c = compile(&p);
        assert_eq!(
            c.claude.allow,
            ["Bash(npm test)", "Bash(git:*)", "WebFetch(domain:docs.rs)"]
        );
        assert_eq!(c.claude.deny, ["Bash(rm -rf:*)", "Read(.env)"]);
        assert_eq!(
            c.claude.ask,
            [
                "Edit(src/**)",
                "Write(src/**)",
                "MultiEdit(src/**)",
                "mcp__jira"
            ]
        );
        assert_eq!(
            c.codex_rules,
            [
                "prefix_rule(pattern=[\"npm\", \"test\"], decision=\"allow\", justification=\"agent-console policy: Bash(npm test)\")",
                "prefix_rule(pattern=[\"git\"], decision=\"allow\", justification=\"agent-console policy: Bash(git:*)\")",
                "prefix_rule(pattern=[\"rm\", \"-rf\"], decision=\"forbidden\", justification=\"agent-console policy: Bash(rm -rf:*)\")",
            ]
        );
        assert_eq!(c.opencode_permission["bash"]["npm test"], "allow");
        assert_eq!(c.opencode_permission["bash"]["git *"], "allow");
        assert_eq!(c.opencode_permission["bash"]["rm -rf *"], "deny");
        assert_eq!(c.opencode_permission["edit"]["src/**"], "ask");
        assert_eq!(c.opencode_permission["read"][".env"], "deny");
        assert_eq!(c.opencode_permission["webfetch"]["*docs.rs*"], "allow");
        assert_eq!(
            c.skipped.len(),
            5,
            "edit, read, fetch for codex; mcp for codex and opencode"
        );
        // The hash is stable and changes with any rule.
        let h = hash(&p);
        assert_eq!(h, hash(&p.clone()));
        let mut q = p.clone();
        q.rules[0].effect = "ask".into();
        assert_ne!(hash(&q), h);
    }

    #[test]
    fn validation_names_the_rule_and_the_reason() {
        let p = Policy {
            version: 1,
            rules: vec![
                rule("maybe", "shell", "npm test"),
                rule("allow", "disk", "x"),
                rule("allow", "shell", "cat x | sh"),
                rule("allow", "shell", " "),
            ],
        };
        let e = validate(&p);
        assert!(e
            .iter()
            .any(|m| m.starts_with("rule 1:") && m.contains("effect")));
        assert!(e
            .iter()
            .any(|m| m.starts_with("rule 2:") && m.contains("tool")));
        assert!(e
            .iter()
            .any(|m| m.starts_with("rule 3:") && m.contains("pipeline")));
        assert!(e
            .iter()
            .any(|m| m.starts_with("rule 4:") && m.contains("empty")));
    }

    #[test]
    fn apply_writes_the_three_files_merging_what_the_user_already_had() {
        let root = std::env::temp_dir().join(format!("ac-policy-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(".claude")).unwrap();
        fs::write(
            root.join(".claude/settings.json"),
            r#"{"permissions":{"allow":["Bash(ls:*)","Bash(npm test)"],"deny":["Read(.env)"]},"theme":"dark"}"#,
        )
        .unwrap();
        fs::write(
            root.join("opencode.json"),
            r#"{"permission":{"bash":"ask","edit":{"docs/**":"allow"}},"model":"x/y"}"#,
        )
        .unwrap();
        let p = Policy {
            version: 1,
            rules: vec![
                rule("ask", "shell", "npm test"),
                rule("allow", "shell", "git *"),
                rule("block", "edit", "secrets/**"),
            ],
        };
        let st = save(&root, &p).unwrap();
        assert!(st.exists);
        assert!(st.errors.is_empty());
        assert_eq!(load(&root).unwrap().unwrap(), p);
        let written = apply(&root, &st.compiled).unwrap();
        assert_eq!(written.len(), 3);
        let settings: Value =
            serde_json::from_str(&fs::read_to_string(root.join(".claude/settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["theme"], "dark", "other keys kept");
        let allow: Vec<&str> = settings["permissions"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(allow.contains(&"Bash(ls:*)"), "the user's own rule stays");
        assert!(
            !allow.contains(&"Bash(npm test)"),
            "moved to ask by the policy"
        );
        assert!(allow.contains(&"Bash(git:*)"));
        assert_eq!(settings["permissions"]["ask"], json!(["Bash(npm test)"]));
        let deny: Vec<&str> = settings["permissions"]["deny"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(deny.contains(&"Read(.env)") && deny.contains(&"Edit(secrets/**)"));
        let rules = fs::read_to_string(root.join(".codex/rules/agent-console.rules")).unwrap();
        assert!(rules.starts_with(MANAGED_MARK));
        assert!(rules.contains("prefix_rule(pattern=[\"git\"], decision=\"allow\""));
        assert!(rules.contains("decision=\"prompt\""));
        let oc: Value =
            serde_json::from_str(&fs::read_to_string(root.join("opencode.json")).unwrap()).unwrap();
        assert_eq!(oc["model"], "x/y");
        assert_eq!(
            oc["permission"]["bash"]["*"], "ask",
            "the plain string became the * of the object"
        );
        assert_eq!(oc["permission"]["bash"]["git *"], "allow");
        assert_eq!(
            oc["permission"]["edit"]["docs/**"], "allow",
            "user pattern kept"
        );
        assert_eq!(oc["permission"]["edit"]["secrets/**"], "deny");
        // Applying again changes nothing.
        let before = fs::read_to_string(root.join(".claude/settings.json")).unwrap();
        apply(&root, &st.compiled).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".claude/settings.json")).unwrap(),
            before
        );
        // No policy file ⇒ a default, empty status that still hashes.
        let other = root.join("other");
        fs::create_dir_all(&other).unwrap();
        let st2 = status(&other).unwrap();
        assert!(!st2.exists && st2.policy.rules.is_empty() && st2.hash.len() == 64);
        let _ = fs::remove_dir_all(&root);
    }
}

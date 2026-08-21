use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Project,
    Global,
}

/// Which agent's permission store a rule lives in. Claude rules go to
/// .claude/settings.json (permissions.allow/deny/ask); Codex rules go to
/// .codex/rules/agent-console.rules as execpolicy `prefix_rule(...)` lines —
/// the same store Codex's own "always approve" writes to, so rules saved from
/// a Codex approval are actually enforced by Codex.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    #[default]
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredRule {
    pub scope: Scope,
    pub effect: Effect,
    pub raw: String,
    pub source: String, // "agent-console" | "external"
    pub created_at_ms: Option<u64>,
    pub settings_path: String, // where it lives (so the UI can show provenance)
    #[serde(default)]
    pub engine: Engine,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PermissionsSnapshot {
    pub rules: Vec<StoredRule>,
    pub project_settings_path: Option<String>,
    pub global_settings_path: String,
}

pub fn snapshot(project_root: Option<&Path>) -> AppResult<PermissionsSnapshot> {
    let global = global_settings_path();
    let mut rules: Vec<StoredRule> = Vec::new();

    let project_settings = project_root.map(|r| r.join(".claude/settings.json"));
    if let Some(p) = &project_settings {
        let sidecar = sidecar_path_for(p);
        rules.extend(read_rules_from(p, Scope::Project, &sidecar)?);
    }
    let global_sidecar = sidecar_path_for(&global);
    rules.extend(read_rules_from(&global, Scope::Global, &global_sidecar)?);

    // Codex execpolicy rules (best-effort: a missing dir or unparseable file
    // never fails the snapshot — Claude rules must always render).
    if let Some(root) = project_root {
        rules.extend(read_codex_rules_dir(
            &root.join(".codex/rules"),
            Scope::Project,
        ));
    }
    rules.extend(read_codex_rules_dir(
        &codex_global_rules_dir(),
        Scope::Global,
    ));

    Ok(PermissionsSnapshot {
        rules,
        project_settings_path: project_settings.map(|p| p.to_string_lossy().to_string()),
        global_settings_path: global.to_string_lossy().to_string(),
    })
}

pub fn add_rule(
    project_root: Option<&Path>,
    scope: Scope,
    effect: Effect,
    raw: &str,
    engine: Engine,
) -> AppResult<StoredRule> {
    if engine == Engine::Codex {
        return codex_add_rule(project_root, scope, effect, raw);
    }
    let path = settings_path_for(scope, project_root)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut settings = read_settings(&path)?;
    let key = effect_key(effect);

    let perms = settings.get("permissions").cloned().unwrap_or(json!({}));
    let mut perms = if perms.is_object() { perms } else { json!({}) };
    let arr = perms
        .get(key)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let already = arr.iter().any(|v| v.as_str() == Some(raw));
    let mut new_arr = arr;
    if !already {
        new_arr.push(Value::String(raw.to_string()));
    }
    perms
        .as_object_mut()
        .unwrap()
        .insert(key.to_string(), Value::Array(new_arr));
    settings
        .as_object_mut()
        .unwrap()
        .insert("permissions".to_string(), perms);

    backup(&path)?;
    write_settings(&path, &settings)?;

    let created_at_ms = now_ms();
    let sidecar_path = sidecar_path_for(&path);
    write_sidecar_entry(&sidecar_path, raw, created_at_ms)?;

    Ok(StoredRule {
        scope,
        effect,
        raw: raw.to_string(),
        source: "agent-console".to_string(),
        created_at_ms: Some(created_at_ms),
        settings_path: path.to_string_lossy().to_string(),
        engine: Engine::Claude,
    })
}

pub fn remove_rule(
    project_root: Option<&Path>,
    scope: Scope,
    effect: Effect,
    raw: &str,
    engine: Engine,
) -> AppResult<()> {
    if engine == Engine::Codex {
        return codex_remove_rule(project_root, scope, effect, raw);
    }
    let path = settings_path_for(scope, project_root)?;
    if !path.exists() {
        return Ok(());
    }
    let mut settings = read_settings(&path)?;
    let key = effect_key(effect);
    if let Some(arr) = settings
        .pointer_mut(&format!("/permissions/{key}"))
        .and_then(|v| v.as_array_mut())
    {
        arr.retain(|v| v.as_str() != Some(raw));
    }
    backup(&path)?;
    write_settings(&path, &settings)?;

    let sidecar_path = sidecar_path_for(&path);
    remove_sidecar_entry(&sidecar_path, raw)?;
    Ok(())
}

// --- helpers -----------------------------------------------------------------

fn effect_key(e: Effect) -> &'static str {
    match e {
        Effect::Allow => "allow",
        Effect::Deny => "deny",
        Effect::Ask => "ask",
    }
}

fn settings_path_for(scope: Scope, project_root: Option<&Path>) -> AppResult<PathBuf> {
    match scope {
        Scope::Project => project_root
            .map(|r| r.join(".claude/settings.json"))
            .ok_or_else(|| AppError::InvalidArgument("no project open".into())),
        Scope::Global => Ok(global_settings_path()),
    }
}

fn global_settings_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".claude/settings.json"))
        .unwrap_or_else(|| PathBuf::from(".claude/settings.json"))
}

fn read_settings(path: &Path) -> AppResult<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let txt = fs::read_to_string(path)?;
    let v: Value = serde_json::from_str(&txt).unwrap_or(json!({}));
    Ok(if v.is_object() { v } else { json!({}) })
}

fn write_settings(path: &Path, v: &Value) -> AppResult<()> {
    // Temp + rename: a crash mid-write must never truncate settings.json.
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(v).unwrap())?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// How many timestamped `.bak` copies of a settings file we keep around.
const MAX_BACKUPS: usize = 5;

fn backup(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    // Keep the original extension in the backup name (settings.json →
    // settings.json.<ts>.bak, agent-console.rules → agent-console.rules.<ts>.bak)
    // so prune_backups' prefix match stays per-file.
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "bak".into());
    let bak = path.with_extension(format!("{ext}.{}.bak", now_ms()));
    fs::copy(path, &bak)?;
    prune_backups(path);
    Ok(())
}

/// Drop all but the newest MAX_BACKUPS `<name>.json.<ts>.bak` siblings.
/// Best-effort: a failed prune never blocks the rule edit that triggered it.
fn prune_backups(path: &Path) {
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    let mut baks: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().is_some_and(|f| {
                let f = f.to_string_lossy();
                f.starts_with(&prefix) && f.ends_with(".bak")
            })
        })
        .collect();
    // Timestamps are fixed-width ms since epoch, so lexicographic = chronological.
    baks.sort();
    if baks.len() > MAX_BACKUPS {
        for old in &baks[..baks.len() - MAX_BACKUPS] {
            let _ = fs::remove_file(old);
        }
    }
}

fn read_rules_from(path: &Path, scope: Scope, sidecar: &Path) -> AppResult<Vec<StoredRule>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let settings = read_settings(path)?;
    let sidecar_map = read_sidecar(sidecar);
    let mut out = Vec::new();
    for (key, effect) in [
        ("allow", Effect::Allow),
        ("deny", Effect::Deny),
        ("ask", Effect::Ask),
    ] {
        let Some(arr) = settings
            .pointer(&format!("/permissions/{key}"))
            .and_then(|v| v.as_array())
        else {
            continue;
        };
        for v in arr {
            let Some(raw) = v.as_str() else { continue };
            let meta = sidecar_map.get(raw).cloned();
            out.push(StoredRule {
                scope,
                effect,
                raw: raw.to_string(),
                source: if meta.is_some() {
                    "agent-console".into()
                } else {
                    "external".into()
                },
                created_at_ms: meta,
                settings_path: path.to_string_lossy().to_string(),
                engine: Engine::Claude,
            });
        }
    }
    Ok(out)
}

fn sidecar_path_for(settings_path: &Path) -> PathBuf {
    settings_path.with_file_name("agent-console-rules.json")
}

fn read_sidecar(path: &Path) -> std::collections::HashMap<String, u64> {
    let Ok(txt) = fs::read_to_string(path) else {
        return Default::default();
    };
    let Ok(v) = serde_json::from_str::<Value>(&txt) else {
        return Default::default();
    };
    let Some(obj) = v.as_object() else {
        return Default::default();
    };
    obj.iter()
        .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n)))
        .collect()
}

fn write_sidecar_entry(path: &Path, raw: &str, ts_ms: u64) -> AppResult<()> {
    let mut obj = match fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    {
        Some(Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    obj.insert(raw.to_string(), json!(ts_ms));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        serde_json::to_string_pretty(&Value::Object(obj)).unwrap(),
    )?;
    Ok(())
}

fn remove_sidecar_entry(path: &Path, raw: &str) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    let Ok(txt) = fs::read_to_string(path) else {
        return Ok(());
    };
    let mut obj = match serde_json::from_str::<Value>(&txt) {
        Ok(Value::Object(m)) => m,
        _ => return Ok(()),
    };
    obj.remove(raw);
    fs::write(
        path,
        serde_json::to_string_pretty(&Value::Object(obj)).unwrap(),
    )?;
    Ok(())
}

// --- Codex execpolicy store --------------------------------------------------
//
// Codex (0.144+) enforces persisted permissions via execpolicy rules: Starlark
// `prefix_rule(pattern=[...], decision=...)` lines in `.rules` files. It loads
// every *.rules file under ~/.codex/rules/ (user layer) and, when the project
// is trusted, <repo>/.codex/rules/. Its own "always approve" flow writes to
// default.rules; we keep ours in a dedicated agent-console.rules so provenance
// and removal never touch rules we don't own.
//
// Only simple Bash command rules translate: `Bash(git push:*)` → pattern
// ["git","push"] (prefix semantics on both sides). Tool-wide rules ("Bash",
// "Edit") and path tools are approval_policy/sandbox territory in Codex — no
// rule equivalent exists, so add_rule refuses and the UI warns instead.

/// File we own inside a `.codex/rules/` layer. Never write anywhere else.
const CODEX_RULES_FILE: &str = "agent-console.rules";
/// Marker prefix in `justification=` that (a) attributes the rule to us and
/// (b) round-trips the original Claude-style raw for display and removal.
const CODEX_JUSTIFICATION_PREFIX: &str = "agent-console: ";

fn codex_global_rules_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".codex/rules"))
        .unwrap_or_else(|| PathBuf::from(".codex/rules"))
}

fn codex_rules_dir(scope: Scope, project_root: Option<&Path>) -> AppResult<PathBuf> {
    match scope {
        Scope::Project => project_root
            .map(|r| r.join(".codex/rules"))
            .ok_or_else(|| AppError::InvalidArgument("no project open".into())),
        Scope::Global => Ok(codex_global_rules_dir()),
    }
}

/// Codex decision for a Claude effect. Codex resolves conflicts most-
/// restrictive-first (forbidden > prompt > allow), which matches the intent
/// of deny/ask/allow.
fn codex_decision(effect: Effect) -> &'static str {
    match effect {
        Effect::Allow => "allow",
        Effect::Deny => "forbidden",
        Effect::Ask => "prompt",
    }
}

fn effect_from_codex_decision(decision: &str) -> Effect {
    match decision {
        "forbidden" => Effect::Deny,
        "prompt" => Effect::Ask,
        _ => Effect::Allow,
    }
}

/// Translate a Claude-style rule into a Codex prefix pattern, or None when no
/// faithful equivalent exists. Conservative on purpose: a rule we can't map
/// cleanly must surface as "won't apply to Codex" in the UI, not silently
/// become a rule Codex never matches (the exact bug this store exists to fix).
pub fn codex_equivalent(raw: &str) -> Option<Vec<String>> {
    let inner = raw.strip_prefix("Bash(")?.strip_suffix(")")?;
    let cmd = inner.strip_suffix(":*").unwrap_or(inner).trim();
    if cmd.is_empty() {
        return None;
    }
    // Shell metacharacters (pipes, subshells, redirects, globs, expansions)
    // don't survive the string→argv translation: Codex matches parsed argv
    // prefixes, not shell source. Control chars additionally can't be written
    // as plain Starlark string literals. Refuse both.
    const SHELL_META: &[char] = &[
        '|', '&', ';', '<', '>', '(', ')', '`', '$', '*', '?', '{', '}', '[', ']', '~', '#', '!',
        '\\',
    ];
    if cmd
        .chars()
        .any(|c| c.is_control() || SHELL_META.contains(&c))
    {
        return None;
    }
    shell_split(cmd).filter(|tokens| !tokens.is_empty())
}

/// Minimal shell-style tokenizer: whitespace-separated words with single/double
/// quote grouping. Metacharacters and escapes are already refused upstream.
/// Unbalanced quotes → None.
fn shell_split(s: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                in_word = true;
            }
            None if c.is_whitespace() => {
                if in_word {
                    tokens.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            None => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return None;
    }
    if in_word {
        tokens.push(cur);
    }
    Some(tokens)
}

/// Starlark string literal. serde_json escaping is a valid Starlark subset for
/// the strings we emit: control characters are refused by codex_equivalent, so
/// the output only ever contains plain chars plus \" and \\.
fn starlark_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

fn format_prefix_rule(pattern: &[String], decision: &str, justification: &str) -> String {
    let items = pattern
        .iter()
        .map(|t| starlark_str(t))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "prefix_rule(pattern=[{items}], decision={}, justification={})",
        starlark_str(decision),
        starlark_str(justification),
    )
}

#[derive(Debug, PartialEq)]
struct ParsedPrefixRule {
    pattern: Vec<String>,
    decision: String,
    justification: Option<String>,
}

/// Parse one `prefix_rule(...)` line. Line-based on purpose: both Codex's own
/// writer and ours emit one rule per line. Anything else (comments, multi-line
/// Starlark, other rule kinds) is skipped by returning None — display of
/// external rules is best-effort, our own rules always round-trip.
fn parse_prefix_rule_line(line: &str) -> Option<ParsedPrefixRule> {
    let s = line.trim();
    let args = s.strip_prefix("prefix_rule(")?.strip_suffix(")")?;

    let mut pattern: Option<Vec<String>> = None;
    let mut decision = "allow".to_string();
    let mut justification = None;

    for (name, value) in split_call_args(args)? {
        match name.as_str() {
            "pattern" => pattern = parse_string_list(&value),
            "decision" => decision = parse_string_literal(value.trim())?,
            "justification" => justification = parse_string_literal(value.trim()),
            _ => {}
        }
    }
    let pattern = pattern.filter(|p| !p.is_empty())?;
    Some(ParsedPrefixRule {
        pattern,
        decision,
        justification,
    })
}

/// Split `name=value, name=value, ...` at top level, tracking string literals
/// and bracket depth so commas inside strings or lists don't split.
fn split_call_args(s: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut start = 0usize;
    let mut idx_byte = 0usize; // byte offset for slicing
    let mut arg_bounds = Vec::new();
    for c in s.chars() {
        if quote.is_some() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if Some(c) == quote {
                quote = None;
            }
        } else {
            match c {
                '"' | '\'' => quote = Some(c),
                '[' | '(' => depth += 1,
                ']' | ')' => depth = depth.checked_sub(1)?,
                ',' if depth == 0 => {
                    arg_bounds.push((start, idx_byte));
                    start = idx_byte + c.len_utf8();
                }
                _ => {}
            }
        }
        idx_byte += c.len_utf8();
    }
    if quote.is_some() || depth != 0 {
        return None;
    }
    arg_bounds.push((start, s.len()));

    for (a, b) in arg_bounds {
        let part = s[a..b].trim();
        if part.is_empty() {
            continue;
        }
        let eq = part.find('=')?;
        out.push((
            part[..eq].trim().to_string(),
            part[eq + 1..].trim().to_string(),
        ));
    }
    Some(out)
}

/// Parse `["a", "b", ...]` into its string items.
fn parse_string_list(s: &str) -> Option<Vec<String>> {
    let inner = s.trim().strip_prefix('[')?.strip_suffix(']')?;
    let mut items = Vec::new();
    let mut rest = inner.trim();
    while !rest.is_empty() {
        let (item, remaining) = take_string_literal(rest)?;
        items.push(item);
        rest = remaining
            .trim()
            .strip_prefix(',')
            .unwrap_or(remaining)
            .trim();
    }
    Some(items)
}

fn parse_string_literal(s: &str) -> Option<String> {
    let (item, rest) = take_string_literal(s)?;
    rest.trim().is_empty().then_some(item)
}

/// Consume one quoted string literal from the front of `s`, decoding the
/// escape subset both writers use (\\, \", \', \n, \t, \r). Unknown escapes
/// keep the escaped char literally — display never fails on them.
fn take_string_literal(s: &str) -> Option<(String, &str)> {
    let mut chars = s.char_indices();
    let (_, q) = chars.next()?;
    if q != '"' && q != '\'' {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for (i, c) in chars {
        if escaped {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                'r' => '\r',
                other => other,
            });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == q {
            return Some((out, &s[i + c.len_utf8()..]));
        } else {
            out.push(c);
        }
    }
    None
}

/// Display form for an external prefix rule we didn't write: reconstruct a
/// Claude-style raw so the panel renders one consistent grammar. Prefix rules
/// are inherently prefix matches, hence the `:*`.
fn codex_display_raw(pattern: &[String]) -> String {
    let joined = pattern
        .iter()
        .map(|t| {
            if t.chars().any(|c| c.is_whitespace()) {
                format!("\"{t}\"")
            } else {
                t.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("Bash({joined}:*)")
}

/// All prefix rules in a `.codex/rules/` layer, ours and external. Best-effort:
/// unreadable files/lines are skipped, never an error.
fn read_codex_rules_dir(dir: &Path, scope: Scope) -> Vec<StoredRule> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "rules"))
        .collect();
    files.sort();

    let mut out = Vec::new();
    for file in files {
        let Ok(txt) = fs::read_to_string(&file) else {
            continue;
        };
        for line in txt.lines() {
            let Some(rule) = parse_prefix_rule_line(line) else {
                continue;
            };
            let ours = rule
                .justification
                .as_deref()
                .and_then(|j| j.strip_prefix(CODEX_JUSTIFICATION_PREFIX));
            out.push(StoredRule {
                scope,
                effect: effect_from_codex_decision(&rule.decision),
                raw: ours
                    .map(str::to_string)
                    .unwrap_or_else(|| codex_display_raw(&rule.pattern)),
                source: if ours.is_some() {
                    "agent-console".into()
                } else {
                    "external".into()
                },
                created_at_ms: None,
                settings_path: file.to_string_lossy().to_string(),
                engine: Engine::Codex,
            });
        }
    }
    out
}

fn codex_add_rule(
    project_root: Option<&Path>,
    scope: Scope,
    effect: Effect,
    raw: &str,
) -> AppResult<StoredRule> {
    let pattern = codex_equivalent(raw).ok_or_else(|| {
        AppError::InvalidArgument(format!(
            "'{raw}' has no Codex equivalent — Codex prefix rules only cover plain shell commands"
        ))
    })?;
    let dir = codex_rules_dir(scope, project_root)?;
    fs::create_dir_all(&dir)?;
    let path = dir.join(CODEX_RULES_FILE);
    let decision = codex_decision(effect);

    let existing = fs::read_to_string(&path).unwrap_or_default();
    let already = existing
        .lines()
        .filter_map(parse_prefix_rule_line)
        .any(|r| r.pattern == pattern && r.decision == decision);
    if !already {
        backup(&path)?;
        let line = format_prefix_rule(
            &pattern,
            decision,
            &format!("{CODEX_JUSTIFICATION_PREFIX}{raw}"),
        );
        let mut next = existing;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&line);
        next.push('\n');
        let tmp = path.with_extension("rules.tmp");
        fs::write(&tmp, next)?;
        fs::rename(&tmp, &path)?;
    }

    Ok(StoredRule {
        scope,
        effect,
        raw: raw.to_string(),
        source: "agent-console".to_string(),
        created_at_ms: Some(now_ms()),
        settings_path: path.to_string_lossy().to_string(),
        engine: Engine::Codex,
    })
}

fn codex_remove_rule(
    project_root: Option<&Path>,
    scope: Scope,
    effect: Effect,
    raw: &str,
) -> AppResult<()> {
    let dir = codex_rules_dir(scope, project_root)?;
    let path = dir.join(CODEX_RULES_FILE);
    if !path.exists() {
        return Ok(());
    }
    let txt = fs::read_to_string(&path)?;
    let decision = codex_decision(effect);
    let pattern = codex_equivalent(raw);
    let marker = format!("{CODEX_JUSTIFICATION_PREFIX}{raw}");

    // Match by round-tripped raw (robust to translation changes) OR by the
    // current translation (covers hand-edited justifications).
    let keep = |line: &str| -> bool {
        let Some(rule) = parse_prefix_rule_line(line) else {
            return true;
        };
        if rule.decision != decision {
            return true;
        }
        let by_marker = rule.justification.as_deref() == Some(marker.as_str());
        let by_pattern = pattern.as_ref() == Some(&rule.pattern);
        !(by_marker || by_pattern)
    };

    let next: String = txt
        .lines()
        .filter(|l| keep(l))
        .map(|l| format!("{l}\n"))
        .collect();
    if next != txt {
        backup(&path)?;
        let tmp = path.with_extension("rules.tmp");
        fs::write(&tmp, next)?;
        fs::rename(&tmp, &path)?;
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ac-perm-{tag}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    /// Only project-scope rules: global scope reads/writes the real
    /// ~/.claude/settings.json, which tests must never touch.
    fn project_rules(root: &Path) -> Vec<StoredRule> {
        snapshot(Some(root))
            .unwrap()
            .rules
            .into_iter()
            .filter(|r| r.scope == Scope::Project)
            .collect()
    }

    #[test]
    fn project_rule_roundtrip_dedup_and_provenance() {
        let root = temp_root("rt");
        assert!(project_rules(&root).is_empty(), "no settings → no rules");

        // Add → visible, attributed to us (sidecar metadata present).
        let rule = add_rule(
            Some(&root),
            Scope::Project,
            Effect::Allow,
            "Bash(npm:*)",
            Engine::Claude,
        )
        .unwrap();
        assert_eq!(rule.source, "agent-console");
        assert!(rule.created_at_ms.is_some());
        let rules = project_rules(&root);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].raw, "Bash(npm:*)");
        assert_eq!(rules[0].effect, Effect::Allow);

        // Re-adding the same rule must not duplicate it.
        add_rule(
            Some(&root),
            Scope::Project,
            Effect::Allow,
            "Bash(npm:*)",
            Engine::Claude,
        )
        .unwrap();
        assert_eq!(project_rules(&root).len(), 1, "add is idempotent");

        // A rule someone wrote by hand (no sidecar entry) reads as external.
        let path = root.join(".claude/settings.json");
        let mut v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        v["permissions"]["deny"] = serde_json::json!(["WebFetch"]);
        fs::write(&path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let rules = project_rules(&root);
        let ext = rules.iter().find(|r| r.raw == "WebFetch").unwrap();
        assert_eq!(ext.source, "external");
        assert_eq!(ext.effect, Effect::Deny);
        assert!(ext.created_at_ms.is_none());

        // Remove only touches the targeted rule; removing a missing rule is a no-op.
        remove_rule(
            Some(&root),
            Scope::Project,
            Effect::Allow,
            "Bash(npm:*)",
            Engine::Claude,
        )
        .unwrap();
        let rules = project_rules(&root);
        assert_eq!(rules.len(), 1, "the external deny rule survives");
        assert_eq!(rules[0].raw, "WebFetch");
        remove_rule(
            Some(&root),
            Scope::Project,
            Effect::Allow,
            "Bash(npm:*)",
            Engine::Claude,
        )
        .unwrap();

        // Other settings keys in the file survive our edits.
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(v.get("permissions").is_some());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_settings_fall_back_instead_of_failing() {
        let root = temp_root("corrupt");
        let dir = root.join(".claude");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), "{not json").unwrap();

        // Snapshot tolerates the corrupt file; add_rule rebuilds it from {}.
        assert!(project_rules(&root).is_empty());
        add_rule(
            Some(&root),
            Scope::Project,
            Effect::Ask,
            "Bash(rm:*)",
            Engine::Claude,
        )
        .unwrap();
        let rules = project_rules(&root);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].effect, Effect::Ask);

        // The pre-edit corrupt content was backed up before being replaced.
        let baks: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
            .collect();
        assert_eq!(baks.len(), 1, "corrupt original preserved as .bak");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn backups_rotate_to_a_bounded_set() {
        let root = temp_root("baks");
        // First add creates the file (no backup); each later edit backs up the
        // previous version. Timestamps are ms, so space the edits out enough
        // that each backup gets a distinct name.
        for i in 0..(MAX_BACKUPS + 4) {
            add_rule(
                Some(&root),
                Scope::Project,
                Effect::Allow,
                &format!("Bash(tool{i}:*)"),
                Engine::Claude,
            )
            .unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let dir = root.join(".claude");
        let baks = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
            .count();
        assert!(
            baks <= MAX_BACKUPS,
            "backups must rotate, found {baks} > {MAX_BACKUPS}"
        );
        assert!(baks > 0, "edits after the first do produce backups");

        // All rules survived the rotation churn.
        assert_eq!(project_rules(&root).len(), MAX_BACKUPS + 4);

        let _ = fs::remove_dir_all(&root);
    }

    // --- Codex store ---------------------------------------------------------

    #[test]
    fn codex_equivalent_translates_only_plain_commands() {
        // Prefix rule → tokens without the :* suffix.
        assert_eq!(
            codex_equivalent("Bash(git push:*)"),
            Some(vec!["git".into(), "push".into()])
        );
        // Exact command → same tokens (prefix semantics on the Codex side).
        assert_eq!(
            codex_equivalent("Bash(npm run build)"),
            Some(vec!["npm".into(), "run".into(), "build".into()])
        );
        // Quoted args keep their grouping.
        assert_eq!(
            codex_equivalent("Bash(git commit -m \"two words\")"),
            Some(vec![
                "git".into(),
                "commit".into(),
                "-m".into(),
                "two words".into()
            ])
        );
        // No equivalent: tool-wide, non-Bash, shell metacharacters, empty.
        assert_eq!(codex_equivalent("Bash"), None);
        assert_eq!(codex_equivalent("Edit(src/**)"), None);
        assert_eq!(codex_equivalent("WebFetch"), None);
        assert_eq!(codex_equivalent("Bash(cat a | grep b)"), None);
        assert_eq!(codex_equivalent("Bash(echo $HOME)"), None);
        assert_eq!(codex_equivalent("Bash(ls *.rs)"), None);
        assert_eq!(codex_equivalent("Bash(a && b)"), None);
        assert_eq!(codex_equivalent("Bash()"), None);
        assert_eq!(codex_equivalent("Bash(\"unbalanced)"), None);
    }

    #[test]
    fn codex_prefix_rule_line_roundtrip() {
        let line = format_prefix_rule(
            &[
                "git".into(),
                "commit".into(),
                "-m".into(),
                "say \"hi\"".into(),
            ],
            "allow",
            "agent-console: Bash(git commit -m \"say \\\"hi\\\"\")",
        );
        let parsed = parse_prefix_rule_line(&line).expect("our own line must parse");
        assert_eq!(
            parsed.pattern,
            vec!["git", "commit", "-m", "say \"hi\""]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(parsed.decision, "allow");
        assert!(parsed
            .justification
            .unwrap()
            .starts_with(CODEX_JUSTIFICATION_PREFIX));

        // Lines Codex itself writes (no justification, default decision).
        let native = r#"prefix_rule(pattern=["git", "-C", "/tmp/x", "push"], decision="allow")"#;
        let parsed = parse_prefix_rule_line(native).unwrap();
        assert_eq!(parsed.pattern.len(), 4);
        assert_eq!(parsed.decision, "allow");
        assert_eq!(parsed.justification, None);

        // Garbage lines are skipped, not errors.
        assert_eq!(parse_prefix_rule_line("# comment"), None);
        assert_eq!(parse_prefix_rule_line("prefix_rule(pattern=[])"), None);
        assert_eq!(parse_prefix_rule_line("other_rule(pattern=[\"x\"])"), None);
    }

    fn codex_project_rules(root: &Path) -> Vec<StoredRule> {
        snapshot(Some(root))
            .unwrap()
            .rules
            .into_iter()
            .filter(|r| r.scope == Scope::Project && r.engine == Engine::Codex)
            .collect()
    }

    #[test]
    fn codex_rule_roundtrip_dedup_and_provenance() {
        let root = temp_root("codex-rt");
        assert!(codex_project_rules(&root).is_empty());

        let rule = add_rule(
            Some(&root),
            Scope::Project,
            Effect::Allow,
            "Bash(git push:*)",
            Engine::Codex,
        )
        .unwrap();
        assert_eq!(rule.engine, Engine::Codex);
        assert!(rule.settings_path.ends_with("agent-console.rules"));

        // The file contains a real execpolicy line Codex will load.
        let file = root.join(".codex/rules").join(CODEX_RULES_FILE);
        let txt = fs::read_to_string(&file).unwrap();
        assert!(txt.contains(r#"prefix_rule(pattern=["git", "push"], decision="allow""#));

        // Visible in the snapshot with the original Claude-style raw.
        let rules = codex_project_rules(&root);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].raw, "Bash(git push:*)");
        assert_eq!(rules[0].effect, Effect::Allow);
        assert_eq!(rules[0].source, "agent-console");

        // Idempotent add.
        add_rule(
            Some(&root),
            Scope::Project,
            Effect::Allow,
            "Bash(git push:*)",
            Engine::Codex,
        )
        .unwrap();
        assert_eq!(codex_project_rules(&root).len(), 1);

        // Deny maps to the most restrictive Codex decision.
        add_rule(
            Some(&root),
            Scope::Project,
            Effect::Deny,
            "Bash(rm -rf build)",
            Engine::Codex,
        )
        .unwrap();
        let txt = fs::read_to_string(&file).unwrap();
        assert!(txt.contains(r#"decision="forbidden""#));
        let deny = codex_project_rules(&root)
            .into_iter()
            .find(|r| r.effect == Effect::Deny)
            .unwrap();
        assert_eq!(deny.raw, "Bash(rm -rf build)");

        // A rule Codex wrote itself (default.rules) reads as external.
        fs::write(
            root.join(".codex/rules/default.rules"),
            "prefix_rule(pattern=[\"cargo\", \"test\"], decision=\"allow\")\n",
        )
        .unwrap();
        let ext = codex_project_rules(&root)
            .into_iter()
            .find(|r| r.source == "external")
            .unwrap();
        assert_eq!(ext.raw, "Bash(cargo test:*)");
        assert!(ext.settings_path.ends_with("default.rules"));

        // Remove only touches our file and only the targeted rule.
        remove_rule(
            Some(&root),
            Scope::Project,
            Effect::Allow,
            "Bash(git push:*)",
            Engine::Codex,
        )
        .unwrap();
        let left = codex_project_rules(&root);
        assert_eq!(left.len(), 2, "deny + external survive");
        assert!(left.iter().all(|r| r.raw != "Bash(git push:*)"));
        let txt = fs::read_to_string(&file).unwrap();
        assert!(!txt.contains("git"));
        assert!(txt.contains("forbidden"));

        // Untranslatable rules are refused with a typed error, never silently
        // written where Codex ignores them.
        let err = add_rule(
            Some(&root),
            Scope::Project,
            Effect::Allow,
            "Edit(src/**)",
            Engine::Codex,
        );
        assert!(err.is_err());

        let _ = fs::remove_dir_all(&root);
    }
}

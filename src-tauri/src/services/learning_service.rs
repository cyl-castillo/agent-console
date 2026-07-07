use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::activity_service::ActivityEvent;

/// One improvement the reflection pass proposes from observed daily activity.
///
/// `kind` drives what the UI offers:
/// - "skill"    → a repeated workflow worth a reusable skill (carries skillName + skillMdContent)
/// - "memory"   → a convention/fact worth persisting so Claude recalls it (carries memoryName + memoryContent)
/// - "plugin"   → a workflow/skill-cluster reusable beyond this project, worth
///   packaging as a shareable Claude Code plugin (carries pluginName +
///   pluginDescription + pluginSkillMd; applied via `create_plugin`)
/// - "hook"     → a rule the user keeps enforcing by hand that a Claude Code
///   hook could enforce automatically; report-only (hooks mutate settings —
///   too trust-sensitive to auto-apply)
/// - "friction" → a recurring pain point (repeated rewinds, repeated failures); report-only, no payload
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningSuggestion {
    pub kind: String,
    pub title: String,
    pub rationale: String,
    /// Concrete observations from the activity that motivated this suggestion.
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_md_content: Option<String>,
    /// kebab-case filename ending in `.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_content: Option<String>,
    /// kind = "plugin": kebab-case plugin name to scaffold in ~/.claude/skills/.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_description: Option<String>,
    /// Starter SKILL.md for the plugin (single-skill layout at the plugin root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_skill_md: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflectionResult {
    pub suggestions: Vec<LearningSuggestion>,
    pub events_analyzed: usize,
    pub raw_excerpt: String,
}

/// Reflect over a window of recorded activity and propose improvements. Mirrors
/// the Advisor (`claude -p` in plan mode → strict JSON), but the input is what
/// the user actually *did* — their prompts and snapshots — not the repo layout.
pub fn reflect(project_root: &Path, events: &[ActivityEvent]) -> AppResult<ReflectionResult> {
    if !project_root.is_dir() {
        return Err(AppError::NotADirectory(project_root.display().to_string()));
    }
    // No activity yet → nothing to learn from. Surface that cleanly rather than
    // paying for a Claude call that can only hallucinate from an empty digest.
    if events.is_empty() {
        return Ok(ReflectionResult {
            suggestions: Vec::new(),
            events_analyzed: 0,
            raw_excerpt: String::new(),
        });
    }

    let digest = build_digest(events);
    let existing_skills = list_existing_skills(project_root);
    let existing_memories = list_existing_memories(project_root);
    let installed_plugins = list_installed_plugins();
    let prompt = build_prompt(&digest, &existing_skills, &existing_memories, &installed_plugins);

    // Same resolver/flags as the Advisor: a GUI launch doesn't inherit the
    // login-shell PATH, so the bare `claude` name would fail to spawn.
    let mut cmd = crate::services::claude_cli::command(&[
        "-p",
        &prompt,
        "--permission-mode",
        "plan",
        "--output-format",
        "text",
    ]);
    cmd.current_dir(project_root);
    let output = cmd
        .output()
        .map_err(|e| AppError::Other(format!("failed to spawn `claude`: {e}. Is it on PATH?")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Other(format!(
            "claude exited with status {}: {}",
            output.status, stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let suggestions = parse_suggestions(&stdout)?;
    Ok(ReflectionResult {
        suggestions,
        events_analyzed: events.len(),
        raw_excerpt: truncate(&stdout, 4000),
    })
}

/// Condense the raw event stream into a compact, Claude-readable digest:
/// headline counts, the slash-commands used most, and the recent prompt texts
/// (the richest signal). Bounded so a long ledger can't blow up the prompt.
fn build_digest(events: &[ActivityEvent]) -> String {
    let prompts: Vec<&ActivityEvent> = events.iter().filter(|e| e.kind == "user_prompt").collect();
    let snapshots = events.iter().filter(|e| e.kind == "snapshot").count();

    let mut skill_freq: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &prompts {
        if let Some(s) = e.skill.as_deref() {
            *skill_freq.entry(s).or_insert(0) += 1;
        }
    }
    let mut skills_sorted: Vec<(&str, usize)> = skill_freq.into_iter().collect();
    skills_sorted.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    let top_skills = if skills_sorted.is_empty() {
        "(none)".to_string()
    } else {
        skills_sorted
            .iter()
            .take(12)
            .map(|(s, n)| format!("{s} ×{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let span = match (events.first(), events.last()) {
        (Some(a), Some(b)) if b.ts > a.ts => {
            let mins = (b.ts - a.ts) / 60_000;
            if mins >= 120 {
                format!("{} hours", mins / 60)
            } else {
                format!("{mins} minutes")
            }
        }
        _ => "unknown".to_string(),
    };

    // Keep the most recent prompts (the tail), since reflection is about recent
    // work. Cap the count and per-prompt length to bound the digest size.
    const MAX_PROMPTS: usize = 120;
    let start = prompts.len().saturating_sub(MAX_PROMPTS);
    let prompt_lines: Vec<String> = prompts[start..]
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let text = e.prompt.as_deref().unwrap_or("").replace('\n', " ");
            format!("{}. {}", start + i + 1, truncate(text.trim(), 240))
        })
        .collect();

    format!(
        "Activity window: {span}, {prompt_count} prompts, {snapshots} working-tree snapshots.\n\
         Slash-commands used: {top_skills}.\n\n\
         Prompts (chronological, most recent last):\n{prompts}",
        prompt_count = prompts.len(),
        prompts = prompt_lines.join("\n"),
    )
}

fn list_existing_skills(root: &Path) -> String {
    let dir = root.join(".claude").join("skills");
    let Ok(read) = fs::read_dir(&dir) else {
        return "(none)".to_string();
    };
    let names: Vec<String> = read
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        // Skip the reserved `_archived` namespace (and any dotfiles).
        .filter(|n| !n.starts_with('_') && !n.starts_with('.'))
        .collect();
    if names.is_empty() {
        "(none)".to_string()
    } else {
        names.join(", ")
    }
}

fn list_existing_memories(root: &Path) -> String {
    let Ok(entries) = crate::services::memory_service::list(root) else {
        return "(none)".to_string();
    };
    let lines: Vec<String> = entries
        .iter()
        .filter(|e| !e.is_index)
        .map(|e| match &e.description {
            Some(d) => format!("{} — {d}", e.name),
            None => e.name.clone(),
        })
        .collect();
    if lines.is_empty() {
        "(none)".to_string()
    } else {
        lines.join("\n")
    }
}

/// Installed plugins as a one-line list for the prompt, so reflection doesn't
/// propose packaging something the user already has. Best-effort — an empty
/// list just means the model gets no dedup signal.
fn list_installed_plugins() -> String {
    let plugins = crate::services::plugins_service::list_installed();
    if plugins.is_empty() {
        return "(none)".to_string();
    }
    plugins
        .iter()
        .map(|p| p.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn build_prompt(
    digest: &str,
    existing_skills: &str,
    existing_memories: &str,
    installed_plugins: &str,
) -> String {
    format!(
        r#"You analyze a developer's recent activity in a project and propose concrete,
high-leverage improvements to how Claude Code assists them. You are given a
digest of the prompts they submitted and the slash-commands they ran.

Propose 3 to 6 suggestions, each of ONE of these kinds:

- "skill": a workflow they clearly repeat by hand that a reusable Claude Code
  skill would automate. Only propose when the repetition is real and specific.
  Project-specific skills beat generic ones — that's where the value is.
- "memory": a durable fact, convention, or decision worth persisting so Claude
  recalls it in future sessions (e.g. "tests are run with X", "deploys go through Y").
- "plugin": ONLY when a workflow (or a cluster of related existing skills) is
  clearly valuable BEYOND this project — reusable across repos, or worth
  sharing with a team. A plugin is the shareable, versioned package form of
  skills. Do not suggest a plugin for a workflow that only makes sense here;
  that's a "skill". Do not duplicate an installed plugin listed below.
- "hook": a rule the user keeps enforcing BY HAND in their prompts (the same
  correction or prohibition repeated — "don't touch X", "always run Y after
  editing") that a Claude Code hook could enforce automatically. Report-only:
  describe the event and the check, the user wires it themselves.
- "friction": a recurring pain point (repeated rewinds/snapshots around the same
  area, repeated failed attempts, the same question asked many ways). Report-only.

Ground EVERY suggestion in the actual activity — cite specific prompts as evidence.
Do NOT invent generic best-practice advice that isn't supported by what they did.
Do NOT re-propose something already covered by existing skills, memories, or
installed plugins below. If the activity is too thin to support a kind, return
fewer suggestions.

EXISTING SKILLS: {existing_skills}

INSTALLED PLUGINS: {installed_plugins}

EXISTING MEMORIES:
{existing_memories}

RECENT ACTIVITY DIGEST
======================
{digest}

OUTPUT FORMAT (STRICT)
======================

Respond with ONLY a JSON object, no prose, no markdown code fences. Shape:

{{
  "suggestions": [
    {{
      "kind": "skill" | "memory" | "plugin" | "hook" | "friction",
      "title": "short imperative title",
      "rationale": "why this helps, grounded in the activity, 1-2 sentences",
      "evidence": ["specific prompt or pattern observed", "..."],
      "skillName": "kebab-case-name (only for kind=skill)",
      "skillMdContent": "---\nname: kebab-case-name\ndescription: ...\n---\n\nBody... (only for kind=skill)",
      "memoryName": "kebab-case-name.md (only for kind=memory)",
      "memoryContent": "---\nname: kebab-case-name\ndescription: one-line\nmetadata:\n  type: project\n---\n\nThe fact. (only for kind=memory)",
      "pluginName": "kebab-case-name (only for kind=plugin)",
      "pluginDescription": "one-line description for the plugin manifest (only for kind=plugin)",
      "pluginSkillMd": "---\nname: kebab-case-name\ndescription: what it does AND when to use it\n---\n\nInstructions... (only for kind=plugin; the plugin's starter SKILL.md)"
    }}
  ]
}}

Omit the skill*/memory*/plugin* fields for kinds they don't belong to.
"#,
        existing_skills = existing_skills,
        existing_memories = existing_memories,
        installed_plugins = installed_plugins,
        digest = digest,
    )
}

#[derive(Deserialize)]
struct Wrapper {
    suggestions: Vec<LearningSuggestion>,
}

fn parse_suggestions(stdout: &str) -> AppResult<Vec<LearningSuggestion>> {
    let trimmed = stdout.trim();
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    let json = match (start, end) {
        (Some(s), Some(e)) if e > s => &trimmed[s..=e],
        _ => {
            return Err(AppError::Other(format!(
                "claude output did not contain a JSON object. First 400 chars: {}",
                truncate(trimmed, 400)
            )))
        }
    };
    let wrapper: Wrapper = serde_json::from_str(json)
        .map_err(|e| AppError::Other(format!("failed to parse suggestions JSON: {e}")))?;
    Ok(wrapper.suggestions)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut cut = max;
        while !s.is_char_boundary(cut) && cut > 0 {
            cut -= 1;
        }
        format!("{}…", &s[..cut])
    }
}

// ============================================================================
// Corpus curation
//
// Where `reflect` *grows* the corpus (activity → new skills/memories), `curate`
// *tends* the corpus that already exists, so it doesn't rot as it grows: fuse
// duplicates, flag entries that point at things the repo no longer has, rewrite
// sloppy ones, and surface dead weight. Same proven shape as `reflect`
// (`claude -p` in plan mode → strict JSON), but the input is the corpus content
// itself plus usage signals, not the activity stream.
// ============================================================================

/// One curation action the model proposes over the existing corpus.
///
/// `action` drives what the UI offers (all suggest-only — the user approves each):
/// - "merge"    → fuse `targets` (≥2) into one entry (`newName` + `newContent`)
/// - "refactor" → rewrite a single target's content in place (`newContent`)
/// - "archive"  → retire a redundant/obsolete entry (no payload)
/// - "rerank"   → report-only insight about value/usage (no payload, no action)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurationSuggestion {
    pub action: String,
    /// "skill" | "memory" — both targets of a suggestion share one kind.
    pub target_kind: String,
    /// Existing entry names this acts on. For skills: the directory name.
    /// For memories: the `*.md` filename.
    pub targets: Vec<String>,
    pub title: String,
    pub rationale: String,
    /// Concrete grounds: overlap, broken refs, usage=0, etc.
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurationResult {
    pub suggestions: Vec<CurationSuggestion>,
    pub skills_analyzed: usize,
    pub memories_analyzed: usize,
    pub raw_excerpt: String,
}

/// One existing corpus entry, with the code-side signals we compute before
/// asking the model: how often it was used, and which references look broken.
struct CorpusEntry {
    kind: &'static str, // "skill" | "memory"
    name: String,
    /// Slash-command invocations seen in the ledger (skills only; memories have
    /// no usage signal there, so this is `None` for them).
    usage: Option<usize>,
    /// Path-like references inside the entry that don't exist in the repo.
    broken_refs: Vec<String>,
    content: String,
}

/// Curate the existing skill/memory corpus and propose improvements. Read-only:
/// it never mutates the corpus — it returns suggestions the user applies.
pub fn curate(project_root: &Path, events: &[ActivityEvent]) -> AppResult<CurationResult> {
    if !project_root.is_dir() {
        return Err(AppError::NotADirectory(project_root.display().to_string()));
    }

    let entries = gather_corpus(project_root, events);
    let skills_analyzed = entries.iter().filter(|e| e.kind == "skill").count();
    let memories_analyzed = entries.iter().filter(|e| e.kind == "memory").count();

    // Nothing to consolidate with fewer than two entries — no overlap is
    // possible and a single entry isn't worth a Claude call.
    if entries.len() < 2 {
        return Ok(CurationResult {
            suggestions: Vec::new(),
            skills_analyzed,
            memories_analyzed,
            raw_excerpt: String::new(),
        });
    }

    let prompt = build_curation_prompt(&entries);
    let mut cmd = crate::services::claude_cli::command(&[
        "-p",
        &prompt,
        "--permission-mode",
        "plan",
        "--output-format",
        "text",
    ]);
    cmd.current_dir(project_root);
    let output = cmd
        .output()
        .map_err(|e| AppError::Other(format!("failed to spawn `claude`: {e}. Is it on PATH?")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Other(format!(
            "claude exited with status {}: {}",
            output.status, stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let suggestions = parse_curation(&stdout)?;
    Ok(CurationResult {
        suggestions,
        skills_analyzed,
        memories_analyzed,
        raw_excerpt: truncate(&stdout, 4000),
    })
}

/// Per-entry content is bounded so a big corpus can't blow up the prompt.
const CORPUS_ENTRY_MAX: usize = 1500;
/// Hard cap on how many entries we feed the model in one pass.
const CORPUS_MAX_ENTRIES: usize = 80;

/// Read the full project corpus (skill bodies + memory bodies) and attach the
/// code-side signals — usage counts from the ledger and broken file references.
fn gather_corpus(root: &Path, events: &[ActivityEvent]) -> Vec<CorpusEntry> {
    let usage = slash_command_usage(events);
    let mut out = Vec::new();

    // Project skills (the ones learning mode creates live under .claude/skills).
    if let Ok(skills) = crate::services::skills_service::list(Some(root)) {
        for s in skills
            .into_iter()
            .filter(|s| s.source == "project" && s.kind == "skill")
        {
            let content = crate::services::skills_service::read_md(&s.path).unwrap_or_default();
            let broken_refs = find_broken_refs(root, &content);
            out.push(CorpusEntry {
                kind: "skill",
                usage: Some(*usage.get(&normalize_skill(&s.name)).unwrap_or(&0)),
                broken_refs,
                content: truncate(&content, CORPUS_ENTRY_MAX),
                name: s.name,
            });
        }
    }

    // Project memories (skip the hand-curated MEMORY.md index).
    if let Ok(mems) = crate::services::memory_service::list(root) {
        for m in mems.into_iter().filter(|m| !m.is_index) {
            let content = crate::services::memory_service::read(root, &m.name).unwrap_or_default();
            let broken_refs = find_broken_refs(root, &content);
            out.push(CorpusEntry {
                kind: "memory",
                name: m.name,
                usage: None,
                broken_refs,
                content: truncate(&content, CORPUS_ENTRY_MAX),
            });
        }
    }

    out.truncate(CORPUS_MAX_ENTRIES);
    out
}

/// Count how often each slash-command was invoked, keyed by normalized name so
/// the ledger's "/deploy" lines up with the "deploy" skill directory.
fn slash_command_usage(events: &[ActivityEvent]) -> BTreeMap<String, usize> {
    let mut freq: BTreeMap<String, usize> = BTreeMap::new();
    for e in events {
        if let Some(s) = e.skill.as_deref() {
            *freq.entry(normalize_skill(s)).or_insert(0) += 1;
        }
    }
    freq
}

fn normalize_skill(s: &str) -> String {
    s.trim().trim_start_matches('/').to_lowercase()
}

/// Heuristic, conservative scan for path-like tokens that don't exist in the
/// repo. Deliberately under-flags (only clear `dir/file.ext`-shaped tokens) —
/// the model makes the final call, so a missed ref costs less than noise.
fn find_broken_refs(root: &Path, content: &str) -> Vec<String> {
    let mut missing = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for raw in content.split(|c: char| {
        c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | '[' | ']' | ',' | '<' | '>')
    }) {
        let tok = raw.trim_matches(|c: char| matches!(c, '.' | ':' | ';' | '#' | '*' | '!'));
        if !looks_like_path(tok) {
            continue;
        }
        if !seen.insert(tok.to_string()) {
            continue;
        }
        // Strip a trailing `:line` (and `:col`) reference like `foo.rs:90`.
        let path_part = tok.split(':').next().unwrap_or(tok);
        if path_part.is_empty() {
            continue;
        }
        if !root.join(path_part).exists() {
            missing.push(tok.to_string());
            if missing.len() >= 8 {
                break;
            }
        }
    }
    missing
}

fn looks_like_path(tok: &str) -> bool {
    if tok.len() < 3 || tok.len() > 200 {
        return false;
    }
    // URLs, home-relative, and absolute system paths aren't repo refs.
    if tok.contains("://") || tok.starts_with('~') || tok.starts_with('/') {
        return false;
    }
    if !tok.contains('/') {
        return false;
    }
    if !tok
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':'))
    {
        return false;
    }
    // The last segment must carry an extension (a dot), so we flag `a/b.rs`
    // but not directory-ish tokens like `and/or`.
    tok.rsplit('/').next().unwrap_or("").contains('.')
}

fn build_curation_prompt(entries: &[CorpusEntry]) -> String {
    let mut corpus = String::new();
    for e in entries {
        corpus.push_str(&format!("[{}] {}", e.kind, e.name));
        if let Some(u) = e.usage {
            corpus.push_str(&format!("  (invoked ×{u})"));
        }
        corpus.push('\n');
        if !e.broken_refs.is_empty() {
            corpus.push_str(&format!(
                "  candidate broken refs: {}\n",
                e.broken_refs.join(", ")
            ));
        }
        corpus.push_str("  ---\n");
        for line in e.content.lines() {
            corpus.push_str("  ");
            corpus.push_str(line);
            corpus.push('\n');
        }
        corpus.push('\n');
    }

    format!(
        r#"You are curating a developer's library of Claude Code skills and memories
so it stays sharp as it grows. You are given every existing entry's content,
plus two code-computed signals: how often each skill was invoked, and any
path-like references that no longer exist in the repo ("candidate broken refs").

Propose concrete curation actions, each of ONE of these kinds:

- "merge": two or more entries overlap heavily (same workflow/fact restated).
  Fuse them into one. List ALL the source entry names in `targets`, and provide
  `newName` plus `newContent` (the consolidated entry, keeping every distinct
  detail — never drop information when merging).
- "refactor": one entry is vague, bloated, outdated, or has broken refs that you
  can fix. Rewrite it. Put the single entry in `targets` and the rewritten body
  in `newContent` (preserve its frontmatter shape). Use this to repair the
  "candidate broken refs" when the correct path is obvious from context.
- "archive": one entry is obsolete (its subject no longer exists) or fully
  redundant with another that survives. Single entry in `targets`, no payload.
- "rerank": report-only insight about value — e.g. a skill invoked ×0 that may be
  dead weight, or the highest-leverage entries. No payload, no mutation; informs
  the user. Use sparingly.

Rules:
- Ground EVERY suggestion in the actual content/signals shown. Cite specifics in
  `evidence` (the overlapping phrasing, the broken ref, the usage count).
- `targetKind` is "skill" or "memory"; a single suggestion never mixes the two.
- Do NOT propose archiving an entry merely for low usage — usage=0 is a `rerank`
  insight, not grounds to delete. Archive only for genuine obsolescence/redundancy.
- Prefer few high-confidence actions over many speculative ones. If the corpus is
  already clean, return an empty list.

EXISTING CORPUS
===============
{corpus}

OUTPUT FORMAT (STRICT)
======================

Respond with ONLY a JSON object, no prose, no markdown code fences. Shape:

{{
  "suggestions": [
    {{
      "action": "merge" | "refactor" | "archive" | "rerank",
      "targetKind": "skill" | "memory",
      "targets": ["existing-entry-name", "..."],
      "title": "short imperative title",
      "rationale": "why this helps, 1-2 sentences",
      "evidence": ["specific overlap / broken ref / usage observed", "..."],
      "newName": "kebab-case name (merge/refactor only; for memory keep the .md)",
      "newContent": "full resulting entry incl. frontmatter (merge/refactor only)"
    }}
  ]
}}

Omit `newName`/`newContent` for "archive" and "rerank".
"#,
        corpus = corpus,
    )
}

#[derive(Deserialize)]
struct CurationWrapper {
    suggestions: Vec<CurationSuggestion>,
}

fn parse_curation(stdout: &str) -> AppResult<Vec<CurationSuggestion>> {
    let trimmed = stdout.trim();
    let start = trimmed.find('{');
    let end = trimmed.rfind('}');
    let json = match (start, end) {
        (Some(s), Some(e)) if e > s => &trimmed[s..=e],
        _ => {
            return Err(AppError::Other(format!(
                "claude output did not contain a JSON object. First 400 chars: {}",
                truncate(trimmed, 400)
            )))
        }
    };
    let wrapper: CurationWrapper = serde_json::from_str(json)
        .map_err(|e| AppError::Other(format!("failed to parse curation JSON: {e}")))?;
    Ok(wrapper.suggestions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt_ev(ts: i64, prompt: &str, skill: Option<&str>) -> ActivityEvent {
        ActivityEvent {
            ts,
            kind: "user_prompt".into(),
            prompt: Some(prompt.into()),
            skill: skill.map(|s| s.into()),
            term_id: None,
            session_id: None,
            snapshot_sha: None,
        }
    }

    #[test]
    fn digest_summarizes_counts_and_top_skills() {
        let events = vec![
            prompt_ev(0, "fix the build", None),
            prompt_ev(60_000, "/deploy now", Some("/deploy")),
            prompt_ev(120_000, "run /deploy again", Some("/deploy")),
        ];
        let d = build_digest(&events);
        assert!(d.contains("3 prompts"), "digest should count prompts: {d}");
        assert!(
            d.contains("/deploy ×2"),
            "digest should rank slash-commands: {d}"
        );
        assert!(
            d.contains("fix the build"),
            "digest should include prompt text"
        );
    }

    /// True end-to-end: seed the real ledger, read it back, reflect through the
    /// real `claude` CLI, then materialize both a memory and a skill. Ignored by
    /// default (spawns Claude, costs tokens, needs auth). Run explicitly:
    ///   cargo test end_to_end_reflect_and_materialize -- --ignored --nocapture
    #[test]
    #[ignore = "spawns real `claude`; run with --ignored"]
    fn end_to_end_reflect_and_materialize() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        // Isolated ledger dir, and a real (empty) project dir to run `claude` in.
        let base = std::env::temp_dir().join(format!("ac-e2e-data-{nanos}"));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
        let project = std::env::temp_dir().join(format!("ac-e2e-proj-{nanos}"));
        std::fs::create_dir_all(&project).unwrap();
        let root = project.to_string_lossy().to_string();

        // 1. Seed the ledger through the real service — a clear repeated-deploy
        //    pattern Claude should latch onto, plus an unrelated testing thread.
        let svc = crate::services::activity_service::ActivityService::new();
        let seed = [
            ("deploy the backend to prod", true),
            ("the deploy script failed, run it again", true),
            ("deploy backend to prod once more", true),
            ("why does the deploy keep timing out on lightsail", true),
            ("add a unit test for the login endpoint", false),
            ("write a test for the user service too", false),
        ];
        for (i, (p, is_deploy)) in seed.iter().enumerate() {
            svc.record(
                &root,
                &ActivityEvent {
                    ts: (i as i64) * 60_000,
                    kind: "user_prompt".into(),
                    prompt: Some((*p).into()),
                    skill: is_deploy.then(|| "/deploy".to_string()),
                    term_id: Some("t1".into()),
                    session_id: None,
                    snapshot_sha: None,
                },
            )
            .unwrap();
        }

        // 2. Read back through the service, then reflect via real `claude`.
        let events = svc.list(&root, Some(400)).unwrap();
        assert_eq!(events.len(), 6, "ledger round-trip");
        let result = reflect(&project, &events).expect("reflect should succeed");
        eprintln!("\n=== events_analyzed: {} ===", result.events_analyzed);
        eprintln!("=== raw excerpt ===\n{}\n", result.raw_excerpt);
        for s in &result.suggestions {
            eprintln!("- [{}] {}\n    {}", s.kind, s.title, s.rationale);
            for e in &s.evidence {
                eprintln!("    · {e}");
            }
        }
        assert_eq!(result.events_analyzed, 6);
        assert!(!result.suggestions.is_empty(), "expected >=1 suggestion");
        assert!(
            result
                .suggestions
                .iter()
                .all(|s| { matches!(s.kind.as_str(), "skill" | "memory" | "friction") }),
            "every suggestion must be a known kind"
        );

        // 3. Materialize both output paths the UI offers.
        let mem = crate::services::memory_service::write(
            &project,
            "e2e-deploy.md",
            "---\nname: e2e-deploy\ndescription: deploy runbook\nmetadata:\n  type: project\n---\n\nDeploy notes.",
        )
        .expect("memory write");
        assert!(mem.exists(), "memory file written");
        let skill = crate::services::advisor_service::create_skill(
            &project,
            "project",
            "e2e-deploy-skill",
            "---\nname: e2e-deploy-skill\ndescription: deploy\n---\n\nBody.",
        )
        .expect("skill create");
        assert!(skill.exists(), "SKILL.md written");

        // Cleanup: temp project (holds .claude/skills) + isolated data dir + the
        // memory slug dir (lives under ~/.claude/projects/<slug>/memory).
        if let Some(memory_dir) = mem.parent() {
            if let Some(slug_dir) = memory_dir.parent() {
                let _ = std::fs::remove_dir_all(slug_dir);
            }
        }
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn looks_like_path_flags_only_repo_refs() {
        assert!(looks_like_path("src/services/learning.rs"));
        assert!(looks_like_path(".claude/skills/x/SKILL.md"));
        // No slash, a URL, home/absolute, or no extension → not a repo ref.
        assert!(!looks_like_path("Cargo.toml"));
        assert!(!looks_like_path("https://example.com/a.rs"));
        assert!(!looks_like_path("~/.claude/foo.md"));
        assert!(!looks_like_path("/etc/hosts"));
        assert!(!looks_like_path("and/or"));
    }

    #[test]
    fn find_broken_refs_reports_missing_only() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("ac-curate-refs-{nanos}"));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/real.rs"), "fn main() {}").unwrap();

        let content = "See `src/real.rs` and `src/gone.rs:42` for details. Run scripts/x.sh.";
        let broken = find_broken_refs(&root, content);
        assert!(
            broken.contains(&"src/gone.rs:42".to_string()),
            "missing ref should be flagged: {broken:?}"
        );
        assert!(
            broken.contains(&"scripts/x.sh".to_string()),
            "missing ref should be flagged: {broken:?}"
        );
        assert!(
            !broken.iter().any(|r| r.starts_with("src/real.rs")),
            "existing ref must not be flagged: {broken:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn slash_command_usage_normalizes_names() {
        let events = vec![
            prompt_ev(0, "/Deploy now", Some("/deploy")),
            prompt_ev(1, "deploy again", Some("deploy")),
            prompt_ev(2, "test it", Some("/test")),
        ];
        let usage = slash_command_usage(&events);
        assert_eq!(usage.get("deploy"), Some(&2));
        assert_eq!(usage.get("test"), Some(&1));
    }

    #[test]
    fn parse_curation_tolerates_surrounding_chatter() {
        let raw = r#"Sure:
        {"suggestions":[{"action":"merge","targetKind":"memory","targets":["a.md","b.md"],"title":"t","rationale":"r","evidence":["overlap"],"newName":"ab.md","newContent":"c"}]}
        done"#;
        let got = parse_curation(raw).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].action, "merge");
        assert_eq!(got[0].targets.len(), 2);
        assert_eq!(got[0].new_name.as_deref(), Some("ab.md"));
    }

    #[test]
    fn parse_tolerates_surrounding_chatter() {
        let raw = r#"Here you go:
        {"suggestions":[{"kind":"memory","title":"t","rationale":"r","evidence":["e"],"memoryName":"x.md","memoryContent":"c"}]}
        hope that helps"#;
        let got = parse_suggestions(raw).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "memory");
        assert_eq!(got[0].memory_name.as_deref(), Some("x.md"));
        assert!(got[0].skill_name.is_none());
    }
}

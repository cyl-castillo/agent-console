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
    let prompt = build_prompt(&digest, &existing_skills, &existing_memories);

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

fn build_prompt(digest: &str, existing_skills: &str, existing_memories: &str) -> String {
    format!(
        r#"You analyze a developer's recent activity in a project and propose concrete,
high-leverage improvements to how Claude Code assists them. You are given a
digest of the prompts they submitted and the slash-commands they ran.

Propose 3 to 6 suggestions, each of ONE of these kinds:

- "skill": a workflow they clearly repeat by hand that a reusable Claude Code
  skill would automate. Only propose when the repetition is real and specific.
- "memory": a durable fact, convention, or decision worth persisting so Claude
  recalls it in future sessions (e.g. "tests are run with X", "deploys go through Y").
- "friction": a recurring pain point (repeated rewinds/snapshots around the same
  area, repeated failed attempts, the same question asked many ways). Report-only.

Ground EVERY suggestion in the actual activity — cite specific prompts as evidence.
Do NOT invent generic best-practice advice that isn't supported by what they did.
Do NOT re-propose something already covered by existing skills or memories below.
If the activity is too thin to support a kind, return fewer suggestions.

EXISTING SKILLS: {existing_skills}

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
      "kind": "skill" | "memory" | "friction",
      "title": "short imperative title",
      "rationale": "why this helps, grounded in the activity, 1-2 sentences",
      "evidence": ["specific prompt or pattern observed", "..."],
      "skillName": "kebab-case-name (only for kind=skill)",
      "skillMdContent": "---\nname: kebab-case-name\ndescription: ...\n---\n\nBody... (only for kind=skill)",
      "memoryName": "kebab-case-name.md (only for kind=memory)",
      "memoryContent": "---\nname: kebab-case-name\ndescription: one-line\nmetadata:\n  type: project\n---\n\nThe fact. (only for kind=memory)"
    }}
  ]
}}

Omit the skill* fields for non-skill kinds and the memory* fields for non-memory kinds.
"#,
        existing_skills = existing_skills,
        existing_memories = existing_memories,
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
        assert!(d.contains("/deploy ×2"), "digest should rank slash-commands: {d}");
        assert!(d.contains("fix the build"), "digest should include prompt text");
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

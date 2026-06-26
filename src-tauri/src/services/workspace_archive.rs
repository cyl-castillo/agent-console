//! Export the user's "work" to a single portable file that another user can
//! import into their own installation.
//!
//! What "work" means here is deliberately scoped to the things a person builds
//! up while using the app — terminal sessions, roundtable rooms, scheduled jobs,
//! and learning-mode skills/memory — NOT machine-specific config or secrets.
//! Two hard rules shape this module:
//!
//! 1. **No secrets leave the machine.** The vault's secret values live in the OS
//!    keychain (never in the JSON on disk), and we never read them here. Nothing
//!    in an exported archive is a credential.
//! 2. **Nothing machine-bound travels verbatim.** Every store keys its data by
//!    the project's *absolute path* (a HashMap key, or a path-hash filename).
//!    That path is meaningless — and privacy-leaking — on another machine, so on
//!    export we strip session `cwd`s and re-key happens on import (Fase 2). We
//!    also drop runtime-only handles (Claude resume ids, engine resume refs, job
//!    backoff/next-due state) that are valid only in the session that wrote them.

use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::services::activity_service::ActivityEvent;
use crate::services::context_service;
use crate::services::memory_service;
use crate::services::roundtable_service::PersistedRoom;
use crate::services::scheduler_service::Job;
use crate::services::sessions_service::PersistedSession;
use crate::services::skills_service;
use crate::state::AppState;

/// Magic string identifying our archive format, checked on import so we reject
/// arbitrary JSON files with a clear error instead of a confusing parse failure.
pub const ARCHIVE_FORMAT: &str = "agent-console/workspace-archive";

/// Bump when the archive shape changes incompatibly. Import refuses a `version`
/// it does not understand rather than silently mis-reading a newer file.
pub const ARCHIVE_VERSION: u32 = 1;

/// Which blocks of work to include. All default to `false` so an omitted field
/// (older frontend, partial call) exports nothing rather than over-sharing.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    #[serde(default)]
    pub sessions: bool,
    #[serde(default)]
    pub rooms: bool,
    #[serde(default)]
    pub schedules: bool,
    #[serde(default)]
    pub learning: bool,
    /// Include the raw activity ledger inside the learning block. Off by default:
    /// it is large and a fine-grained record of what the user did, so we ship the
    /// *outputs* of learning (skills/memory) but not the raw input unless asked.
    #[serde(default)]
    pub include_activity: bool,
}

/// A named markdown document (a skill's SKILL.md, or one memory entry), carried
/// as inline text so the archive is a single self-contained, human-auditable
/// file with no external references.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamedDoc {
    /// Skill folder name, or memory file name (e.g. `voice-input-feature.md`).
    pub name: String,
    pub content: String,
}

/// Learning-mode outputs: project skills and the per-project memory corpus.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LearningBlock {
    #[serde(default)]
    pub skills: Vec<NamedDoc>,
    #[serde(default)]
    pub memory: Vec<NamedDoc>,
    /// Raw activity ledger events (only present when `include_activity` was set).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity: Vec<ActivityEvent>,
}

/// The blocks present in an archive. Each is `Option` so the file records which
/// blocks were *chosen* (present but empty = "I exported sessions, there were
/// none") versus *not chosen* (absent) — the import UI uses that distinction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveBlocks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<PersistedSession>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rooms: Option<Vec<PersistedRoom>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedules: Option<Vec<Job>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning: Option<LearningBlock>,
}

/// The portable archive. `source_project_name` is the *name* only (not the path)
/// so the import UI can say "from agent-console" without leaking the exporter's
/// directory layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceArchive {
    pub format: String,
    pub version: u32,
    pub created_at_ms: u64,
    pub source_project_name: String,
    pub blocks: ArchiveBlocks,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Strip everything that is meaningless or privacy-leaking off this machine: the
/// working directory (a source-machine absolute path — import sets it to the
/// destination project) and the Claude resume id (a handle into *this* machine's
/// `~/.claude` session store; replaying it elsewhere would resume the wrong or a
/// nonexistent conversation).
fn scrub_session(mut s: PersistedSession) -> PersistedSession {
    s.cwd = String::new();
    s.claude_session_id = None;
    s.name_suggested = None;
    s
}

/// Keep the transcript, participants and token total — that is the actual work —
/// but drop the engines' resume refs and per-engine `last_seen` cursors, which
/// point at live runs that exist only in the exporting session.
fn scrub_room(mut r: PersistedRoom) -> PersistedRoom {
    r.resume.clear();
    r.last_seen.clear();
    r
}

/// Carry the job's identity and behaviour (trigger, action, on-missed, cooldown)
/// but reset all scheduling/runtime state and land it **disabled**, so importing
/// someone else's jobs never silently starts firing `claude` on the recipient's
/// machine. They review and enable deliberately.
fn scrub_job(mut j: Job) -> Job {
    j.enabled = false;
    j.last_run_ms = None;
    j.next_due_ms = None;
    j.consecutive_failures = 0;
    j.backoff_until_ms = None;
    j
}

/// Read the per-project memory corpus as inline docs. We skip the `MEMORY.md`
/// index (it is regenerated from the entries, and merging two indexes blindly
/// would corrupt it) and any non-`.md` files; `_archived/` is a subdirectory and
/// is naturally excluded by the top-level-files-only read.
fn read_memory(project_root: &Path) -> Vec<NamedDoc> {
    let dir = match context_service::memory_dir_for(project_root) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "MEMORY.md" || !name.ends_with(".md") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            out.push(NamedDoc { name, content });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Read project-scoped skills (kind "skill": a folder with a SKILL.md) as inline
/// docs. User-level skills and commands/agents are out of scope for the learning
/// block — those are tooling, not the work this project produced.
fn read_skills(project_root: &Path) -> Vec<NamedDoc> {
    let all = match skills_service::list(Some(project_root)) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for skill in all {
        if skill.source != "project" || skill.kind != "skill" {
            continue;
        }
        if let Ok(content) = skills_service::read_md(&skill.path) {
            out.push(NamedDoc {
                name: skill.name,
                content,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Every full persisted room for a project. `RoomsStore` exposes summaries +
/// per-id `get`, so we list the summaries and re-read each room's full state.
fn read_rooms(state: &AppState, project_key: &str) -> AppResult<Vec<PersistedRoom>> {
    let store = state.roundtable.rooms();
    let mut rooms = Vec::new();
    for summary in store.summaries(project_key)? {
        if let Some(room) = store.get(project_key, &summary.id)? {
            rooms.push(scrub_room(room));
        }
    }
    Ok(rooms)
}

/// Build the in-memory archive for a project according to `options`. Pure read
/// of the stores + `.claude` — never mutates anything.
pub fn build_archive(
    state: &AppState,
    project_root: &str,
    options: ExportOptions,
) -> AppResult<WorkspaceArchive> {
    let path = Path::new(project_root);
    let source_project_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());

    let mut blocks = ArchiveBlocks::default();

    if options.sessions {
        let sessions = state
            .sessions
            .list(project_root)?
            .into_iter()
            .map(scrub_session)
            .collect();
        blocks.sessions = Some(sessions);
    }

    if options.rooms {
        blocks.rooms = Some(read_rooms(state, project_root)?);
    }

    if options.schedules {
        let jobs = state
            .scheduler
            .list(project_root)?
            .into_iter()
            .map(scrub_job)
            .collect();
        blocks.schedules = Some(jobs);
    }

    if options.learning {
        let activity = if options.include_activity {
            state.activity.list(project_root, None).unwrap_or_default()
        } else {
            Vec::new()
        };
        blocks.learning = Some(LearningBlock {
            skills: read_skills(path),
            memory: read_memory(path),
            activity,
        });
    }

    Ok(WorkspaceArchive {
        format: ARCHIVE_FORMAT.to_string(),
        version: ARCHIVE_VERSION,
        created_at_ms: now_ms(),
        source_project_name,
        blocks,
    })
}

/// Serialize an archive to pretty JSON (human-auditable: a user can open the
/// file and confirm no secrets are inside).
pub fn to_json(archive: &WorkspaceArchive) -> AppResult<String> {
    serde_json::to_string_pretty(archive)
        .map_err(|e| AppError::Other(format!("serialize archive: {e}")))
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Per-block decision the importing user makes after seeing the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// Don't import this block at all (the default — importing is opt-in).
    #[default]
    Skip,
    /// Add the archive's items, keeping anything already in the destination
    /// (collisions by id/name are left untouched — re-importing is idempotent).
    Merge,
    /// Overwrite colliding items with the archive's version and add new ones.
    /// Never deletes destination-only items (importing is additive, not a wipe).
    Replace,
}

/// One decision per block. Absent fields default to `Skip`.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDecisions {
    #[serde(default)]
    pub sessions: Decision,
    #[serde(default)]
    pub rooms: Decision,
    #[serde(default)]
    pub schedules: Decision,
    #[serde(default)]
    pub skills: Decision,
    #[serde(default)]
    pub memory: Decision,
}

/// What one block contains and how it overlaps the destination, so the UI can
/// show "12 sessions (3 already here)" before the user picks merge/replace/skip.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockManifest {
    /// The block was chosen at export time (present in the file), even if empty.
    pub present: bool,
    pub total: usize,
    /// How many items collide with something already in the destination project.
    pub collisions: usize,
}

/// Preview of an archive against a specific destination project.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportManifest {
    pub source_project_name: String,
    pub created_at_ms: u64,
    pub sessions: BlockManifest,
    pub rooms: BlockManifest,
    pub schedules: BlockManifest,
    pub skills: BlockManifest,
    pub memory: BlockManifest,
}

/// What an import actually applied, for the confirmation toast.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub sessions: usize,
    pub rooms: usize,
    pub schedules: usize,
    pub skills: usize,
    pub memory: usize,
}

/// Parse and validate an archive: reject files that aren't ours, and refuse a
/// `version` newer than we understand rather than silently mis-reading it.
pub fn parse_archive(json: &str) -> AppResult<WorkspaceArchive> {
    let archive: WorkspaceArchive = serde_json::from_str(json)
        .map_err(|e| AppError::InvalidArgument(format!("not a valid workspace archive: {e}")))?;
    if archive.format != ARCHIVE_FORMAT {
        return Err(AppError::InvalidArgument(format!(
            "unrecognized file format: '{}'",
            archive.format
        )));
    }
    if archive.version > ARCHIVE_VERSION {
        return Err(AppError::InvalidArgument(format!(
            "archive version {} is newer than this app supports ({}); update the app",
            archive.version, ARCHIVE_VERSION
        )));
    }
    Ok(archive)
}

/// Count how many of `incoming` ids/names already exist in `existing`.
fn count_collisions(existing: &HashSet<String>, incoming: impl Iterator<Item = String>) -> usize {
    incoming.filter(|k| existing.contains(k)).count()
}

/// Existing project skill folder names in the destination (for collision counts).
fn dest_skill_names(dest_root: &Path) -> HashSet<String> {
    skills_service::list(Some(dest_root))
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.source == "project" && s.kind == "skill")
        .map(|s| s.name)
        .collect()
}

/// Existing memory entry file names in the destination (excluding the index).
fn dest_memory_names(dest_root: &Path) -> HashSet<String> {
    memory_service::list(dest_root)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| !m.is_index)
        .map(|m| m.name)
        .collect()
}

/// Build the preview of `archive` against `dest_root`.
pub fn build_manifest(
    state: &AppState,
    dest_root: &str,
    archive: &WorkspaceArchive,
) -> AppResult<ImportManifest> {
    let dest_path = Path::new(dest_root);
    let b = &archive.blocks;

    let sessions = match &b.sessions {
        Some(items) => {
            let existing: HashSet<String> =
                state.sessions.list(dest_root)?.into_iter().map(|s| s.id).collect();
            BlockManifest {
                present: true,
                total: items.len(),
                collisions: count_collisions(&existing, items.iter().map(|s| s.id.clone())),
            }
        }
        None => BlockManifest::default(),
    };

    let rooms = match &b.rooms {
        Some(items) => {
            let existing: HashSet<String> = state
                .roundtable
                .rooms()
                .summaries(dest_root)?
                .into_iter()
                .map(|s| s.id)
                .collect();
            BlockManifest {
                present: true,
                total: items.len(),
                collisions: count_collisions(&existing, items.iter().map(|r| r.id.clone())),
            }
        }
        None => BlockManifest::default(),
    };

    let schedules = match &b.schedules {
        Some(items) => {
            let existing: HashSet<String> =
                state.scheduler.list(dest_root)?.into_iter().map(|j| j.id).collect();
            BlockManifest {
                present: true,
                total: items.len(),
                collisions: count_collisions(&existing, items.iter().map(|j| j.id.clone())),
            }
        }
        None => BlockManifest::default(),
    };

    let (skills, memory) = match &b.learning {
        Some(l) => {
            let skill_names = dest_skill_names(dest_path);
            let mem_names = dest_memory_names(dest_path);
            (
                BlockManifest {
                    present: true,
                    total: l.skills.len(),
                    collisions: count_collisions(
                        &skill_names,
                        l.skills.iter().map(|d| d.name.clone()),
                    ),
                },
                BlockManifest {
                    present: true,
                    total: l.memory.len(),
                    collisions: count_collisions(
                        &mem_names,
                        l.memory.iter().map(|d| d.name.clone()),
                    ),
                },
            )
        }
        None => (BlockManifest::default(), BlockManifest::default()),
    };

    Ok(ImportManifest {
        source_project_name: archive.source_project_name.clone(),
        created_at_ms: archive.created_at_ms,
        sessions,
        rooms,
        schedules,
        skills,
        memory,
    })
}

/// Apply an archive to `dest_root` according to `decisions`. Every block is
/// re-keyed to the destination project: session `cwd`s become `dest_root`, and
/// per-project store keys are the destination's, never the source's.
pub fn apply_archive(
    state: &AppState,
    dest_root: &str,
    archive: &WorkspaceArchive,
    decisions: ImportDecisions,
) -> AppResult<ImportResult> {
    let dest_path = Path::new(dest_root);
    let b = &archive.blocks;
    let mut result = ImportResult::default();

    // --- Sessions: rewrite cwd to the destination project, save the whole bucket.
    if let (Some(items), d) = (&b.sessions, decisions.sessions) {
        if d != Decision::Skip {
            let mut bucket = if d == Decision::Replace {
                Vec::new()
            } else {
                state.sessions.list(dest_root)?
            };
            let mut ids: HashSet<String> = bucket.iter().map(|s| s.id.clone()).collect();
            for src in items {
                // Merge keeps an existing session untouched; Replace overwrites it.
                if ids.contains(&src.id) {
                    if d == Decision::Merge {
                        continue;
                    }
                    bucket.retain(|s| s.id != src.id);
                }
                let mut s = src.clone();
                s.cwd = dest_root.to_string();
                ids.insert(s.id.clone());
                bucket.push(s);
                result.sessions += 1;
            }
            state.sessions.save(dest_root, bucket)?;
        }
    }

    // --- Rooms: upsert each under the destination key.
    if let (Some(items), d) = (&b.rooms, decisions.rooms) {
        if d != Decision::Skip {
            let store = state.roundtable.rooms();
            let existing: HashSet<String> = store
                .summaries(dest_root)?
                .into_iter()
                .map(|s| s.id)
                .collect();
            for room in items {
                if existing.contains(&room.id) && d == Decision::Merge {
                    continue;
                }
                store.save_room(room, dest_path)?;
                result.rooms += 1;
            }
        }
    }

    // --- Schedules: create() upserts by id and recomputes next_due; jobs stay
    //     disabled (scrubbed at export), so nothing fires on import.
    if let (Some(items), d) = (&b.schedules, decisions.schedules) {
        if d != Decision::Skip {
            let existing: HashSet<String> =
                state.scheduler.list(dest_root)?.into_iter().map(|j| j.id).collect();
            for job in items {
                if existing.contains(&job.id) && d == Decision::Merge {
                    continue;
                }
                state.scheduler.create(dest_root, job.clone())?;
                result.schedules += 1;
            }
        }
    }

    // --- Learning skills + memory: write each doc; Merge skips colliding names,
    //     Replace overwrites them. `write` guards against path-traversal in the
    //     untrusted doc name.
    if let Some(l) = &b.learning {
        if decisions.skills != Decision::Skip {
            let existing = dest_skill_names(dest_path);
            for doc in &l.skills {
                if existing.contains(&doc.name) && decisions.skills == Decision::Merge {
                    continue;
                }
                if skills_service::write(dest_path, &doc.name, &doc.content).is_ok() {
                    result.skills += 1;
                }
            }
        }
        if decisions.memory != Decision::Skip {
            let existing = dest_memory_names(dest_path);
            for doc in &l.memory {
                if existing.contains(&doc.name) && decisions.memory == Decision::Merge {
                    continue;
                }
                if memory_service::write(dest_path, &doc.name, &doc.content).is_ok() {
                    result.memory += 1;
                }
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn session(id: &str) -> PersistedSession {
        PersistedSession {
            id: id.into(),
            name: format!("name-{id}"),
            cwd: "/home/alice/secret-project".into(),
            created_at_ms: 1,
            scrollback: "the work".into(),
            agent: Some("claude".into()),
            claude_session_id: Some("resume-handle-xyz".into()),
            name_suggested: Some(true),
            model: Some("opus".into()),
        }
    }

    /// scrub_session removes the source machine's path and the local resume
    /// handle, but preserves the work (scrollback) and portable prefs (model).
    #[test]
    fn scrub_session_drops_machine_bound_fields() {
        let s = scrub_session(session("s1"));
        assert_eq!(s.cwd, "", "cwd (a source-machine path) must not travel");
        assert!(
            s.claude_session_id.is_none(),
            "local Claude resume id must not travel"
        );
        assert!(s.name_suggested.is_none());
        // Work + portable prefs survive.
        assert_eq!(s.id, "s1");
        assert_eq!(s.scrollback, "the work");
        assert_eq!(s.model.as_deref(), Some("opus"));
    }

    /// scrub_job lands the job disabled with all runtime/backoff state cleared,
    /// so an imported job never silently fires on the recipient's machine.
    #[test]
    fn scrub_job_disables_and_resets_runtime() {
        use crate::services::scheduler_service::{Action, Job, OnMissed, Trigger};
        let j = Job {
            id: "j1".into(),
            name: "nightly".into(),
            enabled: true,
            trigger: Trigger::Interval { every_ms: 1000 },
            action: Action::Prompt { text: "hi".into() },
            on_missed: OnMissed::Catchup,
            cooldown_ms: 5,
            created_at_ms: 42,
            last_run_ms: Some(100),
            next_due_ms: Some(200),
            consecutive_failures: 3,
            backoff_until_ms: Some(300),
        };
        let j = scrub_job(j);
        assert!(!j.enabled, "imported jobs must arrive disabled");
        assert!(j.last_run_ms.is_none());
        assert!(j.next_due_ms.is_none());
        assert_eq!(j.consecutive_failures, 0);
        assert!(j.backoff_until_ms.is_none());
        // Identity + behaviour preserved.
        assert_eq!(j.id, "j1");
        assert_eq!(j.cooldown_ms, 5);
        assert_eq!(j.action, Action::Prompt { text: "hi".into() });
    }

    /// scrub_room keeps the transcript/tokens but drops the live resume refs and
    /// per-engine cursors that only mean something in the exporting session.
    #[test]
    fn scrub_room_drops_resume_state() {
        let mut resume = HashMap::new();
        resume.insert("claude".to_string(), "sess-1".to_string());
        let mut last_seen = HashMap::new();
        last_seen.insert("claude".to_string(), 5usize);
        let room = PersistedRoom {
            version: 1,
            id: "r1".into(),
            problem: "debate".into(),
            participants: Vec::new(),
            transcript: Vec::new(),
            resume,
            last_seen,
            total_tokens: 999,
            updated_at_ms: 7,
        };
        let r = scrub_room(room);
        assert!(r.resume.is_empty(), "engine resume refs must not travel");
        assert!(r.last_seen.is_empty());
        assert_eq!(r.total_tokens, 999, "the work (tokens/transcript) survives");
        assert_eq!(r.id, "r1");
    }

    /// A built archive round-trips through JSON, carries the format/version
    /// envelope, and only includes the blocks that were requested (an un-chosen
    /// block stays `None`, distinct from a chosen-but-empty block).
    #[test]
    fn archive_json_roundtrips_and_omits_unchosen_blocks() {
        let archive = WorkspaceArchive {
            format: ARCHIVE_FORMAT.to_string(),
            version: ARCHIVE_VERSION,
            created_at_ms: 123,
            source_project_name: "agent-console".to_string(),
            blocks: ArchiveBlocks {
                sessions: Some(vec![scrub_session(session("s1"))]),
                rooms: None,
                schedules: None,
                learning: None,
            },
        };
        let json = to_json(&archive).unwrap();
        // Source path must never appear anywhere in the serialized file.
        assert!(
            !json.contains("/home/alice"),
            "archive must not leak the source machine path"
        );
        assert!(!json.contains("resume-handle-xyz"));

        let back: WorkspaceArchive = serde_json::from_str(&json).unwrap();
        assert_eq!(back.format, ARCHIVE_FORMAT);
        assert_eq!(back.version, ARCHIVE_VERSION);
        assert_eq!(back.blocks.sessions.unwrap().len(), 1);
        assert!(
            back.blocks.rooms.is_none(),
            "an un-chosen block stays absent on round-trip"
        );
    }

    /// read_memory returns the entry files but excludes the regenerable MEMORY.md
    /// index and non-markdown files. Exercises the real path resolver against an
    /// isolated HOME.
    #[test]
    fn read_memory_excludes_index_and_non_md() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home =
            std::env::temp_dir().join(format!("ac-wsa-mem-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);

        // The project the memory belongs to (path → slug is what the resolver uses).
        let project = home.join("proj");
        std::fs::create_dir_all(&project).unwrap();
        let mem_dir = context_service::memory_dir_for(&project).unwrap();
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(mem_dir.join("MEMORY.md"), "# index").unwrap();
        std::fs::write(mem_dir.join("feature-a.md"), "fact A").unwrap();
        std::fs::write(mem_dir.join("notes.txt"), "ignore me").unwrap();

        let docs = read_memory(&project);
        let names: Vec<_> = docs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["feature-a.md"], "only entry .md files, no index");
        assert_eq!(docs[0].content, "fact A");

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn parse_archive_rejects_foreign_and_future_files() {
        // Not our format.
        assert!(parse_archive(r#"{"format":"something-else","version":1,"createdAtMs":0,"sourceProjectName":"x","blocks":{}}"#).is_err());
        // A version from the future we can't safely read.
        let future = format!(
            r#"{{"format":"{ARCHIVE_FORMAT}","version":{},"createdAtMs":0,"sourceProjectName":"x","blocks":{{}}}}"#,
            ARCHIVE_VERSION + 1
        );
        assert!(parse_archive(&future).is_err());
        // Plain garbage.
        assert!(parse_archive("not json").is_err());
        // A well-formed current archive parses.
        let ok = format!(
            r#"{{"format":"{ARCHIVE_FORMAT}","version":{ARCHIVE_VERSION},"createdAtMs":0,"sourceProjectName":"x","blocks":{{}}}}"#
        );
        assert!(parse_archive(&ok).is_ok());
    }

    /// End-to-end: export from a source project, import into a *different*
    /// destination, and confirm everything is re-keyed to the destination — and
    /// that a second import (Merge) is idempotent (no duplicates).
    #[test]
    fn export_import_roundtrip_rekeys_to_destination() {
        let _env = crate::test_support::lock_env();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-wsa-rt-{}-{}", std::process::id(), nanos));
        let home = base.join("home");
        let data = base.join("data");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&data).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &data);

        let src = home.join("src-proj");
        let dest = home.join("dest-proj");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dest).unwrap();
        let src_s = src.to_string_lossy().to_string();
        let dest_s = dest.to_string_lossy().to_string();

        let state = AppState::default();

        // Seed source: a session, a project skill, a memory entry.
        state
            .sessions
            .save(&src_s, vec![session("s1"), session("s2")])
            .unwrap();
        skills_service::write(&src, "demo-skill", "---\nname: demo-skill\n---\n\nbody").unwrap();
        memory_service::write(&src, "fact-a.md", "remembered").unwrap();

        // Export sessions + learning.
        let opts = ExportOptions {
            sessions: true,
            rooms: false,
            schedules: false,
            learning: true,
            include_activity: false,
        };
        let archive = build_archive(&state, &src_s, opts).unwrap();
        assert_eq!(archive.blocks.sessions.as_ref().unwrap().len(), 2);
        assert_eq!(archive.blocks.learning.as_ref().unwrap().skills.len(), 1);

        // Import into the (empty) destination, taking everything.
        let decide = ImportDecisions {
            sessions: Decision::Merge,
            rooms: Decision::Skip,
            schedules: Decision::Skip,
            skills: Decision::Merge,
            memory: Decision::Merge,
        };
        let r = apply_archive(&state, &dest_s, &archive, decide).unwrap();
        assert_eq!(r.sessions, 2);
        assert_eq!(r.skills, 1);
        assert_eq!(r.memory, 1);

        // Sessions landed under the destination key, re-keyed to its path.
        let dest_sessions = state.sessions.list(&dest_s).unwrap();
        assert_eq!(dest_sessions.len(), 2);
        assert!(
            dest_sessions.iter().all(|s| s.cwd == dest_s),
            "imported sessions must point at the destination project"
        );
        // Source is untouched.
        assert_eq!(state.sessions.list(&src_s).unwrap().len(), 2);
        // Skill + memory materialized under the destination.
        assert!(dest.join(".claude/skills/demo-skill/SKILL.md").exists());
        assert!(dest_memory_names(&dest).contains("fact-a.md"));

        // Idempotency: a second Merge import adds nothing.
        let r2 = apply_archive(&state, &dest_s, &archive, decide).unwrap();
        assert_eq!(r2.sessions, 0, "merge re-import must not duplicate sessions");
        assert_eq!(state.sessions.list(&dest_s).unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(&base);
    }
}

//! Semantic search over the project's memories and skills (local embeddings).

use std::fs;
use std::path::Path;

use crate::error::AppResult;
use crate::services::context_service::memory_dir_for;
use crate::services::embedding_service::CandleEmbedder;
use crate::services::semantic_index::{self, ReindexReport, SearchHit, SourceDoc};
use crate::services::{memory_service, skills_service};

/// Embedding input cap: e5-small attends ~512 tokens anyway, and memories can
/// carry long tails — the head of a document is where its identity lives.
const MAX_DOC_CHARS: usize = 2000;

fn clip(text: &str) -> String {
    if text.chars().count() <= MAX_DOC_CHARS {
        text.to_string()
    } else {
        text.chars().take(MAX_DOC_CHARS).collect()
    }
}

/// Gather everything indexable for a project: memory .md files (minus the
/// index) and skill/command/agent definitions, project- and user-level.
fn gather_sources(project_root: &str) -> AppResult<Vec<SourceDoc>> {
    let root = Path::new(project_root);
    let mut out: Vec<SourceDoc> = Vec::new();

    let mem_dir = memory_dir_for(root)?;
    for m in memory_service::list(root)? {
        if m.is_index {
            continue;
        }
        let Ok(text) = fs::read_to_string(mem_dir.join(&m.name)) else {
            continue;
        };
        out.push(SourceDoc {
            id: format!("memory:{}", m.name),
            kind: "memory".into(),
            title: m
                .description
                .clone()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| m.name.clone()),
            text: clip(&text),
        });
    }

    for sk in skills_service::list(Some(root))? {
        let Ok(text) = fs::read_to_string(&sk.path) else {
            continue;
        };
        out.push(SourceDoc {
            id: format!("skill:{}:{}:{}", sk.kind, sk.source, sk.name),
            kind: "skill".into(),
            title: match &sk.description {
                Some(d) if !d.is_empty() => format!("{} — {}", sk.name, d),
                _ => sk.name.clone(),
            },
            text: clip(&text),
        });
    }

    Ok(out)
}

/// Rebuild the semantic index (incremental — unchanged docs are not
/// re-embedded). First run initializes/downloads the local embedding model.
/// Sync command on purpose: it runs on tauri's command thread pool, and the
/// model init is blocking work.
#[tauri::command]
pub fn semantic_reindex(project_root: String) -> AppResult<ReindexReport> {
    let sources = gather_sources(&project_root)?;
    semantic_index::reindex(&project_root, &sources, &mut CandleEmbedder::new())
}

/// Top-k semantic search. Auto-builds the index when missing so the first
/// search "just works" (at the cost of that first model download).
#[tauri::command]
pub fn semantic_search(
    project_root: String,
    query: String,
    k: Option<usize>,
) -> AppResult<Vec<SearchHit>> {
    let k = k.unwrap_or(8).clamp(1, 50);
    let hits = semantic_index::search(&project_root, &query, k, &mut CandleEmbedder::new())?;
    if !hits.is_empty() {
        return Ok(hits);
    }
    // Empty could mean "no index yet" — build once and retry.
    let sources = gather_sources(&project_root)?;
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    semantic_index::reindex(&project_root, &sources, &mut CandleEmbedder::new())?;
    semantic_index::search(&project_root, &query, k, &mut CandleEmbedder::new())
}

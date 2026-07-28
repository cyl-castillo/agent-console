//! Semantic search over the project's memories and skills (local embeddings).

use crate::error::AppResult;
use crate::services::embedding_service::CandleEmbedder;
use crate::services::semantic_index::{self, ReindexReport, SearchHit};

/// Rebuild the semantic index (incremental — unchanged docs are not
/// re-embedded). First run initializes/downloads the local embedding model.
/// Sync command on purpose: it runs on tauri's command thread pool, and the
/// model init is blocking work.
#[tauri::command]
pub fn semantic_reindex(project_root: String) -> AppResult<ReindexReport> {
    semantic_index::ensure_fresh(&project_root, &mut CandleEmbedder::new())
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
    let report = semantic_index::ensure_fresh(&project_root, &mut CandleEmbedder::new())?;
    if report.total == 0 {
        return Ok(Vec::new());
    }
    semantic_index::search(&project_root, &query, k, &mut CandleEmbedder::new())
}

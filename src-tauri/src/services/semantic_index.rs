//! Per-project semantic index over memories and skills.
//!
//! Deliberately NOT a vector database: the corpus is hundreds of items, so an
//! exact brute-force cosine over a JSON-persisted matrix beats any ANN setup —
//! zero services, zero extra deps, exact results. If a corpus ever grows past
//! ~50k items this file's API stays and the backend swaps.
//!
//! Incremental: entries are keyed by source id and content hash — reindex only
//! embeds new/changed texts and drops removed sources. A scheme bump (model
//! change) invalidates everything at once.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::services::embedding_service::{cosine, Embedder, EMBEDDING_SCHEME};
use crate::services::persistence::project_file_key;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexEntry {
    /// Stable source id, e.g. "memory:distribution-channels.md" or
    /// "skill:project:release-flow".
    pub id: String,
    /// "memory" | "skill"
    pub kind: String,
    pub title: String,
    /// Short plain-text preview for result lists (never the full content).
    pub snippet: String,
    /// Hash of the embedded text — the incremental-reindex currency.
    pub content_hash: u64,
    pub vector: Vec<f32>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexFile {
    /// Embedding scheme these vectors belong to; mismatch = full re-embed.
    #[serde(default)]
    scheme: String,
    #[serde(default)]
    entries: Vec<IndexEntry>,
}

/// One indexable source document, gathered by the caller.
#[derive(Debug, Clone)]
pub struct SourceDoc {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReindexReport {
    pub indexed: usize,
    pub reused: usize,
    pub removed: usize,
    pub total: usize,
}

fn hash_text(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn snippet_of(text: &str) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut cut: String = clean.chars().take(180).collect();
    if clean.chars().count() > 180 {
        cut.push('…');
    }
    cut
}

fn index_dir() -> AppResult<PathBuf> {
    let dir = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("cannot resolve data dir".into()))?
        .join("agent-console")
        .join("semantic-index");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn index_path(project_root: &str) -> AppResult<PathBuf> {
    Ok(index_dir()?.join(format!("{}.json", project_file_key(project_root))))
}

fn load(path: &Path) -> IndexFile {
    let Ok(raw) = fs::read_to_string(path) else {
        return IndexFile::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save(path: &Path, file: &IndexFile) -> AppResult<()> {
    let raw = serde_json::to_string(file).map_err(|e| AppError::Other(e.to_string()))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Reconcile the index with `sources`: embed new/changed docs, keep unchanged
/// vectors, drop entries whose source disappeared. The embedder is only called
/// for texts that actually need it.
pub fn reindex(
    project_root: &str,
    sources: &[SourceDoc],
    embedder: &mut dyn Embedder,
) -> AppResult<ReindexReport> {
    let path = index_path(project_root)?;
    let mut existing = load(&path);
    if existing.scheme != EMBEDDING_SCHEME {
        existing.entries.clear();
    }
    let old: HashMap<String, IndexEntry> = existing
        .entries
        .into_iter()
        .map(|e| (e.id.clone(), e))
        .collect();

    let mut entries: Vec<IndexEntry> = Vec::with_capacity(sources.len());
    let mut to_embed: Vec<(usize, &SourceDoc, u64)> = Vec::new();
    let mut reused = 0usize;

    for doc in sources {
        let h = hash_text(&doc.text);
        match old.get(&doc.id) {
            Some(prev) if prev.content_hash == h => {
                reused += 1;
                entries.push(IndexEntry {
                    title: doc.title.clone(),
                    snippet: snippet_of(&doc.text),
                    ..prev.clone()
                });
            }
            _ => {
                // Placeholder keeps source order; vector filled below.
                entries.push(IndexEntry {
                    id: doc.id.clone(),
                    kind: doc.kind.clone(),
                    title: doc.title.clone(),
                    snippet: snippet_of(&doc.text),
                    content_hash: h,
                    vector: Vec::new(),
                });
                to_embed.push((entries.len() - 1, doc, h));
            }
        }
    }

    if !to_embed.is_empty() {
        let texts: Vec<String> = to_embed.iter().map(|(_, d, _)| d.text.clone()).collect();
        let vectors = embedder.embed_passages(&texts)?;
        if vectors.len() != to_embed.len() {
            return Err(AppError::Other(
                "embedder returned a mismatched vector count".into(),
            ));
        }
        for ((idx, _, _), v) in to_embed.iter().zip(vectors) {
            entries[*idx].vector = v;
        }
    }

    let indexed = to_embed.len();
    let removed = old.len().saturating_sub(reused);
    let total = entries.len();
    save(
        &path,
        &IndexFile {
            scheme: EMBEDDING_SCHEME.to_string(),
            entries,
        },
    )?;
    Ok(ReindexReport {
        indexed,
        reused,
        removed,
        total,
    })
}

/// Top-k cosine search over the stored index. Returns an empty list (not an
/// error) when no index exists yet.
pub fn search(
    project_root: &str,
    query: &str,
    k: usize,
    embedder: &mut dyn Embedder,
) -> AppResult<Vec<SearchHit>> {
    let path = index_path(project_root)?;
    let file = load(&path);
    if file.entries.is_empty() || file.scheme != EMBEDDING_SCHEME {
        return Ok(Vec::new());
    }
    let qv = embedder.embed_query(query)?;
    let mut hits: Vec<SearchHit> = file
        .entries
        .iter()
        .map(|e| SearchHit {
            id: e.id.clone(),
            kind: e.kind.clone(),
            title: e.title.clone(),
            snippet: e.snippet.clone(),
            score: cosine(&qv, &e.vector),
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(k);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic fake: maps each text to a small vector from its bytes, so
    /// identical texts collide and different texts (almost surely) don't.
    /// Counts calls to prove incremental reindexing skips unchanged docs.
    struct FakeEmbedder {
        calls: usize,
    }

    impl Embedder for FakeEmbedder {
        fn embed_passages(&mut self, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
            self.calls += texts.len();
            Ok(texts.iter().map(|t| fake_vec(t)).collect())
        }
        fn embed_query(&mut self, text: &str) -> AppResult<Vec<f32>> {
            Ok(fake_vec(text))
        }
    }

    fn fake_vec(text: &str) -> Vec<f32> {
        let h = hash_text(text);
        (0..8)
            .map(|i| (((h >> (i * 8)) & 0xff) as f32) - 128.0)
            .collect()
    }

    fn doc(id: &str, text: &str) -> SourceDoc {
        SourceDoc {
            id: id.into(),
            kind: "memory".into(),
            title: id.into(),
            text: text.into(),
        }
    }

    /// One test fn: exercises the whole lifecycle against a real temp index
    /// dir (serialized via the shared env lock — index_dir uses the global
    /// data dir).
    #[test]
    fn reindex_is_incremental_and_search_ranks_by_similarity() {
        let _env = crate::test_support::lock_env();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let data = std::env::temp_dir().join(format!("ac-semidx-{}-{nanos}", std::process::id()));
        let prev = std::env::var("XDG_DATA_HOME").ok();
        std::env::set_var("XDG_DATA_HOME", &data);
        let root = format!("/proj/semantic-{nanos}");

        let run = || -> AppResult<()> {
            let mut emb = FakeEmbedder { calls: 0 };

            // Fresh build embeds everything.
            let sources = vec![doc("a", "alpha text"), doc("b", "beta text")];
            let r = reindex(&root, &sources, &mut emb)?;
            assert_eq!((r.indexed, r.reused, r.removed, r.total), (2, 0, 0, 2));
            assert_eq!(emb.calls, 2);

            // Unchanged sources: zero embedder calls, everything reused.
            let r = reindex(&root, &sources, &mut emb)?;
            assert_eq!((r.indexed, r.reused, r.removed), (0, 2, 0));
            assert_eq!(emb.calls, 2, "unchanged docs must not re-embed");

            // One edit + one removal + one addition.
            let sources = vec![doc("a", "alpha text EDITED"), doc("c", "gamma text")];
            let r = reindex(&root, &sources, &mut emb)?;
            assert_eq!((r.indexed, r.reused, r.removed, r.total), (2, 0, 2, 2));

            // Search: the exact text is the (near-certain) top hit; k caps.
            let hits = search(&root, "alpha text EDITED", 10, &mut emb)?;
            assert_eq!(hits.len(), 2);
            assert_eq!(hits[0].id, "a");
            assert!(hits[0].score > hits[1].score);
            let hits = search(&root, "anything", 1, &mut emb)?;
            assert_eq!(hits.len(), 1);

            // Unknown project: empty, not an error.
            assert!(search("/proj/none", "q", 5, &mut emb)?.is_empty());
            Ok(())
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));

        match prev {
            Some(v) => std::env::set_var("XDG_DATA_HOME", v),
            None => std::env::remove_var("XDG_DATA_HOME"),
        }
        fs::remove_dir_all(&data).ok();
        match result {
            Ok(inner) => inner.unwrap(),
            Err(p) => std::panic::resume_unwind(p),
        }
    }
}
